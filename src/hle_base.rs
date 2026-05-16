use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Once;

use crate::arena::{ArenaAllocOptions, ArenaAllocation, GuestArena};
use crate::cpu::Reg;
use crate::guest_path::GuestPath;
use crate::memory::{align_down, align_up, Memory, PagePerm, WriteContext, PAGE_SIZE};
use crate::pe::PeImage;
use crate::{write_registers, Emulator, Error, Result, StopReason};

const MICROSECONDS_PER_MILLISECOND: u64 = 1_000;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const HLE_BASE: u32 = 0x7000_0000;
const HLE_STRIDE: u32 = 0x10;
const MODULE_BASE: u32 = 0x0010_0000;
// Keep guest-visible mapped arenas inside the 256 MiB identity window. HLE
// thunks and Win32 handles can stay high because they are never memory-mapped.
const MODULE_SIZE: u32 = 0x03f0_0000;
const HEAP_BASE: u32 = 0x0400_0000;
const HOOK_HANDLE_BASE: u32 = 0x4100_0000;
// Bitmap-heavy Win32 games can keep many GDI/DirectDraw-backed pixel buffers
// alive through GlobalAlloc-era APIs. Leave the tracked VirtualAlloc arena
// separate, but start it higher so the ordinary HLE heap has room for them.
const VIRTUAL_BASE: u32 = 0x0c00_0000;
const GDI_HANDLE_BASE: u32 = 0x4000_0000;
// Emulator-owned COM/vtable/static pages live outside the guest data heap.
const HLE_PRIVATE_BASE: u32 = 0x0e00_0000;
const VIRTUAL_HIGH_BASE: u32 = 0x0f00_0000;
const VIRTUAL_HIGH_SIZE: u32 = 0x0120_0000;
const HEAP_SIZE: u32 = VIRTUAL_BASE - HEAP_BASE;
const VIRTUAL_SIZE: u32 = HLE_PRIVATE_BASE - VIRTUAL_BASE;
const HLE_PRIVATE_SIZE: u32 = 0x0100_0000;
const ALLOCATION_GRANULARITY: u32 = 0x0001_0000;
const HLE_ALLOC_GUARD_SIZE: u32 = PAGE_SIZE;
const INVALID_HANDLE_VALUE: u32 = 0xffff_ffff;
const HLE_PROCESS_ID: u32 = 1;
const HLE_THREAD_ID: u32 = 1;
const MESSAGE_FLOOD_DISPATCH_LIMIT: u32 = 64;
const MESSAGE_TYPE_HISTORY_LEN: usize = 10;

pub const HLE_TRACE_FS: u32 = 1 << 0;
pub const HLE_TRACE_ALLOC: u32 = 1 << 1;
pub const HLE_TRACE_DDRAW: u32 = 1 << 2;
pub const HLE_TRACE_GDI: u32 = 1 << 3;

static HLE_TRACE_FLAGS: AtomicU32 = AtomicU32::new(0);
static HLE_TRACE_INIT: Once = Once::new();

pub fn hle_trace_flags() -> u32 {
    HLE_TRACE_FLAGS.load(Ordering::Relaxed)
}

pub fn set_hle_trace_flags(flags: u32) {
    HLE_TRACE_FLAGS.store(flags, Ordering::Relaxed);
}

#[inline(always)]
fn hle_trace_enabled(flag: u32) -> bool {
    (HLE_TRACE_FLAGS.load(Ordering::Relaxed) & flag) != 0
}

fn init_hle_trace_flags_from_env() {
    HLE_TRACE_INIT.call_once(|| {
        HLE_TRACE_FLAGS.store(default_hle_trace_flags(), Ordering::Relaxed);
    });
}

fn default_hle_trace_flags() -> u32 {
    let mut flags = 0;
    if trace_env_enabled("WEMU_TRACE_FS") {
        flags |= HLE_TRACE_FS;
    }
    if trace_env_enabled("WEMU_ALLOC_TRACE") {
        flags |= HLE_TRACE_ALLOC;
    }
    if trace_env_enabled("WEMU_DDRAW_TRACE") {
        flags |= HLE_TRACE_DDRAW;
    }
    if trace_env_enabled("WEMU_GDI_TRACE") {
        flags |= HLE_TRACE_GDI;
    }
    flags
}

fn trace_env_enabled(name: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var(name)
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(false)
    }
}

macro_rules! trace_fs {
    ($($arg:tt)*) => {{
        if hle_trace_enabled(HLE_TRACE_FS) {
            log_create_file(format!($($arg)*));
        }
    }};
}

macro_rules! trace_alloc {
    ($($arg:tt)*) => {{
        if hle_trace_enabled(HLE_TRACE_ALLOC) {
            log_create_file(format!($($arg)*));
        }
    }};
}

macro_rules! trace_ddraw {
    ($($arg:tt)*) => {{
        if hle_trace_enabled(HLE_TRACE_DDRAW) {
            log_create_file(format!($($arg)*));
        }
    }};
}

macro_rules! trace_gdi {
    ($($arg:tt)*) => {{
        if hle_trace_enabled(HLE_TRACE_GDI) {
            log_create_file(format!($($arg)*));
        }
    }};
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HleWaitState {
    Ready,
    Message {
        out: u32,
        filter: MessageFilter,
    },
    VfsRead {
        request_id: u32,
        buf: u32,
        read_out: u32,
        ret_transferred: bool,
        ret_item_size: u32,
        fail_value: u32,
        arg_bytes: u32,
    },
    VfsWrite {
        request_id: u32,
        written_out: u32,
        ret_transferred: bool,
        ret_item_size: u32,
        fail_value: u32,
        arg_bytes: u32,
    },
    Timeout {
        until_ms: u64,
        not_before_frame: u64,
        ret_value: u32,
        arg_bytes: u32,
    },
}

impl HleWaitState {
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageFilter {
    pub hwnd: u32,
    pub min: u32,
    pub max: u32,
}

impl MessageFilter {
    pub const fn new(hwnd: u32, min: u32, max: u32) -> Self {
        Self { hwnd, min, max }
    }

    pub const fn any() -> Self {
        Self {
            hwnd: 0,
            min: 0,
            max: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HleResult {
    Retn(u32),
    Wait(HleWaitState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HleDelayTarget {
    pub delay_ms: u64,
    pub frame_count: u64,
}

impl HleDelayTarget {
    fn millisecond(delay_ms: u64) -> Self {
        let delay_ms = delay_ms.max(1);
        Self {
            delay_ms,
            frame_count: 0,
        }
    }

    pub fn until_ms(self, now_ms: u64) -> u64 {
        now_ms.saturating_add(self.delay_ms)
    }

    pub fn eligible_frame(self, scheduler_frame: u64) -> u64 {
        if scheduler_frame == 0 || self.frame_count == 0 {
            0
        } else {
            scheduler_frame.saturating_add(self.frame_count)
        }
    }
}

fn rounded_live_frame_count(delay_ms: u32, frame_us: u64) -> u64 {
    debug_assert!(delay_ms != 0);
    debug_assert!(frame_us != 0);
    let delay_us = (delay_ms as u128).saturating_mul(MICROSECONDS_PER_MILLISECOND as u128);
    let frame_us = frame_us as u128;
    let divisor = frame_us.saturating_mul(2);
    delay_us
        .saturating_mul(2)
        .saturating_add(frame_us)
        .saturating_div(divisor)
        .max(1)
        .min(u64::MAX as u128) as u64
}

fn next_period_frame(scheduler_frame: u64, period_frames: u64) -> u64 {
    if scheduler_frame == 0 || period_frames == 0 {
        0
    } else {
        scheduler_frame.saturating_add(period_frames)
    }
}

type HleCallback = fn(&mut Emulator, &HleEntry) -> HleResult;

#[derive(Clone, Copy)]
pub struct HleEntry {
    pub addr: u32,
    pub dll: &'static str,
    pub name: &'static str,
    pub callback: HleCallback,
}

trait HleMust<T> {
    fn hle(self) -> T;
}

impl<T, E: std::fmt::Display> HleMust<T> for std::result::Result<T, E> {
    fn hle(self) -> T {
        self.unwrap_or_else(|err| panic!("HLE helper failed: {err}"))
    }
}

pub struct Hle {
    next_thunk: u32,
    entries: Vec<HleEntry>,
    entries_by_name: HashMap<String, u32>,
    unresolved_hle_symbols: Vec<(String, String)>,
    missing_hle_reported: bool,
    missing_hle_report: String,
    strict_hle_imports: bool,
    handles: Vec<Option<Handle>>,
    vfs: Vfs,
    named_kernel_objects: HashMap<String, NamedKernelObject>,
    modules: HashMap<String, u32>,
    module_images: HashMap<u32, PeImage>,
    next_module: u32,
    module_arena: GuestArena,
    heap_arena: GuestArena,
    private_arena: GuestArena,
    private_on_guest_heap: bool,
    next_gdi_handle: u32,
    gdi_fonts: HashMap<u32, GdiFont>,
    gdi_bitmaps: HashMap<u32, GdiBitmap>,
    gdi_brushes: HashMap<u32, GdiBrush>,
    gdi_pens: HashMap<u32, GdiPen>,
    gdi_palettes: HashMap<u32, GdiPalette>,
    gdi_regions: HashMap<u32, WindowRect>,
    gdi_dcs: HashMap<u32, GdiDc>,
    gdi_dc_saves: HashMap<u32, Vec<GdiDc>>,
    windows: HashMap<u32, HleWindow>,
    menus: HashMap<u32, HleMenu>,
    accelerators: HashMap<u32, HleAcceleratorTable>,
    scroll_states: HashMap<(u32, u32), HleScrollState>,
    next_menu_handle: u32,
    next_accelerator_handle: u32,
    active_popup_menu: Option<HlePopupMenu>,
    hle_windows_dirty: bool,
    window_class_procs: HashMap<String, u32>,
    window_class_atoms: HashMap<u32, u32>,
    window_class_menus: HashMap<String, u32>,
    window_class_atom_menus: HashMap<u32, u32>,
    window_class_backgrounds: HashMap<String, u32>,
    window_class_atom_backgrounds: HashMap<u32, u32>,
    next_window_class_atom: u32,
    registered_window_messages: HashMap<String, u32>,
    next_registered_window_message: u32,
    next_hook_handle: u32,
    hooks: Vec<Hook>,
    virtual_arena: GuestArena,
    virtual_high_arena: GuestArena,
    pub last_error: u32,
    wsa_last_error: u32,
    command_line_a: u32,
    command_line_w: u32,
    environment_a: u32,
    tls_slots: Vec<u32>,
    input_messages: Vec<Message>,
    app_messages: Vec<Message>,
    last_queued_message: Option<MessageEvent>,
    last_peek_message: Option<PeekEvent>,
    last_removed_message: Option<MessageEvent>,
    last_dispatched_message: Option<DispatchEvent>,
    message_flood_msg: u32,
    message_flood_proc: u32,
    message_flood_count: u32,
    message_flood_warned: bool,
    recent_message_types: [u32; MESSAGE_TYPE_HISTORY_LEN],
    recent_message_type_pos: u32,
    paint_frame_token: u64,
    cooperative_idle: bool,
    frontend_fps: u32,
    frontend_microseconds_per_frame: u64,
    timers: Vec<Timer>,
    next_timer_id: u32,
    mm_timers: Vec<MmTimer>,
    next_mm_timer_id: u32,
    async_return_thunk: u32,
    button_wndproc_thunk: u32,
    // Guest callbacks entered from HLE return to wemu!__async_return, not to the
    // original Win32 API implementation. Non-tail APIs such as CreateWindowEx
    // need this real HLE callback stack: USER enters a CBT hook, resumes HLE,
    // then enters the window proc before finally returning to the caller.
    hle_callback_stack: Vec<HleCallbackFrame>,
    mm_timer_callback_active: bool,
    window_proc: u32,
    focus_window: u32,
    capture_window: u32,
    key_down: [bool; 256],
    key_pressed: [u8; 256],
    cursor_x: u32,
    cursor_y: u32,
    next_window_handle: u32,
    gdi_screen_surface: u32,
    gdi_present_pending: bool,
    ddraw_vtable: u32,
    ddraw_surface_vtable: u32,
    ddraw_palette_vtable: u32,
    ddraw_clipper_vtable: u32,
    ddraw_enum_continue_thunk: u32,
    ddraw_enum_modes: Option<DDrawEnumModesState>,
    ddraw_width: u32,
    ddraw_height: u32,
    ddraw_bpp: u32,
    dinput_vtable: u32,
    dinput_device_vtable: u32,
    dsound_vtable: u32,
    dsound_buffer_vtable: u32,
    crt_iob: u32,
    crt_initenv: u32,
    crt_winitenv: u32,
    crt_acmdln: u32,
    crt_wcmdln: u32,
    crt_adjust_fdiv: u32,
    crt_fmode: u32,
    crt_commode: u32,
    crt_output: String,
    error_mode: u32,
    fake_time: u32,
    rand_seed: u32,
    module_file_name: String,
    winsock_inet_ntoa_buffer: u32,
}

struct DDrawEnumModesState {
    callback: u32,
    context: u32,
    original_ret: u32,
    callback_esp: u32,
    final_esp: u32,
    modes: Vec<(u32, u32, u32)>,
    next_mode: usize,
}

enum Handle {
    File(FileHandle),
    FileMapping {
        data: Rc<RefCell<Vec<u8>>>,
    },
    Find {
        entries: Vec<FindEntry>,
        index: usize,
    },
    Printer {
        name: String,
    },
    Socket,
    Process,
    Event,
    Mutex,
    Semaphore,
    Thread,
}

enum FileBackend {
    Host(File),
    Memory(Rc<RefCell<Vec<u8>>>),
    Async,
}

struct FileHandle {
    key: String,
    backend: FileBackend,
    pos: u64,
    size: u64,
    writable: bool,
}

enum FileReadResult {
    Ready(std::io::Result<usize>),
    Pending { key: String, offset: u64, len: u32 },
}

enum FileWriteResult {
    Ready(std::io::Result<usize>),
    Pending {
        key: String,
        offset: u64,
        data: Vec<u8>,
    },
}

impl FileHandle {
    fn host(key: String, file: File, writable: bool) -> Self {
        let size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Self {
            key,
            backend: FileBackend::Host(file),
            pos: 0,
            size,
            writable,
        }
    }

    fn memory(key: String, data: Rc<RefCell<Vec<u8>>>, writable: bool) -> Self {
        let size = data.borrow().len() as u64;
        Self {
            key,
            backend: FileBackend::Memory(data),
            pos: 0,
            size,
            writable,
        }
    }

    fn async_file(key: String, size: u64, writable: bool) -> Self {
        Self {
            key,
            backend: FileBackend::Async,
            pos: 0,
            size,
            writable,
        }
    }

    fn read(&mut self, out: &mut [u8]) -> FileReadResult {
        match &mut self.backend {
            FileBackend::Host(file) => {
                let result = file.read(out);
                if let Ok(read) = result {
                    self.pos = self.pos.saturating_add(read as u64);
                }
                FileReadResult::Ready(result)
            }
            FileBackend::Memory(data) => {
                let data = data.borrow();
                let start = (self.pos as usize).min(data.len());
                let read = out.len().min(data.len().saturating_sub(start));
                out[..read].copy_from_slice(&data[start..start + read]);
                self.pos = self.pos.saturating_add(read as u64);
                FileReadResult::Ready(Ok(read))
            }
            FileBackend::Async => {
                let offset = self.pos;
                let read = out.len().min(self.size.saturating_sub(offset) as usize);
                self.pos = self.pos.saturating_add(read as u64);
                FileReadResult::Pending {
                    key: self.key.clone(),
                    offset,
                    len: read as u32,
                }
            }
        }
    }

    fn write(&mut self, data: Vec<u8>) -> FileWriteResult {
        if !self.writable {
            return FileWriteResult::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "file handle is not writable",
            )));
        }
        match &mut self.backend {
            FileBackend::Host(file) => {
                let result = file.write(&data);
                if let Ok(written) = result {
                    self.pos = self.pos.saturating_add(written as u64);
                    self.size = self.size.max(self.pos);
                }
                FileWriteResult::Ready(result)
            }
            FileBackend::Memory(file_data) => {
                let start = self.pos as usize;
                let end = start.saturating_add(data.len());
                let mut file_data = file_data.borrow_mut();
                if file_data.len() < end {
                    file_data.resize(end, 0);
                }
                file_data[start..end].copy_from_slice(&data);
                self.pos = end as u64;
                self.size = self.size.max(self.pos);
                FileWriteResult::Ready(Ok(data.len()))
            }
            FileBackend::Async => {
                let offset = self.pos;
                self.pos = self.pos.saturating_add(data.len() as u64);
                self.size = self.size.max(self.pos);
                FileWriteResult::Pending {
                    key: self.key.clone(),
                    offset,
                    data,
                }
            }
        }
    }

    fn size(&mut self) -> u64 {
        match &mut self.backend {
            FileBackend::Host(file) => file.metadata().map(|metadata| metadata.len()).unwrap_or(self.size),
            FileBackend::Memory(data) => data.borrow().len() as u64,
            FileBackend::Async => self.size,
        }
    }

    fn read_sync(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        match self.read(out) {
            FileReadResult::Ready(result) => result,
            FileReadResult::Pending { .. } => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "async file read requires an HLE wait continuation",
            )),
        }
    }

    fn write_sync(&mut self, data: &[u8]) -> std::io::Result<usize> {
        match self.write(data.to_vec()) {
            FileWriteResult::Ready(result) => result,
            FileWriteResult::Pending { .. } => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "async file write requires an HLE wait continuation",
            )),
        }
    }

    fn current_pos(&mut self) -> u64 {
        if let FileBackend::Host(file) = &mut self.backend {
            self.pos = file.stream_position().unwrap_or(self.pos);
        }
        self.pos
    }

    fn read_exact_at_sync(&mut self, pos: u64, out: &mut [u8]) -> std::io::Result<()> {
        let old = self.current_pos();
        self.seek(pos as i64, 0)?;
        let mut done = 0;
        while done < out.len() {
            let read = self.read_sync(&mut out[done..])?;
            if read == 0 {
                let _ = self.seek(old as i64, 0);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "short file read",
                ));
            }
            done += read;
        }
        self.seek(old as i64, 0)?;
        Ok(())
    }

    fn seek(&mut self, dist: i64, method: u32) -> std::io::Result<u64> {
        match &mut self.backend {
            FileBackend::Host(file) => {
                let seek = match method {
                    0 => SeekFrom::Start(dist.max(0) as u64),
                    1 => SeekFrom::Current(dist),
                    2 => SeekFrom::End(dist),
                    _ => SeekFrom::Current(0),
                };
                let pos = file.seek(seek)?;
                self.pos = pos;
                Ok(pos)
            }
            FileBackend::Memory(data) => {
                let len = data.borrow().len() as i64;
                let next = match method {
                    0 => dist,
                    1 => self.pos as i64 + dist,
                    2 => len + dist,
                    _ => self.pos as i64,
                };
                self.pos = next.max(0) as u64;
                Ok(self.pos)
            }
            FileBackend::Async => {
                let len = self.size as i64;
                let next = match method {
                    0 => dist,
                    1 => self.pos as i64 + dist,
                    2 => len + dist,
                    _ => self.pos as i64,
                };
                self.pos = next.max(0) as u64;
                Ok(self.pos)
            }
        }
    }

    fn set_end(&mut self) -> std::io::Result<()> {
        match &mut self.backend {
            FileBackend::Host(file) => {
                let pos = file.stream_position()?;
                file.set_len(pos)?;
                self.size = pos;
                self.pos = pos;
                Ok(())
            }
            FileBackend::Memory(data) => {
                data.borrow_mut().resize(self.pos as usize, 0);
                self.size = self.pos;
                Ok(())
            }
            FileBackend::Async => {
                self.size = self.pos;
                Ok(())
            }
        }
    }

    fn mapping_bytes(&mut self) -> Vec<u8> {
        match &mut self.backend {
            FileBackend::Host(file) => {
                let old = file.stream_position().unwrap_or(0);
                let mut bytes = Vec::new();
                let _ = file.seek(SeekFrom::Start(0));
                let _ = file.read_to_end(&mut bytes);
                let _ = file.seek(SeekFrom::Start(old));
                bytes
            }
            FileBackend::Memory(data) => data.borrow().clone(),
            // Mapping an async file would require a non-tail async continuation.
            // Keep the handle shape unified, but fail closed until a caller needs it.
            FileBackend::Async => Vec::new(),
        }
    }
}

enum NamedKernelObject {
    Event,
    FileMapping(Rc<RefCell<Vec<u8>>>),
}

