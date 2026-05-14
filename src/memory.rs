use std::sync::{Mutex, MutexGuard};

use crate::{Error, Result};

pub const PAGE_SIZE: u32 = 4096;
pub const GUEST_RAM_BASE: u32 = 0x0001_0000;
pub const GUEST_RAM_SIZE: u32 = 0x1000_0000;
pub const GUEST_RAM_END: u32 = GUEST_RAM_BASE + GUEST_RAM_SIZE;
const PAGE_MASK: u32 = !(PAGE_SIZE - 1);
const PAGE_SHIFT: usize = 12;
const PAGE_COUNT: usize = 1usize << (32 - PAGE_SHIFT);
const PAGE_BITMAP_WORDS: usize = PAGE_COUNT / 64;
#[cfg(debug_assertions)]
const PAGE_OFFSET_MASK: usize = PAGE_SIZE as usize - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagePerm(u8);

impl PagePerm {
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const EXEC: Self = Self(4);
}

impl std::ops::BitOr for PagePerm {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

static IDENTITY_MEMORY_LOCK: Mutex<()> = Mutex::new(());

pub struct Memory {
    mapped: Vec<u64>,
    #[cfg(debug_assertions)]
    write_watch: Option<WriteWatch>,
    #[cfg(debug_assertions)]
    write_context: Option<WriteContext>,
    _identity_lock: MutexGuard<'static, ()>,
}

#[cfg(debug_assertions)]
#[derive(Clone)]
struct WriteWatch {
    start: u32,
    end: u32,
    zero_only: bool,
    limit: u32,
    logged: u32,
}

#[derive(Clone)]
pub struct WriteContext {
    pub eip: u32,
    pub insns: u64,
    pub label: String,
    pub regs: WriteRegisters,
}

#[derive(Clone, Copy)]
pub struct WriteRegisters {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
}

impl Memory {
    pub fn new() -> Self {
        let identity_lock = IDENTITY_MEMORY_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self {
            mapped: vec![0; PAGE_BITMAP_WORDS],
            #[cfg(debug_assertions)]
            write_watch: None,
            #[cfg(debug_assertions)]
            write_context: None,
            _identity_lock: identity_lock,
        }
    }

    #[cfg(debug_assertions)]
    pub fn configure_write_watch_from_env(&mut self) -> Result<()> {
        let Some(spec) = std::env::var_os("WEMU_WRITE_WATCH") else {
            self.write_watch = None;
            return Ok(());
        };
        let spec = spec.to_string_lossy();
        let (start, end) = parse_watch_range(&spec)?;
        let zero_only = std::env::var_os("WEMU_WRITE_WATCH_ZERO_ONLY").is_some();
        let limit = std::env::var("WEMU_WRITE_WATCH_LIMIT")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(200);
        self.write_watch = Some(WriteWatch {
            start,
            end,
            zero_only,
            limit,
            logged: 0,
        });
        eprintln!(
            "WEMU_WRITE_WATCH active start={start:08x} end={end:08x} zero_only={zero_only} limit={limit}"
        );
        Ok(())
    }

    #[cfg(not(debug_assertions))]
    pub fn configure_write_watch_from_env(&mut self) -> Result<()> {
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn write_watch_enabled(&self) -> bool {
        self.write_watch.is_some()
    }

    #[cfg(not(debug_assertions))]
    pub fn write_watch_enabled(&self) -> bool {
        false
    }

    #[cfg(debug_assertions)]
    pub fn set_write_context(&mut self, context: Option<WriteContext>) {
        self.write_context = context;
    }

    #[cfg(not(debug_assertions))]
    pub fn set_write_context(&mut self, _context: Option<WriteContext>) {}

    pub fn map(&mut self, addr: u32, size: u32, _perm: PagePerm) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let start = align_down(addr);
        let end = align_up(addr.checked_add(size).ok_or_else(|| {
            Error::Memory(format!("map overflow addr={addr:08x} size={size:x}"))
        })?)?;
        for index in page_range(start, end) {
            if self.page_mapped(index) {
                return Err(Error::Memory(format!(
                    "page already mapped at {:08x}",
                    page_addr(index)
                )));
            }
        }
        let mut mapped = Vec::new();
        for index in page_range(start, end) {
            let addr = page_addr(index);
            if let Err(err) = map_identity_page(addr) {
                for old_index in mapped {
                    self.set_page_mapped(old_index, false);
                    let _ = unmap_identity_page(page_addr(old_index));
                }
                return Err(err);
            }
            self.set_page_mapped(index, true);
            mapped.push(index);
        }
        Ok(())
    }

