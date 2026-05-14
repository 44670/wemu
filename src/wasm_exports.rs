use std::path::Path;
use std::slice;

use wemu::backend::{BackendEvent, HeadlessBackend};
use wemu::cpu::Reg;
use wemu::memory::{GUEST_RAM_BASE, GUEST_RAM_END};
use wemu::{png, Emulator, Error, Result, StopReason, DEFAULT_FRAME_TIMEOUT_MS};

struct WasmEmulator {
    emu: Emulator,
    last_error: Vec<u8>,
    last_blob: Vec<u8>,
    last_stop: Option<StopReason>,
}

impl WasmEmulator {
    fn new(width: u32, height: u32) -> Self {
        let mut emu = Emulator::new();
        emu.max_insns = u64::MAX;
        emu.backend = Box::new(HeadlessBackend::new_live(
            width.clamp(1, 4096),
            height.clamp(1, 4096),
        ));
        emu.hle.enable_virtual_fs();
        Self {
            emu,
            last_error: Vec::new(),
            last_blob: Vec::new(),
            last_stop: None,
        }
    }

    fn clear_error(&mut self) {
        self.last_error.clear();
    }

    fn set_error(&mut self, err: impl std::fmt::Display) {
        self.last_error = err.to_string().into_bytes();
    }
}

#[no_mangle]
pub extern "C" fn wemu_alloc(len: u32) -> u32 {
    if len == 0 {
        return 0;
    }
    let mut bytes = Vec::<u8>::with_capacity(len as usize);
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    ptr as usize as u32
}

#[no_mangle]
pub unsafe extern "C" fn wemu_free(ptr: u32, len: u32) {
    if ptr == 0 || len == 0 {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(
            ptr as usize as *mut u8,
            0,
            len as usize,
        ));
    }
}

#[no_mangle]
pub extern "C" fn wemu_new(width: u32, height: u32) -> u32 {
    Box::into_raw(Box::new(WasmEmulator::new(width, height))) as usize as u32
}

