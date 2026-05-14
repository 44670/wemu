// LPVOID VirtualAlloc(LPVOID addr, SIZE_T size, DWORD type, DWORD protect)
// Map a tracked 64K-aligned virtual region and zero the requested bytes.
fn hle_virtual_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let requested = arg(emu, 0);
    let size = arg(emu, 1).max(1);
    let protect = arg(emu, 3);
    let perm = win_protect_to_perm(protect);
    let addr = match emu
        .hle
        .virtual_alloc(&mut emu.memory, requested, size, protect, perm)
    {
        Ok(addr) => addr,
        Err(err) => {
            trace_alloc!(
                "VirtualAlloc req={requested:08x} size={size:x} type={:08x} protect={protect:08x} -> failed {err}",
                arg(emu, 2)
            );
            emu.hle.last_error = 487;
            ret(emu, 0);
            return HleResult::Retn(16);
        }
    };
    emu.memory.memset(addr, 0, size).hle();
    trace_alloc!(
        "VirtualAlloc req={requested:08x} size={size:x} type={:08x} protect={protect:08x} -> {addr:08x}",
        arg(emu, 2)
    );
    ret(emu, addr);
    HleResult::Retn(16)
}

// BOOL VirtualFree(LPVOID addr, SIZE_T size, DWORD type)
// Release or decommit a tracked virtual region.
fn hle_virtual_free(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let addr = arg(emu, 0);
    let size = arg(emu, 1);
    let free_type = arg(emu, 2);
    trace_alloc!("VirtualFree addr={addr:08x} size={size:x} type={free_type:08x}");
    let ok = emu
        .hle
        .virtual_free(&mut emu.memory, addr, size, free_type)
        .hle();
    if ok {
        ret(emu, 1);
    } else {
        emu.hle.last_error = 487;
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// BOOL VirtualProtect(LPVOID addr, SIZE_T size, DWORD protect, DWORD *old)
// Update page mapping metadata and report a readable/writable old protect.
fn hle_virtual_protect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let addr = arg(emu, 0);
    let size = arg(emu, 1);
    let protect = arg(emu, 2);
    let old_out = arg(emu, 3);
    let perm = win_protect_to_perm(protect);
    let old = if addr != 0 && size != 0 {
        emu.hle
            .virtual_protect(&mut emu.memory, addr, size, protect, perm)
            .unwrap_or(0x04)
    } else {
        0x04
    };
    if old_out != 0 {
        emu.memory.write_u32(old_out, old).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// SIZE_T VirtualQuery(LPCVOID addr, MEMORY_BASIC_INFORMATION *mbi, SIZE_T len)
// Describe tracked virtual regions or return a default committed page.
fn hle_virtual_query(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let addr = arg(emu, 0);
    let mbi = arg(emu, 1);
    let len = arg(emu, 2);
    if mbi != 0 && len >= 28 {
        let base = align_down(addr);
        if let Some(region) = emu.hle.virtual_region(addr) {
            let end = region.base.saturating_add(region.size);
            emu.memory.write_u32(mbi, base).hle();
            emu.memory.write_u32(mbi + 4, region.base).hle();
            emu.memory.write_u32(mbi + 8, region.protect).hle();
            emu.memory.write_u32(mbi + 12, (end - base).max(0x1000)).hle();
            emu.memory.write_u32(mbi + 16, 0x1000).hle();
            emu.memory.write_u32(mbi + 20, region.protect).hle();
        } else {
            emu.memory.write_u32(mbi, base).hle();
            emu.memory.write_u32(mbi + 4, base).hle();
            emu.memory.write_u32(mbi + 8, 0x04).hle();
            emu.memory.write_u32(mbi + 12, 0x1000).hle();
            emu.memory.write_u32(mbi + 16, 0x1000).hle();
            emu.memory.write_u32(mbi + 20, 0x04).hle();
        }
        emu.memory.write_u32(mbi + 24, 0x20000).hle();
        ret(emu, 28);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// void GlobalMemoryStatus(MEMORYSTATUS *status)
// Fill stable memory totals for legacy capacity probes.
fn hle_global_memory_status(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u32(out, 32).hle();
        emu.memory.write_u32(out + 4, 25).hle();
        emu.memory.write_u32(out + 8, 256 * 1024 * 1024).hle();
        emu.memory.write_u32(out + 12, 192 * 1024 * 1024).hle();
        emu.memory.write_u32(out + 16, 512 * 1024 * 1024).hle();
        emu.memory.write_u32(out + 20, 384 * 1024 * 1024).hle();
        emu.memory.write_u32(out + 24, 512 * 1024 * 1024).hle();
        emu.memory.write_u32(out + 28, 384 * 1024 * 1024).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// HANDLE HeapCreate(DWORD options, SIZE_T initial, SIZE_T maximum)
// Return a stable private-heap handle; allocations share the HLE heap arena.
fn hle_heap_create(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x5000_0001);
    HleResult::Retn(12)
}

// BOOL HeapDestroy(HANDLE heap)
// Accept private-heap destruction without unmapping individual allocations.
fn hle_heap_destroy(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// LPVOID HeapAlloc(HANDLE heap, DWORD flags, SIZE_T size)
// Allocate zeroed guest heap memory and return its pointer.
fn hle_heap_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let size = arg(emu, 2).max(1);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    emu.memory.memset(ptr, 0, size).hle();
    ret(emu, ptr);
    HleResult::Retn(12)
}

// BOOL HeapFree(HANDLE heap, DWORD flags, LPVOID ptr)
// Release a guest heap allocation when it belongs to the HLE heap.
fn hle_heap_free(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 2);
    emu.hle.free_alloc(&mut emu.memory, ptr).hle();
    ret(emu, 1);
    HleResult::Retn(12)
}

// LPVOID HeapReAlloc(HANDLE heap, DWORD flags, LPVOID ptr, SIZE_T size)
// Allocate a replacement HLE block and copy the requested byte count best-effort.
fn hle_heap_re_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 1);
    let old = arg(emu, 2);
    let size = arg(emu, 3).max(1);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    if (flags & 0x0000_0008) != 0 {
        emu.memory.memset(ptr, 0, size).hle();
    }
    if old != 0 {
        if let Ok(bytes) = emu.memory.read_bytes(old, size.min(4096) as usize) {
            emu.memory.write_bytes(ptr, &bytes).hle();
        }
        emu.hle.free_alloc(&mut emu.memory, old).hle();
    }
    ret(emu, ptr);
    HleResult::Retn(16)
}

// SIZE_T HeapSize(HANDLE heap, DWORD flags, LPCVOID ptr)
// Return the tracked allocation size for HLE heap blocks.
fn hle_heap_size(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.alloc_size(arg(emu, 2)).unwrap_or(0));
    HleResult::Retn(12)
}

// BOOL HeapValidate(HANDLE heap, DWORD flags, LPCVOID ptr)
// Treat tracked fake heaps as valid for runtime debug probes.
fn hle_heap_validate(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(12)
}

// HGLOBAL GlobalFree(HGLOBAL mem)
// Free HLE-allocated memory and return NULL on success.
fn hle_global_free(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    emu.hle.free_alloc(&mut emu.memory, ptr).hle();
    ret(emu, 0);
    HleResult::Retn(4)
}

// HLOCAL LocalAlloc(UINT flags, SIZE_T size)
// Allocate zeroed guest memory for local memory APIs.
fn hle_local_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let size = arg(emu, 1).max(1);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    emu.memory.memset(ptr, 0, size).hle();
    ret(emu, ptr);
    HleResult::Retn(8)
}

// HLOCAL LocalFree(HLOCAL mem)
// Release local memory and return NULL on success.
fn hle_local_free(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    emu.hle.free_alloc(&mut emu.memory, ptr).hle();
    ret(emu, 0);
    HleResult::Retn(4)
}

// HLOCAL LocalReAlloc(HLOCAL mem, SIZE_T size, UINT flags)
// Allocate a replacement block and copy the requested byte count best-effort.
fn hle_local_re_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let old = arg(emu, 0);
    let size = arg(emu, 1).max(1);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    if old != 0 {
        if let Ok(bytes) = emu.memory.read_bytes(old, size.min(4096) as usize) {
            emu.memory.write_bytes(ptr, &bytes).hle();
        }
        emu.hle.free_alloc(&mut emu.memory, old).hle();
    }
    ret(emu, ptr);
    HleResult::Retn(12)
}