    pub fn map_or_update(&mut self, addr: u32, size: u32, _perm: PagePerm) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let start = align_down(addr);
        let end = align_up(addr.checked_add(size).ok_or_else(|| {
            Error::Memory(format!(
                "map_or_update overflow addr={addr:08x} size={size:x}"
            ))
        })?)?;
        let mut mapped = Vec::new();
        for index in page_range(start, end) {
            if !self.page_mapped(index) {
                let addr = page_addr(index);
                if let Err(err) = map_identity_page(addr) {
                    for old_index in mapped {
                        self.set_page_mapped(old_index, false);
                        let _ = unmap_identity_page(page_addr(old_index));
                    }
                    return Err(err);
                }
                self.set_page_mapped(index, true);
                mapped.push(index);
            }
        }
        Ok(())
    }

    pub fn unmap(&mut self, addr: u32, size: u32) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let start = align_down(addr);
        let end = align_up(addr.checked_add(size).ok_or_else(|| {
            Error::Memory(format!("unmap overflow addr={addr:08x} size={size:x}"))
        })?)?;
        for index in page_range(start, end) {
            if self.page_mapped(index) {
                unmap_identity_page(page_addr(index))?;
                self.set_page_mapped(index, false);
            }
        }
        Ok(())
    }

    pub fn protect(&mut self, addr: u32, size: u32, _perm: PagePerm) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let start = align_down(addr);
        let end = align_up(addr.checked_add(size).ok_or_else(|| {
            Error::Memory(format!("protect overflow addr={addr:08x} size={size:x}"))
        })?)?;
        for index in page_range(start, end) {
            if !self.page_mapped(index) {
                return Err(Error::Memory(format!(
                    "protect unmapped page {:08x}",
                    page_addr(index)
                )));
            }
        }
        Ok(())
    }

    pub fn is_mapped(&self, addr: u32, _perm: PagePerm) -> bool {
        self.page_mapped(page_index(addr))
    }

    #[inline(always)]
    pub fn read_u8(&self, addr: u32) -> Result<u8> {
        #[cfg(debug_assertions)]
        self.ensure_page_mapped(addr, "read")?;
        Ok(unsafe { std::ptr::read(addr as usize as *const u8) })
    }

    // Diagnostics need real validity checks even in optimized builds. Keep the
    // hot scalar accessors above unchecked in release.
    pub fn checked_read_u8(&self, addr: u32) -> Result<u8> {
        self.ensure_page_mapped(addr, "read")?;
        Ok(unsafe { std::ptr::read(addr as usize as *const u8) })
    }

    #[inline(always)]
    pub fn write_u8(&mut self, addr: u32, value: u8) -> Result<()> {
        #[cfg(debug_assertions)]
        {
            self.trace_watched_write(addr, 1, value == 0, "write_u8", &[value]);
            self.ensure_page_mapped(addr, "write")?;
        }
        unsafe {
            std::ptr::write(addr as usize as *mut u8, value);
        }
        Ok(())
    }

    #[inline(always)]
    pub fn read_u16(&self, addr: u32) -> Result<u16> {
        // Scalar memory access intentionally checks only the starting page.
        // The emulator tracks valid/invalid pages, not permission bits or
        // cross-page sub-accesses; the host load then handles the raw bytes.
        #[cfg(debug_assertions)]
        self.ensure_page_mapped(addr, "read")?;
        let value = unsafe { std::ptr::read_unaligned(addr as usize as *const u16) };
        Ok(u16::from_le(value))
    }

    #[inline(always)]
    pub fn write_u16(&mut self, addr: u32, value: u16) -> Result<()> {
        #[cfg(debug_assertions)]
        {
            let bytes = value.to_le_bytes();
            self.trace_watched_write(addr, bytes.len(), value == 0, "write_u16", &bytes);
            self.ensure_page_mapped(addr, "write")?;
        }
        unsafe {
            std::ptr::write_unaligned(addr as usize as *mut u16, value.to_le());
        }
        Ok(())
    }

    #[inline(always)]
    pub fn read_u32(&self, addr: u32) -> Result<u32> {
        #[cfg(debug_assertions)]
        self.ensure_page_mapped(addr, "read")?;
        let value = unsafe { std::ptr::read_unaligned(addr as usize as *const u32) };
        Ok(u32::from_le(value))
    }

    // Byte-wise so a u32 spanning an unmapped page reports an error instead of
    // letting the host load fault while formatting diagnostics.
    pub fn checked_read_u32(&self, addr: u32) -> Result<u32> {
        let bytes = [
            self.checked_read_u8(addr)?,
            self.checked_read_u8(addr.wrapping_add(1))?,
            self.checked_read_u8(addr.wrapping_add(2))?,
            self.checked_read_u8(addr.wrapping_add(3))?,
        ];
        Ok(u32::from_le_bytes(bytes))
    }

    #[inline(always)]
    pub fn write_u32(&mut self, addr: u32, value: u32) -> Result<()> {
        #[cfg(debug_assertions)]
        {
            let bytes = value.to_le_bytes();
            self.trace_watched_write(addr, bytes.len(), value == 0, "write_u32", &bytes);
            self.ensure_page_mapped(addr, "write")?;
        }
        unsafe {
            std::ptr::write_unaligned(addr as usize as *mut u32, value.to_le());
        }
        Ok(())
    }

    #[inline(always)]
    pub fn read_bytes(&self, addr: u32, len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0; len];
        #[cfg(not(debug_assertions))]
        {
            unsafe {
                std::ptr::copy_nonoverlapping(addr as usize as *const u8, out.as_mut_ptr(), len);
            }
            return Ok(out);
        }
        #[cfg(debug_assertions)]
        {
            let mut copied = 0usize;
            while copied < len {
                let current = addr.wrapping_add(copied as u32);
                self.ensure_page_mapped(current, "read")?;
                let offset = page_offset(current);
                let n = (PAGE_SIZE as usize - offset).min(len - copied);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        current as usize as *const u8,
                        out[copied..copied + n].as_mut_ptr(),
                        n,
                    );
                }
                copied += n;
            }
            Ok(out)
        }
    }

    #[inline(always)]
    pub fn write_bytes(&mut self, addr: u32, bytes: &[u8]) -> Result<()> {
        #[cfg(not(debug_assertions))]
        {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    addr as usize as *mut u8,
                    bytes.len(),
                );
            }
            return Ok(());
        }
        #[cfg(debug_assertions)]
        {
            self.trace_watched_write(
                addr,
                bytes.len(),
                bytes.iter().all(|&byte| byte == 0),
                "write_bytes",
                bytes,
            );
            let mut copied = 0usize;
            while copied < bytes.len() {
                let current = addr.wrapping_add(copied as u32);
                self.ensure_page_mapped(current, "write")?;
                let offset = page_offset(current);
                let n = (PAGE_SIZE as usize - offset).min(bytes.len() - copied);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes[copied..copied + n].as_ptr(),
                        current as usize as *mut u8,
                        n,
                    );
                }
                copied += n;
            }
            Ok(())
        }
    }

    #[inline(always)]
    pub fn memset(&mut self, addr: u32, value: u8, len: u32) -> Result<()> {
        #[cfg(not(debug_assertions))]
        {
            unsafe {
                std::ptr::write_bytes(addr as usize as *mut u8, value, len as usize);
            }
            return Ok(());
        }
        #[cfg(debug_assertions)]
        {
            self.trace_watched_write(addr, len as usize, value == 0, "memset", &[value]);
            let mut filled = 0usize;
            let len = len as usize;
            while filled < len {
                let current = addr.wrapping_add(filled as u32);
                self.ensure_page_mapped(current, "write")?;
                let offset = page_offset(current);
                let n = (PAGE_SIZE as usize - offset).min(len - filled);
                unsafe {
                    std::ptr::write_bytes(current as usize as *mut u8, value, n);
                }
                filled += n;
            }
            Ok(())
        }
    }

    pub fn cstr_lossy(&self, addr: u32, max_len: usize) -> Result<String> {
        if addr == 0 {
            return Ok(String::new());
        }
        let mut bytes = Vec::new();
        for i in 0..max_len {
            let b = self.read_u8(addr.wrapping_add(i as u32))?;
            if b == 0 {
                break;
            }
            bytes.push(b);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn utf16z_lossy(&self, addr: u32, max_chars: usize) -> Result<String> {
        if addr == 0 {
            return Ok(String::new());
        }
        let mut chars = Vec::new();
        for i in 0..max_chars {
            let c = self.read_u16(addr.wrapping_add((i * 2) as u32))?;
            if c == 0 {
                break;
            }
            chars.push(c);
        }
        Ok(String::from_utf16_lossy(&chars))
    }

    pub fn write_cstr(&mut self, addr: u32, s: &str, max_len: usize) -> Result<u32> {
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(max_len.saturating_sub(1));
        self.write_bytes(addr, &bytes[..copy_len])?;
        self.write_u8(addr.wrapping_add(copy_len as u32), 0)?;
        Ok(copy_len as u32)
    }

    pub fn write_utf16z(&mut self, addr: u32, s: &str, max_chars: usize) -> Result<u32> {
        let mut count = 0;
        for (i, c) in s
            .encode_utf16()
            .take(max_chars.saturating_sub(1))
            .enumerate()
        {
            self.write_u16(addr.wrapping_add((i * 2) as u32), c)?;
            count += 1;
        }
        self.write_u16(addr.wrapping_add((count * 2) as u32), 0)?;
        Ok(count as u32)
    }

    #[inline(always)]
    fn ensure_page_mapped(&self, addr: u32, access: &str) -> Result<()> {
        let index = page_index(addr);
        if !self.page_mapped(index) {
            return Err(Error::Memory(format!("unmapped {access} at {addr:08x}")));
        }
        Ok(())
    }

    #[inline(always)]
    fn page_mapped(&self, index: usize) -> bool {
        (self.mapped[index / 64] & (1u64 << (index % 64))) != 0
    }

    #[inline(always)]
    fn set_page_mapped(&mut self, index: usize, mapped: bool) {
        let mask = 1u64 << (index % 64);
        let word = &mut self.mapped[index / 64];
        if mapped {
            *word |= mask;
        } else {
            *word &= !mask;
        }
    }

    #[cfg(debug_assertions)]
    fn trace_watched_write(
        &mut self,
        addr: u32,
        len: usize,
        zero_write: bool,
        kind: &str,
        bytes: &[u8],
    ) {
        let Some(watch) = self.write_watch.as_ref() else {
            return;
        };
        if len == 0 || watch.logged >= watch.limit || (watch.zero_only && !zero_write) {
            return;
        }
        let end = (addr as u64).saturating_add(len as u64);
        let watch_start = watch.start as u64;
        let watch_end = watch.end as u64;
        if !ranges_overlap_u64(addr as u64, end, watch_start, watch_end) {
            return;
        }
        let hit_start = (addr as u64).max(watch_start) as u32;
        let hit_len = end
            .min(watch_end)
            .saturating_sub((hit_start as u64).max(addr as u64));
        let preview_len = usize::try_from(hit_len.min(16)).expect("preview length fits usize");
        let old_preview = self.preview_existing(hit_start, preview_len);
        let new_preview = preview_write_bytes(addr, hit_start, preview_len, bytes);
        let count = {
            let watch = self
                .write_watch
                .as_mut()
                .expect("write watch disappeared during trace");
            watch.logged += 1;
            watch.logged
        };
        if let Some(context) = &self.write_context {
            let regs = context.regs;
            let frame_preview = self.preview_u32s(regs.ebp.wrapping_add(8), 6);
            let stack_preview = self.preview_u32s(regs.esp, 6);
            eprintln!(
                "WRITE_WATCH #{count:04} before {kind} addr={addr:08x} len={len:x} hit={hit_start:08x}+{hit_len:x} zero={zero_write} eip={:08x} insns={} {} eax={:08x} ecx={:08x} edx={:08x} ebx={:08x} esp={:08x} ebp={:08x} esi={:08x} edi={:08x} frame=[{}] stack=[{}] old=[{}] new=[{}]",
                context.eip,
                context.insns,
                context.label,
                regs.eax,
                regs.ecx,
                regs.edx,
                regs.ebx,
                regs.esp,
                regs.ebp,
                regs.esi,
                regs.edi,
                frame_preview,
                stack_preview,
                old_preview,
                new_preview
            );
        } else {
            eprintln!(
                "WRITE_WATCH #{count:04} before {kind} addr={addr:08x} len={len:x} hit={hit_start:08x}+{hit_len:x} zero={zero_write} old=[{}] new=[{}]",
                old_preview,
                new_preview
            );
        }
    }

    #[cfg(debug_assertions)]
    fn preview_existing(&self, addr: u32, len: usize) -> String {
        let mut out = Vec::with_capacity(len);
        for offset in 0..len {
            match self.read_u8(addr.wrapping_add(offset as u32)) {
                Ok(byte) => out.push(format!("{byte:02x}")),
                Err(_) => out.push("??".to_string()),
            }
        }
        out.join(" ")
    }

    #[cfg(debug_assertions)]
    fn preview_u32s(&self, addr: u32, count: usize) -> String {
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            match self.read_u32(addr.wrapping_add((index * 4) as u32)) {
                Ok(value) => out.push(format!("{value:08x}")),
                Err(_) => out.push("????????".to_string()),
            }
        }
        out.join(" ")
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Memory {
    fn drop(&mut self) {
        for word_index in 0..self.mapped.len() {
            let mut word = self.mapped[word_index];
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                let index = word_index * 64 + bit;
                let _ = unmap_identity_page(page_addr(index));
                word &= !(1u64 << bit);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn map_identity_page(addr: u32) -> Result<()> {
    linux_fixed_mapping::map_page(addr)
}

#[cfg(target_os = "linux")]
fn unmap_identity_page(addr: u32) -> Result<()> {
    linux_fixed_mapping::unmap_page(addr)
}

#[cfg(target_arch = "wasm32")]
fn map_identity_page(addr: u32) -> Result<()> {
    if addr < GUEST_RAM_BASE || addr >= GUEST_RAM_END {
        return Err(Error::Memory(format!(
            "wasm identity page outside guest RAM window: {addr:08x} not in {GUEST_RAM_BASE:08x}..{GUEST_RAM_END:08x}"
        )));
    }
    // wasm linear memory is already one contiguous byte array. The linker
    // keeps Rust data/stack/heap above GUEST_RAM_END, so mapping a guest page
    // only registers it in the valid-page bitmap.
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn unmap_identity_page(_addr: u32) -> Result<()> {
    Ok(())
}

#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
fn map_identity_page(addr: u32) -> Result<()> {
    Err(Error::Memory(format!(
        "identity guest memory mapping is not implemented for this host at {addr:08x}"
    )))
}

#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
fn unmap_identity_page(_addr: u32) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux_fixed_mapping {
    use std::ffi::c_void;
    use std::os::raw::{c_int, c_long};

    use crate::{Error, Result};

    use super::PAGE_SIZE;

    const PROT_READ: c_int = 0x1;
    const PROT_WRITE: c_int = 0x2;
    const MAP_PRIVATE: c_int = 0x02;
    const MAP_ANONYMOUS: c_int = 0x20;
    const MAP_FIXED_NOREPLACE: c_int = 0x100000;

    unsafe extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            offset: c_long,
        ) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
    }

    pub fn map_page(addr: u32) -> Result<()> {
        let requested = addr as usize as *mut c_void;
        let mapped = unsafe {
            mmap(
                requested,
                PAGE_SIZE as usize,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE,
                -1,
                0,
            )
        };
        if mapped == !0usize as *mut c_void {
            return Err(Error::Memory(format!(
                "identity mmap failed at {addr:08x}: {}",
                std::io::Error::last_os_error()
            )));
        }
        if mapped != requested {
            let _ = unsafe { munmap(mapped, PAGE_SIZE as usize) };
            return Err(Error::Memory(format!(
                "identity mmap returned {:p}, expected {addr:08x}",
                mapped
            )));
        }
        Ok(())
    }

    pub fn unmap_page(addr: u32) -> Result<()> {
        let rc = unsafe { munmap(addr as usize as *mut c_void, PAGE_SIZE as usize) };
        if rc != 0 {
            return Err(Error::Memory(format!(
                "identity munmap failed at {addr:08x}: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }
}

#[inline(always)]
fn page_index(addr: u32) -> usize {
    (addr as usize) >> PAGE_SHIFT
}

#[cfg(debug_assertions)]
#[inline(always)]
fn page_offset(addr: u32) -> usize {
    (addr as usize) & PAGE_OFFSET_MASK
}

#[inline(always)]
fn page_addr(index: usize) -> u32 {
    (index as u32) << PAGE_SHIFT
}

#[inline(always)]
fn page_range(start: u32, end: u32) -> std::ops::Range<usize> {
    page_index(start)..page_index(end)
}

#[cfg(debug_assertions)]
fn ranges_overlap_u64(lhs_start: u64, lhs_end: u64, rhs_start: u64, rhs_end: u64) -> bool {
    lhs_start < rhs_end && rhs_start < lhs_end
}

#[cfg(debug_assertions)]
fn preview_write_bytes(write_addr: u32, preview_addr: u32, len: usize, bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(len);
    if bytes.len() == 1 {
        out.resize(len, format!("{:02x}", bytes[0]));
    } else {
        let start = preview_addr.wrapping_sub(write_addr) as usize;
        for offset in 0..len {
            if let Some(byte) = bytes.get(start + offset) {
                out.push(format!("{byte:02x}"));
            } else {
                out.push("??".to_string());
            }
        }
    }
    out.join(" ")
}

#[cfg(debug_assertions)]
fn parse_watch_range(spec: &str) -> Result<(u32, u32)> {
    if let Some((start, size)) = spec.split_once(':') {
        let start = parse_num(start)?;
        let size = parse_num(size)?;
        let end = start
            .checked_add(size)
            .ok_or_else(|| Error::Cli(format!("WEMU_WRITE_WATCH range overflow: {spec}")))?;
        return Ok((start, end));
    }
    if let Some((start, end)) = spec.split_once('-') {
        let start = parse_num(start)?;
        let end = parse_num(end)?;
        if end <= start {
            return Err(Error::Cli(format!(
                "WEMU_WRITE_WATCH end must be greater than start: {spec}"
            )));
        }
        return Ok((start, end));
    }
    Err(Error::Cli(format!(
        "WEMU_WRITE_WATCH must look like 0xSTART:0xSIZE or 0xSTART-0xEND, got {spec}"
    )))
}

#[cfg(debug_assertions)]
fn parse_num(s: &str) -> Result<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        s.parse::<u32>()
    }
    .map_err(|err| Error::Cli(format!("invalid WEMU_WRITE_WATCH number {s}: {err}")))
}

#[inline(always)]
pub fn align_down(addr: u32) -> u32 {
    addr & PAGE_MASK
}

#[inline(always)]
pub fn align_up(addr: u32) -> Result<u32> {
    if addr == 0 {
        return Ok(0);
    }
    addr.checked_add(PAGE_SIZE - 1)
        .map(|x| x & PAGE_MASK)
        .ok_or_else(|| Error::Memory(format!("align_up overflow: {addr:08x}")))
}

#[cfg(test)]
mod tests {
    use super::{Memory, PagePerm};

    #[test]
    fn mapped_guest_memory_uses_identity_address() {
        const BASE: u32 = 0x0300_0000;
        let mut memory = Memory::new();

        memory
            .map(BASE, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        memory.write_u32(BASE + 4, 0x1234_5678).unwrap();

        let raw = unsafe { std::ptr::read_unaligned((BASE + 4) as usize as *const u32) };
        assert_eq!(raw, 0x1234_5678);

        unsafe {
            std::ptr::write_unaligned((BASE + 8) as usize as *mut u32, 0x9abc_def0);
        }
        assert_eq!(memory.read_u32(BASE + 8).unwrap(), 0x9abc_def0);
    }

    #[test]
    fn unmap_removes_identity_page_from_allowed_set() {
        const BASE: u32 = 0x0310_0000;
        let mut memory = Memory::new();

        memory
            .map(BASE, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        memory.write_u8(BASE, 0xaa).unwrap();
        memory.unmap(BASE, 0x1000).unwrap();

        assert!(!memory.is_mapped(BASE, PagePerm::READ));
        assert!(memory.checked_read_u8(BASE).is_err());
        #[cfg(debug_assertions)]
        assert!(memory.read_u8(BASE).is_err());
    }
}