fn kernel_object_key(name: &str) -> Option<String> {
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Message {
    hwnd: u32,
    msg: u32,
    wparam: u32,
    lparam: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageQueueKind {
    Input,
    App,
}

#[derive(Clone, Copy)]
struct QueuedMessage {
    kind: MessageQueueKind,
    index: usize,
    message: Message,
}

fn message_id_matches(msg: u32, min: u32, max: u32) -> bool {
    (min == 0 && max == 0) || (msg >= min && msg <= max)
}

fn message_remove_on_pm_remove(message: Message) -> bool {
    const WM_PAINT: u32 = 0x000f;
    message.msg != WM_PAINT
}

#[derive(Clone, Copy)]
struct MessageEvent {
    source: &'static str,
    message: Message,
}

#[derive(Clone, Copy)]
struct PeekEvent {
    source: &'static str,
    remove: u32,
    message: Option<Message>,
}

#[derive(Clone, Copy)]
struct DispatchEvent {
    message: Message,
    proc: u32,
}

#[derive(Clone, Copy)]
struct MessageFloodReport {
    msg: u32,
    proc: u32,
    hwnd: u32,
    count: u32,
    input_len: u32,
    app_len: u32,
    recent: [u32; MESSAGE_TYPE_HISTORY_LEN],
}

#[derive(Clone, Copy)]
struct MouseTarget {
    hwnd: u32,
    client_x: i32,
    client_y: i32,
}

#[derive(Clone)]
struct HleWindow {
    hwnd: u32,
    parent: u32,
    id: u32,
    class_name: String,
    text: String,
    rect: WindowRect,
    style: u32,
    ex_style: u32,
    proc: u32,
    user_data: u32,
    extra: HashMap<i32, u32>,
    enabled: bool,
    visible: bool,
    control_kind: HleControlKind,
    background_brush: u32,
    invalid_rect: Option<WindowRect>,
    erase_pending: bool,
    last_generated_paint_frame: u64,
    ddraw_owned: bool,
}

#[derive(Clone)]
struct HleMenu {
    items: Vec<HleMenuItem>,
}

#[derive(Clone)]
struct HleMenuItem {
    id: u32,
    text: String,
    submenu: u32,
    separator: bool,
    enabled: bool,
    checked: bool,
}

#[derive(Clone)]
struct HleAcceleratorTable {
    items: Vec<HleAccelerator>,
}

#[derive(Clone, Copy)]
struct HleAccelerator {
    flags: u16,
    key: u16,
    cmd: u16,
}

#[derive(Clone)]
struct HlePopupMenu {
    owner: u32,
    rect: WindowRect,
    items: Vec<HlePopupMenuItem>,
}

#[derive(Clone)]
struct HlePopupMenuItem {
    id: u32,
    submenu: u32,
    text: String,
    rect: WindowRect,
    separator: bool,
    enabled: bool,
    checked: bool,
}

#[derive(Clone, Copy)]
struct HleMenuBarHit {
    hwnd: u32,
    id: u32,
    submenu: u32,
    rect: WindowRect,
    enabled: bool,
}

#[derive(Clone, Copy, Default)]
struct HleScrollState {
    min: i32,
    max: i32,
    pos: i32,
}

#[derive(Clone, Copy)]
struct WindowRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl WindowRect {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

fn region_result(rect: WindowRect) -> u32 {
    if rect.is_empty() { 1 } else { 2 }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HleControlKind {
    Window,
    Button,
    Static,
    Edit,
}

#[derive(Clone, Copy)]
struct Timer {
    hwnd: u32,
    id: u32,
    proc: u32,
    period_ms: u64,
    period_frames: u64,
    next_ms: u64,
    eligible_frame: u64,
    due_count: u64,
    post_count: u64,
}

#[derive(Clone, Copy)]
struct MmTimer {
    id: u32,
    callback: u32,
    user: u32,
    event: u32,
    period_ms: u64,
    period_frames: u64,
    next_ms: u64,
    eligible_frame: u64,
    due_count: u64,
    call_count: u64,
}

#[derive(Clone, Copy)]
struct MmTimerDue {
    id: u32,
    callback: u32,
    user: u32,
}

#[derive(Clone, Copy)]
struct CreateWindowContinuation {
    hwnd: u32,
    proc: u32,
    msg: u32,
    next_msg: Option<u32>,
    args: CreateWindowArgs,
}

#[derive(Clone, Copy)]
struct DialogInitContinuation {
    hwnd: u32,
    proc: u32,
    param: u32,
}

#[derive(Clone)]
struct WindowProcMessageChain {
    proc: u32,
    messages: Vec<Message>,
}

#[derive(Clone)]
enum HleCallbackContinuation {
    CreateWindow(CreateWindowContinuation),
    DialogInit(DialogInitContinuation),
    WindowProcMessageChain(WindowProcMessageChain),
    OwnerDrawButton {
        hdc: u32,
        draw_item: u32,
        chain: Option<OwnerDrawChain>,
    },
    AsyncCpu {
        cpu: crate::cpu::Cpu,
    },
}

#[derive(Clone)]
struct OwnerDrawChain {
    children: Vec<u32>,
    next_index: usize,
}

#[derive(Clone, Default)]
struct HleCallbackFrame {
    return_value: u32,
    continuation: Option<HleCallbackContinuation>,
}

#[derive(Clone, Copy)]
struct Hook {
    handle: u32,
    id: i32,
    proc: u32,
    hmod: u32,
    thread_id: u32,
}

#[derive(Clone, Copy)]
struct VirtualRegion {
    base: u32,
    size: u32,
    protect: u32,
}

fn virtual_region_from_allocation(allocation: ArenaAllocation) -> VirtualRegion {
    VirtualRegion {
        base: allocation.base,
        size: allocation.size,
        protect: allocation.protect,
    }
}

fn range_within_arena(base: u32, size: u32, arena_base: u32, arena_size: u32) -> bool {
    let end = (base as u64).saturating_add(size as u64);
    let arena_end = arena_base as u64 + arena_size as u64;
    base as u64 >= arena_base as u64 && end <= arena_end
}

#[derive(Clone, Copy)]
struct GdiFont {
    height: u32,
}

#[derive(Clone, Copy)]
struct GdiBitmap {
    surface: u32,
}

#[derive(Clone, Copy)]
struct GdiBrush {
    color: u32,
    bitmap: u32,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct GdiPen {
    style: u32,
    width: u32,
    color: u32,
}

struct GdiPalette {
    entries: Vec<[u8; 4]>,
}

const R2_BLACK: u32 = 1;
const R2_NOTMERGEPEN: u32 = 2;
const R2_MASKNOTPEN: u32 = 3;
const R2_NOTCOPYPEN: u32 = 4;
const R2_MASKPENNOT: u32 = 5;
const R2_NOT: u32 = 6;
const R2_XORPEN: u32 = 7;
const R2_NOTMASKPEN: u32 = 8;
const R2_MASKPEN: u32 = 9;
const R2_NOTXORPEN: u32 = 10;
const R2_NOP: u32 = 11;
const R2_MERGENOTPEN: u32 = 12;
const R2_COPYPEN: u32 = 13;
const R2_MERGEPENNOT: u32 = 14;
const R2_MERGEPEN: u32 = 15;
const R2_WHITE: u32 = 16;
const MM_TEXT: u32 = 1;
const TA_LEFT: u32 = 0x0000;
const TA_RIGHT: u32 = 0x0002;
const TA_CENTER: u32 = 0x0006;
const TA_TOP: u32 = 0x0000;
const TA_BOTTOM: u32 = 0x0008;
const TA_BASELINE: u32 = 0x0018;
const TA_UPDATECP: u32 = 0x0001;

#[derive(Clone, Copy)]
struct GdiDc {
    surface: u32,
    hwnd: u32,
    selected_font: u32,
    selected_bitmap: u32,
    selected_brush: u32,
    selected_pen: u32,
    selected_palette: u32,
    rop2: u32,
    layout: u32,
    map_mode: u32,
    text_align: u32,
    text_extra: i32,
    text_color: u32,
    bk_color: u32,
    bk_mode: u32,
    origin_x: i32,
    origin_y: i32,
    brush_origin_x: i32,
    brush_origin_y: i32,
    current_x: i32,
    current_y: i32,
}

fn align_up_to(value: u32, alignment: u32) -> Result<u32> {
    if value == 0 {
        return Ok(0);
    }
    value
        .checked_add(alignment - 1)
        .map(|x| x & !(alignment - 1))
        .ok_or_else(|| Error::Memory(format!("align_up_to overflow: {value:08x}")))
}

impl Hle {
    pub fn new() -> Self {
        init_hle_trace_flags_from_env();
        let mut hle = Self {
            next_thunk: HLE_BASE,
            entries: Vec::new(),
            entries_by_name: HashMap::new(),
            unresolved_hle_symbols: Vec::new(),
            missing_hle_reported: false,
            missing_hle_report: String::new(),
            strict_hle_imports: cfg!(debug_assertions),
            handles: Vec::new(),
            vfs: Vfs::default(),
            named_kernel_objects: HashMap::new(),
            modules: HashMap::new(),
            module_images: HashMap::new(),
            next_module: 0x6000_0000,
            module_arena: GuestArena::new("module", MODULE_BASE, MODULE_SIZE),
            heap_arena: GuestArena::new("hle-heap", HEAP_BASE, HEAP_SIZE).retain_freed_pages(),
            private_arena: GuestArena::new("hle-private", HLE_PRIVATE_BASE, HLE_PRIVATE_SIZE),
            private_on_guest_heap: std::env::var_os("WEMU_HLE_PRIVATE_ON_GUEST_HEAP").is_some(),
            next_gdi_handle: GDI_HANDLE_BASE,
            gdi_fonts: HashMap::new(),
            gdi_bitmaps: HashMap::new(),
            gdi_brushes: HashMap::new(),
            gdi_pens: HashMap::new(),
            gdi_palettes: HashMap::new(),
            gdi_regions: HashMap::new(),
            gdi_dcs: HashMap::new(),
            gdi_dc_saves: HashMap::new(),
            windows: HashMap::new(),
            menus: HashMap::new(),
            accelerators: HashMap::new(),
            scroll_states: HashMap::new(),
            next_menu_handle: 0x5200_0000,
            next_accelerator_handle: 0x5200_2000,
            active_popup_menu: None,
            hle_windows_dirty: false,
            window_class_procs: HashMap::new(),
            window_class_atoms: HashMap::new(),
            window_class_menus: HashMap::new(),
            window_class_atom_menus: HashMap::new(),
            window_class_backgrounds: HashMap::new(),
            window_class_atom_backgrounds: HashMap::new(),
            next_window_class_atom: 1,
            registered_window_messages: HashMap::new(),
            next_registered_window_message: 0xc000,
            next_hook_handle: HOOK_HANDLE_BASE,
            hooks: Vec::new(),
            virtual_arena: GuestArena::new("virtual", VIRTUAL_BASE, VIRTUAL_SIZE),
            virtual_high_arena: GuestArena::new("virtual-high", VIRTUAL_HIGH_BASE, VIRTUAL_HIGH_SIZE),
            last_error: 0,
            wsa_last_error: 0,
            command_line_a: 0,
            command_line_w: 0,
            environment_a: 0,
            tls_slots: Vec::new(),
            input_messages: Vec::new(),
            app_messages: Vec::new(),
            last_queued_message: None,
            last_peek_message: None,
            last_removed_message: None,
            last_dispatched_message: None,
            message_flood_msg: 0,
            message_flood_proc: 0,
            message_flood_count: 0,
            message_flood_warned: false,
            recent_message_types: [0; MESSAGE_TYPE_HISTORY_LEN],
            recent_message_type_pos: 0,
            paint_frame_token: 0,
            cooperative_idle: false,
            frontend_fps: 0,
            frontend_microseconds_per_frame: 0,
            timers: Vec::new(),
            next_timer_id: 1,
            mm_timers: Vec::new(),
            next_mm_timer_id: 1,
            async_return_thunk: 0,
            button_wndproc_thunk: 0,
            hle_callback_stack: Vec::new(),
            mm_timer_callback_active: false,
            window_proc: 0,
            focus_window: 0,
            capture_window: 0,
            key_down: [false; 256],
            key_pressed: [0; 256],
            cursor_x: 0,
            cursor_y: 0,
            next_window_handle: 0x0002_0001,
            gdi_screen_surface: 0,
            gdi_present_pending: false,
            ddraw_vtable: 0,
            ddraw_surface_vtable: 0,
            ddraw_palette_vtable: 0,
            ddraw_clipper_vtable: 0,
            ddraw_enum_continue_thunk: 0,
            ddraw_enum_modes: None,
            ddraw_width: 640,
            ddraw_height: 480,
            ddraw_bpp: 16,
            dinput_vtable: 0,
            dinput_device_vtable: 0,
            dsound_vtable: 0,
            dsound_buffer_vtable: 0,
            crt_iob: 0,
            crt_initenv: 0,
            crt_winitenv: 0,
            crt_acmdln: 0,
            crt_wcmdln: 0,
            crt_adjust_fdiv: 0,
            crt_fmode: 0,
            crt_commode: 0,
            crt_output: String::new(),
            error_mode: 0,
            fake_time: 1,
            rand_seed: 1,
            module_file_name: "C:\\wemu.exe".to_string(),
            winsock_inet_ntoa_buffer: 0,
        };
        hle.async_return_thunk = hle.register_symbol("wemu", "__async_return", hle_async_return);
        hle.button_wndproc_thunk =
            hle.register_symbol("wemu", "__button_wndproc", hle_button_wndproc);
        hle.ddraw_enum_continue_thunk = hle.register_symbol(
            "wemu",
            "__ddraw_enum_modes_continue",
            hle_ddraw_enum_modes_continue,
        );
        hle
    }

    pub fn set_frontend_timing(&mut self, fps: u32, microseconds_per_frame: u64) {
        self.frontend_fps = fps;
        self.frontend_microseconds_per_frame = if microseconds_per_frame != 0 {
            microseconds_per_frame
        } else if fps != 0 {
            MICROSECONDS_PER_SECOND.saturating_add((fps / 2) as u64) / fps as u64
        } else {
            0
        };
    }

    pub fn delay_target(&self, delay_ms: u32, scheduler_frame: u64) -> HleDelayTarget {
        if scheduler_frame == 0 {
            return HleDelayTarget::millisecond(delay_ms as u64);
        }

        let frame_us = self.frontend_microseconds_per_frame;
        if frame_us == 0 {
            return HleDelayTarget {
                delay_ms: delay_ms as u64,
                frame_count: 1,
            };
        }

        let frame_count = if delay_ms == 0 {
            1
        } else {
            rounded_live_frame_count(delay_ms, frame_us)
        };
        let rounded_us = (frame_count as u128).saturating_mul(frame_us as u128);
        let effective_ms = if delay_ms == 0 {
            0
        } else {
            ((rounded_us / MICROSECONDS_PER_MILLISECOND as u128).min(u64::MAX as u128) as u64)
                .max(1)
        };
        HleDelayTarget {
            delay_ms: effective_ms,
            frame_count,
        }
    }

    pub fn contains_addr(&self, addr: u32) -> bool {
        hle_index(addr).is_some_and(|index| index < self.entries.len())
    }

    pub fn entry_at(&self, addr: u32) -> Option<HleEntry> {
        hle_index(addr).and_then(|index| self.entries.get(index)).copied()
    }

    pub fn output(&self) -> &str {
        &self.crt_output
    }

    pub fn window_ready(&self) -> bool {
        self.window_proc != 0
            || self
                .windows
                .values()
                .any(|window| window.parent == 0 && window.proc != 0)
    }

    pub fn has_messages(&self) -> bool {
        self.has_matching_message(MessageFilter::any())
    }

    pub fn has_input_messages(&self) -> bool {
        !self.input_messages.is_empty()
    }

    pub fn has_matching_message(&self, filter: MessageFilter) -> bool {
        self.matching_queued_message(filter).is_some()
    }

    fn matching_queued_message(&self, filter: MessageFilter) -> Option<QueuedMessage> {
        self.matching_message_index(&self.input_messages, filter)
            .map(|index| QueuedMessage {
                kind: MessageQueueKind::Input,
                index,
                message: self.input_messages[index],
            })
            .or_else(|| {
                self.matching_message_index(&self.app_messages, filter)
                    .map(|index| QueuedMessage {
                        kind: MessageQueueKind::App,
                        index,
                        message: self.app_messages[index],
                    })
            })
    }

    fn matching_message_index(&self, messages: &[Message], filter: MessageFilter) -> Option<usize> {
        messages
            .iter()
            .position(|message| self.message_matches_filter(*message, filter))
    }

    fn message_matches_filter(&self, message: Message, filter: MessageFilter) -> bool {
        !self.paint_message_suppressed_this_frame(message)
            && self.message_hwnd_matches(message.hwnd, filter.hwnd)
            && message_id_matches(message.msg, filter.min, filter.max)
    }

    fn message_hwnd_matches(&self, mut message_hwnd: u32, hwnd_filter: u32) -> bool {
        if hwnd_filter == 0 {
            return true;
        }
        if hwnd_filter == 0xffff_ffff {
            return message_hwnd == 0;
        }
        loop {
            if message_hwnd == hwnd_filter {
                return true;
            }
            let Some(parent) = self.window(message_hwnd).map(|window| window.parent) else {
                return false;
            };
            if parent == 0 {
                return false;
            }
            message_hwnd = parent;
        }
    }

    fn remove_queued_message(&mut self, queued: QueuedMessage, source: &'static str) -> bool {
        if !message_remove_on_pm_remove(queued.message) {
            return false;
        }
        match queued.kind {
            MessageQueueKind::Input => {
                self.input_messages.remove(queued.index);
            }
            MessageQueueKind::App => {
                self.app_messages.remove(queued.index);
            }
        }
        self.note_removed_message(source, queued.message);
        true
    }

    pub fn has_active_popup_menu(&self) -> bool {
        self.active_popup_menu.is_some()
    }

    pub fn begin_frame(&mut self) {
        self.cooperative_idle = false;
        self.paint_frame_token = self.paint_frame_token.wrapping_add(1);
        if self.paint_frame_token == 0 {
            self.paint_frame_token = 1;
        }
        // Message flood reports are frame-local: a generated WM_PAINT that
        // survives validation should be throttled to the next frame, while
        // many identical dispatches inside one frame still indicate starvation.
        self.message_flood_count = 0;
        self.message_flood_warned = false;
    }

    fn paint_message_suppressed_this_frame(&self, message: Message) -> bool {
        const WM_PAINT: u32 = 0x000f;
        if message.msg != WM_PAINT || self.paint_frame_token == 0 {
            return false;
        }
        self.window(message.hwnd).is_some_and(|window| {
            window.invalid_rect.is_some()
                && window.last_generated_paint_frame == self.paint_frame_token
        })
    }

    fn note_generated_paint_delivered(&mut self, message: Message) {
        const WM_PAINT: u32 = 0x000f;
        if message.msg != WM_PAINT || self.paint_frame_token == 0 {
            return;
        }
        let frame = self.paint_frame_token;
        if let Some(window) = self.window_mut(message.hwnd) {
            window.last_generated_paint_frame = frame;
        }
    }

    fn note_cooperative_idle(&mut self) {
        self.cooperative_idle = true;
    }

    pub(crate) fn take_cooperative_idle(&mut self) -> bool {
        let idle = self.cooperative_idle;
        self.cooperative_idle = false;
        idle
    }

    pub fn mark_hle_windows_dirty(&mut self) {
        self.hle_windows_dirty = true;
    }

    fn mark_gdi_present_pending(&mut self) {
        self.gdi_present_pending = true;
    }

    fn clear_gdi_present_pending(&mut self) {
        self.gdi_present_pending = false;
    }

    fn take_gdi_present_pending(&mut self) -> bool {
        let pending = self.gdi_present_pending;
        self.gdi_present_pending = false;
        pending
    }

    fn take_hle_windows_dirty(&mut self) -> bool {
        let dirty = self.hle_windows_dirty;
        self.hle_windows_dirty = false;
        dirty
    }

    pub fn is_menu_bar_at(&self, x: u32, y: u32) -> bool {
        self.menu_bar_hit(x as i32, y as i32).is_some()
    }

    pub fn state_summary(&self, now_ms: u64) -> String {
        let next_timer = self
            .timers
            .iter()
            .map(|timer| timer.next_ms.saturating_sub(now_ms))
            .min()
            .map(|ms| ms.to_string())
            .unwrap_or_else(|| "-".to_string());
        let next_mm_timer = self
            .mm_timers
            .iter()
            .map(|timer| timer.next_ms.saturating_sub(now_ms))
            .min()
            .map(|ms| ms.to_string())
            .unwrap_or_else(|| "-".to_string());
        format!(
            "window={} input={} app={} timers={} next_timer_ms={} registered_timers={} mm_timers={} next_mm_ms={} hooks={} messages={} cursor={},{} last_error={:08x}",
            if self.window_ready() { 1 } else { 0 },
            self.input_messages.len(),
            self.app_messages.len(),
            self.timers.len(),
            next_timer,
            self.registered_timer_summary(now_ms),
            self.mm_timer_summary(now_ms),
            next_mm_timer,
            self.hook_summary(),
            self.message_summary(),
            self.cursor_x,
            self.cursor_y,
            self.last_error,
        )
    }

    fn registered_timer_summary(&self, now_ms: u64) -> String {
        if self.timers.is_empty() {
            return "-".to_string();
        }
        self.timers
            .iter()
            .map(|timer| {
                let pending = self.app_messages.iter().any(|message| {
                    message.hwnd == timer.hwnd
                        && message.msg == 0x0113
                        && message.wparam == timer.id
                });
                format!(
                    "{{hwnd={:08x},id={:08x},period={},next={},proc={:08x},due={},posted={},pending={}}}",
                    timer.hwnd,
                    timer.id,
                    timer.period_ms,
                    timer.next_ms.saturating_sub(now_ms),
                    timer.proc,
                    timer.due_count,
                    timer.post_count,
                    if pending { 1 } else { 0 },
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    fn mm_timer_summary(&self, now_ms: u64) -> String {
        if self.mm_timers.is_empty() {
            return "-".to_string();
        }
        self.mm_timers
            .iter()
            .map(|timer| {
                format!(
                    "{{id={:08x},period={},next={},cb={:08x},user={:08x},event={:08x},due={},called={}}}",
                    timer.id,
                    timer.period_ms,
                    timer.next_ms.saturating_sub(now_ms),
                    timer.callback,
                    timer.user,
                    timer.event,
                    timer.due_count,
                    timer.call_count,
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    fn hook_summary(&self) -> String {
        if self.hooks.is_empty() {
            return "-".to_string();
        }
        self.hooks
            .iter()
            .map(|hook| {
                format!(
                    "{{handle={:08x},id={},proc={:08x},hmod={:08x},tid={:08x}}}",
                    hook.handle, hook.id, hook.proc, hook.hmod, hook.thread_id,
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    fn message_summary(&self) -> String {
        format!(
            "post={} peek={} removed={} dispatch={}",
            self.message_event_summary(self.last_queued_message),
            self.peek_event_summary(self.last_peek_message),
            self.message_event_summary(self.last_removed_message),
            self.dispatch_event_summary(self.last_dispatched_message),
        )
    }

    fn message_event_summary(&self, event: Option<MessageEvent>) -> String {
        event
            .map(|event| format!("{}:{}", event.source, format_message(event.message)))
            .unwrap_or_else(|| "-".to_string())
    }

    fn peek_event_summary(&self, event: Option<PeekEvent>) -> String {
        event
            .map(|event| {
                let message = event
                    .message
                    .map(format_message)
                    .unwrap_or_else(|| "none".to_string());
                format!("{}:remove={}:{}", event.source, event.remove, message)
            })
            .unwrap_or_else(|| "-".to_string())
    }

    fn dispatch_event_summary(&self, event: Option<DispatchEvent>) -> String {
        event
            .map(|event| format!("{}->proc={:08x}", format_message(event.message), event.proc))
            .unwrap_or_else(|| "-".to_string())
    }

    fn note_queued_message(&mut self, source: &'static str, message: Message) {
        self.last_queued_message = Some(MessageEvent { source, message });
    }

    fn note_peek_message(&mut self, source: &'static str, remove: u32, message: Option<Message>) {
        self.last_peek_message = Some(PeekEvent {
            source,
            remove,
            message,
        });
    }

    fn note_removed_message(&mut self, source: &'static str, message: Message) {
        self.last_removed_message = Some(MessageEvent { source, message });
    }

    fn note_dispatched_message(&mut self, message: Message, proc: u32) -> Option<MessageFloodReport> {
        self.last_dispatched_message = Some(DispatchEvent { message, proc });
        self.note_dispatched_message_type(message.msg);
        self.note_message_flood(message, proc)
    }

    fn note_dispatched_message_type(&mut self, msg: u32) {
        let index = (self.recent_message_type_pos as usize) % MESSAGE_TYPE_HISTORY_LEN;
        self.recent_message_types[index] = msg;
        self.recent_message_type_pos = (self.recent_message_type_pos + 1)
            % MESSAGE_TYPE_HISTORY_LEN as u32;
    }

    fn note_message_flood(&mut self, message: Message, proc: u32) -> Option<MessageFloodReport> {
        const WM_TIMER: u32 = 0x0113;
        if message.msg == WM_TIMER {
            return None;
        }
        if self.message_flood_count == 0
            || self.message_flood_msg != message.msg
            || self.message_flood_proc != proc
        {
            self.message_flood_msg = message.msg;
            self.message_flood_proc = proc;
            self.message_flood_count = 1;
            self.message_flood_warned = false;
            return None;
        }
        self.message_flood_count = self.message_flood_count.saturating_add(1);
        if self.message_flood_count == MESSAGE_FLOOD_DISPATCH_LIMIT && !self.message_flood_warned {
            self.message_flood_warned = true;
            return Some(MessageFloodReport {
                msg: message.msg,
                proc,
                hwnd: message.hwnd,
                count: self.message_flood_count,
                input_len: self.input_messages.len() as u32,
                app_len: self.app_messages.len() as u32,
                recent: self.recent_message_type_snapshot(),
            });
        }
        None
    }

    fn recent_message_type_snapshot(&self) -> [u32; MESSAGE_TYPE_HISTORY_LEN] {
        let mut out = [0; MESSAGE_TYPE_HISTORY_LEN];
        let pos = self.recent_message_type_pos as usize;
        let mut index = 0;
        while index < MESSAGE_TYPE_HISTORY_LEN {
            out[index] = self.recent_message_types[(pos + index) % MESSAGE_TYPE_HISTORY_LEN];
            index += 1;
        }
        out
    }

    pub fn post_mouse_move(&mut self, x: u32, y: u32) {
        const WM_MOUSEMOVE: u32 = 0x0200;
        self.cursor_x = x;
        self.cursor_y = y;
        let target = self.mouse_target(x, y);
        let new_message = Message {
            hwnd: target.hwnd,
            msg: WM_MOUSEMOVE,
            wparam: 0,
            lparam: mouse_lparam(target.client_x, target.client_y),
        };
        if let Some(last) = self.input_messages.last_mut() {
            if last.msg != WM_MOUSEMOVE {
                self.input_messages.push(new_message);
                self.note_queued_message("input-move", new_message);
                return;
            }
            *last = new_message;
            self.note_queued_message("input-move", new_message);
            return;
        }
        self.input_messages.push(new_message);
        self.note_queued_message("input-move", new_message);
    }

    pub fn post_mouse_button_down(&mut self, x: u32, y: u32) {
        const WM_LBUTTONDOWN: u32 = 0x0201;
        const MK_LBUTTON: u32 = 0x0001;
        self.cursor_x = x;
        self.cursor_y = y;
        if self.active_popup_menu.is_some() {
            return;
        }
        let target = self.mouse_target(x, y);
        self.focus_from_mouse(target.hwnd);
        let message = Message {
            hwnd: target.hwnd,
            msg: WM_LBUTTONDOWN,
            wparam: MK_LBUTTON,
            lparam: mouse_lparam(target.client_x, target.client_y),
        };
        self.input_messages.push(message);
        self.note_queued_message("input-down", message);
    }

    pub fn activate_menu_bar_at(
        &mut self,
        x: u32,
        y: u32,
        screen_w: u32,
        screen_h: u32,
    ) -> bool {
        let Some(hit) = self.menu_bar_hit(x as i32, y as i32) else {
            return false;
        };
        if !hit.enabled {
            return true;
        }
        if hit.submenu != 0 {
            self.active_popup_menu = build_popup_menu_from_hle(
                self,
                hit.submenu,
                hit.hwnd,
                hit.rect.left,
                hit.rect.bottom,
                screen_w as i32,
                screen_h as i32,
            );
            return true;
        }
        if hit.id != 0 {
            let message = Message {
                hwnd: hit.hwnd,
                msg: 0x0111,
                wparam: hit.id,
                lparam: 0,
            };
            self.app_messages.push(message);
            self.note_queued_message("menu-bar-command", message);
        }
        true
    }

    pub fn post_mouse_button_up(&mut self, x: u32, y: u32) {
        const WM_LBUTTONUP: u32 = 0x0202;
        self.cursor_x = x;
        self.cursor_y = y;
        if let Some(command) = self.take_popup_menu_command(x as i32, y as i32) {
            self.app_messages.push(command);
            self.note_queued_message("menu-click", command);
            return;
        }
        let target = self.mouse_target(x, y);
        let message = Message {
            hwnd: target.hwnd,
            msg: WM_LBUTTONUP,
            wparam: 0,
            lparam: mouse_lparam(target.client_x, target.client_y),
        };
        self.input_messages.push(message);
        self.note_queued_message("input-up", message);
        if let Some(command) = self.command_from_click(target.hwnd) {
            self.app_messages.push(command);
            self.note_queued_message("control-click", command);
        }
    }

    pub fn post_mouse_right_button_down(&mut self, x: u32, y: u32) {
        const WM_RBUTTONDOWN: u32 = 0x0204;
        const MK_RBUTTON: u32 = 0x0002;
        self.cursor_x = x;
        self.cursor_y = y;
        if self.active_popup_menu.is_some() {
            return;
        }
        let target = self.mouse_target(x, y);
        let message = Message {
            hwnd: target.hwnd,
            msg: WM_RBUTTONDOWN,
            wparam: MK_RBUTTON,
            lparam: mouse_lparam(target.client_x, target.client_y),
        };
        self.input_messages.push(message);
        self.note_queued_message("input-right-down", message);
    }

    pub fn post_mouse_right_button_up(&mut self, x: u32, y: u32) {
        const WM_RBUTTONUP: u32 = 0x0205;
        self.cursor_x = x;
        self.cursor_y = y;
        if self.active_popup_menu.is_some() {
            return;
        }
        let target = self.mouse_target(x, y);
        let message = Message {
            hwnd: target.hwnd,
            msg: WM_RBUTTONUP,
            wparam: 0,
            lparam: mouse_lparam(target.client_x, target.client_y),
        };
        self.input_messages.push(message);
        self.note_queued_message("input-right-up", message);
    }

    pub fn post_click(&mut self, x: u32, y: u32) {
        self.post_mouse_button_down(x, y);
        self.post_mouse_button_up(x, y);
    }

    pub fn post_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.post_char(ch as u32);
        }
    }

    pub fn post_text_input(&mut self, text: &str) {
        for ch in text.chars() {
            self.post_wm_char(ch as u32);
        }
    }

    pub fn post_key_down(&mut self, vk: u32) {
        const WM_KEYDOWN: u32 = 0x0100;
        self.note_key_down(vk);
        self.queue_key_message(self.focused_key_window(), WM_KEYDOWN, vk, 1);
    }

    pub fn post_key_up(&mut self, vk: u32) {
        const WM_KEYUP: u32 = 0x0101;
        self.note_key_up(vk);
        self.queue_key_message(self.focused_key_window(), WM_KEYUP, vk, 0xc000_0001);
    }

    fn post_char(&mut self, ch: u32) {
        const WM_KEYDOWN: u32 = 0x0100;
        const WM_KEYUP: u32 = 0x0101;
        const VK_SHIFT: u32 = 0x10;
        let hwnd = self.focused_key_window();
        if char_requires_shift(ch) {
            self.queue_key_message(hwnd, WM_KEYDOWN, VK_SHIFT, 1);
        }
        if let Some(vk) = virtual_key_for_char(ch) {
            self.queue_key_message(hwnd, WM_KEYDOWN, vk, 1);
        }
        self.post_wm_char(ch);
        if let Some(vk) = virtual_key_for_char(ch) {
            self.queue_key_message(hwnd, WM_KEYUP, vk, 0xc000_0001);
        }
        if char_requires_shift(ch) {
            self.queue_key_message(hwnd, WM_KEYUP, VK_SHIFT, 0xc000_0001);
        }
    }

    fn post_wm_char(&mut self, ch: u32) {
        const WM_CHAR: u32 = 0x0102;
        let hwnd = self.focused_key_window();
        let message = Message {
            hwnd,
            msg: WM_CHAR,
            wparam: ch,
            lparam: 0,
        };
        self.input_messages.push(message);
        self.note_queued_message("input-char", message);
    }

    fn focused_key_window(&self) -> u32 {
        if self.focus_window != 0 {
            self.focus_window
        } else {
            0x0002_0001
        }
    }

    fn queue_key_message(&mut self, hwnd: u32, msg: u32, vk: u32, lparam: u32) {
        const WM_KEYDOWN: u32 = 0x0100;
        let scan = vk_to_scan_code(vk);
        let lparam = lparam | ((scan & 0xff) << 16);
        let message = Message {
            hwnd,
            msg,
            wparam: vk,
            lparam,
        };
        self.input_messages.push(message);
        self.note_queued_message(
            if msg == WM_KEYDOWN {
                "input-keydown"
            } else {
                "input-keyup"
            },
            message,
        );
    }

    fn note_key_down(&mut self, vk: u32) {
        let key = (vk & 0xff) as usize;
        if !self.key_down[key] {
            self.key_pressed[key] = self.key_pressed[key].saturating_add(1);
        }
        self.key_down[key] = true;
    }

    fn note_key_up(&mut self, vk: u32) {
        self.key_down[(vk & 0xff) as usize] = false;
    }

    fn keyboard_state_byte(&self, vk: u32) -> u8 {
        if self.key_down[(vk & 0xff) as usize] {
            0x80
        } else {
            0
        }
    }

    fn key_state_word(&mut self, vk: u32, consume_press: bool) -> u32 {
        let key = (vk & 0xff) as usize;
        let mut value = if self.key_down[key] { 0x8000 } else { 0 };
        if consume_press && self.key_pressed[key] != 0 {
            self.key_pressed[key] -= 1;
            value |= 1;
        }
        value
    }

    fn focus_from_mouse(&mut self, hwnd: u32) {
        if self
            .window(hwnd)
            .is_some_and(|window| window.enabled && window.visible)
        {
            self.focus_window = hwnd;
        }
    }

    fn mouse_target(&self, x: u32, y: u32) -> MouseTarget {
        let x = x as i32;
        let y = y as i32;
        let hwnd = if self.capture_window != 0 && self.window(self.capture_window).is_some() {
            self.capture_window
        } else if let Some(parent) = self.top_window_at(x, y) {
            self.child_at(parent.hwnd, x, y)
                .map(|window| window.hwnd)
                .unwrap_or(parent.hwnd)
        } else {
            0x0002_0001
        };
        let (origin_x, origin_y) = self.mouse_client_origin(hwnd);
        MouseTarget {
            hwnd,
            client_x: x.saturating_sub(origin_x),
            client_y: y.saturating_sub(origin_y),
        }
    }

    fn mouse_client_origin(&self, hwnd: u32) -> (i32, i32) {
        self.window(hwnd)
            .map(|window| {
                let menu_h = self.menu_bar_height_for_window(window);
                if window_has_hle_frame(window) {
                    (
                        window.rect.left.saturating_add(DIALOG_BORDER),
                        window
                            .rect
                            .top
                            .saturating_add(DIALOG_TITLE_HEIGHT)
                            .saturating_add(menu_h),
                    )
                } else {
                    (window.rect.left, window.rect.top.saturating_add(menu_h))
                }
            })
            .unwrap_or((0, 0))
    }

    fn command_from_click(&self, hwnd: u32) -> Option<Message> {
        let window = self.window(hwnd)?;
        if window.control_kind != HleControlKind::Button
            || window.parent == 0
            || !is_clickable_button_style(window.style)
        {
            return None;
        }
        Some(Message {
            hwnd: window.parent,
            msg: 0x0111,
            wparam: window.id,
            lparam: window.hwnd,
        })
    }

    fn take_popup_menu_command(&mut self, x: i32, y: i32) -> Option<Message> {
        let popup = self.active_popup_menu.take()?;
        let item = popup
            .items
            .iter()
            .find(|item| item.enabled && !item.separator && item.rect.contains(x, y));
        let Some(item) = item else {
            return None;
        };
        if item.submenu != 0 || item.id == 0 {
            return None;
        }
        Some(Message {
            hwnd: popup.owner,
            msg: 0x0111,
            wparam: item.id,
            lparam: 0,
        })
    }

    fn menu_bar_hit(&self, x: i32, y: i32) -> Option<HleMenuBarHit> {
        let window = self.top_window_at(x, y)?;
        let bar = self.menu_bar_rect_for_window(window)?;
        if !bar.contains(x, y) {
            return None;
        }
        let menu = self.menu(self.window_menu_handle(window)?)?;
        let mut left = bar.left + 4;
        for item in menu.items.iter().filter(|item| !item.separator) {
            let width = menu_bar_item_width(&item.text);
            let rect = WindowRect {
                left,
                top: bar.top,
                right: (left + width).min(bar.right),
                bottom: bar.bottom,
            };
            if rect.contains(x, y) {
                return Some(HleMenuBarHit {
                    hwnd: window.hwnd,
                    id: item.id,
                    submenu: item.submenu,
                    rect,
                    enabled: item.enabled,
                });
            }
            left += width;
            if left >= bar.right {
                break;
            }
        }
        None
    }

    fn menu_bar_rect_for_window(&self, window: &HleWindow) -> Option<WindowRect> {
        self.window_menu_handle(window)?;
        let (left, top, right, bottom_limit) = if window_has_hle_frame(window) {
            (
                window.rect.left.saturating_add(DIALOG_BORDER),
                window.rect.top.saturating_add(DIALOG_TITLE_HEIGHT),
                window.rect.right.saturating_sub(DIALOG_BORDER),
                window.rect.bottom.saturating_sub(DIALOG_BORDER),
            )
        } else {
            (
                window.rect.left,
                window.rect.top,
                window.rect.right,
                window.rect.bottom,
            )
        };
        let rect = WindowRect {
            left,
            top,
            right,
            bottom: top.saturating_add(MENU_BAR_HEIGHT).min(bottom_limit),
        };
        (!rect.is_empty()).then_some(rect)
    }

    fn menu_bar_height_for_window(&self, window: &HleWindow) -> i32 {
        self.menu_bar_rect_for_window(window)
            .map(|rect| rect.bottom.saturating_sub(rect.top))
            .unwrap_or(0)
    }

    fn window_menu_handle(&self, window: &HleWindow) -> Option<u32> {
        (window.parent == 0 && self.menus.contains_key(&window.id)).then_some(window.id)
    }

    fn alloc_menu_handle(&mut self, menu: HleMenu) -> u32 {
        let handle = self.next_menu_handle;
        self.next_menu_handle = self.next_menu_handle.wrapping_add(0x10);
        self.menus.insert(handle, menu);
        handle
    }

    fn menu(&self, handle: u32) -> Option<&HleMenu> {
        self.menus.get(&handle)
    }

    fn alloc_accelerator_handle(&mut self, table: HleAcceleratorTable) -> u32 {
        let handle = self.next_accelerator_handle;
        self.next_accelerator_handle = self.next_accelerator_handle.wrapping_add(0x10);
        self.accelerators.insert(handle, table);
        handle
    }

    fn accelerator_table(&self, handle: u32) -> Option<&HleAcceleratorTable> {
        self.accelerators.get(&handle)
    }

    pub fn set_timer(
        &mut self,
        hwnd: u32,
        requested_id: u32,
        proc: u32,
        now_ms: u64,
        scheduler_frame: u64,
        target: HleDelayTarget,
    ) -> u32 {
        let id = if requested_id == 0 {
            let id = self.next_timer_id.max(1);
            self.next_timer_id = id.wrapping_add(1).max(1);
            id
        } else {
            requested_id
        };
        let period_ms = target.delay_ms;
        let period_frames = target.frame_count;
        if let Some(timer) = self
            .timers
            .iter_mut()
            .find(|timer| timer.hwnd == hwnd && timer.id == id)
        {
            timer.proc = proc;
            timer.period_ms = period_ms;
            timer.period_frames = period_frames;
            timer.next_ms = target.until_ms(now_ms);
            timer.eligible_frame = target.eligible_frame(scheduler_frame);
        } else {
            self.timers.push(Timer {
                hwnd,
                id,
                proc,
                period_ms,
                period_frames,
                next_ms: target.until_ms(now_ms),
                eligible_frame: target.eligible_frame(scheduler_frame),
                due_count: 0,
                post_count: 0,
            });
        }
        id
    }

    pub fn kill_timer(&mut self, hwnd: u32, id: u32) {
        self.timers
            .retain(|timer| !(timer.hwnd == hwnd && timer.id == id));
    }

    pub fn next_message_timer_ms(&self) -> Option<u64> {
        self.timers.iter().map(|timer| timer.next_ms).min()
    }

    pub fn set_mm_timer(
        &mut self,
        callback: u32,
        user: u32,
        event: u32,
        now_ms: u64,
        scheduler_frame: u64,
        target: HleDelayTarget,
    ) -> u32 {
        if callback == 0 {
            return 0;
        }
        let id = self.next_mm_timer_id.max(1);
        self.next_mm_timer_id = id.wrapping_add(1).max(1);
        let period_ms = target.delay_ms;
        let period_frames = target.frame_count;
        self.mm_timers.push(MmTimer {
            id,
            callback,
            user,
            event,
            period_ms,
            period_frames,
            next_ms: target.until_ms(now_ms),
            eligible_frame: target.eligible_frame(scheduler_frame),
            due_count: 0,
            call_count: 0,
        });
        id
    }

    pub fn kill_mm_timer(&mut self, id: u32) -> bool {
        let old_len = self.mm_timers.len();
        self.mm_timers.retain(|timer| timer.id != id);
        self.mm_timers.len() != old_len
    }

    fn take_due_mm_timer(&mut self, now_ms: u64, scheduler_frame: u64) -> Option<MmTimerDue> {
        const TIME_PERIODIC: u32 = 0x0001;
        if self.mm_timer_callback_active {
            return None;
        }
        let index = self
            .mm_timers
            .iter()
            .position(|timer| now_ms >= timer.next_ms && scheduler_frame >= timer.eligible_frame)?;
        let timer = self.mm_timers[index];
        self.mm_timers[index].due_count = self.mm_timers[index].due_count.saturating_add(1);
        self.mm_timers[index].call_count = self.mm_timers[index].call_count.saturating_add(1);
        if (timer.event & TIME_PERIODIC) != 0 {
            self.mm_timers[index].next_ms = now_ms.saturating_add(timer.period_ms);
            self.mm_timers[index].eligible_frame =
                next_period_frame(scheduler_frame, timer.period_frames);
        } else {
            self.mm_timers.remove(index);
        }
        self.mm_timer_callback_active = true;
        Some(MmTimerDue {
            id: timer.id,
            callback: timer.callback,
            user: timer.user,
        })
    }

    fn async_return_thunk(&self) -> u32 {
        self.async_return_thunk
    }

    fn push_hle_callback_return(&mut self, return_value: u32) {
        self.hle_callback_stack.push(HleCallbackFrame {
            return_value,
            continuation: None,
        });
    }

    fn push_async_cpu_callback_return(&mut self, cpu: crate::cpu::Cpu) {
        self.hle_callback_stack.push(HleCallbackFrame {
            return_value: 0,
            continuation: Some(HleCallbackContinuation::AsyncCpu { cpu }),
        });
    }

    fn push_owner_draw_button_callback_return(
        &mut self,
        return_value: u32,
        hdc: u32,
        draw_item: u32,
        chain: Option<OwnerDrawChain>,
    ) {
        self.hle_callback_stack.push(HleCallbackFrame {
            return_value,
            continuation: Some(HleCallbackContinuation::OwnerDrawButton {
                hdc,
                draw_item,
                chain,
            }),
        });
    }

    fn push_create_window_callback_return(
        &mut self,
        return_value: u32,
        continuation: CreateWindowContinuation,
    ) {
        self.hle_callback_stack.push(HleCallbackFrame {
            return_value,
            continuation: Some(HleCallbackContinuation::CreateWindow(continuation)),
        });
    }

    fn push_dialog_init_callback_return(
        &mut self,
        return_value: u32,
        continuation: DialogInitContinuation,
    ) {
        self.hle_callback_stack.push(HleCallbackFrame {
            return_value,
            continuation: Some(HleCallbackContinuation::DialogInit(continuation)),
        });
    }

    fn push_window_proc_message_chain_return(
        &mut self,
        return_value: u32,
        chain: WindowProcMessageChain,
    ) {
        self.hle_callback_stack.push(HleCallbackFrame {
            return_value,
            continuation: Some(HleCallbackContinuation::WindowProcMessageChain(chain)),
        });
    }

    #[cfg(test)]
    fn finish_async_callback(&mut self) -> u32 {
        self.pop_hle_callback_frame().return_value
    }

    fn pop_hle_callback_frame(&mut self) -> HleCallbackFrame {
        self.mm_timer_callback_active = false;
        self.hle_callback_stack.pop().unwrap_or_default()
    }

    pub fn set_windows_hook(&mut self, id: i32, proc: u32, hmod: u32, thread_id: u32) -> u32 {
        if proc == 0 {
            return 0;
        }
        let handle = self.next_hook_handle;
        self.next_hook_handle = self.next_hook_handle.wrapping_add(4);
        self.hooks.push(Hook {
            handle,
            id,
            proc,
            hmod,
            thread_id,
        });
        handle
    }

    fn latest_windows_hook(&self, id: i32) -> Option<Hook> {
        self.hooks
            .iter()
            .rev()
            .copied()
            .find(|hook| hook.id == id && (hook.thread_id == 0 || hook.thread_id == HLE_THREAD_ID))
    }

    pub fn unhook_windows_hook(&mut self, handle: u32) -> bool {
        let old_len = self.hooks.len();
        self.hooks.retain(|hook| hook.handle != handle);
        self.hooks.len() != old_len
    }

    pub fn pump_timers(&mut self, now_ms: u64, scheduler_frame: u64) {
        const WM_TIMER: u32 = 0x0113;
        for index in 0..self.timers.len() {
            let timer = self.timers[index];
            if now_ms < timer.next_ms || scheduler_frame < timer.eligible_frame {
                continue;
            }
            self.timers[index].due_count = self.timers[index].due_count.saturating_add(1);
            let pending = self.app_messages.iter().any(|message| {
                message.hwnd == timer.hwnd && message.msg == WM_TIMER && message.wparam == timer.id
            });
            if !pending {
                let message = Message {
                    hwnd: timer.hwnd,
                    msg: WM_TIMER,
                    wparam: timer.id,
                    lparam: timer.proc,
                };
                self.app_messages.push(message);
                self.note_queued_message("timer", message);
                self.timers[index].post_count = self.timers[index].post_count.saturating_add(1);
            }
            self.timers[index].next_ms = now_ms.saturating_add(timer.period_ms);
            self.timers[index].eligible_frame =
                next_period_frame(scheduler_frame, timer.period_frames);
        }
    }

    pub fn resolve_import(&mut self, dll: &str, name: &str) -> u32 {
        let key = symbol_key(dll, name);
        if let Some(addr) = self.entries_by_name.get(&key) {
            return *addr;
        }
        let callback = match import_callback_for(dll, name) {
            Some(callback) => callback,
            None => {
                self.record_unresolved_hle_symbol(dll, name);
                hle_default
            }
        };
        self.register_symbol(dll, name, callback)
    }

    pub fn set_strict_hle_imports(&mut self, enabled: bool) {
        self.strict_hle_imports = enabled || cfg!(debug_assertions);
    }

    pub fn check_strict_hle_imports(&mut self) -> Result<()> {
        if !self.strict_hle_imports || self.unresolved_hle_symbols.is_empty() {
            return Ok(());
        }
        let reason = "missing HLE imports";
        self.emit_missing_hle_report_once(reason);
        let mut message = String::from("strict HLE imports: unresolved ");
        for (index, (dll, name)) in self.unresolved_hle_symbols.iter().enumerate() {
            if index != 0 {
                message.push_str(", ");
            }
            message.push_str(dll);
            message.push('!');
            message.push_str(name);
        }
        Err(Error::Hle(message))
    }

    pub fn resolve_pe_import(&mut self, mem: &mut Memory, dll: &str, name: &str) -> Result<u32> {
        let dll = dll.to_ascii_lowercase();
        if let Some(addr) = self.resolve_loaded_module_export(&dll, name) {
            return Ok(addr);
        }
        if is_hle_runtime_dll(&dll) && dll == "msvcrt.dll" {
            match name {
                "__initenv" => {
                    if self.crt_initenv == 0 {
                        self.crt_initenv = self.alloc(mem, 4, PagePerm::READ | PagePerm::WRITE)?;
                        mem.write_u32(self.crt_initenv, 0)?;
                    }
                    return Ok(self.crt_initenv);
                }
                "__winitenv" => {
                    if self.crt_winitenv == 0 {
                        self.crt_winitenv =
                            self.alloc(mem, 4, PagePerm::READ | PagePerm::WRITE)?;
                        mem.write_u32(self.crt_winitenv, 0)?;
                    }
                    return Ok(self.crt_winitenv);
                }
                "_acmdln" => {
                    if self.crt_acmdln == 0 {
                        self.crt_acmdln = self.alloc(mem, 4, PagePerm::READ | PagePerm::WRITE)?;
                        mem.write_u32(self.crt_acmdln, self.command_line_a)?;
                    }
                    return Ok(self.crt_acmdln);
                }
                "_wcmdln" => {
                    if self.crt_wcmdln == 0 {
                        self.crt_wcmdln = self.alloc(mem, 4, PagePerm::READ | PagePerm::WRITE)?;
                        mem.write_u32(self.crt_wcmdln, self.command_line_w)?;
                    }
                    return Ok(self.crt_wcmdln);
                }
                "_adjust_fdiv" => {
                    if self.crt_adjust_fdiv == 0 {
                        self.crt_adjust_fdiv =
                            self.alloc(mem, 4, PagePerm::READ | PagePerm::WRITE)?;
                        mem.write_u32(self.crt_adjust_fdiv, 0)?;
                    }
                    return Ok(self.crt_adjust_fdiv);
                }
                "_iob" => {
                    if self.crt_iob == 0 {
                        self.crt_iob = self.alloc(mem, 3 * 32, PagePerm::READ | PagePerm::WRITE)?;
                    }
                    return Ok(self.crt_iob);
                }
                _ => {}
            }
        }
        if is_hle_runtime_dll(&dll) {
            return Ok(self.resolve_import(&dll, name));
        }
        if self.load_mounted_pe_module(mem, &dll)?.is_some() {
            return self.resolve_loaded_module_export(&dll, name).ok_or_else(|| {
                Error::Pe(format!("mapped DLL {dll} does not export {name}"))
            });
        }
        Err(Error::Pe(format!(
            "refusing to HLE non-runtime DLL import {dll}!{name}"
        )))
    }

    fn resolve_loaded_module_export(&self, dll: &str, name: &str) -> Option<u32> {
        self.modules
            .get(dll)
            .and_then(|handle| self.module_images.get(handle))
            .and_then(|image| image.resolve_export(name))
    }

    fn load_mounted_pe_module(&mut self, mem: &mut Memory, name: &str) -> Result<Option<u32>> {
        let key = name.to_ascii_lowercase();
        if let Some(handle) = self.modules.get(&key) {
            if self.module_images.contains_key(handle) {
                return Ok(Some(*handle));
            }
        }
        if let Some(bytes) = self.virtual_file_bytes(&key) {
            let image = crate::pe::load_pe32_dll_bytes(PathBuf::from(&key), &bytes, mem, self)?;
            let handle = image.image_base;
            trace_fs!("Load mounted virtual PE module {key} -> {handle:08x}");
            self.module_images.insert(handle, image);
            self.modules.insert(key.clone(), handle);
            if let Some(file_name) = key.rsplit('\\').next() {
                self.modules.insert(file_name.to_string(), handle);
            }
            return Ok(Some(handle));
        }
        let path = match self.translate_raw_path(&key) {
            Ok(path) => path,
            Err(_) => return Ok(None),
        };
        let path = if path.is_file() {
            Some(path)
        } else {
            case_insensitive_existing_path(&path).filter(|candidate| candidate.is_file())
        };
        let Some(path) = path else {
            return Ok(None);
        };
        let image = crate::pe::load_pe32_dll(&path, mem, self)?;
        let handle = image.image_base;
        self.module_images.insert(handle, image);
        self.modules.insert(key, handle);
        if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
            self.modules.insert(file_name.to_ascii_lowercase(), handle);
        }
        Ok(Some(handle))
    }

    fn virtual_file_bytes(&self, raw: &str) -> Option<Vec<u8>> {
        let key = self.guest_path_key(raw);
        self.vfs.files
            .get(&key)
            .map(|data| data.borrow().clone())
    }

    fn hle_runtime_module_for_handle(&self, handle: u32) -> Option<String> {
        self.modules
            .iter()
            .find(|(name, module)| **module == handle && is_hle_runtime_dll(name))
            .map(|(name, _)| name.clone())
    }

    pub fn dll_process_attach_entries(&self) -> Vec<(u32, u32)> {
        let mut entries: Vec<(u32, u32)> = self
            .module_images
            .values()
            .filter(|image| image.entry != image.image_base)
            .map(|image| (image.image_base, image.entry))
            .collect();
        entries.sort_by_key(|(image_base, _)| *image_base);
        entries
    }

    pub fn register_symbol(&mut self, dll: &str, name: &str, callback: HleCallback) -> u32 {
        let key = symbol_key(dll, name);
        if let Some(addr) = self.entries_by_name.get(&key) {
            return *addr;
        }
        let addr = self.next_thunk;
        self.next_thunk = self.next_thunk.wrapping_add(HLE_STRIDE);
        let entry = HleEntry {
            addr,
            dll: leak_hle_str(dll),
            name: leak_hle_str(name),
            callback,
        };
        self.entries_by_name.insert(key, addr);
        self.entries.push(entry);
        addr
    }

    fn record_unresolved_hle_symbol(&mut self, dll: &str, name: &str) {
        if self
            .unresolved_hle_symbols
            .iter()
            .any(|(seen_dll, seen_name)| seen_dll.eq_ignore_ascii_case(dll) && seen_name == name)
        {
            return;
        }
        self.unresolved_hle_symbols
            .push((dll.to_string(), name.to_string()));
    }

    fn emit_missing_hle_report_once(&mut self, reason: &str) {
        if self.missing_hle_reported {
            return;
        }
        self.missing_hle_reported = true;
        self.missing_hle_report = missing_hle_report_json(reason, &self.unresolved_hle_symbols);
        eprintln!("{}", self.missing_hle_report);
    }

    pub fn missing_hle_report(&self) -> &str {
        self.missing_hle_report.as_str()
    }

    pub fn dispatch(emu: &mut Emulator) -> Result<Option<StopReason>> {
        let addr = emu.cpu.eip;
        let entry = hle_index(addr)
            .and_then(|index| emu.hle.entries.get(index))
            .copied()
            .ok_or_else(|| Error::Hle(format!("missing HLE thunk at {addr:08x}")))?;
        let ret_addr = emu.memory.read_u32(emu.cpu.reg(Reg::Esp))?;
        emu.record_hle_call(&entry.dll, &entry.name);
        if emu.should_trace() {
            eprintln!(
                "hle {}!{} esp={:08x} ret={:08x}",
                entry.dll,
                entry.name,
                emu.cpu.reg(Reg::Esp),
                ret_addr
            );
        }
        let watch_writes = emu.memory.write_watch_enabled();
        if watch_writes {
            emu.memory.set_write_context(Some(WriteContext {
                eip: entry.addr,
                insns: emu.insns,
                label: format!("HLE {}!{}", entry.dll, entry.name),
                regs: write_registers(&emu.cpu),
            }));
        }
        let result = (entry.callback)(emu, &entry);
        if watch_writes {
            emu.memory.set_write_context(None);
        }
        let arg_bytes = match result {
            HleResult::Retn(arg_bytes) => arg_bytes,
            HleResult::Wait(wait) => {
                flush_gdi_present_if_pending(emu)?;
                emu.park_current_hle_task(wait);
                return Ok(None);
            }
        };
        if let Some(reason) = emu.stopped {
            return Ok(Some(reason));
        }
        if emu.cpu.eip != entry.addr {
            return Ok(None);
        }
        let new_esp = emu
            .cpu
            .reg(Reg::Esp)
            .wrapping_add(4)
            .wrapping_add(arg_bytes);
        #[cfg(debug_assertions)]
        {
            emu.cpu.debug_finish_call_return(
                entry.addr,
                ret_addr,
                emu.cpu.reg(Reg::Esp),
                arg_bytes,
                &format!("HLE {}!{}", entry.dll, entry.name),
            )?;
        }
        emu.cpu.set_reg(Reg::Esp, new_esp);
        emu.cpu.eip = ret_addr;
        Ok(None)
    }

    pub fn bootstrap_process_strings(
        &mut self,
        mem: &mut Memory,
        image_base: u32,
        module_file_name: &str,
        command_line: &str,
    ) -> Result<()> {
        self.module_file_name = module_file_name.to_string();
        let full = command_line.to_string();
        self.command_line_a =
            self.alloc(mem, full.len() as u32 + 1, PagePerm::READ | PagePerm::WRITE)?;
        mem.write_cstr(self.command_line_a, &full, full.len() + 1)?;
        if self.crt_acmdln != 0 {
            mem.write_u32(self.crt_acmdln, self.command_line_a)?;
        }
        self.command_line_w = self.alloc(
            mem,
            (full.encode_utf16().count() as u32 + 1) * 2,
            PagePerm::READ | PagePerm::WRITE,
        )?;
        mem.write_utf16z(self.command_line_w, &full, full.encode_utf16().count() + 1)?;
        if self.crt_wcmdln != 0 {
            mem.write_u32(self.crt_wcmdln, self.command_line_w)?;
        }
        let env = b"SystemRoot=C:\\WINDOWS\0PATH=C:\\WINDOWS;C:\\WINDOWS\\SYSTEM32\0\0";
        self.environment_a = self.alloc(mem, env.len() as u32, PagePerm::READ | PagePerm::WRITE)?;
        mem.write_bytes(self.environment_a, env)?;
        for dll in [
            "ntdll.dll",
            "kernel32.dll",
            "user32.dll",
            "gdi32.dll",
            "ddraw.dll",
            "dsound.dll",
            "winmm.dll",
        ] {
            let handle = self.alloc_module();
            self.modules.insert(dll.to_string(), handle);
        }
        self.modules
            .insert(module_file_name.to_ascii_lowercase(), image_base);
        Ok(())
    }

    pub fn alloc(&mut self, mem: &mut Memory, size: u32, perm: PagePerm) -> Result<u32> {
        self.alloc_from(mem, size, perm, false, HLE_ALLOC_GUARD_SIZE)
    }

    pub fn alloc_compact(&mut self, mem: &mut Memory, size: u32, perm: PagePerm) -> Result<u32> {
        let size = size.max(1);
        let allocation =
            self.heap_arena
                .alloc_with_options(mem, size, perm, ArenaAllocOptions::new())?;
        trace_alloc!(
            "HLE alloc compact guest addr={:08x} size={size:x} aligned={:x}",
            allocation.base,
            allocation.size
        );
        Ok(allocation.base)
    }

    pub fn alloc_private(&mut self, mem: &mut Memory, size: u32, perm: PagePerm) -> Result<u32> {
        self.alloc_from(
            mem,
            size,
            perm,
            !self.private_on_guest_heap,
            HLE_ALLOC_GUARD_SIZE,
        )
    }

    pub fn alloc_guarded(
        &mut self,
        mem: &mut Memory,
        size: u32,
        perm: PagePerm,
        guard_size: u32,
    ) -> Result<u32> {
        self.alloc_with_guards(mem, size, perm, 0, guard_size)
    }

    pub fn alloc_with_guards(
        &mut self,
        mem: &mut Memory,
        size: u32,
        perm: PagePerm,
        guard_before: u32,
        guard_after: u32,
    ) -> Result<u32> {
        let size = size.max(1);
        let options = ArenaAllocOptions::new()
            .guard_before(guard_before)
            .guard_after(guard_after.max(HLE_ALLOC_GUARD_SIZE));
        let allocation = self
            .heap_arena
            .alloc_with_options(mem, size, perm, options)?;
        trace_alloc!(
            "HLE alloc guest addr={:08x} size={size:x} aligned={:x} guard_before={:x} guard_after={:x}",
            allocation.base,
            allocation.size,
            allocation.guard_before,
            allocation.guard_after
        );
        Ok(allocation.base)
    }

    pub fn reserve_module_image(
        &mut self,
        mem: &mut Memory,
        image_base: u32,
        image_size: u32,
    ) -> Result<()> {
        // The PE loader writes headers/sections after this reservation. Mapping
        // the whole image up front catches fixed-base collisions early.
        self.module_arena.alloc_at_with_options(
            mem,
            image_base,
            image_size.max(1),
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
            ArenaAllocOptions::new()
                .alignment(ALLOCATION_GRANULARITY)
                .protect(0x40),
        )?;
        Ok(())
    }

    pub fn alloc_module_image(&mut self, mem: &mut Memory, image_size: u32) -> Result<u32> {
        let allocation = self.module_arena.alloc_with_options(
            mem,
            image_size.max(1),
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
            ArenaAllocOptions::new()
                .alignment(ALLOCATION_GRANULARITY)
                .protect(0x40),
        )?;
        Ok(allocation.base)
    }

    fn create_gdi_font(&mut self, height: u32) -> u32 {
        let handle = self.alloc_gdi_handle();
        self.gdi_fonts.insert(
            handle,
            GdiFont {
                height: height.clamp(1, 200),
            },
        );
        handle
    }

    fn create_gdi_bitmap(&mut self, surface: u32) -> u32 {
        let handle = self.alloc_gdi_handle();
        self.gdi_bitmaps.insert(handle, GdiBitmap { surface });
        handle
    }

    fn create_gdi_brush(&mut self, color: u32, bitmap: u32) -> u32 {
        let handle = self.alloc_gdi_handle();
        self.gdi_brushes.insert(handle, GdiBrush { color, bitmap });
        handle
    }

    fn create_gdi_pen(&mut self, style: u32, width: u32, color: u32) -> u32 {
        let handle = self.alloc_gdi_handle();
        self.gdi_pens.insert(
            handle,
            GdiPen {
                style,
                width,
                color,
            },
        );
        handle
    }

    fn create_gdi_palette(&mut self, entries: Vec<[u8; 4]>) -> u32 {
        let handle = self.alloc_gdi_handle();
        self.gdi_palettes.insert(handle, GdiPalette { entries });
        handle
    }

    fn create_gdi_region(&mut self, rect: WindowRect) -> u32 {
        let handle = self.alloc_gdi_handle();
        self.gdi_regions.insert(handle, rect);
        handle
    }

    fn create_surface_dc(&mut self, surface: u32) -> u32 {
        let handle = self.alloc_gdi_handle();
        self.gdi_dcs.insert(
            handle,
            GdiDc {
                surface,
                hwnd: 0,
                selected_font: 0,
                selected_bitmap: 0,
                selected_brush: stock_object_handle(STOCK_WHITE_BRUSH),
                selected_pen: stock_object_handle(STOCK_BLACK_PEN),
                selected_palette: 0,
                rop2: R2_COPYPEN,
                layout: 0,
                map_mode: MM_TEXT,
                text_align: TA_LEFT | TA_TOP,
                text_extra: 0,
                text_color: 0x00ff_ffff,
                bk_color: 0x00ff_ffff,
                bk_mode: 1,
                origin_x: 0,
                origin_y: 0,
                brush_origin_x: 0,
                brush_origin_y: 0,
                current_x: 0,
                current_y: 0,
            },
        );
        handle
    }

    fn alloc_gdi_handle(&mut self) -> u32 {
        let handle = self.next_gdi_handle;
        self.next_gdi_handle = self.next_gdi_handle.wrapping_add(4);
        handle
    }

    fn alloc_from(
        &mut self,
        mem: &mut Memory,
        size: u32,
        perm: PagePerm,
        private: bool,
        guard_size: u32,
    ) -> Result<u32> {
        let size = size.max(1);
        let options = ArenaAllocOptions::new().guard_after(guard_size);
        let allocation = if private {
            self.private_arena
                .alloc_with_options(mem, size, perm, options)?
        } else {
            self.heap_arena
                .alloc_with_options(mem, size, perm, options)?
        };
        trace_alloc!(
            "HLE alloc {} addr={:08x} size={size:x} aligned={:x} guard={:x}",
            if private { "private" } else { "guest" },
            allocation.base,
            allocation.size,
            allocation.guard_after
        );
        Ok(allocation.base)
    }

    pub fn free_alloc(&mut self, mem: &mut Memory, addr: u32) -> Result<bool> {
        if addr == 0 {
            return Ok(false);
        }
        if self.heap_arena.try_free(mem, addr)?.is_some() {
            return Ok(true);
        }
        Ok(self.private_arena.try_free(mem, addr)?.is_some())
    }

    fn alloc_size(&self, addr: u32) -> Option<u32> {
        self.heap_arena
            .allocation_by_base(addr)
            .or_else(|| self.private_arena.allocation_by_base(addr))
            .map(|allocation| allocation.size)
    }

    pub fn virtual_alloc(
        &mut self,
        mem: &mut Memory,
        requested: u32,
        size: u32,
        protect: u32,
        perm: PagePerm,
    ) -> Result<u32> {
        let size = size.max(1);
        let aligned = align_up(size)?;
        if requested != 0 {
            let base = align_down(requested);
            let end = requested.checked_add(size).ok_or_else(|| {
                Error::Memory(format!(
                    "virtual alloc requested range overflow addr={requested:08x} size={size:x}"
                ))
            })?;
            let region_size = align_up(end.wrapping_sub(base))?;
            if let Some(region) = self
                .virtual_arena
                .allocation_containing_range(base, region_size)
            {
                mem.map_or_update(base, region_size, perm)?;
                self.virtual_arena
                    .update_metadata(region.base, protect, perm)?;
                return Ok(requested);
            }
            if let Some(region) = self
                .virtual_high_arena
                .allocation_containing_range(base, region_size)
            {
                mem.map_or_update(base, region_size, perm)?;
                self.virtual_high_arena
                    .update_metadata(region.base, protect, perm)?;
                return Ok(requested);
            }
            if range_within_arena(base, region_size, VIRTUAL_BASE, VIRTUAL_SIZE) {
                self.virtual_arena.alloc_at_with_options(
                    mem,
                    base,
                    region_size,
                    perm,
                    ArenaAllocOptions::new().protect(protect),
                )?;
            } else if range_within_arena(base, region_size, VIRTUAL_HIGH_BASE, VIRTUAL_HIGH_SIZE) {
                self.virtual_high_arena.alloc_at_with_options(
                    mem,
                    base,
                    region_size,
                    perm,
                    ArenaAllocOptions::new().protect(protect),
                )?;
            } else {
                return Err(Error::Memory(format!(
                    "virtual fixed range outside tracked arenas base={base:08x} size={region_size:x}"
                )));
            }
            Ok(requested)
        } else {
            let region_size = align_up_to(aligned, ALLOCATION_GRANULARITY)?;
            let allocation = self.virtual_arena.alloc_with_options(
                mem,
                region_size,
                perm,
                ArenaAllocOptions::new()
                    .alignment(ALLOCATION_GRANULARITY)
                    .protect(protect),
            )?;
            Ok(allocation.base)
        }
    }

    pub fn virtual_free(
        &mut self,
        mem: &mut Memory,
        addr: u32,
        size: u32,
        free_type: u32,
    ) -> Result<bool> {
        const MEM_RELEASE: u32 = 0x8000;
        const MEM_DECOMMIT: u32 = 0x4000;

        if addr == 0 {
            return Ok(false);
        }
        if (free_type & MEM_RELEASE) != 0 || size == 0 {
            if self.virtual_arena.try_free(mem, addr)?.is_some() {
                return Ok(true);
            }
            return Ok(self.virtual_high_arena.try_free(mem, addr)?.is_some());
        }
        if (free_type & MEM_DECOMMIT) != 0 {
            let region = self
                .virtual_arena
                .allocation_containing(addr)
                .or_else(|| self.virtual_high_arena.allocation_containing(addr));
            let Some(region) = region else { return Ok(false) };
            let decommit_end = (addr as u64).saturating_add(size as u64);
            let region_end = region.base as u64 + region.size as u64;
            if decommit_end > region_end {
                return Ok(false);
            }
            mem.unmap(addr, size)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn virtual_protect(
        &mut self,
        mem: &mut Memory,
        addr: u32,
        size: u32,
        protect: u32,
        perm: PagePerm,
    ) -> Result<u32> {
        let old = self
            .virtual_region(addr)
            .map(|region| region.protect)
            .unwrap_or(0x04);
        if addr != 0 && size != 0 {
            mem.protect(addr, align_up(size)?, perm)?;
            if let Some(region) = self.virtual_arena.allocation_containing(addr) {
                self.virtual_arena
                    .update_metadata(region.base, protect, perm)?;
            } else if let Some(region) = self.virtual_high_arena.allocation_containing(addr) {
                self.virtual_high_arena
                    .update_metadata(region.base, protect, perm)?;
            }
        }
        Ok(old)
    }

    fn virtual_region(&self, addr: u32) -> Option<VirtualRegion> {
        self.virtual_arena
            .allocation_containing(addr)
            .or_else(|| self.virtual_high_arena.allocation_containing(addr))
            .map(virtual_region_from_allocation)
    }

    fn alloc_module(&mut self) -> u32 {
        let handle = self.next_module;
        self.next_module = self.next_module.wrapping_add(0x10000);
        handle
    }

    fn alloc_handle(&mut self, handle: Handle) -> u32 {
        for (i, slot) in self.handles.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(handle);
                return 0x100 + i as u32;
            }
        }
        self.handles.push(Some(handle));
        0x100 + (self.handles.len() as u32 - 1)
    }

    fn alloc_window_handle(&mut self, parent: u32) -> u32 {
        self.next_window_handle = self.next_window_handle.wrapping_add(4).max(0x0002_0005);
        if parent == 0 && !self.windows.contains_key(&0x0002_0001) {
            0x0002_0001
        } else {
            self.next_window_handle
        }
    }

    fn register_window(&mut self, window: HleWindow) {
        self.windows.insert(window.hwnd, window);
    }

    fn register_window_class(&mut self, name: &str, proc: u32) -> u32 {
        let atom = self.next_window_class_atom;
        self.next_window_class_atom = self.next_window_class_atom.wrapping_add(1).max(1);
        if !name.is_empty() {
            self.window_class_procs
                .insert(name.to_ascii_lowercase(), proc);
        }
        self.window_class_atoms.insert(atom, proc);
        atom
    }

    fn register_window_message(&mut self, name: &str) -> u32 {
        if name.is_empty() {
            return 0;
        }
        if let Some(message) = self.registered_window_messages.get(name).copied() {
            return message;
        }
        let message = self.next_registered_window_message;
        self.next_registered_window_message = self
            .next_registered_window_message
            .saturating_add(1)
            .min(0xffff);
        self.registered_window_messages
            .insert(name.to_string(), message);
        message
    }

    fn set_window_class_menu(&mut self, name: &str, atom: u32, menu: u32) {
        if menu == 0 {
            return;
        }
        if !name.is_empty() {
            self.window_class_menus
                .insert(name.to_ascii_lowercase(), menu);
        }
        self.window_class_atom_menus.insert(atom, menu);
    }

    fn set_window_class_background(&mut self, name: &str, atom: u32, brush: u32) {
        if brush == 0 {
            return;
        }
        if !name.is_empty() {
            self.window_class_backgrounds
                .insert(name.to_ascii_lowercase(), brush);
        }
        self.window_class_atom_backgrounds.insert(atom, brush);
    }

    fn window_proc_for_class(&self, name: &str, atom: u32) -> Option<u32> {
        if atom != 0 {
            return self.window_class_atoms.get(&atom).copied();
        }
        self.window_class_procs
            .get(&name.to_ascii_lowercase())
            .copied()
    }

    fn window_menu_for_class(&self, name: &str, atom: u32) -> Option<u32> {
        if atom != 0 {
            return self.window_class_atom_menus.get(&atom).copied();
        }
        self.window_class_menus
            .get(&name.to_ascii_lowercase())
            .copied()
    }

    fn window_background_for_class(&self, name: &str, atom: u32) -> u32 {
        if atom != 0 {
            return self
                .window_class_atom_backgrounds
                .get(&atom)
                .copied()
                .unwrap_or(0);
        }
        self.window_class_backgrounds
            .get(&name.to_ascii_lowercase())
            .copied()
            .unwrap_or(0)
    }

    fn window(&self, hwnd: u32) -> Option<&HleWindow> {
        self.windows.get(&hwnd)
    }

    fn window_mut(&mut self, hwnd: u32) -> Option<&mut HleWindow> {
        self.windows.get_mut(&hwnd)
    }

    fn mark_window_ddraw_owned(&mut self, hwnd: u32) -> bool {
        let hwnd = if hwnd == 0 { 0x0002_0001 } else { hwnd };
        let Some(window) = self.windows.get_mut(&hwnd) else {
            return false;
        };
        window.ddraw_owned = true;
        true
    }

    fn control_by_id(&self, parent: u32, id: u32) -> Option<&HleWindow> {
        self.windows
            .values()
            .find(|window| window.parent == parent && window.id == id)
    }

    fn control_by_id_mut(&mut self, parent: u32, id: u32) -> Option<&mut HleWindow> {
        self.windows
            .values_mut()
            .find(|window| window.parent == parent && window.id == id)
    }

    fn child_at(&self, parent: u32, x: i32, y: i32) -> Option<&HleWindow> {
        let mut children = self
            .windows
            .values()
            .filter(|window| window.parent == parent && window.enabled && window.visible)
            .filter(|window| window.rect.contains(x, y))
            .map(|window| window.hwnd)
            .collect::<Vec<_>>();
        children.sort_unstable_by(|a, b| b.cmp(a));
        for hwnd in children {
            if let Some(child) = self.child_at(hwnd, x, y) {
                return Some(child);
            }
            if let Some(window) = self.window(hwnd) {
                return Some(window);
            }
        }
        None
    }

    fn top_window_at(&self, x: i32, y: i32) -> Option<&HleWindow> {
        self.windows
            .values()
            .filter(|window| window.parent == 0 && window.visible && window.rect.contains(x, y))
            .max_by_key(|window| window.hwnd)
    }

    fn handle_mut(&mut self, h: u32) -> Option<&mut Handle> {
        let idx = h.checked_sub(0x100)? as usize;
        self.handles.get_mut(idx)?.as_mut()
    }

    fn close_handle(&mut self, h: u32) -> bool {
        let Some(idx) = h.checked_sub(0x100).map(|x| x as usize) else {
            return false;
        };
        if let Some(slot) = self.handles.get_mut(idx) {
            return slot.take().is_some();
        }
        false
    }

    fn ensure_ddraw_tables(&mut self, mem: &mut Memory) -> Result<()> {
        if self.ddraw_vtable != 0 {
            return Ok(());
        }
        self.ddraw_vtable = self.alloc_private(mem, 23 * 4, PagePerm::READ | PagePerm::WRITE)?;
        let ddraw_methods = [
            ("DDraw_QueryInterface", hle_ok_this as HleCallback),
            ("DDraw_AddRef", hle_ret_one as HleCallback),
            ("DDraw_Release", hle_ret_one as HleCallback),
            ("DDraw_Compact", hle_ret_ok_4 as HleCallback),
            (
                "DDraw_CreateClipper",
                hle_ddraw_create_clipper as HleCallback,
            ),
            (
                "DDraw_CreatePalette",
                hle_ddraw_create_palette as HleCallback,
            ),
            (
                "DDraw_CreateSurface",
                hle_ddraw_create_surface as HleCallback,
            ),
            ("DDraw_DuplicateSurface", hle_ret_notimpl_12 as HleCallback),
            (
                "DDraw_EnumDisplayModes",
                hle_ddraw_enum_display_modes as HleCallback,
            ),
            ("DDraw_EnumSurfaces", hle_ret_ok_20 as HleCallback),
            ("DDraw_FlipToGDISurface", hle_ret_ok_4 as HleCallback),
            ("DDraw_GetCaps", hle_ret_ok_12 as HleCallback),
            (
                "DDraw_GetDisplayMode",
                hle_ddraw_get_display_mode as HleCallback,
            ),
            ("DDraw_GetFourCCCodes", hle_ret_ok_12 as HleCallback),
            ("DDraw_GetGDISurface", hle_ret_notimpl_8 as HleCallback),
            (
                "DDraw_GetMonitorFrequency",
                hle_write_60_ret_ok as HleCallback,
            ),
            ("DDraw_GetScanLine", hle_write_zero_ret_ok as HleCallback),
            (
                "DDraw_GetVerticalBlankStatus",
                hle_write_zero_ret_ok as HleCallback,
            ),
            ("DDraw_Initialize", hle_ret_ok_8 as HleCallback),
            (
                "DDraw_RestoreDisplayMode",
                hle_ddraw_restore_display_mode as HleCallback,
            ),
            (
                "DDraw_SetCooperativeLevel",
                hle_ddraw_set_cooperative_level as HleCallback,
            ),
            (
                "DDraw_SetDisplayMode",
                hle_ddraw_set_display_mode as HleCallback,
            ),
            ("DDraw_WaitForVerticalBlank", hle_ret_ok_12 as HleCallback),
        ];
        for (i, (name, cb)) in ddraw_methods.iter().enumerate() {
            let addr = self.register_symbol("ddraw.com", name, *cb);
            mem.write_u32(self.ddraw_vtable + (i as u32 * 4), addr)?;
        }

        self.ddraw_surface_vtable =
            self.alloc_private(mem, 36 * 4, PagePerm::READ | PagePerm::WRITE)?;
        let surface_methods = [
            ("DDS_QueryInterface", hle_ok_this as HleCallback),
            ("DDS_AddRef", hle_ret_one as HleCallback),
            ("DDS_Release", hle_ret_one as HleCallback),
            (
                "DDS_AddAttachedSurface",
                hle_surface_add_attached as HleCallback,
            ),
            ("DDS_AddOverlayDirtyRect", hle_ret_ok_8 as HleCallback),
            ("DDS_Blt", hle_surface_blt as HleCallback),
            ("DDS_BltBatch", hle_ret_ok_16 as HleCallback),
            ("DDS_BltFast", hle_surface_blt_fast as HleCallback),
            ("DDS_DeleteAttachedSurface", hle_ret_ok_12 as HleCallback),
            ("DDS_EnumAttachedSurfaces", hle_ret_ok_12 as HleCallback),
            ("DDS_EnumOverlayZOrders", hle_ret_ok_16 as HleCallback),
            ("DDS_Flip", hle_surface_flip as HleCallback),
            (
                "DDS_GetAttachedSurface",
                hle_surface_get_attached as HleCallback,
            ),
            ("DDS_GetBltStatus", hle_ret_ok_8 as HleCallback),
            ("DDS_GetCaps", hle_surface_get_caps as HleCallback),
            ("DDS_GetClipper", hle_ret_notimpl_8 as HleCallback),
            ("DDS_GetColorKey", hle_surface_get_color_key as HleCallback),
            ("DDS_GetDC", hle_surface_get_dc as HleCallback),
            ("DDS_GetFlipStatus", hle_ret_ok_8 as HleCallback),
            (
                "DDS_GetOverlayPosition",
                hle_write_zero2_ret_ok as HleCallback,
            ),
            ("DDS_GetPalette", hle_surface_get_palette as HleCallback),
            (
                "DDS_GetPixelFormat",
                hle_surface_get_pixel_format as HleCallback,
            ),
            ("DDS_GetSurfaceDesc", hle_surface_get_desc as HleCallback),
            ("DDS_Initialize", hle_ret_ok_12 as HleCallback),
            ("DDS_IsLost", hle_ret_ok_4 as HleCallback),
            ("DDS_Lock", hle_surface_lock as HleCallback),
            ("DDS_ReleaseDC", hle_surface_release_dc as HleCallback),
            ("DDS_Restore", hle_ret_ok_4 as HleCallback),
            ("DDS_SetClipper", hle_ret_ok_8 as HleCallback),
            ("DDS_SetColorKey", hle_surface_set_color_key as HleCallback),
            ("DDS_SetOverlayPosition", hle_ret_ok_12 as HleCallback),
            ("DDS_SetPalette", hle_surface_set_palette as HleCallback),
            ("DDS_Unlock", hle_surface_unlock as HleCallback),
            ("DDS_UpdateOverlay", hle_ret_ok_24 as HleCallback),
            ("DDS_UpdateOverlayDisplay", hle_ret_ok_8 as HleCallback),
            ("DDS_UpdateOverlayZOrder", hle_ret_ok_12 as HleCallback),
        ];
        for (i, (name, cb)) in surface_methods.iter().enumerate() {
            let addr = self.register_symbol("ddraw.com", name, *cb);
            mem.write_u32(self.ddraw_surface_vtable + (i as u32 * 4), addr)?;
        }

        self.ddraw_palette_vtable =
            self.alloc_private(mem, 7 * 4, PagePerm::READ | PagePerm::WRITE)?;
        for (i, (name, cb)) in [
            ("DDP_QueryInterface", hle_ok_this as HleCallback),
            ("DDP_AddRef", hle_ret_one as HleCallback),
            ("DDP_Release", hle_ret_one as HleCallback),
            ("DDP_GetCaps", hle_write_zero_ret_ok as HleCallback),
            ("DDP_GetEntries", hle_palette_get_entries as HleCallback),
            ("DDP_Initialize", hle_ret_ok_16 as HleCallback),
            ("DDP_SetEntries", hle_palette_set_entries as HleCallback),
        ]
        .iter()
        .enumerate()
        {
            let addr = self.register_symbol("ddraw.com", name, *cb);
            mem.write_u32(self.ddraw_palette_vtable + (i as u32 * 4), addr)?;
        }

        self.ddraw_clipper_vtable =
            self.alloc_private(mem, 9 * 4, PagePerm::READ | PagePerm::WRITE)?;
        for (i, (name, cb)) in [
            ("DDC_QueryInterface", hle_ok_this as HleCallback),
            ("DDC_AddRef", hle_ret_one as HleCallback),
            ("DDC_Release", hle_ret_one as HleCallback),
            ("DDC_GetClipList", hle_ret_ok_16 as HleCallback),
            ("DDC_GetHWnd", hle_write_zero_ret_ok as HleCallback),
            ("DDC_Initialize", hle_ret_ok_12 as HleCallback),
            (
                "DDC_IsClipListChanged",
                hle_write_zero_ret_ok as HleCallback,
            ),
            ("DDC_SetClipList", hle_ret_ok_12 as HleCallback),
            ("DDC_SetHWnd", hle_clipper_set_hwnd as HleCallback),
        ]
        .iter()
        .enumerate()
        {
            let addr = self.register_symbol("ddraw.com", name, *cb);
            mem.write_u32(self.ddraw_clipper_vtable + (i as u32 * 4), addr)?;
        }
        Ok(())
    }

    fn ensure_dinput_tables(&mut self, mem: &mut Memory) -> Result<()> {
        if self.dinput_vtable != 0 {
            return Ok(());
        }

        self.dinput_vtable = self.alloc_private(mem, 8 * 4, PagePerm::READ | PagePerm::WRITE)?;
        let dinput_methods = [
            ("DI_QueryInterface", hle_ok_this as HleCallback),
            ("DI_AddRef", hle_com_add_ref as HleCallback),
            ("DI_Release", hle_com_release as HleCallback),
            ("DI_CreateDevice", hle_dinput_create_device as HleCallback),
            ("DI_EnumDevices", hle_dinput_enum_devices as HleCallback),
            ("DI_GetDeviceStatus", hle_dinput_get_device_status as HleCallback),
            ("DI_RunControlPanel", hle_ret_ok_12 as HleCallback),
            ("DI_Initialize", hle_ret_ok_12 as HleCallback),
        ];
        for (i, (name, cb)) in dinput_methods.iter().enumerate() {
            let addr = self.register_symbol("dinput.com", name, *cb);
            mem.write_u32(self.dinput_vtable + (i as u32 * 4), addr)?;
        }

        self.dinput_device_vtable =
            self.alloc_private(mem, 18 * 4, PagePerm::READ | PagePerm::WRITE)?;
        let device_methods = [
            ("DID_QueryInterface", hle_ok_this as HleCallback),
            ("DID_AddRef", hle_com_add_ref as HleCallback),
            ("DID_Release", hle_com_release as HleCallback),
            (
                "DID_GetCapabilities",
                hle_dinput_device_get_capabilities as HleCallback,
            ),
            ("DID_EnumObjects", hle_ret_ok_16 as HleCallback),
            ("DID_GetProperty", hle_ret_ok_12 as HleCallback),
            ("DID_SetProperty", hle_ret_ok_12 as HleCallback),
            ("DID_Acquire", hle_dinput_device_acquire as HleCallback),
            ("DID_Unacquire", hle_dinput_device_unacquire as HleCallback),
            (
                "DID_GetDeviceState",
                hle_dinput_device_get_device_state as HleCallback,
            ),
            (
                "DID_GetDeviceData",
                hle_dinput_device_get_device_data as HleCallback,
            ),
            ("DID_SetDataFormat", hle_ret_ok_8 as HleCallback),
            ("DID_SetEventNotification", hle_ret_ok_8 as HleCallback),
            ("DID_SetCooperativeLevel", hle_ret_ok_12 as HleCallback),
            ("DID_GetObjectInfo", hle_ret_ok_16 as HleCallback),
            (
                "DID_GetDeviceInfo",
                hle_dinput_device_get_device_info as HleCallback,
            ),
            ("DID_RunControlPanel", hle_ret_ok_12 as HleCallback),
            ("DID_Initialize", hle_ret_ok_16 as HleCallback),
        ];
        for (i, (name, cb)) in device_methods.iter().enumerate() {
            let addr = self.register_symbol("dinput.com", name, *cb);
            mem.write_u32(self.dinput_device_vtable + (i as u32 * 4), addr)?;
        }

        Ok(())
    }

    fn ensure_dsound_tables(&mut self, mem: &mut Memory) -> Result<()> {
        if self.dsound_vtable != 0 {
            return Ok(());
        }

        self.dsound_vtable = self.alloc_private(mem, 11 * 4, PagePerm::READ | PagePerm::WRITE)?;
        let dsound_methods = [
            ("DS_QueryInterface", hle_ok_this as HleCallback),
            ("DS_AddRef", hle_com_add_ref as HleCallback),
            ("DS_Release", hle_com_release as HleCallback),
            (
                "DS_CreateSoundBuffer",
                hle_dsound_create_sound_buffer as HleCallback,
            ),
            ("DS_GetCaps", hle_dsound_get_caps as HleCallback),
            (
                "DS_DuplicateSoundBuffer",
                hle_dsound_duplicate_sound_buffer as HleCallback,
            ),
            ("DS_SetCooperativeLevel", hle_ret_ok_12 as HleCallback),
            ("DS_Compact", hle_ret_ok_4 as HleCallback),
            (
                "DS_GetSpeakerConfig",
                hle_dsound_get_speaker_config as HleCallback,
            ),
            ("DS_SetSpeakerConfig", hle_ret_ok_8 as HleCallback),
            ("DS_Initialize", hle_ret_ok_8 as HleCallback),
        ];
        for (i, (name, cb)) in dsound_methods.iter().enumerate() {
            let addr = self.register_symbol("dsound.com", name, *cb);
            mem.write_u32(self.dsound_vtable + (i as u32 * 4), addr)?;
        }

        self.dsound_buffer_vtable =
            self.alloc_private(mem, 21 * 4, PagePerm::READ | PagePerm::WRITE)?;
        let buffer_methods = [
            ("DSB_QueryInterface", hle_ok_this as HleCallback),
            ("DSB_AddRef", hle_com_add_ref as HleCallback),
            ("DSB_Release", hle_com_release as HleCallback),
            ("DSB_GetCaps", hle_dsound_buffer_get_caps as HleCallback),
            (
                "DSB_GetCurrentPosition",
                hle_dsound_buffer_get_current_position as HleCallback,
            ),
            ("DSB_GetFormat", hle_dsound_buffer_get_format as HleCallback),
            ("DSB_GetVolume", hle_write_zero_ret_ok as HleCallback),
            ("DSB_GetPan", hle_write_zero_ret_ok as HleCallback),
            ("DSB_GetFrequency", hle_write_zero_ret_ok as HleCallback),
            ("DSB_GetStatus", hle_dsound_buffer_get_status as HleCallback),
            ("DSB_Initialize", hle_ret_ok_12 as HleCallback),
            ("DSB_Lock", hle_dsound_buffer_lock as HleCallback),
            ("DSB_Play", hle_dsound_buffer_play as HleCallback),
            (
                "DSB_SetCurrentPosition",
                hle_dsound_buffer_set_current_position as HleCallback,
            ),
            ("DSB_SetFormat", hle_ret_ok_8 as HleCallback),
            ("DSB_SetVolume", hle_ret_ok_8 as HleCallback),
            ("DSB_SetPan", hle_ret_ok_8 as HleCallback),
            ("DSB_SetFrequency", hle_ret_ok_8 as HleCallback),
            ("DSB_Stop", hle_dsound_buffer_stop as HleCallback),
            ("DSB_Unlock", hle_ret_ok_20 as HleCallback),
            ("DSB_Restore", hle_ret_ok_4 as HleCallback),
        ];
        for (i, (name, cb)) in buffer_methods.iter().enumerate() {
            let addr = self.register_symbol("dsound.com", name, *cb);
            mem.write_u32(self.dsound_buffer_vtable + (i as u32 * 4), addr)?;
        }

        Ok(())
    }
}

fn mouse_lparam(x: i32, y: i32) -> u32 {
    (((y as u32) & 0xffff) << 16) | ((x as u32) & 0xffff)
}

fn virtual_key_for_char(ch: u32) -> Option<u32> {
    match ch {
        0x61..=0x7a => Some(ch - 32),
        0x41..=0x5a | 0x30..=0x39 => Some(ch),
        0x20 => Some(0x20),
        0x0a | 0x0d => Some(0x0d),
        _ => None,
    }
}

fn char_requires_shift(ch: u32) -> bool {
    matches!(ch, 0x41..=0x5a)
}

fn format_message(message: Message) -> String {
    format!(
        "{{hwnd={:08x},msg={:04x},w={:08x},l={:08x}}}",
        message.hwnd, message.msg, message.wparam, message.lparam
    )
}

impl Default for Hle {
    fn default() -> Self {
        Self::new()
    }
}

fn symbol_key(dll: &str, name: &str) -> String {
    format!("{}!{}", dll.to_ascii_lowercase(), name.to_ascii_lowercase())
}

fn is_hle_runtime_dll(dll: &str) -> bool {
    let normalized = dll.replace('/', "\\");
    let name = normalized
        .rsplit('\\')
        .next()
        .unwrap_or(normalized.as_str())
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "advapi32.dll"
            | "comctl32.dll"
            | "comdlg32.dll"
            | "ddraw.dll"
            | "dinput.dll"
            | "dsound.dll"
            | "gdi32.dll"
            | "imm32.dll"
            | "kernel32.dll"
            | "msacm32.dll"
            | "msvcrt.dll"
            | "ntdll.dll"
            | "ole32.dll"
            | "oleaut32.dll"
            | "shell32.dll"
            | "user32.dll"
            | "version.dll"
            | "winmm.dll"
            | "winspool.drv"
            | "winspool.dll"
            | "wsock32.dll"
            | "ws2_32.dll"
    )
}

#[inline(always)]
fn hle_index(addr: u32) -> Option<usize> {
    let offset = addr.checked_sub(HLE_BASE)?;
    if offset % HLE_STRIDE != 0 {
        return None;
    }
    Some((offset / HLE_STRIDE) as usize)
}

fn leak_hle_str(value: &str) -> &'static str {
    Box::leak(value.to_string().into_boxed_str())
}

fn import_callback_for(dll: &str, name: &str) -> Option<HleCallback> {
    let dll_name = dll
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(dll)
        .to_ascii_lowercase();
    if matches!(dll_name.as_str(), "wsock32.dll" | "ws2_32.dll") {
        if let Some(callback) = callback_for_winsock(name) {
            return Some(callback);
        }
    }
    if matches!(dll_name.as_str(), "comctl32" | "comctl32.dll") {
        if let Some(callback) = comctl32_import_callback_for(name) {
            return Some(callback);
        }
    }
    callback_for_known(name)
}

fn comctl32_import_callback_for(name: &str) -> Option<HleCallback> {
    match name {
        // COMCTL32 ordinal 17 is InitCommonControls on Win9x/NT-era exports.
        "#17" => Some(hle_init_common_controls),
        _ => None,
    }
}

fn callback_for_known(name: &str) -> Option<HleCallback> {
    let callback = match name {
        "ExitProcess" => hle_exit_process,
        "ExitThread" => hle_exit_thread,
        "InitCommonControls" => hle_init_common_controls,
        "InitCommonControlsEx" => hle_init_common_controls_ex,
        "ImageList_Destroy" => hle_image_list_destroy,
        "VirtualAlloc" => hle_virtual_alloc,
        "VirtualFree" => hle_virtual_free,
        "VirtualProtect" => hle_virtual_protect,
        "VirtualQuery" => hle_virtual_query,
        "CreateFileA" => hle_create_file_a,
        "ReadFile" => hle_read_file,
        "WriteFile" => hle_write_file,
        "CloseHandle" => hle_close_handle,
        "GetFileSize" => hle_get_file_size,
        "GetFileType" => hle_get_file_type,
        "SetFilePointer" => hle_set_file_pointer,
        "SetFilePointerEx" => hle_set_file_pointer_ex,
        "SetEndOfFile" => hle_set_end_of_file,
        "FlushFileBuffers" => hle_flush_file_buffers,
        "GetOverlappedResult" => hle_get_overlapped_result,
        "DeviceIoControl" => hle_device_io_control,
        "OpenFile" => hle_open_file,
        "_lopen" => hle_lopen,
        "_lcreat" => hle_lcreat,
        "_lread" | "_hread" => hle_lread,
        "_lwrite" | "_hwrite" => hle_lwrite,
        "_llseek" => hle_llseek,
        "_lclose" => hle_lclose,
        "GetLogicalDrives" => hle_get_logical_drives,
        "GetLogicalDriveStringsA" => hle_get_logical_drive_strings_a,
        "DeleteFileA" => hle_delete_file_a,
        "DeleteFileW" => hle_delete_file_w,
        "AreFileApisANSI" => hle_are_file_apis_ansi,
        "SetFileApisToANSI" => hle_set_file_apis_to_ansi,
        "SetFileApisToOEM" => hle_set_file_apis_to_oem,
        "MoveFileA" => hle_move_file_a,
        "MoveFileExA" => hle_move_file_ex_a,
        "CreateDirectoryA" => hle_create_directory_a,
        "RemoveDirectoryA" => hle_remove_directory_a,
        "SetFileAttributesA" => hle_set_file_attributes_a,
        "FindFirstFileA" => hle_find_first_file_a,
        "FindFirstFileExW" => hle_find_first_file_ex_w,
        "FindNextFileA" => hle_find_next_file_a,
        "FindNextFileW" => hle_find_next_file_w,
        "FindClose" => hle_find_close,
        "GetFileAttributesA" => hle_get_file_attributes_a,
        "GetFileAttributesW" => hle_get_file_attributes_w,
        "GetFileAttributesExW" => hle_get_file_attributes_ex_w,
        "GetFullPathNameA" => hle_get_full_path_name_a,
        "GetFullPathNameW" => hle_get_full_path_name_w,
        "GetCurrentDirectoryA" => hle_get_current_directory_a,
        "GetCurrentDirectoryW" => hle_get_current_directory_w,
        "SetCurrentDirectoryA" => hle_set_current_directory_a,
        "WinExec" => hle_win_exec,
        "GetProcessAffinityMask" => hle_get_process_affinity_mask,
        "GetSystemDirectoryA" => hle_get_system_directory_a,
        "GetWindowsDirectoryA" => hle_get_windows_directory_a,
        "GetWindowsDirectoryW" => hle_get_windows_directory_w,
        "GetDriveTypeA" => hle_get_drive_type_a,
        "GetVolumeInformationA" => hle_get_volume_information_a,
        "GetDiskFreeSpaceA" => hle_get_disk_free_space_a,
        "GlobalMemoryStatus" => hle_global_memory_status,
        "CreateFileW" => hle_create_file_w,
        "CreateFileMappingA" => hle_create_file_mapping_a,
        "CreateFileMappingW" => hle_create_file_mapping_w,
        "OpenFileMappingA" => hle_open_file_mapping_a,
        "OpenFileMappingW" => hle_open_file_mapping_w,
        "MapViewOfFile" => hle_map_view_of_file,
        "UnmapViewOfFile" => hle_unmap_view_of_file,
        "CreateMutexA" => hle_create_mutex_a,
        "CreateMutexW" => hle_create_mutex_w,
        "CreateSemaphoreA" => hle_create_semaphore_a,
        "CreateSemaphoreW" => hle_create_semaphore_w,
        "GetModuleFileNameA" => hle_get_module_file_name_a,
        "GetModuleFileNameW" => hle_get_module_file_name_w,
        "GetCommandLineA" => hle_get_command_line_a,
        "GetCommandLineW" => hle_get_command_line_w,
        "GetEnvironmentStrings" => hle_get_environment_strings,
        "GetEnvironmentStringsW" => hle_get_environment_strings_w,
        "FreeEnvironmentStringsA" => hle_free_environment_strings_a,
        "FreeEnvironmentStringsW" => hle_free_environment_strings_w,
        "GetEnvironmentVariableA" => hle_get_environment_variable_a,
        "GetEnvironmentVariableW" => hle_get_environment_variable_w,
        "GetModuleHandleA" => hle_get_module_handle_a,
        "GetModuleHandleW" => hle_get_module_handle_w,
        "GetModuleHandleExA" => hle_get_module_handle_ex_a,
        "GetModuleHandleExW" => hle_get_module_handle_ex_w,
        "LoadLibraryA" => hle_load_library_a,
        "LoadLibraryW" => hle_load_library_w,
        "LoadLibraryExA" => hle_load_library_ex_a,
        "LoadLibraryExW" => hle_load_library_ex_w,
        "LoadModule" => hle_load_module,
        "FreeLibrary" => hle_free_library,
        "GetProcAddress" => hle_get_proc_address,
        "FindResourceA" => hle_find_resource_a,
        "FindResourceW" => hle_find_resource_w,
        "LoadResource" => hle_load_resource,
        "LockResource" => hle_lock_resource,
        "SizeofResource" => hle_sizeof_resource,
        "FreeResource" => hle_free_resource,
        "GetLastError" => hle_get_last_error,
        "SetLastError" => hle_set_last_error,
        "DecodePointer" => hle_decode_pointer,
        "SetErrorMode" => hle_set_error_mode,
        "GetErrorMode" => hle_get_error_mode,
        "GetTickCount" | "timeGetTime" => hle_get_tick_count,
        "QueryPerformanceFrequency" => hle_query_performance_frequency,
        "GetSystemTimeAsFileTime" => hle_get_system_time_as_file_time,
        "DosDateTimeToFileTime" => hle_dos_date_time_to_file_time,
        "FileTimeToDosDateTime" => hle_file_time_to_dos_date_time,
        "FileTimeToLocalFileTime" => hle_file_time_to_local_file_time,
        "LocalFileTimeToFileTime" => hle_local_file_time_to_file_time,
        "FileTimeToSystemTime" => hle_file_time_to_system_time,
        "SystemTimeToFileTime" => hle_system_time_to_file_time,
        "GetFileTime" => hle_get_file_time,
        "GetFileInformationByHandle" => hle_get_file_information_by_handle,
        "SetFileTime" => hle_set_file_time,
        "GetLocalTime" => hle_get_local_time,
        "GetSystemTime" => hle_get_system_time,
        "GetTimeZoneInformation" => hle_get_time_zone_information,
        "WritePrivateProfileStringA" => hle_write_private_profile_string_a,
        "GetProfileIntA" => hle_get_profile_int_a,
        "GetPrivateProfileIntA" => hle_get_private_profile_int_a,
        "GetPrivateProfileIntW" => hle_get_private_profile_int_w,
        "GetPrivateProfileStringA" => hle_get_private_profile_string_a,
        "GetPrivateProfileStringW" => hle_get_private_profile_string_w,
        "GetDateFormatW" => hle_get_date_format_w,
        "GetTimeFormatW" => hle_get_time_format_w,
        "EnumCalendarInfoA" => hle_enum_calendar_info_a,
        "GetLocaleInfoW" => hle_get_locale_info_w,
        "GetLocaleInfoA" => hle_get_locale_info_a,
        "GetThreadLocale" => hle_get_thread_locale,
        "SetThreadLocale" => hle_set_thread_locale,
        "GetSystemDefaultLangID" => hle_get_system_default_lang_id,
        "GetUserDefaultLCID" => hle_get_user_default_lcid,
        "IsValidLocale" => hle_is_valid_locale,
        "LCMapStringA" => hle_lc_map_string_a,
        "LCMapStringEx" => hle_lc_map_string_ex,
        "LCMapStringW" => hle_lc_map_string_w,
        "GetStringTypeA" => hle_get_string_type_a,
        "GetStringTypeW" => hle_get_string_type_w,
        "CompareStringA" => hle_compare_string_a,
        "CompareStringW" => hle_compare_string_w,
        "QueryPerformanceCounter" => hle_query_performance_counter,
        "GetVersion" => hle_get_version,
        "GetVersionExA" => hle_get_version_ex_a,
        "GetVersionExW" => hle_get_version_ex_w,
        "GetProcessVersion" => hle_get_process_version,
        "InterlockedDecrement" => hle_interlocked_decrement,
        "InterlockedExchange" => hle_interlocked_exchange,
        "InterlockedIncrement" => hle_interlocked_increment,
        "GetSystemInfo" => hle_get_system_info,
        "GetExitCodeProcess" => hle_get_exit_code_process,
        "GetExitCodeThread" => hle_get_exit_code_thread,
        "IsBadCodePtr" => hle_is_bad_code_ptr,
        "IsBadReadPtr" => hle_is_bad_read_ptr,
        "IsBadWritePtr" => hle_is_bad_write_ptr,
        "WSAStartup" => hle_wsa_startup,
        "WSACleanup" => hle_wsa_cleanup,
        "WSAGetLastError" => hle_wsa_get_last_error,
        "WSASetLastError" => hle_wsa_set_last_error,
        "WSAAsyncSelect" => hle_wsa_async_select,
        "WSAAsyncGetHostByAddr" => hle_wsa_async_get_host_by_addr,
        "WSAAsyncGetHostByName" => hle_wsa_async_get_host_by_name,
        "WSACancelAsyncRequest" => hle_wsa_cancel_async_request,
        "accept" => hle_accept,
        "bind" => hle_bind,
        "getsockopt" => hle_getsockopt,
        "htonl" => hle_htonl,
        "htons" => hle_htons,
        "inet_addr" => hle_inet_addr,
        "inet_ntoa" => hle_inet_ntoa,
        "ioctlsocket" => hle_ioctlsocket,
        "listen" => hle_listen,
        "gethostbyname" => hle_gethostbyname,
        "GetCurrentProcess" => hle_get_current_process,
        "SetHandleCount" => hle_set_handle_count,
        "SetThreadPriority" => hle_set_thread_priority,
        "SetPriorityClass" => hle_set_priority_class,
        "GetPriorityClass" => hle_get_priority_class,
        "CreateProcessA" => hle_create_process_a,
        "DuplicateHandle" => hle_duplicate_handle,
        "GetThreadContext" => hle_get_thread_context,
        "ResetEvent" => hle_reset_event,
        "OutputDebugStringA" => hle_output_debug_string_a,
        "ClearCommBreak" => hle_clear_comm_break,
        "SetCommBreak" => hle_set_comm_break,
        "EscapeCommFunction" => hle_escape_comm_function,
        "SetupComm" => hle_setup_comm,
        "PurgeComm" => hle_purge_comm,
        "GetCommModemStatus" => hle_get_comm_modem_status,
        "ClearCommError" => hle_clear_comm_error,
        "GetCommState" => hle_get_comm_state,
        "SetCommState" => hle_set_comm_state,
        "SetCommTimeouts" => hle_set_comm_timeouts,
        "TerminateThread" => hle_terminate_thread,
        "RtlUnwind" => hle_rtl_unwind,
        "GetProcessHeap" => hle_get_process_heap,
        "DebugBreak" => hle_debug_break,
        "DisableThreadLibraryCalls" => hle_disable_thread_library_calls,
        "GetStartupInfoW" => hle_get_startup_info_w,
        "GetStartupInfoA" => hle_get_startup_info_a,
        "GetUserDefaultLangID" => hle_get_user_default_lang_id,
        "GetUserDefaultUILanguage" => hle_get_user_default_ui_language,
        "ImmGetContext" => hle_imm_get_context,
        "ImmAssociateContext" => hle_imm_associate_context,
        "ImmReleaseContext" => hle_imm_release_context,
        "ImmSetOpenStatus" => hle_imm_set_open_status,
        "ImmNotifyIME" => hle_imm_notify_ime,
        "TerminateProcess" => hle_terminate_process,
        "MulDiv" => hle_mul_div,
        "HeapCreate" => hle_heap_create,
        "HeapDestroy" => hle_heap_destroy,
        "HeapAlloc" => hle_heap_alloc,
        "HeapFree" => hle_heap_free,
        "HeapReAlloc" => hle_heap_re_alloc,
        "HeapSize" => hle_heap_size,
        "HeapValidate" => hle_heap_validate,
        "GlobalAlloc" => hle_global_alloc,
        "GlobalReAlloc" => hle_global_re_alloc,
        "GlobalFree" => hle_global_free,
        "GlobalLock" => hle_global_lock,
        "GlobalUnlock" => hle_global_unlock,
        "GlobalHandle" => hle_global_handle,
        "GlobalSize" => hle_global_size,
        "LocalAlloc" => hle_local_alloc,
        "LocalFree" => hle_local_free,
        "LocalLock" => hle_local_lock,
        "LocalUnlock" => hle_local_unlock,
        "LocalReAlloc" => hle_local_re_alloc,
        "FormatMessageW" => hle_format_message_w,
        "FormatMessageA" => hle_format_message_a,
        "lstrcpyA" => hle_lstrcpy_a,
        "OemToCharA" => hle_oem_to_char_a,
        "OemToCharBuffA" => hle_oem_to_char_buff_a,
        "lstrcpyW" => hle_lstrcpy_w,
        "lstrcatA" => hle_lstrcat_a,
        "lstrcatW" => hle_lstrcat_w,
        "lstrcmpA" => hle_lstrcmp_a,
        "lstrcmpiA" => hle_lstrcmpi_a,
        "lstrcpynA" => hle_lstrcpyn_a,
        "lstrcpynW" => hle_lstrcpyn_w,
        "lstrlenA" => hle_lstrlen_a,
        "lstrlenW" => hle_lstrlen_w,
        "GetACP" => hle_get_acp,
        "GetOEMCP" => hle_get_oemcp,
        "GetConsoleCP" => hle_get_console_cp,
        "IsValidCodePage" => hle_is_valid_code_page,
        "GetCPInfo" => hle_get_cp_info,
        "MultiByteToWideChar" => hle_multi_byte_to_wide_char,
        "WideCharToMultiByte" => hle_wide_char_to_multi_byte,
        "TlsAlloc" => hle_tls_alloc,
        "TlsFree" => hle_tls_free,
        "TlsGetValue" => hle_tls_get_value,
        "TlsSetValue" => hle_tls_set_value,
        "FlsAlloc" => hle_fls_alloc,
        "FlsFree" => hle_fls_free,
        "FlsGetValue" => hle_fls_get_value,
        "FlsSetValue" => hle_fls_set_value,
        "OpenProcess" => hle_open_process,
        "IsDebuggerPresent" => hle_is_debugger_present,
        "IsProcessorFeaturePresent" => hle_is_processor_feature_present,
        "FlushInstructionCache" => hle_flush_instruction_cache,
        "CreateEventA" => hle_create_event_a,
        "CreateEventW" => hle_create_event_w,
        "OpenEventA" => hle_open_event_a,
        "OpenEventW" => hle_open_event_w,
        "WaitForSingleObject" => hle_wait_for_single_object,
        "ReleaseMutex" => hle_release_mutex,
        "ReleaseSemaphore" => hle_release_semaphore,
        "Sleep" => hle_sleep,
        "SleepEx" => hle_sleep_ex,
        "__getmainargs" => hle_crt_getmainargs,
        "__wgetmainargs" => hle_crt_wgetmainargs,
        "__initenv" => hle_crt_initenv,
        "__winitenv" => hle_crt_winitenv,
        "__p__commode" => hle_crt_p_commode,
        "__p__fmode" => hle_crt_p_fmode,
        "__dllonexit"
        | "__lconv_init"
        | "__set_app_type"
        | "__setusermatherr"
        | "_c_exit"
        | "_cexit"
        | "_controlfp"
        | "_fpreset"
        | "_initterm"
        | "_lock"
        | "_unlock"
        | "iswctype" => hle_crt_ret_zero,
        "towupper" => hle_crt_towupper,
        "_amsg_exit" | "_exit" | "abort" | "exit" => hle_crt_exit,
        "_except_handler3" => hle_crt_except_handler3,
        "_purecall" => hle_crt_purecall,
        "_XcptFilter" => hle_crt_xcpt_filter,
        "_CIacos" => hle_crt_ci_acos,
        "_CIasin" => hle_crt_ci_asin,
        "_CIatan" => hle_crt_ci_atan,
        "_CIcos" => hle_crt_ci_cos,
        "_CIexp" => hle_crt_ci_exp,
        "_CIlog" => hle_crt_ci_log,
        "_CIlog10" => hle_crt_ci_log10,
        "_CIsin" => hle_crt_ci_sin,
        "_CIsqrt" => hle_crt_ci_sqrt,
        "_CItan" => hle_crt_ci_tan,
        "_iob" => hle_crt_iob,
        "_itoa" => hle_crt_itoa,
        "_ltoa" => hle_crt_ltoa,
        "_wcmdln" => hle_crt_wcmdln,
        "_ultoa" => hle_crt_ultoa,
        "??2@YAPAXI@Z" | "??_U@YAPAXI@Z" => hle_crt_malloc,
        "??3@YAXPAX@Z" | "??_V@YAXPAX@Z" => hle_crt_free,
        "_onexit" | "_set_invalid_parameter_handler" | "signal" => hle_crt_return_arg0,
        "_finite" => hle_crt_finite,
        "_ftol" | "_ftol2" => hle_crt_ftol,
        "_isnan" => hle_crt_isnan,
        "_strdup" => hle_crt_strdup,
        "_stricmp" | "stricmp" => hle_crt_stricmp,
        "_strnicmp" | "strnicmp" => hle_crt_strnicmp,
        "_strupr" => hle_crt_strupr,
        "_wcsicmp" => hle_crt_wcsicmp,
        "_wcsnicmp" => hle_crt_wcsnicmp,
        "_vsnwprintf" => hle_crt_vsnwprintf,
        "atoi" => hle_crt_atoi,
        "atol" => hle_crt_atol,
        "isalnum" => hle_crt_isalnum,
        "isdigit" => hle_crt_isdigit,
        "isspace" => hle_crt_isspace,
        "acos" => hle_crt_acos,
        "asin" => hle_crt_asin,
        "atan" => hle_crt_atan,
        "calloc" => hle_crt_calloc,
        "ceil" => hle_crt_ceil,
        "cos" => hle_crt_cos,
        "cosh" => hle_crt_cosh,
        "exp" => hle_crt_exp,
        "fclose" => hle_crt_fclose,
        "floor" => hle_crt_floor,
        "fmod" => hle_crt_fmod,
        "fopen" => hle_crt_fopen,
        "free" => hle_crt_free,
        "fprintf" => hle_crt_fprintf,
        "fputc" => hle_crt_fputc,
        "fread" => hle_crt_fread,
        "fseek" => hle_crt_fseek,
        "ftell" => hle_crt_ftell,
        "fwrite" => hle_crt_fwrite,
        "log" => hle_crt_log,
        "log10" => hle_crt_log10,
        "malloc" => hle_crt_malloc,
        "memcpy" => hle_crt_memcpy,
        "memmove" => hle_crt_memmove,
        "memset" => hle_crt_memset,
        "modf" => hle_crt_modf,
        "pow" => hle_crt_pow,
        "printf" => hle_crt_printf,
        "sprintf" => hle_crt_sprintf,
        "vsprintf" => hle_crt_vsprintf,
        "sscanf" => hle_crt_sscanf,
        "putchar" => hle_crt_putchar,
        "puts" => hle_crt_puts,
        "rand" => hle_crt_rand,
        "realloc" => hle_crt_realloc,
        "sin" => hle_crt_sin,
        "sinh" => hle_crt_sinh,
        "sqrt" => hle_crt_sqrt,
        "srand" => hle_crt_srand,
        "swprintf" => hle_wsprintf_w,
        "swscanf" => hle_crt_swscanf,
        "tan" => hle_crt_tan,
        "tanh" => hle_crt_tanh,
        "strcmp" => hle_crt_strcmp,
        "strlen" => hle_crt_strlen,
        "strncmp" => hle_crt_strncmp,
        "strstr" => hle_crt_strstr,
        "strcpy" => hle_crt_strcpy,
        "time" => hle_crt_time,
        "vfprintf" => hle_crt_vfprintf,
        "wcscat" => hle_crt_wcscat,
        "wcscmp" => hle_crt_wcscmp,
        "wcscpy" => hle_crt_wcscpy,
        "wcslen" => hle_crt_wcslen,
        "wcsncmp" => hle_crt_wcsncmp,
        "wcsncpy" => hle_crt_wcsncpy,
        "wcschr" => hle_crt_wcschr,
        "wcsrchr" => hle_crt_wcsrchr,
        "wcsstr" => hle_crt_wcsstr,
        "CreateWindowExA" => hle_create_window_ex_a,
        "CreateWindowExW" => hle_create_window_ex_w,
        "CreateDialogParamA" => hle_create_dialog_param_a,
        "CreateDialogParamW" => hle_create_dialog_param_w,
        "CreateDialogIndirectParamA" => hle_create_dialog_indirect_param_a,
        "CreateDialogIndirectParamW" => hle_create_dialog_indirect_param_w,
        "ShowWindow" => hle_show_window,
        "RegisterClassA" => hle_register_class_a,
        "RegisterClassW" => hle_register_class_w,
        "RegisterClassExA" => hle_register_class_ex_a,
        "RegisterClassExW" => hle_register_class_ex_w,
        "UnregisterClassA" => hle_unregister_class_a,
        "UnregisterClassW" => hle_unregister_class_w,
        "RegisterWindowMessageA" => hle_register_window_message_a,
        "RegisterWindowMessageW" => hle_register_window_message_w,
        "CharUpperA" => hle_char_upper_a,
        "EnumChildWindows" => hle_enum_child_windows,
        "EnableWindow" => hle_enable_window,
        "CheckRadioButton" => hle_check_radio_button,
        "CheckDlgButton" => hle_check_dlg_button,
        "CopyRect" => hle_copy_rect,
        "IsDlgButtonChecked" => hle_is_dlg_button_checked,
        "GetMenu" => hle_get_menu,
        "LoadMenuA" => hle_load_menu_a,
        "LoadMenuW" => hle_load_menu_w,
        "GetSubMenu" => hle_get_sub_menu,
        "DeleteMenu" => hle_delete_menu,
        "RemoveMenu" => hle_remove_menu,
        "LoadAcceleratorsA" => hle_load_accelerators_a,
        "LoadAcceleratorsW" => hle_load_accelerators_w,
        "FindWindowA" => hle_find_window_a,
        "PostMessageA" => hle_post_message_a,
        "PostMessageW" => hle_post_message_w,
        "PostThreadMessageA" => hle_post_thread_message_a,
        "PostThreadMessageW" => hle_post_thread_message_w,
        "DispatchMessageA" => hle_dispatch_message_a,
        "DispatchMessageW" => hle_dispatch_message_w,
        "DrawTextExA" => hle_draw_text_ex_a,
        "GetMessageA" => hle_get_message_a,
        "GetMessageW" => hle_get_message_w,
        "WaitMessage" => hle_wait_message,
        "TranslateMessage" => hle_translate_message,
        "TranslateAcceleratorA" => hle_translate_accelerator_a,
        "TranslateAcceleratorW" => hle_translate_accelerator_w,
        "GetDesktopWindow" => hle_get_desktop_window,
        "InvalidateRect" => hle_invalidate_rect,
        "GetUpdateRect" => hle_get_update_rect,
        "GetUpdateRgn" => hle_get_update_rgn,
        "UpdateWindow" => hle_update_window,
        "RedrawWindow" => hle_redraw_window,
        "ExitWindowsEx" => hle_exit_windows_ex,
        "WaitForInputIdle" => hle_wait_for_input_idle,
        "ToAscii" => hle_to_ascii,
        "DdeInitializeA" => hle_dde_initialize_a,
        "DdeUninitialize" => hle_dde_uninitialize,
        "DdeCreateStringHandleA" => hle_dde_create_string_handle_a,
        "DdeQueryStringA" => hle_dde_query_string_a,
        "DdeConnect" => hle_dde_connect,
        "DdeDisconnect" => hle_dde_disconnect,
        "DdeNameService" => hle_dde_name_service,
        "DdeAccessData" => hle_dde_access_data,
        "DdeUnaccessData" => hle_dde_unaccess_data,
        "DdeClientTransaction" => hle_dde_client_transaction,
        "AdjustWindowRect" => hle_adjust_window_rect,
        "AdjustWindowRectEx" => hle_adjust_window_rect_ex,
        "MonitorFromRect" => hle_monitor_from_rect,
        "GetMonitorInfoW" => hle_get_monitor_info_w,
        "PtInRect" => hle_pt_in_rect,
        "SetCapture" => hle_set_capture,
        "ReleaseCapture" => hle_release_capture,
        "ClientToScreen" => hle_client_to_screen,
        "ScreenToClient" => hle_screen_to_client,
        "GetSystemMetrics" => hle_get_system_metrics,
        "SetScrollRange" => hle_set_scroll_range,
        "SetScrollPos" => hle_set_scroll_pos,
        "GetCursorPos" => hle_get_cursor_pos,
        "SetCursorPos" => hle_set_cursor_pos,
        "GetCapture" => hle_get_capture,
        "GetActiveWindow" | "GetForegroundWindow" => hle_get_active_window,
        "GetLastActivePopup" => hle_get_last_active_popup,
        "GetKeyboardState" => hle_get_keyboard_state,
        "GetAsyncKeyState" | "GetKeyState" => hle_get_key_state,
        "GetKeyboardLayoutNameA" => hle_get_keyboard_layout_name_a,
        "GetKeyboardLayoutNameW" => hle_get_keyboard_layout_name_w,
        "MapVirtualKeyA" => hle_map_virtual_key_a,
        "MapVirtualKeyW" => hle_map_virtual_key_w,
        "GetKeyNameTextA" => hle_get_key_name_text_a,
        "GetKeyNameTextW" => hle_get_key_name_text_w,
        "DestroyIcon" => hle_destroy_icon,
        "TrackPopupMenu" => hle_track_popup_menu,
        "MessageBoxA" => hle_message_box_a,
        "MessageBeep" => hle_message_beep,
        "FlashWindow" => hle_flash_window,
        "MessageBoxW" => hle_message_box_w,
        "PeekMessageA" => hle_peek_message_a,
        "PeekMessageW" => hle_peek_message_w,
        "BeginPaint" => hle_begin_paint,
        "GetDC" => hle_get_dc,
        "GetWindowDC" => hle_get_window_dc,
        "ReleaseDC" => hle_release_dc,
        "EndPaint" => hle_end_paint,
        "FillRect" => hle_fill_rect,
        "PostQuitMessage" => hle_post_quit_message,
        "SetTimer" => hle_set_timer,
        "KillTimer" => hle_kill_timer,
        "SetWindowsHookExA" => hle_set_windows_hook_ex_a,
        "UnhookWindowsHookEx" => hle_unhook_windows_hook_ex,
        "CallNextHookEx" => hle_call_next_hook_ex,
        "CreateFontA" => hle_create_font_a,
        "CreateFontIndirectA" => hle_create_font_indirect_a,
        "CreateFontIndirectW" => hle_create_font_indirect_w,
        "CreateStatusWindowW" => hle_create_status_window_w,
        "IsTextUnicode" => hle_is_text_unicode,
        "RegCloseKey" => hle_reg_close_key,
        "RegCreateKeyA" => hle_reg_create_key_a,
        "RegCreateKeyW" => hle_reg_create_key_w,
        "RegCreateKeyExA" => hle_reg_create_key_ex_a,
        "RegCreateKeyExW" => hle_reg_create_key_ex_w,
        "RegOpenKeyExA" => hle_reg_open_key_ex_a,
        "RegOpenKeyExW" => hle_reg_open_key_ex_w,
        "RegOpenKeyA" => hle_reg_open_key_a,
        "RegOpenKeyW" => hle_reg_open_key_w,
        "RegQueryValueA" => hle_reg_query_value_a,
        "RegQueryValueExA" => hle_reg_query_value_ex_a,
        "RegQueryValueExW" => hle_reg_query_value_ex_w,
        "RegQueryInfoKeyA" => hle_reg_query_info_key_a,
        "RegSetValueExA" => hle_reg_set_value_ex_a,
        "RegSetValueExW" => hle_reg_set_value_ex_w,
        "RegEnumValueA" => hle_reg_enum_value_a,
        "RegEnumKeyExA" => hle_reg_enum_key_ex_a,
        "RegDeleteKeyA" => hle_reg_delete_key_a,
        "RegDeleteValueA" => hle_reg_delete_value_a,
        "RegDeleteValueW" => hle_reg_delete_value_w,
        "RegFlushKey" => hle_reg_flush_key,
        "RaiseException" => hle_raise_exception,
        "OpenProcessToken" => hle_open_process_token,
        "LookupPrivilegeValueA" => hle_lookup_privilege_value_a,
        "AdjustTokenPrivileges" => hle_adjust_token_privileges,
        "GetUserNameA" => hle_get_user_name_a,
        "DrawIconEx" | "Pie" => hle_ret_ok_36,
        "CreateThread" => hle_create_thread,
        "Ellipse" | "WriteConsoleA" => hle_ret_ok_20,
        "DrawTextA" => hle_draw_text_a,
        "ExtTextOutA" => hle_ext_text_out_a,
        "TextOutA" => hle_text_out_a,
        "BitBlt" => hle_bit_blt,
        "StretchBlt" => hle_stretch_blt,
        "GetDIBits" => hle_get_dibits,
        "SetDIBitsToDevice" => hle_set_dibits_to_device,
        "StretchDIBits" => hle_stretch_dibits,
        "PatBlt" => hle_pat_blt,
        "BringWindowToTop" => hle_bring_window_to_top,
        "CreateBitmap" => hle_create_bitmap,
        "CreateCompatibleDC" => hle_create_compatible_dc,
        "CreateCompatibleBitmap" => hle_create_compatible_bitmap,
        "CreateDIBitmap" => hle_create_dibitmap,
        "CreateDIBSection" => hle_create_dib_section,
        "CreateICA" => hle_create_ic_a,
        "CreateICW" => hle_create_ic_w,
        "CombineRgn" => hle_combine_rgn,
        "CreatePalette" => hle_create_palette,
        "CreatePatternBrush" => hle_create_pattern_brush,
        "CreateRectRgn" => hle_create_rect_rgn,
        "CreateRectRgnIndirect" => hle_create_rect_rgn_indirect,
        "CreateSolidBrush" => hle_create_solid_brush,
        "CreatePen" => hle_create_pen,
        "LineTo" => hle_line_to,
        "PlaySoundA" | "PlaySoundW" | "auxGetDevCapsA" => hle_ret_ok_12,
        "CoInitialize" => hle_co_initialize,
        "CoUninitialize" => hle_co_uninitialize,
        "OleInitialize" => hle_ole_initialize,
        "OleUninitialize" => hle_ole_uninitialize,
        "CoCreateInstance" => hle_co_create_instance,
        "SysAllocStringLen" => hle_sys_alloc_string_len,
        "SysFreeString" => hle_sys_free_string,
        "SysReAllocStringLen" => hle_sys_re_alloc_string_len,
        "SysStringLen" => hle_sys_string_len,
        "VariantChangeTypeEx" => hle_variant_change_type_ex,
        "VariantClear" => hle_variant_clear,
        "VariantCopyInd" => hle_variant_copy_ind,
        "DefWindowProcA" => hle_def_window_proc_a,
        "DefWindowProcW" => hle_def_window_proc_w,
        "FloodFill" | "ReadConsoleInputA" => hle_ret_ok_16,
        "ReadConsoleW" => hle_read_console_w,
        "ExcludeClipRect" => hle_ret_ok_20,
        "WriteConsoleW" => hle_write_console_w,
        "IntersectRect" => hle_intersect_rect,
        "InflateRect" => hle_inflate_rect,
        "InitializeSListHead" => hle_initialize_slist_head,
        "InterlockedFlushSList" => hle_interlocked_flush_slist,
        "InitializeCriticalSectionAndSpinCount" => hle_initialize_critical_section_and_spin_count,
        "InitializeCriticalSectionEx" => hle_initialize_critical_section_ex,
        "NtSetInformationThread" | "ZwSetInformationThread" => hle_nt_set_information_thread,
        "CharUpperBuffA"
        | "GetConsoleMode"
        | "LoadCursorA"
        | "LoadCursorW"
        | "LoadIconA"
        | "LoadIconW"
        | "SetConsoleCtrlHandler"
        | "SetConsoleMode"
        | "SetEnvironmentVariableA"
        | "SetEnvironmentVariableW"
        | "SetStdHandle"
        | "auxGetVolume"
        | "auxSetVolume"
        | "midiOutGetVolume"
        | "midiOutSetVolume" => hle_ret_ok_8,
        "ShellAboutA" => hle_shell_about_a,
        "ShellAboutW" => hle_ret_ok_16,
        "CommDlgExtendedError" => hle_comm_dlg_extended_error,
        "FindExecutableA" => hle_find_executable_a,
        "ShellExecuteA" => hle_shell_execute_a,
        "ShellExecuteW" => hle_ret_ok_24,
        "ClipCursor" => hle_ret_one,
        "GetKeyboardType" => hle_get_keyboard_type,
        "SelectObject" => hle_select_object,
        "SelectClipRgn" => hle_ret_ok_8,
        "SelectPalette" => hle_select_palette,
        "SetBkColor" => hle_set_bk_color,
        "SetBkMode" => hle_set_bk_mode,
        "SetTextCharacterExtra" => hle_set_text_character_extra,
        "SetTextColor" => hle_set_text_color,
        "AddFontResourceA" => hle_add_font_resource_a,
        "AddFontResourceW" => hle_add_font_resource_w,
        "RemoveFontResourceA" => hle_remove_font_resource_a,
        "RemoveFontResourceW" => hle_remove_font_resource_w,
        "GetObjectA" => hle_get_object_a,
        "GetObjectW" => hle_get_object_w,
        "GetRegionData" => hle_get_region_data,
        "GetSysColorBrush" => hle_get_sys_color_brush,
        "GetStockObject" => hle_get_stock_object,
        "PtVisible" => hle_pt_visible,
        "RectVisible" => hle_rect_visible,
        "LoadBitmapA" => hle_load_bitmap_a,
        "LoadBitmapW" => hle_load_bitmap_w,
        "LoadImageA" => hle_load_image_a,
        "LoadImageW" => hle_load_image_w,
        "RealizePalette" => hle_realize_palette,
        "DeleteCriticalSection"
        | "EnterCriticalSection"
        | "GetStdHandle"
        | "InitializeCriticalSection"
        | "LeaveCriticalSection"
        | "SetEvent"
        | "SetUnhandledExceptionFilter"
        | "ShowCursor"
        | "UnhandledExceptionFilter" => hle_ret_ok_4,
        "DeleteObject" => hle_delete_object,
        "DestroyWindow" => hle_destroy_window,
        "GetClassInfoA" => hle_get_class_info_a,
        "GetClassNameA" => hle_get_class_name_a,
        "GetClassNameW" => hle_get_class_name_w,
        "GetClassLongA" => hle_get_class_long_a,
        "GetDlgCtrlID" => hle_get_dlg_ctrl_id,
        "GetFocus" => hle_get_focus,
        "GetParent" => hle_get_parent,
        "GetTopWindow" => hle_get_top_window,
        "GetWindow" => hle_get_window,
        "GetWindowLongA" => hle_get_window_long_a,
        "GetWindowLongW" => hle_get_window_long_w,
        "GetWindowWord" => hle_get_window_word,
        "IsWindow" => hle_is_window,
        "IsChild" => hle_is_child,
        "IsWindowEnabled" => hle_is_window_enabled,
        "IsWindowVisible" => hle_is_window_visible,
        "GetClientRect" => hle_get_client_rect,
        "GetDeviceCaps" => hle_get_device_caps,
        "GetDCOrgEx" => hle_get_dc_org_ex,
        "GetBkMode" => hle_get_bk_mode,
        "GetTextColor" => hle_get_text_color,
        "GdiFlush" => hle_gdi_flush,
        "GetNearestPaletteIndex" => hle_get_nearest_palette_index,
        "GetPixel" => hle_get_pixel,
        "GetPaletteEntries" => hle_get_palette_entries,
        "GetSysColor" => hle_get_sys_color,
        "GetSystemPaletteEntries" => hle_get_system_palette_entries,
        "GetSystemPaletteUse" => hle_get_system_palette_use,
        "GetTextExtentPoint32A" => hle_get_text_extent_point32_a,
        "GetTextExtentPoint32W" => hle_get_text_extent_point32_w,
        "GetTextExtentPointA" => hle_get_text_extent_point_a,
        "GetTextMetricsA" => hle_get_text_metrics_a,
        "GetTextMetricsW" => hle_get_text_metrics_w,
        "GetWindowThreadProcessId" => hle_get_window_thread_process_id,
        "GetWindowPlacement" => hle_get_window_placement,
        "GetWindowRect" => hle_get_window_rect,
        "GetWindowTextLengthA" => hle_get_window_text_length_a,
        "GetWindowTextLengthW" => hle_get_window_text_length_w,
        "GetWindowTextA" => hle_get_window_text_a,
        "GetWindowTextW" => hle_get_window_text_w,
        "IsIconic" => hle_is_iconic,
        "IsZoomed" => hle_is_zoomed,
        "LoadStringA" => hle_load_string_a,
        "LoadStringW" => hle_load_string_w,
        "MoveWindow" => hle_move_window,
        "OffsetRect" => hle_offset_rect,
        "SetRectEmpty" => hle_set_rect_empty,
        "IsRectEmpty" => hle_is_rect_empty,
        "EqualRect" => hle_equal_rect,
        "UnionRect" => hle_union_rect,
        "FrameRect" => hle_frame_rect,
        "InvertRect" => hle_invert_rect,
        "Rectangle" => hle_rectangle,
        "ResizePalette" => hle_resize_palette,
        "GetLayout" => hle_get_layout,
        "GetMapMode" => hle_get_map_mode,
        "GetTextAlign" => hle_get_text_align,
        "SetLayout" => hle_set_layout,
        "SetMapMode" => hle_set_map_mode,
        "SetROP2" => hle_set_rop2,
        "SetStretchBltMode" => hle_ret_ok_8,
        "SetProcessDefaultLayout" => hle_set_process_default_layout,
        "SetClassLongA" => hle_set_class_long_a,
        "ChangeDisplaySettingsA" => hle_change_display_settings_a,
        "EnumThreadWindows" => hle_enum_thread_windows,
        "GetDoubleClickTime" => hle_get_double_click_time,
        "SetFocus" => hle_set_focus,
        "SetForegroundWindow" => hle_set_foreground_window,
        "SetMenu" => hle_set_menu,
        "SetParent" => hle_set_parent,
        "SetPaletteEntries" => hle_set_palette_entries,
        "SetPixel" => hle_set_pixel,
        "SetSystemPaletteUse" => hle_set_system_palette_use,
        "SetRect" => hle_set_rect,
        "SetRectRgn" => hle_set_rect_rgn,
        "SetSysColors" => hle_set_sys_colors,
        "SetTextAlign" => hle_set_text_align,
        "SetWindowLongA" => hle_set_window_long_a,
        "SetWindowLongW" => hle_set_window_long_w,
        "SetWindowWord" => hle_set_window_word,
        "SetWindowPlacement" => hle_set_window_placement,
        "SetWindowPos" => hle_set_window_pos,
        "BeginDeferWindowPos" => hle_begin_defer_window_pos,
        "DeferWindowPos" => hle_defer_window_pos,
        "EndDeferWindowPos" => hle_end_defer_window_pos,
        "SetWindowTextA" => hle_set_window_text_a,
        "SetWindowTextW" => hle_set_window_text_w,
        "SendMessageA" => hle_send_message_a,
        "SendMessageW" => hle_send_message_w,
        "SendDlgItemMessageA" => hle_send_dlg_item_message_a,
        "SendDlgItemMessageW" => hle_send_dlg_item_message_w,
        "CallWindowProcA" => hle_call_window_proc_a,
        "CallWindowProcW" => hle_call_window_proc_w,
        "DrawTextW" => hle_draw_text_w,
        "TextOutW" => hle_text_out_w,
        "wvsprintfA" => hle_wvsprintf_a,
        "wsprintfA" => hle_wsprintf_a,
        "wsprintfW" => hle_wsprintf_w,
        "EnableMenuItem" | "CheckMenuItem" => hle_ret_ok_12,
        "DragAcceptFiles" => hle_ret_ok_8,
        "DestroyAcceleratorTable" | "DestroyCursor" | "DragFinish" | "IsClipboardFormatAvailable"
        | "SetCursor" => hle_ret_ok_4,
        "CloseClipboard" => hle_close_clipboard,
        "GetCaretBlinkTime" => hle_get_caret_blink_time,
        "GetClipboardData" => hle_get_clipboard_data,
        "OpenClipboard" => hle_open_clipboard,
        "DrawMenuBar" => hle_draw_menu_bar,
        "SystemParametersInfoA" => hle_system_parameters_info_a,
        "SystemParametersInfoW" => hle_system_parameters_info_w,
        "WinHelpA" | "WinHelpW" => hle_win_help_a,
        "EndDialog" => hle_end_dialog,
        "DeleteDC" => hle_delete_dc,
        "SaveDC" => hle_save_dc,
        "RestoreDC" => hle_restore_dc,
        "DragQueryFileW" => hle_ret_ok_16,
        "GetDlgItem" => hle_get_dlg_item,
        "GetDlgItemInt" => hle_get_dlg_item_int,
        "GetDlgItemTextA" => hle_get_dlg_item_text_a,
        "GetDlgItemTextW" => hle_get_dlg_item_text_w,
        "SetDlgItemInt" => hle_set_dlg_item_int,
        "SetDlgItemTextA" => hle_set_dlg_item_text_a,
        "SetDlgItemTextW" => hle_set_dlg_item_text_w,
        "SetBrushOrgEx" => hle_set_brush_org_ex,
        "MoveToEx" => hle_move_to_ex,
        "DialogBoxIndirectParamA" => hle_dialog_box_indirect_param_a,
        "DialogBoxIndirectParamW" => hle_dialog_box_indirect_param_w,
        "DialogBoxParamA" => hle_dialog_box_param_a,
        "DialogBoxParamW" => hle_dialog_box_param_w,
        "IsDialogMessageA" => hle_is_dialog_message_a,
        "IsDialogMessageW" => hle_is_dialog_message_w,
        "MapWindowPoints" => hle_map_window_points,
        "GetMenuItemRect" => hle_get_menu_item_rect,
        "ChooseFontW"
        | "FindTextW"
        | "GetOpenFileNameA"
        | "GetOpenFileNameW"
        | "GetSaveFileNameA"
        | "GetSaveFileNameW"
        | "PageSetupDlgW"
        | "PrintDlgW"
        | "ReplaceTextW" => hle_ret_ok_4,
        "GetFileTitleW" => hle_ret_ok_12,
        "AbortDoc" | "EndDoc" | "EndPage" | "StartPage" => hle_ret_ok_4,
        "StartDocW" => hle_ret_ok_8,
        "OpenPrinterA" => hle_open_printer_a,
        "ClosePrinter" => hle_close_printer,
        "DocumentPropertiesA" => hle_document_properties_a,
        "ValidateRect" => hle_validate_rect,
        "GetCurrentProcessId" => hle_get_current_process_id,
        "GetCurrentThread" => hle_get_current_thread,
        "GetCurrentThreadId" => hle_get_current_thread_id,
        "auxGetNumDevs" => hle_ret_ok_0,
        "mixerGetNumDevs" => hle_mixer_get_num_devs,
        "mixerOpen" => hle_mixer_open,
        "mixerClose" => hle_mixer_close,
        "mixerGetLineInfoA" | "mixerGetLineControlsA" | "mixerGetControlDetailsA"
        | "mixerSetControlDetails" => hle_mixer_no_driver_12,
        "DirectDrawCreate" => hle_direct_draw_create,
        "DirectInputCreateA" | "DirectInputCreateW" => hle_direct_input_create,
        "DirectSoundCreate" => hle_direct_sound_create,
        "mciSendStringA" => hle_mci_send_string_a,
        "mciSendCommandA" => hle_mci_send_command_a,
        "mciGetErrorStringA" => hle_mci_get_error_string_a,
        "mciGetDeviceIDA" => hle_mci_get_device_id_a,
        "ICInfo" => hle_ic_info,
        "MCIWndCreateA" => hle_mci_wnd_create_a,
        "acmMetrics" => hle_acm_metrics,
        "#1" => hle_direct_play_create,
        "#2" => hle_direct_play_enumerate_a,
        "#4" => hle_direct_play_lobby_create_a,
        "timeSetEvent" => hle_time_set_event,
        "timeKillEvent" => hle_time_kill_event,
        "timeBeginPeriod" | "timeEndPeriod" => hle_ret_ok_4,
        "timeGetDevCaps" => hle_time_get_dev_caps,
        "waveOutOpen" => hle_wave_out_open,
        "waveOutGetDevCapsA" => hle_wave_out_get_dev_caps_a,
        "waveOutGetPosition" => hle_wave_out_get_position,
        "waveOutGetID" => hle_wave_out_get_id,
        "midiOutOpen" => hle_midi_out_open,
        "midiOutClose" | "midiOutReset" => hle_ret_ok_4,
        "midiOutShortMsg" => hle_ret_ok_8,
        "midiOutPrepareHeader" | "midiOutUnprepareHeader" | "midiOutLongMsg" => hle_ret_ok_12,
        "midiStreamOpen" => hle_midi_stream_open,
        "midiStreamClose" | "midiStreamPause" | "midiStreamRestart" => hle_ret_ok_4,
        "midiStreamOut" | "midiStreamProperty" => hle_ret_ok_12,
        "mmioOpenA" => hle_mmio_open_a,
        "mmioClose" => hle_mmio_close,
        "mmioRead" => hle_mmio_read,
        "mmioSeek" => hle_mmio_seek,
        "mmioGetInfo" => hle_mmio_get_info,
        "mmioSetInfo" => hle_mmio_set_info,
        "mmioAdvance" => hle_mmio_advance,
        "mmioDescend" => hle_mmio_descend,
        "mmioAscend" => hle_mmio_ascend,
        "sndPlaySoundA" => hle_ret_ok_8,
        "joyGetNumDevs" | "waveOutGetNumDevs" => hle_ret_ok_0,
        "joyGetPos" => hle_joy_get_pos,
        "joyGetPosEx" => hle_joy_get_pos_ex,
        "joyGetDevCapsA" => hle_joy_get_dev_caps_a,
        "joySetCapture" => hle_joy_set_capture,
        "joyReleaseCapture" => hle_joy_release_capture,
        "waveOutSetVolume" => hle_ret_ok_8,
        "waveOutClose"
        | "waveOutPause"
        | "waveOutRestart"
        | "waveOutReset"
        | "PropertySheetA" => hle_ret_ok_4,
        "waveOutPrepareHeader" | "waveOutUnprepareHeader" | "waveOutWrite" => hle_ret_ok_12,
        "Shell_NotifyIconA" => hle_shell_notify_icon_a,
        "SHGetSpecialFolderPathW" => hle_sh_get_special_folder_path_w,
        "DragQueryFileA" => hle_drag_query_file_a,
        _ => return None,
    };
    Some(callback)
}

#[cfg(test)]
fn callback_for(name: &str) -> HleCallback {
    callback_for_known(name).unwrap_or(hle_default)
}

fn arg(emu: &Emulator, index: u32) -> u32 {
    emu.memory
        .read_u32(emu.cpu.reg(Reg::Esp).wrapping_add(4 + index * 4))
        .hle()
}

fn ret(emu: &mut Emulator, value: u32) {
    emu.cpu.set_reg(Reg::Eax, value);
}

// DWORD __wemu_async_return(void)
// Complete a synthetic callback return and restore the HLE return value.
fn hle_async_return(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let frame = emu.hle.pop_hle_callback_frame();
    if let Some(continuation) = frame.continuation {
        match continuation {
            HleCallbackContinuation::CreateWindow(continuation) => {
                dispatch_create_window_callback_after_async(emu, entry, continuation);
                return HleResult::Retn(0);
            }
            HleCallbackContinuation::DialogInit(continuation) => {
                dispatch_dialog_init_callback_after_async(
                    emu,
                    entry,
                    continuation,
                    frame.return_value,
                );
                return HleResult::Retn(0);
            }
            HleCallbackContinuation::WindowProcMessageChain(chain) => {
                dispatch_window_proc_message_chain_after_async(
                    emu,
                    entry,
                    chain,
                    frame.return_value,
                );
                return HleResult::Retn(0);
            }
            HleCallbackContinuation::OwnerDrawButton {
                hdc,
                draw_item,
                chain,
            } => {
                if hdc != 0 {
                    present_and_drop_gdi_dc(emu, hdc);
                }
                emu.hle.free_alloc(&mut emu.memory, draw_item).hle();
                if let Some(chain) = chain {
                    if dispatch_next_owner_draw_child_after_async(
                        emu,
                        entry,
                        chain,
                        frame.return_value,
                    ) {
                        return HleResult::Retn(0);
                    }
                }
            }
            HleCallbackContinuation::AsyncCpu { cpu } => {
                emu.cpu = cpu;
                return HleResult::Retn(0);
            }
        }
    }
    let esp = emu.cpu.reg(Reg::Esp);
    let ret_addr = emu.memory.read_u32(esp).hle();
    ret(emu, frame.return_value);
    emu.cpu.set_reg(Reg::Esp, esp.wrapping_add(4));
    emu.cpu.eip = ret_addr;
    HleResult::Retn(0)
}

// HLE default missing_import(...)
// Emit a one-shot JSON bug report and panic with the unresolved DLL/function name.
fn hle_default(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let reason = format!("missing HLE {}!{}", entry.dll, entry.name);
    emu.hle.record_unresolved_hle_symbol(entry.dll, entry.name);
    emu.hle.emit_missing_hle_report_once(&reason);
    panic!("{reason}");
}

fn missing_hle_report_json(reason: &str, unresolved: &[(String, String)]) -> String {
    let mut out = String::from("{\"reason\":\"");
    push_json_string_content(&mut out, reason);
    out.push_str("\",\"unresolved\":[");
    for (index, (dll, name)) in unresolved.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push('"');
        push_json_string_content(&mut out, dll);
        out.push('!');
        push_json_string_content(&mut out, name);
        out.push('"');
    }
    out.push_str("]}");
    out
}

fn push_json_string_content(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\0'..='\u{1f}' => push_json_u00(out, ch as u8),
            _ => out.push(ch),
        }
    }
}

fn push_json_u00(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push_str("\\u00");
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0f) as usize] as char);
}

// HLE helper ret_ok(retn_bytes)
// Return success-style zero and clean the requested stack bytes.
fn hle_ret_ok(emu: &mut Emulator, retn_bytes: u32) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(retn_bytes)
}

// HLE helper ret_ok_0()
// Return zero with cdecl stack cleanup.
fn hle_ret_ok_0(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 0)
}

// HLE helper ret_ok_4(arg0)
// Return zero and clean one stdcall argument.
fn hle_ret_ok_4(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 4)
}

// HLE helper ret_ok_8(arg0, arg1)
// Return zero and clean two stdcall arguments.
fn hle_ret_ok_8(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 8)
}

// HLE helper ret_ok_12(arg0, arg1, arg2)
// Return zero and clean three stdcall arguments.
fn hle_ret_ok_12(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 12)
}

// HLE helper ret_ok_16(arg0, arg1, arg2, arg3)
// Return zero and clean four stdcall arguments.
fn hle_ret_ok_16(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 16)
}

