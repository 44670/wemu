pub(crate) mod app_db;
pub mod arena;
pub mod backend;
pub mod cpu;
pub mod debugger;
pub mod guest_path;
pub mod hle;
pub mod journal;
pub mod memory;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod native_zip;
pub mod pe;
pub mod png;
pub mod text_encoding;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::guest_path::GuestPath;

const HEADLESS_INSNS_PER_MS: u64 = 1000;
const STATE_CHECK_INSNS: u64 = 1_024;
const FRAME_DEADLINE_CHECK_INSNS: u64 = 4_096;
#[cfg(not(target_arch = "wasm32"))]
const LIVE_WAIT_SLEEP_MS: u64 = 8;
const DEFAULT_FRONTEND_FPS: u32 = 60;
const DEFAULT_SCREEN_WIDTH: u32 = 800;
const DEFAULT_SCREEN_HEIGHT: u32 = 600;
pub const DEFAULT_FRAME_TIMEOUT_MS: u32 = 1_000;

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn wemu_now_ms() -> f64;
}

#[inline]
fn host_time_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        let now = unsafe { wemu_now_ms() };
        return if now.is_finite() && now > 0.0 {
            now as u64
        } else {
            0
        };
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_millis() as u64
    }
}

fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch < ' ' => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn push_hex_byte(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0f) as usize] as char);
}

fn default_mount_root(exe: &Path) -> Result<PathBuf> {
    exe_parent_for_default_mount(exe)
}

fn exe_parent_for_default_mount(exe: &Path) -> Result<PathBuf> {
    if exe.as_os_str().is_empty() {
        return Ok(PathBuf::from("."));
    }
    Ok(exe
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf())
}

fn default_guest_exe_path(exe: &Path, c_root: &Path) -> Result<String> {
    if exe.as_os_str().is_empty() {
        return Err(Error::Cli("--cmdline is required".to_string()));
    }
    if let Some(raw) = path_as_guest_path(exe) {
        return Ok(GuestPath::resolve(&raw, 'C', "\\").display_path());
    }
    let rel = path_relative_to_root(exe, c_root).unwrap_or_else(|| {
        exe.file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| exe.to_path_buf())
    });
    let mut parts = Vec::new();
    for component in rel.components() {
        if let std::path::Component::Normal(part) = component {
            parts.push(part.to_string_lossy().to_string());
        }
    }
    if parts.is_empty() {
        let name = exe
            .file_name()
            .ok_or_else(|| Error::Cli(format!("invalid executable path {}", exe.display())))?;
        parts.push(name.to_string_lossy().to_string());
    }
    Ok(format!("C:\\{}", parts.join("\\")))
}

fn path_relative_to_root(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(rel.to_path_buf());
    }
    let abs_path = path.canonicalize().ok()?;
    let abs_root = root.canonicalize().ok()?;
    abs_path.strip_prefix(abs_root).ok().map(Path::to_path_buf)
}

fn first_command_line_token(command_line: &str) -> Option<String> {
    let s = command_line.trim_start();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"').unwrap_or(rest.len());
        let token = &rest[..end];
        return (!token.is_empty()).then(|| token.to_string());
    }
    s.split_whitespace().next().map(str::to_string)
}

fn guest_parent_dir(raw: &str, cwd_drive: char, cwd_path: &str) -> (char, String) {
    GuestPath::resolve(raw, cwd_drive, cwd_path).parent_dir()
}

fn push_unique_path(out: &mut Vec<PathBuf>, path: PathBuf) {
    if !out.iter().any(|old| old == &path) {
        out.push(path);
    }
}

fn case_insensitive_existing_dir(path: &Path) -> Option<PathBuf> {
    use std::path::Component;

    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => {
                current.push(component.as_os_str());
            }
            Component::Normal(part) => {
                let dir = if current.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    current.as_path()
                };
                let wanted = part.to_string_lossy();
                let found = fs::read_dir(dir).ok()?.find_map(|entry| {
                    let entry = entry.ok()?;
                    entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&wanted)
                        .then_some(entry.path())
                })?;
                current = found;
            }
        }
    }
    current.is_dir().then_some(current)
}

fn host_named_directory_candidates(
    exe_host_path: Option<&Path>,
    c_root: &Path,
    name: &str,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(exe_host_path) = exe_host_path {
        let mut current = exe_host_path.parent();
        while let Some(dir) = current {
            push_unique_path(&mut out, dir.join(name));
            current = dir.parent();
        }
    }
    push_unique_path(&mut out, c_root.join(name));
    if let Some(parent) = c_root.parent() {
        push_unique_path(&mut out, parent.join(name));
    }
    out
}

fn find_existing_host_dir(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    for path in candidates {
        if path.is_dir() {
            return Some(path);
        }
        if let Some(path) = case_insensitive_existing_dir(&path) {
            return Some(path);
        }
    }
    None
}

fn append_command_line_args(mut command_line: String, args: &[String]) -> String {
    for arg in args {
        command_line.push(' ');
        command_line.push_str(arg);
    }
    command_line
}