#[no_mangle]
pub unsafe extern "C" fn wemu_destroy(handle: u32) {
    if handle != 0 {
        unsafe {
            drop(Box::from_raw(handle as usize as *mut WasmEmulator));
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_load_exe(
    handle: u32,
    path_ptr: u32,
    path_len: u32,
    exe_ptr: u32,
    exe_len: u32,
) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = unsafe {
        (|| -> Result<i32> {
            let path = read_string(path_ptr, path_len)?;
            let exe = read_bytes(exe_ptr, exe_len)?;
            let (cwd_drive, cwd_path) = guest_parent_dir(&path);
            wasm.emu.hle.set_cwd(cwd_drive, cwd_path);
            wasm.emu.hle.add_virtual_file(&path, exe);
            wasm.emu.apply_app_db_virtual_mounts_for_exe(&path);
            wasm.emu.load_exe_bytes(Path::new(&path), exe)?;
            Ok(0)
        })()
    };
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_add_file(
    handle: u32,
    path_ptr: u32,
    path_len: u32,
    data_ptr: u32,
    data_len: u32,
) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = unsafe {
        (|| -> Result<i32> {
            let path = read_string(path_ptr, path_len)?;
            let data = read_bytes(data_ptr, data_len)?;
            wasm.emu.hle.add_virtual_file(&path, data);
            Ok(0)
        })()
    };
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_add_file_owned(
    handle: u32,
    path_ptr: u32,
    path_len: u32,
    data_ptr: u32,
    data_len: u32,
) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = unsafe {
        (|| -> Result<i32> {
            let path = read_string(path_ptr, path_len)?;
            if data_ptr == 0 && data_len != 0 {
                return Err(Error::Cli("null owned wasm pointer".to_string()));
            }
            let data = Vec::from_raw_parts(
                data_ptr as usize as *mut u8,
                data_len as usize,
                data_len as usize,
            );
            wasm.emu.hle.add_virtual_file_owned(&path, data);
            Ok(0)
        })()
    };
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_add_async_file(
    handle: u32,
    path_ptr: u32,
    path_len: u32,
    size_lo: u32,
    size_hi: u32,
    writable: u32,
) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = unsafe {
        (|| -> Result<i32> {
            let path = read_string(path_ptr, path_len)?;
            let size = ((size_hi as u64) << 32) | size_lo as u64;
            wasm.emu
                .hle
                .add_async_virtual_file(&path, size, writable != 0);
            Ok(0)
        })()
    };
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_enable_async_vfs_writes(handle: u32) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    wasm.emu.hle.enable_async_vfs_writes();
    wasm.clear_error();
    0
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_id(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_id())
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_kind(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_kind())
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_path_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_path().as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_path_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_path().len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_offset_lo(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_offset() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_offset_hi(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| (wasm.emu.hle.pending_vfs_request_offset() >> 32) as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_len())
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_data_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_data().as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_pending_vfs_request_data_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.pending_vfs_request_data().len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_complete_vfs_request(
    handle: u32,
    request_id: u32,
    status: u32,
    transferred: u32,
    data_ptr: u32,
    data_len: u32,
) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = unsafe {
        (|| -> Result<i32> {
            let data = read_bytes(data_ptr, data_len)?.to_vec();
            if wasm
                .emu
                .hle
                .complete_vfs_request(request_id, status, transferred, data)
            {
                Ok(0)
            } else {
                Err(Error::Cli(format!(
                    "unknown async VFS request id {request_id}"
                )))
            }
        })()
    };
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_run_for(handle: u32, instruction_budget: u32) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = wasm.emu.run_for(instruction_budget).map(|stop| {
        wasm.last_stop = stop;
        stop.map(stop_code).unwrap_or(0)
    });
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_run_one_frame(handle: u32, input_ptr: u32, input_len: u32) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = unsafe {
        (|| -> Result<i32> {
            let input = read_bytes(input_ptr, input_len)?;
            let events = decode_frame_input(input)?;
            if let Some(stop) = wasm.emu.apply_frontend_events(&events)? {
                wasm.last_stop = Some(stop);
                return Ok(stop_code(stop));
            }
            let outcome = wasm.emu.run_one_frame(DEFAULT_FRAME_TIMEOUT_MS)?;
            if let wemu::FrameOutcome::Stopped(stop) = outcome {
                wasm.last_stop = Some(stop);
                Ok(stop_code(stop))
            } else {
                Ok(0)
            }
        })()
    };
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_mouse_move(handle: u32, x: u32, y: u32) {
    if let Some(wasm) = instance_mut(handle) {
        wasm.emu.hle.post_mouse_move(x, y);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_mouse_down(handle: u32, x: u32, y: u32) {
    if let Some(wasm) = instance_mut(handle) {
        wasm.emu.hle.post_mouse_button_down(x, y);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_mouse_up(handle: u32, x: u32, y: u32) {
    if let Some(wasm) = instance_mut(handle) {
        wasm.emu.hle.post_mouse_button_up(x, y);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_click(handle: u32, x: u32, y: u32) {
    if let Some(wasm) = instance_mut(handle) {
        wasm.emu.hle.post_click(x, y);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_key_down(handle: u32, vk: u32) {
    if let Some(wasm) = instance_mut(handle) {
        wasm.emu.hle.post_key_down(vk);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_key_up(handle: u32, vk: u32) {
    if let Some(wasm) = instance_mut(handle) {
        wasm.emu.hle.post_key_up(vk);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_text(handle: u32, text_ptr: u32, text_len: u32) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = unsafe {
        (|| -> Result<i32> {
            let text = read_string(text_ptr, text_len)?;
            wasm.emu.hle.post_text(&text);
            Ok(0)
        })()
    };
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_screenshot_png(handle: u32) -> i32 {
    let Some(wasm) = instance_mut(handle) else {
        return -1;
    };
    let result = (|| -> Result<i32> {
        wemu::hle::flush_gdi_present_if_pending(&mut wasm.emu)?;
        let png = png::encode_rgba_png(
            wasm.emu.backend.width(),
            wasm.emu.backend.height(),
            wasm.emu.backend.framebuffer(),
        )?;
        wasm.last_blob = png;
        Ok(0)
    })();
    finish_i32(wasm, result)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_framebuffer_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.backend.framebuffer().as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_framebuffer_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.backend.framebuffer().len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_width(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.backend.width())
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_height(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.backend.height())
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_blob_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.last_blob.as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_blob_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.last_blob.len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_last_error_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.last_error.as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_last_error_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.last_error.len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_last_hle_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.last_hle_call_symbol().as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_last_hle_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.last_hle_call_symbol().len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_missing_hle_report_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.missing_hle_report().as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_missing_hle_report_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.missing_hle_report().len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_output_ptr(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.output().as_ptr() as usize as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_output_len(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.hle.output().len() as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_stop_reason(handle: u32) -> i32 {
    instance_mut(handle)
        .and_then(|wasm| wasm.last_stop)
        .map(stop_code)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_exit_code(handle: u32) -> u32 {
    match instance_mut(handle).and_then(|wasm| wasm.last_stop) {
        Some(StopReason::ExitProcess(code)) => code,
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn wemu_insns_lo(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.insns as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_insns_hi(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| (wasm.emu.insns >> 32) as u32)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_eip(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.cpu.eip)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_eflags(handle: u32) -> u32 {
    instance_mut(handle)
        .map(|wasm| wasm.emu.cpu.eflags)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn wemu_reg(handle: u32, index: u32) -> u32 {
    let Some(wasm) = instance_mut(handle) else {
        return 0;
    };
    let reg = match index {
        0 => Reg::Eax,
        1 => Reg::Ecx,
        2 => Reg::Edx,
        3 => Reg::Ebx,
        4 => Reg::Esp,
        5 => Reg::Ebp,
        6 => Reg::Esi,
        7 => Reg::Edi,
        _ => return 0,
    };
    wasm.emu.cpu.reg(reg)
}

#[no_mangle]
pub extern "C" fn wemu_guest_ram_base() -> u32 {
    GUEST_RAM_BASE
}

#[no_mangle]
pub extern "C" fn wemu_guest_ram_end() -> u32 {
    GUEST_RAM_END
}

unsafe fn instance_mut<'a>(handle: u32) -> Option<&'a mut WasmEmulator> {
    if handle == 0 {
        None
    } else {
        Some(unsafe { &mut *(handle as usize as *mut WasmEmulator) })
    }
}

unsafe fn read_bytes<'a>(ptr: u32, len: u32) -> Result<&'a [u8]> {
    if ptr == 0 && len != 0 {
        return Err(Error::Cli("null wasm pointer".to_string()));
    }
    Ok(unsafe { slice::from_raw_parts(ptr as usize as *const u8, len as usize) })
}

unsafe fn read_string(ptr: u32, len: u32) -> Result<String> {
    let bytes = unsafe { read_bytes(ptr, len)? };
    String::from_utf8(bytes.to_vec())
        .map_err(|err| Error::Cli(format!("wasm string is not utf-8: {err}")))
}

fn decode_frame_input(bytes: &[u8]) -> Result<Vec<BackendEvent>> {
    const RECORD_BYTES: usize = 16;
    const EVENT_QUIT: u32 = 1;
    const EVENT_MOUSE_MOVE: u32 = 2;
    const EVENT_MOUSE_DOWN: u32 = 3;
    const EVENT_MOUSE_UP: u32 = 4;
    const EVENT_MOUSE_RIGHT_DOWN: u32 = 5;
    const EVENT_MOUSE_RIGHT_UP: u32 = 6;
    const EVENT_KEY_DOWN: u32 = 7;
    const EVENT_KEY_UP: u32 = 8;
    const EVENT_TEXT_CHAR: u32 = 9;

    if bytes.len() % RECORD_BYTES != 0 {
        return Err(Error::Cli(format!(
            "frame input byte length {} is not a multiple of {RECORD_BYTES}",
            bytes.len()
        )));
    }
    let mut events = Vec::with_capacity(bytes.len() / RECORD_BYTES);
    for record in bytes.chunks_exact(RECORD_BYTES) {
        let kind = le_u32(&record[0..4]);
        let a = le_u32(&record[4..8]);
        let b = le_u32(&record[8..12]);
        let event = match kind {
            EVENT_QUIT => BackendEvent::Quit,
            EVENT_MOUSE_MOVE => BackendEvent::MouseMove { x: a, y: b },
            EVENT_MOUSE_DOWN => BackendEvent::MouseButtonDown { x: a, y: b },
            EVENT_MOUSE_UP => BackendEvent::MouseButtonUp { x: a, y: b },
            EVENT_MOUSE_RIGHT_DOWN => BackendEvent::MouseRightButtonDown { x: a, y: b },
            EVENT_MOUSE_RIGHT_UP => BackendEvent::MouseRightButtonUp { x: a, y: b },
            EVENT_KEY_DOWN => BackendEvent::KeyDown { vk: a },
            EVENT_KEY_UP => BackendEvent::KeyUp { vk: a },
            EVENT_TEXT_CHAR => {
                let ch = char::from_u32(a)
                    .ok_or_else(|| Error::Cli(format!("invalid frame text character U+{a:04X}")))?;
                BackendEvent::Text {
                    text: ch.to_string(),
                }
            }
            _ => return Err(Error::Cli(format!("unknown frame input event kind {kind}"))),
        };
        events.push(event);
    }
    Ok(events)
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn finish_i32(wasm: &mut WasmEmulator, result: Result<i32>) -> i32 {
    match result {
        Ok(value) => {
            wasm.clear_error();
            value
        }
        Err(err) => {
            wasm.set_error(err);
            -1
        }
    }
}

fn guest_parent_dir(path: &str) -> (char, String) {
    let normalized = path.replace('/', "\\");
    let bytes = normalized.as_bytes();
    let absolute_drive = bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    let drive = if absolute_drive {
        bytes[0].to_ascii_uppercase() as char
    } else {
        'C'
    };
    let rest = if absolute_drive {
        &normalized[2..]
    } else {
        normalized.as_str()
    };
    let parent = rest.rfind('\\').map(|pos| &rest[..pos]).unwrap_or("\\");
    let parent = if parent.is_empty() { "\\" } else { parent };
    (drive, parent.to_string())
}

fn stop_code(stop: StopReason) -> i32 {
    match stop {
        StopReason::ExitProcess(_) => 1,
        StopReason::MaxInstructions => 2,
        StopReason::Breakpoint(_) => 3,
        StopReason::HleBooted(_) => 4,
        StopReason::CpuHalted => 5,
        StopReason::FrontendQuit => 6,
        StopReason::Waiting => 7,
    }
}