// HLE helper ret_ok_20(arg0, arg1, arg2, arg3, arg4)
// Return zero and clean five stdcall arguments.
fn hle_ret_ok_20(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 20)
}

// HLE helper ret_ok_24(arg0, arg1, arg2, arg3, arg4, arg5)
// Return zero and clean six stdcall arguments.
fn hle_ret_ok_24(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 24)
}

// HLE helper ret_ok_36(...)
// Return zero and clean nine stdcall arguments.
fn hle_ret_ok_36(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_ok(emu, 36)
}

// HLE helper ret_notimpl(retn_bytes)
// Return DDERR_UNSUPPORTED-style failure and clean the requested stack bytes.
fn hle_ret_notimpl(emu: &mut Emulator, retn_bytes: u32) -> HleResult {
    ret(emu, 0x8876_0001);
    HleResult::Retn(retn_bytes)
}

// HLE helper ret_notimpl_8(arg0, arg1)
// Return unsupported and clean two stdcall arguments.
fn hle_ret_notimpl_8(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_notimpl(emu, 8)
}

// HLE helper ret_notimpl_12(arg0, arg1, arg2)
// Return unsupported and clean three stdcall arguments.
fn hle_ret_notimpl_12(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    hle_ret_notimpl(emu, 12)
}

// HLE helper ret_one(...)
// Return one for simple Win32/COM success stubs.
fn hle_ret_one(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// ULONG IUnknown::AddRef(IUnknown *this)
// Increment the fake COM reference count and return it.
fn hle_com_add_ref(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let refs = emu.memory.read_u32(this + 4).unwrap_or(0).saturating_add(1);
    if this != 0 {
        emu.memory.write_u32(this + 4, refs).hle();
    }
    ret(emu, refs);
    HleResult::Retn(4)
}

// ULONG IUnknown::Release(IUnknown *this)
// Decrement the fake COM reference count while keeping the object mapped.
fn hle_com_release(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let refs = emu.memory.read_u32(this + 4).unwrap_or(1).saturating_sub(1);
    if this != 0 {
        emu.memory.write_u32(this + 4, refs).hle();
    }
    // Keep the object mapped after refcount reaches zero. Old game code may hold
    // stale COM pointers during shutdown, and dangling reads are less useful here
    // than detecting the next missing API behavior.
    ret(emu, refs);
    HleResult::Retn(4)
}

// HRESULT IUnknown::QueryInterface(IUnknown *this, REFIID iid, void **out)
// Return the same fake COM object for interface queries.
fn hle_ok_this(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    if out != 0 {
        let this = arg(emu, 0);
        emu.memory.write_u32(out, this).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

#[cfg(test)]
mod hle_base_tests {
    use super::{
        mouse_lparam, FileBackend, FileReadResult, FileWriteResult, Handle, Hle, HleControlKind,
        HleDelayTarget, HleEntry, HleMenu, HleMenuItem, HlePopupMenu, HlePopupMenuItem, HleResult,
        HleWindow, VirtualOpen, WindowRect, DIALOG_BORDER, DIALOG_TITLE_HEIGHT, MENU_BAR_HEIGHT,
        WS_CAPTION,
    };
    use crate::Emulator;
    use crate::memory::{Memory, PagePerm};
    use std::fs;

    #[test]
    fn guarded_alloc_leaves_large_oob_unmapped() {
        let mut hle = Hle::new();
        let mut memory = Memory::new();
        let buffer = hle
            .alloc_guarded(
                &mut memory,
                0x0003_2000,
                PagePerm::READ | PagePerm::WRITE,
                0x0010_0000,
            )
            .unwrap();

        assert!(memory.is_mapped(buffer + 0x0003_1fff, PagePerm::WRITE));
        assert!(!memory.is_mapped(buffer + 0x0003_2000, PagePerm::WRITE));
        assert!(!memory.is_mapped(buffer + 0x0005_c000, PagePerm::WRITE));
    }

    #[test]
    fn module_image_reservation_maps_fixed_base_and_rejects_overlap() {
        let mut hle = Hle::new();
        let mut memory = Memory::new();

        hle.reserve_module_image(&mut memory, 0x0040_0000, 0x0012_3000)
            .unwrap();

        assert!(memory.is_mapped(0x0040_0000, PagePerm::READ));
        assert!(memory.is_mapped(0x0052_2fff, PagePerm::READ));
        assert!(!memory.is_mapped(0x0052_3000, PagePerm::READ));
        assert!(hle
            .reserve_module_image(&mut memory, 0x0050_0000, 0x0001_0000)
            .is_err());
    }

    #[test]
    fn module_image_reservation_stays_below_stack_gap() {
        let mut hle = Hle::new();
        let mut memory = Memory::new();

        assert!(hle
            .reserve_module_image(&mut memory, 0x0f00_0000, 0x0001_0000)
            .is_err());
    }

    #[test]
    fn msvcrt_imported_data_slots_are_guest_memory() {
        let mut hle = Hle::new();
        let mut memory = Memory::new();

        let adjust_fdiv = hle
            .resolve_pe_import(&mut memory, "MSVCRT.dll", "_adjust_fdiv")
            .unwrap();
        assert!(memory.is_mapped(adjust_fdiv, PagePerm::READ));
        assert_eq!(memory.read_u32(adjust_fdiv).unwrap(), 0);

        let acmdln = hle
            .resolve_pe_import(&mut memory, "MSVCRT.dll", "_acmdln")
            .unwrap();
        assert!(memory.is_mapped(acmdln, PagePerm::READ));
        assert_eq!(memory.read_u32(acmdln).unwrap(), 0);

        hle.bootstrap_process_strings(
            &mut memory,
            0x0100_0000,
            "C:\\freecell.exe",
            "C:\\freecell.exe",
        )
        .unwrap();

        let command_line = memory.read_u32(acmdln).unwrap();
        assert_ne!(command_line, 0);
        assert_eq!(
            memory.cstr_lossy(command_line, 128).unwrap(),
            "C:\\freecell.exe"
        );
    }

    #[test]
    fn non_runtime_dll_imports_do_not_fallback_to_hle_thunks() {
        let mut hle = Hle::new();
        let mut memory = Memory::new();

        let err = hle
            .resolve_pe_import(&mut memory, "CARDS.dll", "cdtInit")
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("refusing to HLE non-runtime DLL import cards.dll!cdtInit"));
    }

    #[test]
    fn runtime_dll_imports_prefer_hle_over_mounted_wrapper() {
        let root = std::env::temp_dir().join(format!(
            "wemu-runtime-hle-precedence-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ddraw.dll"), b"not a pe").unwrap();

        let mut hle = Hle::new();
        let mut memory = Memory::new();
        hle.set_drive_mount('C', root.clone());
        hle.set_cwd('C', "\\".to_string());

        let thunk = hle
            .resolve_pe_import(&mut memory, "ddraw.dll", "DirectDrawCreate")
            .unwrap();
        assert_ne!(thunk, 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn virtual_files_are_case_insensitive_and_cwd_relative() {
        let mut hle = Hle::new();
        hle.set_cwd('C', "\\DATA".to_string());
        hle.add_virtual_file("C:\\Data\\File.TXT", b"abc");

        assert_eq!(hle.drive_type('C'), Some(3));
        assert!(!hle.drive_is_mounted('D'));

        let h = match hle.open_virtual_file("file.txt", 0x8000_0000, 3) {
            VirtualOpen::Opened(h) => h,
            _ => panic!("virtual file did not open"),
        };
        match hle.handle_mut(h) {
            Some(Handle::File(file)) => {
                match &file.backend {
                    FileBackend::Memory(data) => assert_eq!(&*data.borrow(), b"abc"),
                    _ => panic!("unexpected file backend"),
                }
                assert_eq!(file.pos, 0);
            }
            _ => panic!("unexpected handle type"),
        }
    }

    #[test]
    fn virtual_drive_aliases_map_files_and_drive_type() {
        let mut hle = Hle::new();
        hle.add_virtual_file("C:\\Game\\Data\\File.TXT", b"alias");
        hle.set_virtual_drive_alias('D', "C:\\Game\\Data", "cdrom");

        assert!(hle.drive_is_mounted('D'));
        assert_eq!(hle.drive_type('D'), Some(5));

        let h = match hle.open_virtual_file("D:\\file.txt", 0x8000_0000, 3) {
            VirtualOpen::Opened(h) => h,
            _ => panic!("aliased virtual file did not open"),
        };
        match hle.handle_mut(h) {
            Some(Handle::File(file)) => match &file.backend {
                FileBackend::Memory(data) => assert_eq!(&*data.borrow(), b"alias"),
                _ => panic!("unexpected file backend"),
            },
            _ => panic!("unexpected handle type"),
        }
    }

    #[test]
    fn enabled_virtual_fs_reports_missing_files() {
        let mut hle = Hle::new();
        hle.enable_virtual_fs();

        match hle.open_virtual_file("C:\\missing.dat", 0x8000_0000, 3) {
            VirtualOpen::Failed(2) => {}
            _ => panic!("missing virtual file did not report ERROR_FILE_NOT_FOUND"),
        }
    }

    #[test]
    fn vfs_cwd_resolves_display_paths_and_keys() {
        let mut hle = Hle::new();
        hle.set_cwd('d', "\\Games\\Rich4".to_string());

        assert_eq!(hle.cwd_display(), "D:\\Games\\Rich4");
        assert_eq!(
            hle.full_guest_path("data\\map.dat"),
            "D:\\Games\\Rich4\\data\\map.dat"
        );
        assert_eq!(
            hle.vfs_key_for_guest("..\\save\\GAME.DAT"),
            "d:\\games\\save\\game.dat"
        );
    }

    #[test]
    fn virtual_find_entries_lists_immediate_files_and_directories() {
        let mut hle = Hle::new();
        hle.add_virtual_file("C:\\Data\\Readme.TXT", b"abc");
        hle.add_virtual_file("C:\\Data\\Sub\\Nested.DAT", b"nested");
        hle.add_async_virtual_file("C:\\Data\\Async.BIN", 7, false);

        let mut entries = hle.virtual_find_entries("c:\\data").unwrap();
        entries.sort_by_key(|entry| entry.name.clone());

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "async.bin");
        assert_eq!(entries[0].attrs, 0x80);
        assert_eq!(entries[0].size, 7);
        assert_eq!(entries[1].name, "readme.txt");
        assert_eq!(entries[1].attrs, 0x80);
        assert_eq!(entries[1].size, 3);
        assert_eq!(entries[2].name, "sub");
        assert_eq!(entries[2].attrs, 0x10);
    }

    #[test]
    fn async_vfs_requests_are_tracked_by_vfs_state() {
        let mut hle = Hle::new();
        hle.add_async_virtual_file("C:\\Data\\Async.BIN", 4, true);

        let h = match hle.open_virtual_file("C:\\Data\\Async.BIN", 0xc000_0000, 3) {
            VirtualOpen::Opened(h) => h,
            _ => panic!("async virtual file did not open"),
        };

        let mut buf = [0; 2];
        let (key, offset, len) = match hle.handle_mut(h) {
            Some(Handle::File(file)) => match file.read(&mut buf) {
                FileReadResult::Pending { key, offset, len } => (key, offset, len),
                _ => panic!("async read did not produce a pending request"),
            },
            _ => panic!("unexpected handle type"),
        };
        let read_id = hle.begin_vfs_read(&key, offset, len);
        assert_eq!(hle.pending_vfs_request_id(), read_id);
        assert_eq!(hle.pending_vfs_request_kind(), 1);
        assert_eq!(hle.pending_vfs_request_path(), b"c:\\data\\async.bin");
        assert_eq!(hle.pending_vfs_request_offset(), 0);
        assert_eq!(hle.pending_vfs_request_len(), 2);

        assert!(hle.complete_vfs_request(read_id, 0, 2, vec![1, 2]));
        let completed = hle.take_completed_vfs_request(read_id).unwrap();
        assert_eq!(completed.data, vec![1, 2]);

        let (key, offset, data) = match hle.handle_mut(h) {
            Some(Handle::File(file)) => match file.write(vec![3, 4, 5]) {
                FileWriteResult::Pending { key, offset, data } => (key, offset, data),
                _ => panic!("async write did not produce a pending request"),
            },
            _ => panic!("unexpected handle type"),
        };
        hle.note_async_vfs_write(&key, offset, data.len());
        let write_id = hle.begin_vfs_write(&key, offset, data);
        assert_eq!(hle.pending_vfs_request_kind(), 2);
        assert_eq!(hle.pending_vfs_request_offset(), 2);
        assert_eq!(hle.pending_vfs_request_len(), 3);
        assert_eq!(hle.pending_vfs_request_data(), &[3, 4, 5]);

        assert!(hle.complete_vfs_request(write_id, 0, 3, Vec::new()));
        assert!(hle.has_completed_vfs_request(write_id));
        let entries = hle.virtual_find_entries("C:\\Data").unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.name == "async.bin")
            .unwrap();
        assert_eq!(entry.size, 5);
    }

    #[test]
    fn mouse_moves_are_coalesced_in_input_queue() {
        let mut hle = Hle::new();

        hle.post_mouse_move(10, 20);
        hle.post_mouse_move(30, 40);

        assert_eq!(hle.input_messages.len(), 1);
        assert_eq!(hle.input_messages[0].lparam, mouse_lparam(30, 40));
    }

    #[test]
    fn click_keeps_button_edges_after_coalesced_move() {
        let mut hle = Hle::new();

        hle.post_mouse_move(10, 20);
        hle.post_click(30, 40);

        assert_eq!(hle.input_messages.len(), 3);
        assert_eq!(hle.input_messages[0].msg, 0x0200);
        assert_eq!(hle.input_messages[1].msg, 0x0201);
        assert_eq!(hle.input_messages[2].msg, 0x0202);
    }

    #[test]
    fn move_after_click_stays_after_button_edges() {
        let mut hle = Hle::new();

        hle.post_click(10, 20);
        hle.post_mouse_move(30, 40);

        assert_eq!(hle.input_messages.len(), 3);
        assert_eq!(hle.input_messages[0].msg, 0x0201);
        assert_eq!(hle.input_messages[1].msg, 0x0202);
        assert_eq!(hle.input_messages[2].msg, 0x0200);
    }

    #[test]
    fn text_input_posts_wm_char_to_focused_window() {
        let mut hle = Hle::new();
        hle.focus_window = 0x0002_0005;

        hle.post_text("Az");

        assert_eq!(hle.input_messages.len(), 8);
        assert_eq!(hle.input_messages[0].hwnd, 0x0002_0005);
        assert_eq!(hle.input_messages[0].msg, 0x0100);
        assert_eq!(hle.input_messages[0].wparam, 0x10);
        assert_eq!(hle.input_messages[1].msg, 0x0100);
        assert_eq!(hle.input_messages[1].wparam, b'A' as u32);
        assert_eq!(hle.input_messages[2].msg, 0x0102);
        assert_eq!(hle.input_messages[2].wparam, b'A' as u32);
        assert_eq!(hle.input_messages[3].msg, 0x0101);
        assert_eq!(hle.input_messages[3].wparam, b'A' as u32);
        assert_eq!(hle.input_messages[4].msg, 0x0101);
        assert_eq!(hle.input_messages[4].wparam, 0x10);
        assert_eq!(hle.input_messages[5].msg, 0x0100);
        assert_eq!(hle.input_messages[5].wparam, b'Z' as u32);
        assert_eq!(hle.input_messages[6].msg, 0x0102);
        assert_eq!(hle.input_messages[6].wparam, b'z' as u32);
        assert_eq!(hle.input_messages[7].msg, 0x0101);
    }

    #[test]
    fn sdl_text_input_does_not_release_held_key() {
        let mut hle = Hle::new();
        hle.focus_window = 0x0002_0005;

        hle.post_key_down(0x20);
        hle.post_text_input(" ");

        assert_eq!(hle.input_messages.len(), 2);
        assert_eq!(hle.input_messages[0].msg, 0x0100);
        assert_eq!(hle.input_messages[0].wparam, 0x20);
        assert_eq!(hle.input_messages[1].msg, 0x0102);
        assert_eq!(hle.input_messages[1].wparam, b' ' as u32);
        assert_eq!(hle.keyboard_state_byte(0x20), 0x80);
    }

    #[test]
    fn key_events_update_virtual_key_state_until_keyup() {
        let mut hle = Hle::new();
        hle.focus_window = 0x0002_0005;

        hle.post_key_down(0x41);

        assert_eq!(hle.input_messages.len(), 1);
        assert_eq!(hle.input_messages[0].hwnd, 0x0002_0005);
        assert_eq!(hle.input_messages[0].msg, 0x0100);
        assert_eq!(hle.keyboard_state_byte(0x41), 0x80);
        assert_eq!(hle.key_state_word(0x41, true), 0x8001);
        assert_eq!(hle.key_state_word(0x41, true), 0x8000);

        hle.post_key_up(0x41);

        assert_eq!(hle.input_messages[1].msg, 0x0101);
        assert_eq!(hle.keyboard_state_byte(0x41), 0);
        assert_eq!(hle.key_state_word(0x41, false), 0);
    }

    #[test]
    fn popup_menu_click_posts_owner_command() {
        let mut hle = Hle::new();
        hle.active_popup_menu = Some(HlePopupMenu {
            owner: 0x0002_0001,
            rect: WindowRect {
                left: 10,
                top: 10,
                right: 160,
                bottom: 40,
            },
            items: vec![HlePopupMenuItem {
                id: 0x9c68,
                submenu: 0,
                text: "Playlist Editor".to_string(),
                rect: WindowRect {
                    left: 12,
                    top: 12,
                    right: 158,
                    bottom: 32,
                },
                separator: false,
                enabled: true,
                checked: false,
            }],
        });

        hle.post_mouse_button_down(20, 20);
        hle.post_mouse_button_up(20, 20);

        assert!(hle.input_messages.is_empty());
        assert_eq!(hle.app_messages.len(), 1);
        assert_eq!(hle.app_messages[0].hwnd, 0x0002_0001);
        assert_eq!(hle.app_messages[0].msg, 0x0111);
        assert_eq!(hle.app_messages[0].wparam, 0x9c68);
    }

    #[test]
    fn menu_bar_click_opens_top_level_submenu() {
        let mut hle = Hle::new();
        let submenu = hle.alloc_menu_handle(HleMenu {
            items: vec![HleMenuItem {
                id: 0x1234,
                text: "&Start".to_string(),
                submenu: 0,
                separator: false,
                enabled: true,
                checked: false,
            }],
        });
        let menu = hle.alloc_menu_handle(HleMenu {
            items: vec![HleMenuItem {
                id: 0,
                text: "&Game".to_string(),
                submenu,
                separator: false,
                enabled: true,
                checked: false,
            }],
        });
        hle.register_window(HleWindow {
            hwnd: 0x0002_0001,
            parent: 0,
            id: menu,
            class_name: "main".to_string(),
            text: "Main".to_string(),
            rect: WindowRect {
                left: 10,
                top: 20,
                right: 220,
                bottom: 180,
            },
            style: WS_CAPTION,
            ex_style: 0,
            proc: 0x0040_1000,
            user_data: 0,
            extra: Default::default(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });

        let x = (10 + DIALOG_BORDER + 8) as u32;
        let y = (20 + DIALOG_TITLE_HEIGHT + 4) as u32;
        assert!(hle.activate_menu_bar_at(x, y, 640, 480));

        let popup = hle.active_popup_menu.as_ref().unwrap();
        assert_eq!(popup.owner, 0x0002_0001);
        assert_eq!(popup.rect.top, 20 + DIALOG_TITLE_HEIGHT + MENU_BAR_HEIGHT);
        assert_eq!(popup.items.len(), 1);
        assert_eq!(popup.items[0].id, 0x1234);
    }

    #[test]
    fn render_hle_windows_draws_menu_bar() {
        let mut emu = Emulator::new();
        let submenu = emu.hle.alloc_menu_handle(HleMenu {
            items: vec![HleMenuItem {
                id: 0x1234,
                text: "&Start".to_string(),
                submenu: 0,
                separator: false,
                enabled: true,
                checked: false,
            }],
        });
        let menu = emu.hle.alloc_menu_handle(HleMenu {
            items: vec![HleMenuItem {
                id: 0,
                text: "&Game".to_string(),
                submenu,
                separator: false,
                enabled: true,
                checked: false,
            }],
        });
        emu.hle.register_window(HleWindow {
            hwnd: 0x0002_0001,
            parent: 0,
            id: menu,
            class_name: "main".to_string(),
            text: "Main".to_string(),
            rect: WindowRect {
                left: 0,
                top: 0,
                right: 200,
                bottom: 100,
            },
            style: 0,
            ex_style: 0,
            proc: 0,
            user_data: 0,
            extra: Default::default(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });

        super::render_hle_windows(&mut emu);

        let fb = emu.backend.framebuffer();
        let offset = (5 * emu.backend.width() as usize + 5) * 4;
        assert_ne!(&fb[offset..offset + 3], &[0, 0, 0]);
    }

    #[test]
    fn top_level_mouse_messages_use_client_coordinates() {
        let mut hle = Hle::new();
        hle.register_window(HleWindow {
            hwnd: 0x0002_0001,
            parent: 0,
            id: 0,
            class_name: "MineWindow".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 10,
                top: 20,
                right: 180,
                bottom: 160,
            },
            style: WS_CAPTION,
            ex_style: 0,
            proc: 0x0040_1000,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });

        hle.post_mouse_button_down(
            (10 + DIALOG_BORDER + 12) as u32,
            (20 + DIALOG_TITLE_HEIGHT + 34) as u32,
        );

        assert_eq!(hle.input_messages[0].hwnd, 0x0002_0001);
        assert_eq!(hle.input_messages[0].lparam, mouse_lparam(12, 34));
    }

    #[test]
    fn undecorated_top_level_mouse_messages_use_window_coordinates() {
        let mut hle = Hle::new();
        hle.register_window(HleWindow {
            hwnd: 0x0002_0001,
            parent: 0,
            id: 0,
            class_name: "SkinWindow".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 10,
                top: 20,
                right: 180,
                bottom: 160,
            },
            style: 0,
            ex_style: 0,
            proc: 0x0040_1000,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });

        hle.post_mouse_button_down(22, 54);

        assert_eq!(hle.input_messages[0].hwnd, 0x0002_0001);
        assert_eq!(hle.input_messages[0].lparam, mouse_lparam(12, 34));
    }

    #[test]
    fn child_mouse_messages_use_child_client_coordinates() {
        let mut hle = Hle::new();
        hle.register_window(HleWindow {
            hwnd: 0x0002_0001,
            parent: 0,
            id: 0,
            class_name: "Parent".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 10,
                top: 20,
                right: 240,
                bottom: 180,
            },
            style: WS_CAPTION,
            ex_style: 0,
            proc: 0x0040_1000,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });
        hle.register_window(HleWindow {
            hwnd: 0x0002_0005,
            parent: 0x0002_0001,
            id: 7,
            class_name: "Child".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 10 + DIALOG_BORDER + 30,
                top: 20 + DIALOG_TITLE_HEIGHT + 40,
                right: 10 + DIALOG_BORDER + 80,
                bottom: 20 + DIALOG_TITLE_HEIGHT + 90,
            },
            style: 0,
            ex_style: 0,
            proc: 0x0040_2000,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });

        hle.post_mouse_button_down((10 + DIALOG_BORDER + 37) as u32, (20 + DIALOG_TITLE_HEIGHT + 49) as u32);

        assert_eq!(hle.input_messages[0].hwnd, 0x0002_0005);
        assert_eq!(hle.input_messages[0].lparam, mouse_lparam(7, 9));
    }

    #[test]
    fn clicking_edit_control_focuses_keyboard_input() {
        let mut hle = Hle::new();
        hle.register_window(HleWindow {
            hwnd: 0x0002_0001,
            parent: 0,
            id: 0,
            class_name: "Parent".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 10,
                top: 20,
                right: 240,
                bottom: 180,
            },
            style: WS_CAPTION,
            ex_style: 0,
            proc: 0x0040_1000,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });
        hle.register_window(HleWindow {
            hwnd: 0x0002_0005,
            parent: 0x0002_0001,
            id: 7,
            class_name: "Edit".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 10 + DIALOG_BORDER + 30,
                top: 20 + DIALOG_TITLE_HEIGHT + 40,
                right: 10 + DIALOG_BORDER + 120,
                bottom: 20 + DIALOG_TITLE_HEIGHT + 60,
            },
            style: 0,
            ex_style: 0,
            proc: 0,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Edit,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });

        hle.post_mouse_button_down(
            (10 + DIALOG_BORDER + 37) as u32,
            (20 + DIALOG_TITLE_HEIGHT + 49) as u32,
        );
        hle.post_text("A");

        assert_eq!(hle.focus_window, 0x0002_0005);
        assert_eq!(hle.input_messages[1].hwnd, 0x0002_0005);
        assert_eq!(hle.input_messages[1].msg, 0x0100);
        let typed = hle
            .input_messages
            .iter()
            .find(|message| message.msg == 0x0102)
            .unwrap();
        assert_eq!(typed.hwnd, 0x0002_0005);
        assert_eq!(typed.wparam, b'A' as u32);
    }

    #[test]
    fn window_class_lookup_tracks_names_and_atoms() {
        let mut hle = Hle::new();

        let named = hle.register_window_class("CardWnd", 0x0040_1000);
        let atom_only = hle.register_window_class("", 0x0040_2000);

        assert_ne!(named, atom_only);
        assert_eq!(hle.window_proc_for_class("cardwnd", 0), Some(0x0040_1000));
        assert_eq!(hle.window_proc_for_class("CARDWND", 0), Some(0x0040_1000));
        assert_eq!(hle.window_proc_for_class("", named), Some(0x0040_1000));
        assert_eq!(hle.window_proc_for_class("ignored", atom_only), Some(0x0040_2000));
        assert_eq!(hle.window_proc_for_class("missing", 0), None);
    }

    #[test]
    fn hle_callback_stack_is_lifo_for_nested_callbacks() {
        let mut hle = Hle::new();

        hle.push_hle_callback_return(0x1111_1111);
        hle.push_hle_callback_return(0x2222_2222);

        assert_eq!(hle.finish_async_callback(), 0x2222_2222);
        assert_eq!(hle.finish_async_callback(), 0x1111_1111);
        assert_eq!(hle.finish_async_callback(), 0);
    }

    #[test]
    fn midi_out_short_msg_cleans_two_stdcall_args() {
        let callback = super::callback_for("midiOutShortMsg");
        let entry = HleEntry {
            addr: 0,
            dll: "winmm.dll",
            name: "midiOutShortMsg",
            callback,
        };
        let mut emu = Emulator::new();

        assert_eq!(callback(&mut emu, &entry), HleResult::Retn(8));
    }

    #[test]
    fn timers_use_millisecond_clock() {
        let mut hle = Hle::new();

        let target = hle.delay_target(10, 0);
        hle.set_timer(0x20001, 7, 0, 100, 0, target);
        hle.pump_timers(109, 0);
        assert!(hle.app_messages.is_empty());

        hle.pump_timers(110, 0);
        assert_eq!(hle.app_messages.len(), 1);
        assert_eq!(hle.app_messages[0].msg, 0x0113);
        assert_eq!(hle.app_messages[0].wparam, 7);
        assert_eq!(hle.timers[0].due_count, 1);
        assert_eq!(hle.timers[0].post_count, 1);

        hle.pump_timers(120, 0);
        assert_eq!(hle.app_messages.len(), 1);
        assert_eq!(hle.timers[0].due_count, 2);
        assert_eq!(hle.timers[0].post_count, 1);
        assert!(hle.state_summary(120).contains("pending=1"));
    }

    #[test]
    fn live_delay_target_rounds_guest_ms_to_frontend_frames() {
        let mut hle = Hle::new();
        hle.set_frontend_timing(100, 10_000);

        assert_eq!(
            hle.delay_target(0, 7),
            HleDelayTarget {
                delay_ms: 0,
                frame_count: 1,
            }
        );
        assert_eq!(
            hle.delay_target(14, 7),
            HleDelayTarget {
                delay_ms: 10,
                frame_count: 1,
            }
        );
        assert_eq!(
            hle.delay_target(15, 7),
            HleDelayTarget {
                delay_ms: 20,
                frame_count: 2,
            }
        );
    }

    #[test]
    fn user_timers_are_not_posted_before_eligible_frame() {
        let mut hle = Hle::new();
        hle.set_frontend_timing(100, 10_000);

        let target = hle.delay_target(1, 1);
        hle.set_timer(0x20001, 7, 0, 100, 1, target);
        assert_eq!(hle.timers[0].next_ms, 110);
        hle.pump_timers(109, 2);
        assert!(hle.app_messages.is_empty());
        assert_eq!(hle.timers[0].due_count, 0);

        hle.pump_timers(110, 1);
        assert!(hle.app_messages.is_empty());
        assert_eq!(hle.timers[0].due_count, 0);

        hle.pump_timers(110, 2);
        assert_eq!(hle.app_messages.len(), 1);
        assert_eq!(hle.app_messages[0].msg, 0x0113);
        assert_eq!(hle.timers[0].due_count, 1);
        assert_eq!(hle.timers[0].eligible_frame, 3);
        assert_eq!(hle.timers[0].next_ms, 120);
    }

    #[test]
    fn multimedia_timers_schedule_periodic_callbacks() {
        let mut hle = Hle::new();

        let target = hle.delay_target(20, 0);
        let id = hle.set_mm_timer(0x401f2d, 0xfeed, 1, 100, 0, target);

        assert_eq!(id, 1);
        assert!(hle.take_due_mm_timer(119, 0).is_none());
        let due = hle.take_due_mm_timer(120, 0).unwrap();
        assert_eq!(due.id, id);
        assert_eq!(due.callback, 0x401f2d);
        assert_eq!(due.user, 0xfeed);
        assert!(hle.take_due_mm_timer(140, 0).is_none());
        assert_eq!(hle.finish_async_callback(), 0);
        assert!(hle.take_due_mm_timer(139, 0).is_none());
        assert!(hle.take_due_mm_timer(140, 0).is_some());
        assert!(hle.state_summary(140).contains("cb=00401f2d"));
    }

    #[test]
    fn multimedia_timers_are_not_taken_before_eligible_frame() {
        let mut hle = Hle::new();
        hle.set_frontend_timing(100, 10_000);

        let target = hle.delay_target(1, 1);
        let id = hle.set_mm_timer(0x401f2d, 0xfeed, 1, 100, 1, target);

        assert_eq!(id, 1);
        assert!(hle.take_due_mm_timer(109, 2).is_none());
        assert!(hle.take_due_mm_timer(110, 1).is_none());
        let due = hle.take_due_mm_timer(110, 2).unwrap();
        assert_eq!(due.callback, 0x401f2d);
        assert_eq!(hle.mm_timers[0].eligible_frame, 3);
    }

    #[test]
    fn windows_hooks_are_recorded_and_removed() {
        let mut hle = Hle::new();

        let hook = hle.set_windows_hook(2, 0x401010, 0x400000, 0);

        assert_ne!(hook, 0);
        assert!(hle.state_summary(0).contains("id=2"));
        assert!(hle.state_summary(0).contains("proc=00401010"));
        assert!(hle.unhook_windows_hook(hook));
        assert!(!hle.unhook_windows_hook(hook));
        assert!(hle.state_summary(0).contains("hooks=-"));
    }
}