fn path_as_guest_path(path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let bytes = raw.as_bytes();
    (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
        .then(|| raw.to_string())
}

#[cfg(feature = "sdl2")]
use backend::SdlBackend;
use backend::{Backend, BackendEvent, HeadlessBackend};
use cpu::{Cpu, Reg, StepOutcome};
#[cfg(test)]
use hle::MessageFilter;
use hle::{Hle, HleWaitState};
use journal::{Journal, JournalEvent, JournalRecorder};
use memory::{Memory, PagePerm, WriteContext, WriteRegisters};
use pe::PeImage;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Pe(String),
    Memory(String),
    Cpu(String),
    Hle(String),
    Cli(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "io error: {err}"),
            Error::Pe(err) => write!(f, "pe error: {err}"),
            Error::Memory(err) => write!(f, "memory error: {err}"),
            Error::Cpu(err) => write!(f, "cpu error: {err}"),
            Error::Hle(err) => write!(f, "hle error: {err}"),
            Error::Cli(err) => write!(f, "cli error: {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendKind {
    Headless,
    Sdl2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalInput {
    Path(PathBuf),
    Inline(String),
}

impl JournalInput {
    pub fn from_cli(value: String) -> Self {
        if let Some(script) = value.strip_prefix("inline:") {
            return Self::Inline(decode_inline_journal(script));
        }
        if value.contains('\n') {
            return Self::Inline(value);
        }
        Self::Path(PathBuf::from(value))
    }

    fn load(&self) -> Result<Journal> {
        match self {
            Self::Path(path) => Journal::from_path(path),
            Self::Inline(script) => Journal::parse(script),
        }
    }

    fn path(&self) -> Option<&Path> {
        match self {
            Self::Path(path) => Some(path.as_path()),
            Self::Inline(_) => None,
        }
    }
}

fn decode_inline_journal(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    let mut chars = script.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else if ch == ';' {
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub exe: PathBuf,
    pub zip: Option<PathBuf>,
    pub cmdline: Option<String>,
    pub args: Vec<String>,
    pub cwd_drive: char,
    pub cwd_path: String,
    pub mounts: Vec<(char, PathBuf)>,
    pub max_insns: u64,
    pub breakpoints: Vec<u32>,
    pub screenshot: Option<PathBuf>,
    pub journal: Option<JournalInput>,
    pub record: Option<PathBuf>,
    pub frontend: FrontendKind,
    pub frontend_fps: u32,
    pub sdl_ws: Option<String>,
    pub trace: bool,
    pub trace_after: u64,
    pub debug_on_crash: bool,
    pub strict_hle_imports: bool,
    pub state_interval: Option<Duration>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            exe: PathBuf::new(),
            zip: None,
            cmdline: None,
            args: Vec::new(),
            cwd_drive: 'C',
            cwd_path: String::new(),
            mounts: Vec::new(),
            max_insns: u64::MAX,
            breakpoints: Vec::new(),
            screenshot: Some(PathBuf::from("/tmp/wemu.png")),
            journal: None,
            record: None,
            frontend: FrontendKind::Headless,
            frontend_fps: DEFAULT_FRONTEND_FPS,
            sdl_ws: None,
            trace: false,
            trace_after: 0,
            debug_on_crash: false,
            strict_hle_imports: cfg!(debug_assertions),
            state_interval: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    ExitProcess(u32),
    MaxInstructions,
    Breakpoint(u32),
    HleBooted(&'static str),
    CpuHalted,
    FrontendQuit,
    Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    Presented,
    Waiting,
    TimedOut,
    Stopped(StopReason),
}

#[derive(Clone)]
pub struct HleTask {
    pub id: u32,
    pub cpu: Cpu,
    pub wait: HleWaitState,
}

impl HleTask {
    fn new(id: u32, cpu: Cpu) -> Self {
        Self {
            id,
            cpu,
            wait: HleWaitState::Ready,
        }
    }
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
struct EipHistoryEntry {
    insns: u64,
    task_id: u32,
    kind: &'static str,
    eip: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esp: u32,
    ebp: u32,
    esi: u32,
    edi: u32,
}

#[cfg(debug_assertions)]
struct EipHistory {
    entries: Vec<EipHistoryEntry>,
    next: usize,
    len: usize,
}

#[cfg(debug_assertions)]
impl EipHistory {
    fn from_env() -> Self {
        let Some(value) = std::env::var_os("WEMU_EIP_HISTORY") else {
            return Self::disabled();
        };
        let value = value.to_string_lossy();
        let cap = if value.is_empty() {
            4096
        } else {
            value.parse::<usize>().unwrap_or(4096)
        };
        if cap == 0 {
            return Self::disabled();
        }
        Self {
            entries: Vec::with_capacity(cap),
            next: 0,
            len: 0,
        }
    }

    fn disabled() -> Self {
        Self {
            entries: Vec::new(),
            next: 0,
            len: 0,
        }
    }

    fn enabled(&self) -> bool {
        self.entries.capacity() != 0
    }

    fn push(&mut self, entry: EipHistoryEntry) {
        if !self.enabled() {
            return;
        }
        let cap = self.entries.capacity();
        if self.entries.len() < cap {
            self.entries.push(entry);
        } else {
            self.entries[self.next] = entry;
        }
        self.next = (self.next + 1) % cap;
        self.len = self.len.saturating_add(1).min(cap);
    }

    fn format_recent(&self, max_entries: usize) -> String {
        if !self.enabled() || self.len == 0 {
            return String::new();
        }
        let cap = self.entries.capacity();
        let start = if self.len == cap { self.next } else { 0 };
        let first = self.len.saturating_sub(max_entries);
        let mut out = String::from("\nrecent EIP history, oldest first:");
        for logical in first..self.len {
            let entry = self.entries[(start + logical) % cap];
            out.push_str(&format!(
                "\n  #{:>5} task={} {:<3} eip={:08x} eax={:08x} ecx={:08x} edx={:08x} ebx={:08x} esp={:08x} ebp={:08x} esi={:08x} edi={:08x}",
                entry.insns,
                entry.task_id,
                entry.kind,
                entry.eip,
                entry.eax,
                entry.ecx,
                entry.edx,
                entry.ebx,
                entry.esp,
                entry.ebp,
                entry.esi,
                entry.edi,
            ));
        }
        out
    }
}

pub struct Emulator {
    pub memory: Memory,
    pub cpu: Cpu,
    pub hle: Hle,
    pub backend: Box<dyn Backend>,
    pub image: Option<PeImage>,
    pub insns: u64,
    pub max_insns: u64,
    pub breakpoints: Vec<u32>,
    pub trace: bool,
    pub trace_after: u64,
    pub stopped: Option<StopReason>,
    pub argv: Vec<String>,
    guest_module_file_name: String,
    guest_command_line: String,
    pub journal: Journal,
    pub recorder: Option<JournalRecorder>,
    pub guest_time_ms: u64,
    pub present_generation: u64,
    scheduler_frame: u64,
    pub hle_tasks: Vec<HleTask>,
    current_hle_task: usize,
    ui_clock_start_ms: Option<u64>,
    state_interval: Option<Duration>,
    state_start: Option<Instant>,
    state_next_print: Option<Instant>,
    state_last_print: Option<(Instant, u64)>,
    state_next_check_insns: u64,
    last_hle_call_at: Option<Instant>,
    last_hle_call_symbol: String,
    #[cfg(debug_assertions)]
    eip_history: EipHistory,
}

impl Emulator {
    pub fn new() -> Self {
        Self {
            memory: Memory::new(),
            cpu: Cpu::new(),
            hle: Hle::new(),
            backend: Box::new(HeadlessBackend::new(
                DEFAULT_SCREEN_WIDTH,
                DEFAULT_SCREEN_HEIGHT,
            )),
            image: None,
            insns: 0,
            max_insns: u64::MAX,
            breakpoints: Vec::new(),
            trace: false,
            trace_after: 0,
            stopped: None,
            argv: Vec::new(),
            guest_module_file_name: String::new(),
            guest_command_line: String::new(),
            journal: Journal::default(),
            recorder: None,
            guest_time_ms: 0,
            present_generation: 0,
            scheduler_frame: 0,
            hle_tasks: vec![HleTask::new(1, Cpu::new())],
            current_hle_task: 0,
            ui_clock_start_ms: None,
            state_interval: None,
            state_start: None,
            state_next_print: None,
            state_last_print: None,
            state_next_check_insns: 0,
            last_hle_call_at: None,
            last_hle_call_symbol: String::new(),
            #[cfg(debug_assertions)]
            eip_history: EipHistory::disabled(),
        }
    }

    pub fn configure(&mut self, cfg: &RunConfig) -> Result<()> {
        self.max_insns = cfg.max_insns;
        self.breakpoints = cfg.breakpoints.clone();
        self.trace = cfg.trace;
        self.trace_after = cfg.trace_after;
        self.state_interval = cfg.state_interval.filter(|interval| !interval.is_zero());
        self.hle
            .set_strict_hle_imports(cfg.strict_hle_imports || cfg!(debug_assertions));
        self.state_start = None;
        self.state_next_print = None;
        self.state_last_print = None;
        self.state_next_check_insns = 0;
        self.last_hle_call_at = None;
        self.last_hle_call_symbol.clear();
        #[cfg(debug_assertions)]
        {
            self.eip_history = EipHistory::from_env();
        }
        self.present_generation = 0;
        self.scheduler_frame = 0;
        self.ui_clock_start_ms = None;
        self.memory.configure_write_watch_from_env()?;
        self.backend = create_backend(cfg.frontend, cfg.sdl_ws.as_deref())?;
        if self.backend.uses_wall_clock() && cfg.frontend_fps != 0 {
            self.set_frontend_timing(cfg.frontend_fps, 0);
        }
        self.argv = cfg.args.clone();
        self.journal = if let Some(input) = &cfg.journal {
            input.load()?
        } else {
            Journal::default()
        };
        if let (Some(replay), Some(record)) = (&cfg.journal, &cfg.record) {
            if replay.path() == Some(record.as_path()) {
                return Err(Error::Cli(
                    "--record must not point at the same file as --replay".to_string(),
                ));
            }
        }
        self.recorder = if let Some(path) = &cfg.record {
            Some(JournalRecorder::from_path(path)?)
        } else {
            None
        };
        if cfg.zip.is_some() {
            return Ok(());
        }

        let default_root = default_mount_root(&cfg.exe)?;
        let c_root = cfg
            .mounts
            .iter()
            .find(|(drive, _)| drive.to_ascii_uppercase() == 'C')
            .map(|(_, path)| path.clone())
            .unwrap_or(default_root);
        self.hle.set_drive_mount('C', c_root.clone());
        self.hle.set_drive_mount('D', c_root.clone());
        for (drive, path) in &cfg.mounts {
            self.hle.set_drive_mount(*drive, path.clone());
        }
        let module_file_name = if let Some(cmdline) = &cfg.cmdline {
            let token = first_command_line_token(cmdline).ok_or_else(|| {
                Error::Cli("--cmdline must start with the guest executable path".to_string())
            })?;
            GuestPath::resolve(&token, cfg.cwd_drive, &cfg.cwd_path).display_path()
        } else {
            default_guest_exe_path(&cfg.exe, &c_root)?
        };
        let exe_host_path = self.hle.host_path_for_guest(&module_file_name).ok();
        self.apply_app_db_host_mounts(
            &module_file_name,
            exe_host_path.as_deref(),
            &c_root,
            &cfg.mounts,
        );
        if cfg.cwd_path.is_empty() {
            let (cwd_drive, cwd_path) =
                guest_parent_dir(&module_file_name, cfg.cwd_drive, &cfg.cwd_path);
            self.hle.set_cwd(cwd_drive, cwd_path);
        } else {
            self.hle.set_cwd(cfg.cwd_drive, cfg.cwd_path.clone());
        }
        self.guest_module_file_name = module_file_name.clone();
        self.guest_command_line = append_command_line_args(
            cfg.cmdline.clone().unwrap_or(module_file_name),
            cfg.args.as_slice(),
        );
        Ok(())
    }

    fn apply_app_db_host_mounts(
        &mut self,
        module_file_name: &str,
        exe_host_path: Option<&Path>,
        c_root: &Path,
        explicit_mounts: &[(char, PathBuf)],
    ) {
        let Some(entry) = app_db::find_by_exe_path(module_file_name) else {
            return;
        };
        let explicit = |drive: char| {
            explicit_mounts
                .iter()
                .any(|(mounted, _)| mounted.eq_ignore_ascii_case(&drive))
        };
        for asset in entry.required_assets {
            let Some(mount) = asset.mount else {
                continue;
            };
            if asset.asset_type != "directory" || asset.locator != "named-directory" {
                continue;
            }
            if explicit(mount.drive) {
                self.hle.set_drive_device(mount.drive, mount.device);
                continue;
            }
            let candidates = host_named_directory_candidates(exe_host_path, c_root, asset.name);
            if let Some(path) = find_existing_host_dir(candidates) {
                self.hle.set_drive_mount(mount.drive, path);
                self.hle.set_drive_device(mount.drive, mount.device);
            }
        }
    }

    pub fn apply_app_db_virtual_mounts_for_exe(&mut self, module_file_name: &str) {
        let Some(entry) = app_db::find_by_exe_path(module_file_name) else {
            return;
        };
        for asset in entry.required_assets {
            let Some(mount) = asset.mount else {
                continue;
            };
            if asset.asset_type != "directory" || asset.locator != "named-directory" {
                continue;
            }
            if self.hle.drive_is_mounted(mount.drive) {
                self.hle.set_drive_device(mount.drive, mount.device);
                continue;
            }
            if let Some(target) = self
                .hle
                .find_virtual_named_directory_near(module_file_name, asset.name)
            {
                self.hle
                    .set_virtual_drive_alias(mount.drive, &target, mount.device);
            }
        }
    }

    pub fn load_exe(&mut self, exe: &Path) -> Result<()> {
        let image = pe::load_pe32(exe, &mut self.memory, &mut self.hle)?;
        self.hle.check_strict_hle_imports()?;
        self.finish_load_exe(exe, image)
    }

    pub fn load_exe_bytes(&mut self, exe: &Path, bytes: &[u8]) -> Result<()> {
        let image = pe::load_pe32_bytes(exe.to_path_buf(), bytes, &mut self.memory, &mut self.hle)?;
        self.hle.check_strict_hle_imports()?;
        self.finish_load_exe(exe, image)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_zip_config(&mut self, cfg: &RunConfig) -> Result<()> {
        let zip_path = cfg
            .zip
            .as_deref()
            .ok_or_else(|| Error::Cli("--zip is required".to_string()))?;
        let zip = native_zip::read_zip(zip_path)?;
        if zip.exes.is_empty() {
            return Err(Error::Cli(format!(
                "ZIP {} has no .exe files",
                zip_path.display()
            )));
        }

        let selected_exe = if let Some(cmdline) = &cfg.cmdline {
            let token = first_command_line_token(cmdline).ok_or_else(|| {
                Error::Cli("--cmdline must start with the guest executable path".to_string())
            })?;
            GuestPath::resolve(&token, cfg.cwd_drive, &cfg.cwd_path).display_path()
        } else if zip.exes.len() == 1 {
            zip.exes[0].clone()
        } else {
            return Err(Error::Cli(format!(
                "ZIP {} has multiple .exe files; pass --cmdline with one of:\n  {}",
                zip_path.display(),
                zip.exes.join("\n  ")
            )));
        };

        let selected_key = self.hle.vfs_key_for_guest(&selected_exe);
        let mut exe_bytes = None;
        for file in zip.files {
            let is_selected = self.hle.vfs_key_for_guest(&file.guest_path) == selected_key;
            if is_selected {
                exe_bytes = Some(file.data.clone());
            }
            self.hle.add_virtual_file_owned(&file.guest_path, file.data);
        }
        let exe_bytes = exe_bytes.ok_or_else(|| {
            Error::Cli(format!(
                "{} is not present in ZIP {}; available .exe files:\n  {}",
                selected_exe,
                zip_path.display(),
                zip.exes.join("\n  ")
            ))
        })?;

        self.apply_app_db_virtual_mounts_for_exe(&selected_exe);
        if cfg.cwd_path.is_empty() {
            let (cwd_drive, cwd_path) =
                guest_parent_dir(&selected_exe, cfg.cwd_drive, &cfg.cwd_path);
            self.hle.set_cwd(cwd_drive, cwd_path);
        } else {
            self.hle.set_cwd(cfg.cwd_drive, cfg.cwd_path.clone());
        }
        self.guest_module_file_name = selected_exe.clone();
        let command_line = cfg.cmdline.clone().unwrap_or_else(|| selected_exe.clone());
        self.guest_command_line = append_command_line_args(command_line, cfg.args.as_slice());
        self.load_exe_bytes(Path::new(&selected_exe), &exe_bytes)
    }

    fn resolve_load_exe(&self, cfg: &RunConfig) -> Result<PathBuf> {
        if !cfg.exe.as_os_str().is_empty() {
            if let Some(raw) = path_as_guest_path(&cfg.exe) {
                return self.hle.host_path_for_guest(&raw);
            }
            return Ok(cfg.exe.clone());
        }
        let cmdline = cfg
            .cmdline
            .as_deref()
            .ok_or_else(|| Error::Cli("--cmdline is required".to_string()))?;
        let token = first_command_line_token(cmdline).ok_or_else(|| {
            Error::Cli("--cmdline must start with the guest executable path".to_string())
        })?;
        self.hle.host_path_for_guest(&token)
    }

    fn finish_load_exe(&mut self, exe: &Path, image: PeImage) -> Result<()> {
        let exe_entry = image.entry;
        let stack_base = 0x0f00_0000;
        let stack_size = 0x0010_0000;
        self.memory
            .map(stack_base, stack_size, PagePerm::READ | PagePerm::WRITE)?;
        let stack_top = stack_base + stack_size - 0x10;

        let module_file_name = if self.guest_module_file_name.is_empty() {
            default_guest_exe_path(exe, &exe_parent_for_default_mount(exe)?)?
        } else {
            self.guest_module_file_name.clone()
        };
        let command_line = if self.guest_command_line.is_empty() {
            append_command_line_args(module_file_name.clone(), &self.argv)
        } else {
            self.guest_command_line.clone()
        };
        self.hle.bootstrap_process_strings(
            &mut self.memory,
            image.image_base,
            &module_file_name,
            &command_line,
        )?;

        self.cpu = Cpu::new();
        self.cpu.eip = exe_entry;
        self.cpu.set_reg(cpu::Reg::Esp, stack_top);
        self.cpu.set_reg(cpu::Reg::Ebp, stack_top);
        let exit_process = self.hle.resolve_import("kernel32.dll", "ExitProcess");
        self.memory.write_u32(stack_top, exit_process)?;
        self.memory.write_u32(stack_top + 4, 0)?;
        self.memory.write_u32(stack_top + 8, 0)?;
        #[cfg(target_arch = "wasm32")]
        let (teb_base, peb_base) = {
            let teb_base = self.hle.alloc_private(
                &mut self.memory,
                0x1000,
                PagePerm::READ | PagePerm::WRITE,
            )?;
            let peb_base = self.hle.alloc_private(
                &mut self.memory,
                0x1000,
                PagePerm::READ | PagePerm::WRITE,
            )?;
            (teb_base, peb_base)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let (teb_base, peb_base) = {
            let teb_base = 0x7ffd_e000;
            let peb_base = 0x7ffd_f000;
            self.memory
                .map_or_update(teb_base, 0x1000, PagePerm::READ | PagePerm::WRITE)?;
            self.memory
                .map_or_update(peb_base, 0x1000, PagePerm::READ | PagePerm::WRITE)?;
            (teb_base, peb_base)
        };
        self.memory.write_u32(teb_base, 0xffff_ffff)?;
        self.memory.write_u32(teb_base + 0x18, teb_base)?;
        // Old PE stubs read TEB/PEB fields directly through FS instead of
        // calling IsDebuggerPresent, so keep the core self/PEB layout real.
        self.memory.write_u32(teb_base + 0x30, peb_base)?;
        self.memory.write_u8(peb_base + 0x02, 0)?;
        self.cpu.set_segment_base(4, teb_base);
        self.cpu.eflags = cpu::FLAG_IF | 0x2;
        let dll_entries = self.hle.dll_process_attach_entries();
        if !dll_entries.is_empty() {
            let chain_esp = stack_top.wrapping_sub((dll_entries.len() as u32) * 16);
            for (index, (module, entry)) in dll_entries.iter().enumerate() {
                let frame = chain_esp.wrapping_add((index as u32) * 16);
                let next = dll_entries
                    .get(index + 1)
                    .map(|(_, next_entry)| *next_entry)
                    .unwrap_or(exe_entry);
                self.memory.write_u32(frame, next)?;
                self.memory.write_u32(frame + 4, *module)?;
                self.memory.write_u32(frame + 8, 1)?; // DLL_PROCESS_ATTACH
                self.memory.write_u32(frame + 12, 0)?;
                if index == 0 {
                    self.cpu.eip = *entry;
                    self.cpu.set_reg(cpu::Reg::Esp, frame);
                    self.cpu.set_reg(cpu::Reg::Ebp, frame);
                }
            }
        }
        self.image = Some(image);
        self.reset_hle_tasks();
        Ok(())
    }

    pub fn run(&mut self) -> Result<StopReason> {
        if self.backend.uses_wall_clock() {
            loop {
                match self.run_one_frame(DEFAULT_FRAME_TIMEOUT_MS)? {
                    FrameOutcome::Stopped(reason) => return Ok(reason),
                    FrameOutcome::Waiting => {
                        #[cfg(not(target_arch = "wasm32"))]
                        std::thread::sleep(Duration::from_millis(LIVE_WAIT_SLEEP_MS));
                    }
                    FrameOutcome::TimedOut => {
                        #[cfg(not(target_arch = "wasm32"))]
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    FrameOutcome::Presented => {}
                }
            }
        }

        loop {
            self.maybe_print_state(false);
            if !self.backend.uses_wall_clock() {
                self.refresh_guest_time();
                self.hle.pump_timers(self.guest_time_ms, 0);
                self.pump_journal();
                self.wake_hle_tasks()?;
                if let Some(reason) = self.handle_headless_wait()? {
                    return Ok(reason);
                }
                if hle::dispatch_due_mm_timer_interrupt(self) {
                    continue;
                }
            }
            if let Some(reason) = self.run_one_instruction()? {
                return Ok(reason);
            }
        }
    }

    pub fn run_for(&mut self, instruction_budget: u32) -> Result<Option<StopReason>> {
        if let Some(reason) = self.stopped {
            return Ok(Some(reason));
        }
        let target = self.insns.saturating_add(instruction_budget as u64);
        let mut next_service_insns = self.insns;
        while self.insns < target {
            self.maybe_print_state(false);
            if !self.backend.uses_wall_clock() {
                self.refresh_guest_time();
                self.hle.pump_timers(self.guest_time_ms, 0);
                self.pump_journal();
                if hle::dispatch_due_mm_timer_interrupt(self) {
                    continue;
                }
            } else if self.insns >= next_service_insns {
                self.service_frontend()?;
                if let Some(reason) = self.stopped {
                    return Ok(Some(reason));
                }
                if self.current_hle_task_waiting() {
                    return Ok(None);
                }
                next_service_insns = self.insns.saturating_add(FRAME_DEADLINE_CHECK_INSNS);
            }
            if let Some(reason) = self.run_one_instruction_without_max()? {
                return Ok(Some(reason));
            }
            if self.current_hle_task_waiting() {
                return Ok(None);
            }
        }
        Ok(None)
    }

    pub fn run_one_frame(&mut self, timeout_ms: u32) -> Result<FrameOutcome> {
        if let Some(reason) = self.stopped {
            return Ok(FrameOutcome::Stopped(reason));
        }

        let frame_start_host_ms = host_time_ms();
        let hard_deadline_host_ms = frame_start_host_ms.saturating_add(timeout_ms as u64);
        self.begin_scheduler_frame();
        self.hle.begin_frame();
        self.begin_frontend_frame()?;
        let start_present = self.present_generation;
        if let Some(reason) = self.stopped {
            hle::flush_gdi_present_if_pending(self)?;
            return Ok(FrameOutcome::Stopped(reason));
        }
        if let Some(outcome) = self.yield_if_no_task_runnable()? {
            return Ok(outcome);
        }

        let mut next_check_insns = self.insns.saturating_add(FRAME_DEADLINE_CHECK_INSNS);

        loop {
            self.maybe_print_state(false);
            if let Some(reason) = self.run_one_instruction()? {
                hle::flush_gdi_present_if_pending(self)?;
                return Ok(FrameOutcome::Stopped(reason));
            }
            if self.present_generation != start_present {
                return Ok(FrameOutcome::Presented);
            }
            if let Some(outcome) = self.yield_if_no_task_runnable()? {
                return Ok(outcome);
            }
            if self.hle.take_cooperative_idle() {
                if hle::flush_gdi_present_if_pending(self)? {
                    return Ok(FrameOutcome::Presented);
                }
                return Ok(FrameOutcome::Waiting);
            }
            if self.insns >= next_check_insns {
                self.service_guest()?;
                if let Some(reason) = self.stopped {
                    hle::flush_gdi_present_if_pending(self)?;
                    return Ok(FrameOutcome::Stopped(reason));
                }
                if self.present_generation != start_present {
                    return Ok(FrameOutcome::Presented);
                }
                if let Some(outcome) = self.yield_if_no_task_runnable()? {
                    return Ok(outcome);
                }
                if host_time_ms() >= hard_deadline_host_ms {
                    if hle::flush_gdi_present_if_pending(self)? {
                        return Ok(FrameOutcome::Presented);
                    }
                    return Ok(FrameOutcome::TimedOut);
                }
                next_check_insns = self.insns.saturating_add(FRAME_DEADLINE_CHECK_INSNS);
            }
        }
    }

    fn yield_if_no_task_runnable(&mut self) -> Result<Option<FrameOutcome>> {
        if !self.current_hle_task_waiting() || self.select_ready_hle_task() {
            return Ok(None);
        }
        if hle::flush_gdi_present_if_pending(self)? {
            return Ok(Some(FrameOutcome::Presented));
        }
        Ok(Some(FrameOutcome::Waiting))
    }

    fn handle_headless_wait(&mut self) -> Result<Option<StopReason>> {
        if !self.current_hle_task_waiting() || self.select_ready_hle_task() {
            return Ok(None);
        }
        if self.max_insns != u64::MAX && self.insns >= self.max_insns {
            self.stopped = Some(StopReason::MaxInstructions);
            return Ok(Some(StopReason::MaxInstructions));
        }
        if let Some(wake_ms) = self.next_headless_wakeup_ms() {
            let target_insns = wake_ms.saturating_mul(HEADLESS_INSNS_PER_MS);
            if target_insns > self.insns {
                if self.max_insns != u64::MAX && target_insns > self.max_insns {
                    self.insns = self.max_insns;
                    self.stopped = Some(StopReason::MaxInstructions);
                    return Ok(Some(StopReason::MaxInstructions));
                }
                self.insns = target_insns;
                self.guest_time_ms = self.insns / HEADLESS_INSNS_PER_MS;
                return Ok(None);
            }
        }
        hle::flush_gdi_present_if_pending(self)?;
        self.stopped = Some(StopReason::Waiting);
        Ok(Some(StopReason::Waiting))
    }

    fn next_headless_wakeup_ms(&self) -> Option<u64> {
        [
            self.hle.next_message_timer_ms(),
            self.journal.next_wakeup_ms(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn refresh_guest_time(&mut self) {
        if !self.backend.uses_wall_clock() {
            self.guest_time_ms = self.insns / HEADLESS_INSNS_PER_MS;
            return;
        }
        let now = host_time_ms();
        let start = *self.ui_clock_start_ms.get_or_insert(now);
        self.guest_time_ms = now.saturating_sub(start);
    }

    pub fn set_frontend_timing(&mut self, fps: u32, microseconds_per_frame: u64) {
        self.hle.set_frontend_timing(fps, microseconds_per_frame);
    }

    pub(crate) fn delay_target(&mut self, delay_ms: u32) -> hle::HleDelayTarget {
        self.refresh_guest_time();
        let scheduler_frame = if self.has_live_frontend() {
            self.scheduler_frame
        } else {
            0
        };
        self.hle.delay_target(delay_ms, scheduler_frame)
    }

    pub fn poll_frontend_events_no_timers(&mut self) -> Result<()> {
        self.refresh_guest_time();
        if self.backend.wants_event_poll() {
            if let Some(reason) = self.pump_backend_events()? {
                self.stopped = Some(reason);
            }
        }
        self.maybe_print_state(true);
        Ok(())
    }

    pub fn has_live_frontend(&self) -> bool {
        self.backend.uses_wall_clock()
    }

    pub(crate) fn current_scheduler_frame(&self) -> u64 {
        self.scheduler_frame
    }

    fn begin_scheduler_frame(&mut self) {
        self.scheduler_frame = self.scheduler_frame.wrapping_add(1);
        if self.scheduler_frame == 0 {
            self.scheduler_frame = 1;
        }
    }

    pub(crate) fn note_present(&mut self) {
        self.present_generation = self.present_generation.wrapping_add(1);
    }

    pub(crate) fn park_current_hle_task(&mut self, wait: HleWaitState) {
        if self.hle_tasks.is_empty() {
            self.reset_hle_tasks();
        }
        let task = &mut self.hle_tasks[self.current_hle_task];
        task.cpu = self.cpu.clone();
        task.wait = wait;
    }

    fn reset_hle_tasks(&mut self) {
        self.hle_tasks.clear();
        self.hle_tasks.push(HleTask::new(1, self.cpu.clone()));
        self.current_hle_task = 0;
    }

    fn current_hle_task_waiting(&self) -> bool {
        self.hle_tasks
            .get(self.current_hle_task)
            .is_some_and(|task| !task.wait.is_ready())
    }

    fn select_ready_hle_task(&mut self) -> bool {
        if !self.current_hle_task_waiting() {
            return false;
        }
        let Some(index) = self.hle_tasks.iter().position(|task| task.wait.is_ready()) else {
            return false;
        };
        self.current_hle_task = index;
        self.cpu = self.hle_tasks[index].cpu.clone();
        true
    }

    fn wake_hle_tasks(&mut self) -> Result<()> {
        let now_ms = self.guest_time_ms;
        let mut restore_current = false;
        for index in 0..self.hle_tasks.len() {
            match self.hle_tasks[index].wait {
                HleWaitState::Ready => {}
                HleWaitState::Message { filter, .. } if self.hle.has_matching_message(filter) => {
                    self.hle_tasks[index].wait = HleWaitState::Ready;
                    restore_current |= index == self.current_hle_task;
                }
                HleWaitState::Message { .. } => {}
                HleWaitState::VfsRead { request_id, .. }
                | HleWaitState::VfsWrite { request_id, .. }
                    if self.hle.has_completed_vfs_request(request_id) =>
                {
                    self.complete_hle_vfs_io(index, request_id)?;
                    restore_current |= index == self.current_hle_task;
                }
                HleWaitState::VfsRead { .. } | HleWaitState::VfsWrite { .. } => {}
                HleWaitState::Timeout {
                    until_ms,
                    not_before_frame,
                    ret_value,
                    arg_bytes,
                } if now_ms >= until_ms && self.scheduler_frame >= not_before_frame => {
                    self.complete_hle_timeout(index, ret_value, arg_bytes)?;
                    restore_current |= index == self.current_hle_task;
                }
                HleWaitState::Timeout { .. } => {}
            }
        }
        if restore_current {
            self.cpu = self.hle_tasks[self.current_hle_task].cpu.clone();
        }
        Ok(())
    }

    fn complete_hle_timeout(
        &mut self,
        task_index: usize,
        ret_value: u32,
        arg_bytes: u32,
    ) -> Result<()> {
        let task = &mut self.hle_tasks[task_index];
        let esp = task.cpu.reg(Reg::Esp);
        let ret_addr = self.memory.read_u32(esp)?;
        #[cfg(debug_assertions)]
        let entry_addr = task.cpu.eip;
        #[cfg(debug_assertions)]
        if let Some(entry) = self.hle.entry_at(entry_addr) {
            task.cpu.debug_finish_call_return(
                entry.addr,
                ret_addr,
                esp,
                arg_bytes,
                &format!("HLE {}!{} timeout", entry.dll, entry.name),
            )?;
        }
        task.cpu.set_reg(Reg::Eax, ret_value);
        task.cpu
            .set_reg(Reg::Esp, esp.wrapping_add(4).wrapping_add(arg_bytes));
        task.cpu.eip = ret_addr;
        task.wait = HleWaitState::Ready;
        Ok(())
    }

    fn complete_hle_vfs_io(&mut self, task_index: usize, request_id: u32) -> Result<()> {
        let Some(completion) = self.hle.take_completed_vfs_request(request_id) else {
            return Ok(());
        };
        let wait = self.hle_tasks[task_index].wait;
        let (ret_value, arg_bytes) = match wait {
            HleWaitState::VfsRead {
                request_id: wait_id,
                buf,
                read_out,
                ret_transferred,
                ret_item_size,
                fail_value,
                arg_bytes,
            } if wait_id == request_id => {
                if completion.status == 0 {
                    let read = completion.data.len().min(completion.transferred as usize);
                    self.memory.write_bytes(buf, &completion.data[..read])?;
                    if read_out != 0 {
                        self.memory.write_u32(read_out, read as u32)?;
                    }
                    let ret_count = if ret_item_size == 0 {
                        read as u32
                    } else {
                        (read as u32) / ret_item_size
                    };
                    (if ret_transferred { ret_count } else { 1 }, arg_bytes)
                } else {
                    self.hle.last_error = completion.status;
                    if read_out != 0 {
                        self.memory.write_u32(read_out, 0)?;
                    }
                    (if ret_transferred { fail_value } else { 0 }, arg_bytes)
                }
            }
            HleWaitState::VfsWrite {
                request_id: wait_id,
                written_out,
                ret_transferred,
                ret_item_size,
                fail_value,
                arg_bytes,
            } if wait_id == request_id => {
                if completion.status == 0 {
                    if written_out != 0 {
                        self.memory.write_u32(written_out, completion.transferred)?;
                    }
                    let ret_count = if ret_item_size == 0 {
                        completion.transferred
                    } else {
                        completion.transferred / ret_item_size
                    };
                    (if ret_transferred { ret_count } else { 1 }, arg_bytes)
                } else {
                    self.hle.last_error = completion.status;
                    if written_out != 0 {
                        self.memory.write_u32(written_out, 0)?;
                    }
                    (if ret_transferred { fail_value } else { 0 }, arg_bytes)
                }
            }
            _ => return Ok(()),
        };

        let task = &mut self.hle_tasks[task_index];
        let esp = task.cpu.reg(Reg::Esp);
        let ret_addr = self.memory.read_u32(esp)?;
        #[cfg(debug_assertions)]
        let entry_addr = task.cpu.eip;
        #[cfg(debug_assertions)]
        if let Some(entry) = self.hle.entry_at(entry_addr) {
            task.cpu.debug_finish_call_return(
                entry.addr,
                ret_addr,
                esp,
                arg_bytes,
                &format!("HLE {}!{} async-vfs", entry.dll, entry.name),
            )?;
        }
        task.cpu.set_reg(Reg::Eax, ret_value);
        task.cpu
            .set_reg(Reg::Esp, esp.wrapping_add(4).wrapping_add(arg_bytes));
        task.cpu.eip = ret_addr;
        task.wait = HleWaitState::Ready;
        Ok(())
    }

    fn begin_frontend_frame(&mut self) -> Result<()> {
        self.refresh_guest_time();
        if self.backend.wants_event_poll() {
            if let Some(reason) = self.pump_backend_events()? {
                self.stopped = Some(reason);
            }
        }
        self.service_guest()
    }

    fn service_frontend(&mut self) -> Result<()> {
        self.begin_frontend_frame()
    }

    fn service_guest(&mut self) -> Result<()> {
        self.refresh_guest_time();
        self.hle
            .pump_timers(self.guest_time_ms, self.scheduler_frame);
        self.pump_journal();
        self.wake_hle_tasks()?;
        if !self.current_hle_task_waiting() {
            hle::dispatch_due_mm_timer_interrupt(self);
        }
        Ok(())
    }

    fn run_one_instruction(&mut self) -> Result<Option<StopReason>> {
        self.run_one_instruction_inner(Some(self.max_insns))
    }

    fn run_one_instruction_without_max(&mut self) -> Result<Option<StopReason>> {
        self.run_one_instruction_inner(None)
    }

    fn run_one_instruction_inner(&mut self, max_insns: Option<u64>) -> Result<Option<StopReason>> {
        if let Some(reason) = self.stopped {
            return Ok(Some(reason));
        }
        if max_insns.is_some_and(|limit| self.insns >= limit) {
            self.stopped = Some(StopReason::MaxInstructions);
            return Ok(Some(StopReason::MaxInstructions));
        }
        if self.breakpoints.contains(&self.cpu.eip) {
            let reason = StopReason::Breakpoint(self.cpu.eip);
            self.stopped = Some(reason);
            return Ok(Some(reason));
        }

        if self.hle.contains_addr(self.cpu.eip) {
            let eip = self.cpu.eip;
            self.record_eip_history("hle");
            let reason = match hle::Hle::dispatch(self) {
                Ok(reason) => reason,
                Err(err) => {
                    return Err(Error::Cpu(format!(
                        "{err} while dispatching HLE at eip={eip:08x} insns={}{}",
                        self.insns,
                        self.eip_history_suffix()
                    )));
                }
            };
            if let Some(reason) = reason {
                self.stopped = Some(reason);
                return Ok(Some(reason));
            }
            self.insns += 1;
            return Ok(None);
        }

        if self.should_trace() {
            eprintln!(
                "eip={:08x} eax={:08x} ecx={:08x} edx={:08x} ebx={:08x} esp={:08x} ebp={:08x} esi={:08x} edi={:08x}",
                self.cpu.eip,
                self.cpu.reg(cpu::Reg::Eax),
                self.cpu.reg(cpu::Reg::Ecx),
                self.cpu.reg(cpu::Reg::Edx),
                self.cpu.reg(cpu::Reg::Ebx),
                self.cpu.reg(cpu::Reg::Esp),
                self.cpu.reg(cpu::Reg::Ebp),
                self.cpu.reg(cpu::Reg::Esi),
                self.cpu.reg(cpu::Reg::Edi),
            );
        }

        let eip = self.cpu.eip;
        self.record_eip_history("cpu");
        let watch_writes = self.memory.write_watch_enabled();
        if watch_writes {
            self.memory.set_write_context(Some(WriteContext {
                eip,
                insns: self.insns,
                label: "CPU".to_string(),
                regs: write_registers(&self.cpu),
            }));
        }
        let step_result = self.cpu.step(&mut self.memory);
        if watch_writes {
            self.memory.set_write_context(None);
        }
        let step_outcome = match step_result {
            Ok(outcome) => outcome,
            Err(err) => {
                return Err(Error::Cpu(format!(
                    "{err} while executing eip={eip:08x} insns={} eax={:08x} ecx={:08x} edx={:08x} ebx={:08x} esp={:08x} ebp={:08x} esi={:08x} edi={:08x}{}",
                    self.insns,
                    self.cpu.reg(cpu::Reg::Eax),
                    self.cpu.reg(cpu::Reg::Ecx),
                    self.cpu.reg(cpu::Reg::Edx),
                    self.cpu.reg(cpu::Reg::Ebx),
                    self.cpu.reg(cpu::Reg::Esp),
                    self.cpu.reg(cpu::Reg::Ebp),
                    self.cpu.reg(cpu::Reg::Esi),
                    self.cpu.reg(cpu::Reg::Edi),
                    self.eip_history_suffix(),
                )));
            }
        };
        match step_outcome {
            StepOutcome::Continue => {
                self.insns += 1;
            }
            StepOutcome::JumpedToHle => {
                self.insns += 1;
            }
            StepOutcome::Halted => {
                self.stopped = Some(StopReason::CpuHalted);
                return Ok(Some(StopReason::CpuHalted));
            }
        }
        Ok(None)
    }

    #[cfg(debug_assertions)]
    fn record_eip_history(&mut self, kind: &'static str) {
        let task_id = self
            .hle_tasks
            .get(self.current_hle_task)
            .map(|task| task.id)
            .unwrap_or(0);
        self.eip_history.push(EipHistoryEntry {
            insns: self.insns,
            task_id,
            kind,
            eip: self.cpu.eip,
            eax: self.cpu.reg(cpu::Reg::Eax),
            ecx: self.cpu.reg(cpu::Reg::Ecx),
            edx: self.cpu.reg(cpu::Reg::Edx),
            ebx: self.cpu.reg(cpu::Reg::Ebx),
            esp: self.cpu.reg(cpu::Reg::Esp),
            ebp: self.cpu.reg(cpu::Reg::Ebp),
            esi: self.cpu.reg(cpu::Reg::Esi),
            edi: self.cpu.reg(cpu::Reg::Edi),
        });
    }

    #[cfg(not(debug_assertions))]
    fn record_eip_history(&mut self, _: &'static str) {}

    #[cfg(debug_assertions)]
    fn eip_history_suffix(&self) -> String {
        self.eip_history.format_recent(64)
    }

    #[cfg(not(debug_assertions))]
    fn eip_history_suffix(&self) -> String {
        String::new()
    }

    pub fn run_config(cfg: &RunConfig) -> Result<(StopReason, Self)> {
        let mut emu = Self::new();
        emu.configure(cfg)?;
        if cfg.zip.is_some() {
            #[cfg(not(target_arch = "wasm32"))]
            {
                emu.load_zip_config(cfg)?;
            }
            #[cfg(target_arch = "wasm32")]
            {
                return Err(Error::Cli(
                    "--zip is only supported by the native CLI; browser ZIPs are mounted by JavaScript"
                        .to_string(),
                ));
            }
        } else {
            let exe = emu.resolve_load_exe(cfg)?;
            emu.load_exe(&exe)?;
        }
        let stop = if cfg.debug_on_crash {
            let old_panic_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| emu.run()));
            std::panic::set_hook(old_panic_hook);
            match run_result {
                Ok(Ok(stop)) => stop,
                Ok(Err(err)) => {
                    debugger::interactive(&mut emu, &err);
                    return Err(err);
                }
                Err(panic) => {
                    let err = debugger::panic_to_error(panic);
                    debugger::interactive(&mut emu, &err);
                    return Err(err);
                }
            }
        } else {
            emu.run()?
        };
        if let Some(path) = &cfg.screenshot {
            hle::flush_gdi_present_if_pending(&mut emu)?;
            emu.backend.write_png(path)?;
        }
        Ok((stop, emu))
    }

    pub fn should_trace(&self) -> bool {
        self.trace && self.insns >= self.trace_after
    }

    pub(crate) fn record_hle_call(&mut self, dll: &str, name: &str) {
        if self.state_interval.is_none() {
            return;
        }
        self.last_hle_call_at = Some(Instant::now());
        self.last_hle_call_symbol.clear();
        self.last_hle_call_symbol.push_str(dll);
        self.last_hle_call_symbol.push('!');
        self.last_hle_call_symbol.push_str(name);
    }

    fn maybe_print_state(&mut self, force_time_check: bool) {
        let Some(interval) = self.state_interval else {
            return;
        };
        if !force_time_check && self.insns < self.state_next_check_insns {
            return;
        }
        self.state_next_check_insns = self.insns.saturating_add(STATE_CHECK_INSNS);

        let now = Instant::now();
        let start = *self.state_start.get_or_insert(now);
        let mut next_print = self.state_next_print.unwrap_or(now + interval);
        if now < next_print {
            self.state_next_print = Some(next_print);
            return;
        }

        let (last_at, last_insns) = *self.state_last_print.get_or_insert((start, 0));
        let elapsed = now.duration_since(start);
        let delta_time = now.duration_since(last_at).as_secs_f64();
        let delta_insns = self.insns.saturating_sub(last_insns);
        let rate = if delta_time > 0.0 {
            delta_insns as f64 / delta_time
        } else {
            0.0
        };
        eprintln!("{}", self.state_line(now, elapsed, delta_insns, rate));

        self.state_last_print = Some((now, self.insns));
        while next_print <= now {
            next_print += interval;
        }
        self.state_next_print = Some(next_print);
    }

    fn state_line(&self, now: Instant, elapsed: Duration, delta_insns: u64, rate: f64) -> String {
        let (last_hle, last_hle_us_ago) = self.last_hle_summary(now);
        format!(
            "wemu state: t={:.1}s insns={} +{} rate={:.0}/s guest_ms={} last_hle={} last_hle_us_ago={} eip={:08x} eflags={:08x} eax={:08x} ecx={:08x} edx={:08x} ebx={:08x} esp={:08x} ebp={:08x} esi={:08x} edi={:08x} hle=[{}] stack=[{}]",
            elapsed.as_secs_f64(),
            self.insns,
            delta_insns,
            rate,
            self.guest_time_ms,
            last_hle,
            last_hle_us_ago,
            self.cpu.eip,
            self.cpu.eflags,
            self.cpu.reg(cpu::Reg::Eax),
            self.cpu.reg(cpu::Reg::Ecx),
            self.cpu.reg(cpu::Reg::Edx),
            self.cpu.reg(cpu::Reg::Ebx),
            self.cpu.reg(cpu::Reg::Esp),
            self.cpu.reg(cpu::Reg::Ebp),
            self.cpu.reg(cpu::Reg::Esi),
            self.cpu.reg(cpu::Reg::Edi),
            self.hle.state_summary(self.guest_time_ms),
            self.stack_preview(),
        )
    }

    pub(crate) fn emit_abnormal_report(&self, kind: &str, extra_fields: &str) {
        let task = self
            .hle_tasks
            .get(self.current_hle_task)
            .map(|task| task.id)
            .unwrap_or(0);
        let last_hle = if self.last_hle_call_symbol.is_empty() {
            "-"
        } else {
            self.last_hle_call_symbol.as_str()
        };
        let mut out = String::with_capacity(640);
        out.push_str("{\"v\":1,\"kind\":");
        push_json_string(&mut out, kind);
        let _ = write!(
            out,
            ",\"ins\":{},\"t\":{},\"task\":{},\"eip\":{},\"regs\":[{},{},{},{},{},{},{},{},{}],\"stk\":[",
            self.insns,
            self.guest_time_ms,
            task,
            self.cpu.eip,
            self.cpu.reg(cpu::Reg::Eax),
            self.cpu.reg(cpu::Reg::Ecx),
            self.cpu.reg(cpu::Reg::Edx),
            self.cpu.reg(cpu::Reg::Ebx),
            self.cpu.reg(cpu::Reg::Esp),
            self.cpu.reg(cpu::Reg::Ebp),
            self.cpu.reg(cpu::Reg::Esi),
            self.cpu.reg(cpu::Reg::Edi),
            self.cpu.eflags,
        );
        let esp = self.cpu.reg(cpu::Reg::Esp);
        for index in 0..8 {
            if index != 0 {
                out.push(',');
            }
            match self.memory.checked_read_u32(esp.wrapping_add(index * 4)) {
                Ok(value) => {
                    let _ = write!(out, "{value}");
                }
                Err(_) => out.push_str("null"),
            }
        }
        out.push_str("],\"code\":\"");
        for index in 0..32 {
            match self
                .memory
                .checked_read_u8(self.cpu.eip.wrapping_add(index))
            {
                Ok(byte) => push_hex_byte(&mut out, byte),
                Err(_) => break,
            }
        }
        out.push_str("\",\"hle\":");
        push_json_string(&mut out, last_hle);
        out.push_str(extra_fields);
        out.push('}');
        eprintln!("{out}");
    }

    pub fn last_hle_call_symbol(&self) -> &str {
        self.last_hle_call_symbol.as_str()
    }

    fn last_hle_summary(&self, now: Instant) -> (&str, String) {
        let Some(last_at) = self.last_hle_call_at else {
            return ("-", "-".to_string());
        };
        (
            self.last_hle_call_symbol.as_str(),
            now.duration_since(last_at).as_micros().to_string(),
        )
    }

    fn stack_preview(&self) -> String {
        let esp = self.cpu.reg(cpu::Reg::Esp);
        let mut words = Vec::new();
        for index in 0..4 {
            let addr = esp.wrapping_add(index * 4);
            match self.memory.read_u32(addr) {
                Ok(value) => words.push(format!("{value:08x}")),
                Err(_) => words.push("????????".to_string()),
            }
        }
        words.join(" ")
    }

    fn pump_journal(&mut self) {
        if !self.hle.window_ready() {
            return;
        }
        loop {
            match self.journal.next_event(self.guest_time_ms) {
                Some(JournalEvent::Move { x, y }) => {
                    if self.trace {
                        eprintln!("journal move x={x} y={y} t={}ms", self.guest_time_ms);
                    }
                    self.hle.post_mouse_move(x, y);
                }
                Some(JournalEvent::ButtonDown { x, y }) => {
                    if self.trace {
                        eprintln!("journal down x={x} y={y} t={}ms", self.guest_time_ms);
                    }
                    self.post_hle_mouse_button_down(x, y);
                }
                Some(JournalEvent::ButtonUp { x, y }) => {
                    if self.trace {
                        eprintln!("journal up x={x} y={y} t={}ms", self.guest_time_ms);
                    }
                    self.post_hle_mouse_button_up(x, y);
                }
                Some(JournalEvent::Click { x, y }) => {
                    if self.trace {
                        eprintln!("journal click x={x} y={y} t={}ms", self.guest_time_ms);
                    }
                    self.post_hle_mouse_button_down(x, y);
                    self.post_hle_mouse_button_up(x, y);
                }
                Some(JournalEvent::KeyDown { vk }) => {
                    if self.trace {
                        eprintln!("journal keydown vk={vk:02x} t={}ms", self.guest_time_ms);
                    }
                    self.hle.post_key_down(vk);
                }
                Some(JournalEvent::KeyUp { vk }) => {
                    if self.trace {
                        eprintln!("journal keyup vk={vk:02x} t={}ms", self.guest_time_ms);
                    }
                    self.hle.post_key_up(vk);
                }
                Some(JournalEvent::Text { text }) => {
                    if self.trace {
                        eprintln!("journal text len={} t={}ms", text.len(), self.guest_time_ms);
                    }
                    self.hle.post_text(&text);
                }
                None => break,
            }
        }
    }

    fn pump_backend_events(&mut self) -> Result<Option<StopReason>> {
        let events = self.backend.poll_events()?;
        self.apply_frontend_events(&events)
    }

    pub fn apply_frontend_events(&mut self, events: &[BackendEvent]) -> Result<Option<StopReason>> {
        let window_ready = self.hle.window_ready();
        if window_ready {
            if let Some(recorder) = self.recorder.as_mut() {
                recorder.start(self.guest_time_ms);
            }
        }
        for event in events {
            match event {
                BackendEvent::Quit => return Ok(Some(StopReason::FrontendQuit)),
                BackendEvent::MouseMove { x, y } if window_ready => {
                    self.record_input(JournalEvent::Move { x: *x, y: *y })?;
                    self.hle.post_mouse_move(*x, *y);
                }
                BackendEvent::MouseButtonDown { x, y } if window_ready => {
                    self.record_input(JournalEvent::ButtonDown { x: *x, y: *y })?;
                    self.post_hle_mouse_button_down(*x, *y);
                }
                BackendEvent::MouseButtonUp { x, y } if window_ready => {
                    self.record_input(JournalEvent::ButtonUp { x: *x, y: *y })?;
                    self.post_hle_mouse_button_up(*x, *y);
                }
                BackendEvent::MouseRightButtonDown { x, y } if window_ready => {
                    self.hle.post_mouse_right_button_down(*x, *y);
                }
                BackendEvent::MouseRightButtonUp { x, y } if window_ready => {
                    self.hle.post_mouse_right_button_up(*x, *y);
                }
                BackendEvent::KeyDown { vk } if window_ready => {
                    self.record_input(JournalEvent::KeyDown { vk: *vk })?;
                    self.hle.post_key_down(*vk);
                }
                BackendEvent::KeyUp { vk } if window_ready => {
                    self.record_input(JournalEvent::KeyUp { vk: *vk })?;
                    self.hle.post_key_up(*vk);
                }
                BackendEvent::TextInput { text } if window_ready => {
                    self.record_input(JournalEvent::Text { text: text.clone() })?;
                    self.hle.post_text_input(text);
                }
                BackendEvent::Text { text } if window_ready => {
                    self.record_input(JournalEvent::Text { text: text.clone() })?;
                    self.hle.post_text(text);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn post_hle_mouse_button_down(&mut self, x: u32, y: u32) {
        if !self.hle.is_menu_bar_at(x, y) {
            self.hle.post_mouse_button_down(x, y);
        }
    }

    fn post_hle_mouse_button_up(&mut self, x: u32, y: u32) {
        let had_popup = self.hle.has_active_popup_menu();
        if had_popup && self.hle.is_menu_bar_at(x, y) {
            self.hle
                .activate_menu_bar_at(x, y, self.backend.width(), self.backend.height());
            hle::render_hle_windows(self);
        } else if had_popup {
            self.hle.post_mouse_button_up(x, y);
            hle::render_hle_windows(self);
        } else if self
            .hle
            .activate_menu_bar_at(x, y, self.backend.width(), self.backend.height())
        {
            hle::render_hle_windows(self);
        } else {
            self.hle.post_mouse_button_up(x, y);
        }
    }

    fn record_input(&mut self, event: JournalEvent) -> Result<()> {
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.record_event(self.guest_time_ms, event)?;
        }
        Ok(())
    }
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn write_registers(cpu: &Cpu) -> WriteRegisters {
    WriteRegisters {
        eax: cpu.reg(cpu::Reg::Eax),
        ecx: cpu.reg(cpu::Reg::Ecx),
        edx: cpu.reg(cpu::Reg::Edx),
        ebx: cpu.reg(cpu::Reg::Ebx),
        esp: cpu.reg(cpu::Reg::Esp),
        ebp: cpu.reg(cpu::Reg::Ebp),
        esi: cpu.reg(cpu::Reg::Esi),
        edi: cpu.reg(cpu::Reg::Edi),
    }
}

fn create_backend(frontend: FrontendKind, sdl_ws: Option<&str>) -> Result<Box<dyn Backend>> {
    match frontend {
        FrontendKind::Headless => Ok(Box::new(HeadlessBackend::new(
            DEFAULT_SCREEN_WIDTH,
            DEFAULT_SCREEN_HEIGHT,
        ))),
        FrontendKind::Sdl2 => create_sdl2_backend(sdl_ws),
    }
}

#[cfg(feature = "sdl2")]
fn create_sdl2_backend(ws_addr: Option<&str>) -> Result<Box<dyn Backend>> {
    Ok(Box::new(SdlBackend::new(
        DEFAULT_SCREEN_WIDTH,
        DEFAULT_SCREEN_HEIGHT,
        ws_addr,
    )?))
}

#[cfg(not(feature = "sdl2"))]
fn create_sdl2_backend(_: Option<&str>) -> Result<Box<dyn Backend>> {
    Err(Error::Cli(
        "--frontend sdl2 requires building wemu with --features sdl2".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_STACK: u32 = 0x0001_0000;
    const TEST_MSG: u32 = TEST_STACK + 0x100;
    const TEST_RET: u32 = 0x0040_0000;

    fn setup_get_message(emu: &mut Emulator) {
        emu.memory
            .map(TEST_STACK, 0x2000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK);
        emu.memory.write_u32(TEST_STACK, TEST_RET).unwrap();
        emu.memory.write_u32(TEST_STACK + 4, TEST_MSG).unwrap();
        emu.memory.write_u32(TEST_STACK + 8, 0).unwrap();
        emu.memory.write_u32(TEST_STACK + 12, 0).unwrap();
        emu.memory.write_u32(TEST_STACK + 16, 0).unwrap();
        emu.cpu.eip = emu.hle.resolve_import("user32.dll", "GetMessageA");
    }

    #[test]
    fn live_sleep_timeout_completes_hle_return() {
        let mut emu = Emulator::new();
        emu.backend = Box::new(HeadlessBackend::new_live(640, 480));
        emu.memory
            .map(TEST_STACK, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK);
        emu.memory.write_u32(TEST_STACK, 0x0040_0000).unwrap();
        emu.memory.write_u32(TEST_STACK + 4, 10).unwrap();

        let sleep = emu.hle.resolve_import("kernel32.dll", "Sleep");
        emu.cpu.eip = sleep;
        emu.scheduler_frame = 3;
        assert_eq!(Hle::dispatch(&mut emu).unwrap(), None);
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Timeout {
                ret_value: 0,
                arg_bytes: 4,
                ..
            }
        ));
        emu.hle_tasks[0].wait = HleWaitState::Timeout {
            until_ms: 0,
            not_before_frame: 4,
            ret_value: 0,
            arg_bytes: 4,
        };

        emu.service_frontend().unwrap();
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Timeout { .. }
        ));

        emu.scheduler_frame = 4;
        emu.service_frontend().unwrap();

        assert!(emu.hle_tasks[0].wait.is_ready());
        assert_eq!(emu.cpu.eip, 0x0040_0000);
        assert_eq!(emu.cpu.reg(Reg::Esp), TEST_STACK + 8);
        assert_eq!(emu.cpu.reg(Reg::Eax), 0);
    }

    #[test]
    fn live_sleep_zero_yields_until_next_frame() {
        let mut emu = Emulator::new();
        emu.backend = Box::new(HeadlessBackend::new_live(640, 480));
        emu.memory
            .map(TEST_STACK, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK);
        emu.memory.write_u32(TEST_STACK, 0x0040_0000).unwrap();
        emu.memory.write_u32(TEST_STACK + 4, 0).unwrap();

        let sleep = emu.hle.resolve_import("kernel32.dll", "Sleep");
        emu.cpu.eip = sleep;
        emu.scheduler_frame = 7;
        assert_eq!(Hle::dispatch(&mut emu).unwrap(), None);
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Timeout {
                not_before_frame: 8,
                ret_value: 0,
                arg_bytes: 4,
                ..
            }
        ));

        emu.service_guest().unwrap();
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Timeout { .. }
        ));

        emu.scheduler_frame = 8;
        emu.service_guest().unwrap();
        assert!(emu.hle_tasks[0].wait.is_ready());
        assert_eq!(emu.cpu.eip, 0x0040_0000);
        assert_eq!(emu.cpu.reg(Reg::Esp), TEST_STACK + 8);
        assert_eq!(emu.cpu.reg(Reg::Eax), 0);
    }

    #[test]
    fn live_sleep_ex_is_not_shortened_by_due_mm_timer() {
        let mut emu = Emulator::new();
        emu.backend = Box::new(HeadlessBackend::new_live(640, 480));
        emu.set_frontend_timing(100, 10_000);
        emu.memory
            .map(TEST_STACK, 0x2000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK + 0x800);
        emu.memory
            .write_u32(TEST_STACK + 0x800, 0x0040_0000)
            .unwrap();
        emu.memory.write_u32(TEST_STACK + 0x804, 20).unwrap();
        emu.memory.write_u32(TEST_STACK + 0x808, 0).unwrap();
        emu.scheduler_frame = 1;
        emu.ui_clock_start_ms = Some(host_time_ms());
        let mm_target = emu.hle.delay_target(1, emu.scheduler_frame);
        emu.hle
            .set_mm_timer(0x0040_1f2d, 0xfeed, 1, 0, emu.scheduler_frame, mm_target);

        let sleep_ex = emu.hle.resolve_import("kernel32.dll", "SleepEx");
        emu.cpu.eip = sleep_ex;
        assert_eq!(Hle::dispatch(&mut emu).unwrap(), None);
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Timeout {
                not_before_frame: 3,
                ..
            }
        ));
        assert_eq!(emu.cpu.eip, sleep_ex);
        emu.hle_tasks[0].wait = HleWaitState::Timeout {
            until_ms: 20,
            not_before_frame: 3,
            ret_value: 0,
            arg_bytes: 8,
        };

        emu.guest_time_ms = 10;
        emu.scheduler_frame = 2;
        emu.wake_hle_tasks().unwrap();
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Timeout { .. }
        ));
        assert_eq!(emu.cpu.eip, sleep_ex);

        emu.guest_time_ms = 20;
        emu.scheduler_frame = 3;
        emu.wake_hle_tasks().unwrap();
        assert!(emu.hle_tasks[0].wait.is_ready());
        assert!(hle::dispatch_due_mm_timer_interrupt(&mut emu));
        assert_eq!(emu.cpu.eip, 0x0040_1f2d);
    }

    #[test]
    fn live_sleep_ex_is_not_shortened_by_due_user_timer() {
        let mut emu = Emulator::new();
        emu.backend = Box::new(HeadlessBackend::new_live(640, 480));
        emu.set_frontend_timing(100, 10_000);
        emu.memory
            .map(TEST_STACK, 0x2000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK + 0x800);
        emu.memory
            .write_u32(TEST_STACK + 0x800, 0x0040_0000)
            .unwrap();
        emu.memory.write_u32(TEST_STACK + 0x804, 20).unwrap();
        emu.memory.write_u32(TEST_STACK + 0x808, 0).unwrap();
        emu.scheduler_frame = 1;
        emu.ui_clock_start_ms = Some(host_time_ms());
        let timer_target = emu.hle.delay_target(1, emu.scheduler_frame);
        emu.hle
            .set_timer(0, 1, 0, 0, emu.scheduler_frame, timer_target);

        let sleep_ex = emu.hle.resolve_import("kernel32.dll", "SleepEx");
        emu.cpu.eip = sleep_ex;
        assert_eq!(Hle::dispatch(&mut emu).unwrap(), None);

        emu.guest_time_ms = 10;
        emu.scheduler_frame = 2;
        emu.hle
            .pump_timers(emu.guest_time_ms, emu.current_scheduler_frame());
        emu.wake_hle_tasks().unwrap();
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Timeout { .. }
        ));
        assert!(emu
            .hle
            .has_matching_message(MessageFilter::new(0, 0x0113, 0x0113)));
        assert_eq!(emu.cpu.eip, sleep_ex);

        emu.guest_time_ms = 20;
        emu.scheduler_frame = 3;
        emu.wake_hle_tasks().unwrap();
        assert!(emu.hle_tasks[0].wait.is_ready());
        assert_eq!(emu.cpu.eip, 0x0040_0000);
    }

    #[test]
    fn ready_hle_task_prevents_global_waiting() {
        let mut emu = Emulator::new();
        emu.guest_time_ms = 100;
        emu.hle_tasks[0].wait = HleWaitState::Timeout {
            until_ms: 180,
            not_before_frame: 0,
            ret_value: 0,
            arg_bytes: 0,
        };

        let mut cpu = emu.cpu.clone();
        cpu.eip = 0x0040_1234;
        emu.hle_tasks.push(HleTask::new(2, cpu));

        assert!(emu.select_ready_hle_task());
        assert_eq!(emu.current_hle_task, 1);
        assert_eq!(emu.cpu.eip, 0x0040_1234);
    }

    #[test]
    fn message_wait_wakes_only_for_matching_filter() {
        let mut emu = Emulator::new();
        emu.hle_tasks[0].wait = HleWaitState::Message {
            out: 0,
            filter: MessageFilter::new(0, 0x0113, 0x0113),
        };

        emu.hle.post_key_down(b'A' as u32);
        emu.wake_hle_tasks().unwrap();
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Message { .. }
        ));

        let target = emu.hle.delay_target(1, 0);
        emu.hle.set_timer(0, 1, 0, 0, 0, target);
        emu.hle.pump_timers(1, 0);
        emu.wake_hle_tasks().unwrap();
        assert!(emu.hle_tasks[0].wait.is_ready());
    }

    #[test]
    fn native_headless_empty_get_message_stops_waiting() {
        let mut emu = Emulator::new();
        setup_get_message(&mut emu);
        emu.memory.write_u32(TEST_MSG + 4, 0xdead_beef).unwrap();

        let stop = emu.run().unwrap();

        assert_eq!(stop, StopReason::Waiting);
        assert!(emu.insns < HEADLESS_INSNS_PER_MS);
        assert_eq!(emu.memory.read_u32(TEST_MSG + 4).unwrap(), 0xdead_beef);
    }

    #[test]
    fn native_headless_get_message_timer_wakes_waiting_task() {
        let mut emu = Emulator::new();
        setup_get_message(&mut emu);
        emu.breakpoints.push(TEST_RET);
        let target = emu.hle.delay_target(10, 0);
        emu.hle.set_timer(0, 1, 0, 0, 0, target);

        let stop = emu.run().unwrap();

        assert_eq!(stop, StopReason::Breakpoint(TEST_RET));
        assert!(emu.insns >= 10 * HEADLESS_INSNS_PER_MS);
        assert_eq!(emu.memory.read_u32(TEST_MSG).unwrap(), 0);
        assert_eq!(emu.memory.read_u32(TEST_MSG + 4).unwrap(), 0x0113);
        assert_eq!(emu.memory.read_u32(TEST_MSG + 8).unwrap(), 1);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn eip_history_wraps_and_formats_recent_entries() {
        let mut history = EipHistory {
            entries: Vec::with_capacity(2),
            next: 0,
            len: 0,
        };
        for eip in [0x401000, 0x402000, 0x403000] {
            history.push(EipHistoryEntry {
                insns: eip as u64,
                task_id: 1,
                kind: "cpu",
                eip,
                eax: 1,
                ecx: 2,
                edx: 3,
                ebx: 4,
                esp: 5,
                ebp: 6,
                esi: 7,
                edi: 8,
            });
        }

        let formatted = history.format_recent(8);
        assert!(!formatted.contains("eip=00401000"));
        assert!(formatted.contains("eip=00402000"));
        assert!(formatted.contains("eip=00403000"));
    }
}
