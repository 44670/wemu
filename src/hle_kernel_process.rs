// void ExitProcess(UINT code)
// Stop the emulator with the process exit code.
fn hle_exit_process(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let code = arg(emu, 0);
    ret(emu, 0);
    emu.stopped = Some(StopReason::ExitProcess(code));
    HleResult::Retn(4)
}

// UINT WinExec(LPCSTR cmdline, UINT show)
// Pretend the shell accepted a guest-side launch without spawning host processes.
fn hle_win_exec(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 33);
    HleResult::Retn(8)
}

// void ExitThread(DWORD code)
// Treat the main guest thread exit as process termination.
fn hle_exit_thread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let code = arg(emu, 0);
    ret(emu, 0);
    emu.stopped = Some(StopReason::ExitProcess(code));
    HleResult::Retn(4)
}

// BOOL GetExitCodeProcess(HANDLE process, DWORD *exit_code)
// Report the single emulated process as still active.
fn hle_get_exit_code_process(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const STILL_ACTIVE: u32 = 259;
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, STILL_ACTIVE).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL GetExitCodeThread(HANDLE thread, DWORD *exit_code)
// Report fake worker threads as still active.
fn hle_get_exit_code_thread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const STILL_ACTIVE: u32 = 259;
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, STILL_ACTIVE).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// HANDLE OpenProcess(DWORD access, BOOL inherit, DWORD pid)
// Return a fake process handle for the single emulated process.
fn hle_open_process(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let pid = arg(emu, 2);
    if pid == 0 {
        emu.hle.last_error = 87;
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    let handle = emu.hle.alloc_handle(Handle::Process);
    ret(emu, handle);
    HleResult::Retn(12)
}

// BOOL IsDebuggerPresent(void)
// Report no debugger attached for anti-debug feature probes.
fn hle_is_debugger_present(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// void DebugBreak(void)
// Ignore guest breakpoint traps unless a real debugger integration is added.
fn hle_debug_break(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// BOOL DisableThreadLibraryCalls(HMODULE module)
// Report success because HLE DLL thread attach/detach callbacks are absent.
fn hle_disable_thread_library_calls(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL IsProcessorFeaturePresent(DWORD feature)
// Conservatively report optional CPU feature probes as unavailable.
fn hle_is_processor_feature_present(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// BOOL FlushInstructionCache(HANDLE process, void *base, SIZE_T size)
// Accept self-modifying-code cache flush requests; the emulator is coherent.
fn hle_flush_instruction_cache(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(12)
}

// LPSTR GetCommandLineA(void)
// Return the process command line pointer.
fn hle_get_command_line_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.command_line_a);
    HleResult::Retn(0)
}

// LPWSTR GetCommandLineW(void)
// Return the UTF-16 process command line pointer.
fn hle_get_command_line_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.command_line_w);
    HleResult::Retn(0)
}

// LPCH GetEnvironmentStringsA(void)
// Return the fake process environment block.
fn hle_get_environment_strings(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.environment_a);
    HleResult::Retn(0)
}

// LPWCH GetEnvironmentStringsW(void)
// Return a widened fake process environment block.
fn hle_get_environment_strings_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let env = "SystemRoot=C:\\WINDOWS\0PATH=C:\\WINDOWS;C:\\WINDOWS\\SYSTEM32\0\0";
    let units = env.encode_utf16().count() as u32;
    let ptr = emu
        .hle
        .alloc(
            &mut emu.memory,
            units.saturating_mul(2),
            PagePerm::READ | PagePerm::WRITE,
        )
        .hle();
    let mut addr = ptr;
    for unit in env.encode_utf16() {
        emu.memory.write_u16(addr, unit).hle();
        addr = addr.wrapping_add(2);
    }
    ret(emu, ptr);
    HleResult::Retn(0)
}

// BOOL FreeEnvironmentStringsA(LPCH env)
// Accept release of the process environment block.
fn hle_free_environment_strings_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL FreeEnvironmentStringsW(LPWCH env)
// Release widened environment blocks allocated by GetEnvironmentStringsW.
fn hle_free_environment_strings_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    emu.hle.free_alloc(&mut emu.memory, ptr).hle();
    ret(emu, 1);
    HleResult::Retn(4)
}

// DWORD GetEnvironmentVariableA(LPCSTR name, LPSTR dst, DWORD size)
// Look up a variable in the fake ANSI environment block.
fn hle_get_environment_variable_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.cstr_lossy(arg(emu, 0), 256).unwrap_or_default();
    let dst = arg(emu, 1);
    let size = arg(emu, 2) as usize;
    let value = lookup_environment_variable(emu, &name);
    if let Some(value) = value {
        if dst != 0 && size != 0 {
            emu.memory.write_cstr(dst, &value, size).hle();
        }
        ret(emu, value.len() as u32);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// DWORD GetEnvironmentVariableW(LPCWSTR name, LPWSTR dst, DWORD size)
// Look up a variable in the fake ANSI environment block and widen the value.
fn hle_get_environment_variable_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.utf16z_lossy(arg(emu, 0), 256).unwrap_or_default();
    let dst = arg(emu, 1);
    let size = arg(emu, 2) as usize;
    let value = lookup_environment_variable(emu, &name);
    if let Some(value) = value {
        if dst != 0 && size != 0 {
            emu.memory.write_utf16z(dst, &value, size).hle();
        }
        ret(emu, value.encode_utf16().count() as u32);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// DWORD TlsAlloc(void)
// Allocate a fake TLS slot initialized to zero.
fn hle_tls_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let idx = emu.hle.tls_slots.len();
    emu.hle.tls_slots.push(0);
    ret(emu, idx as u32);
    HleResult::Retn(0)
}

// BOOL TlsFree(DWORD slot)
// Clear a fake TLS slot if it exists.
fn hle_tls_free(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// LPVOID TlsGetValue(DWORD slot)
// Return the stored fake TLS value or zero.
fn hle_tls_get_value(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let idx = arg(emu, 0) as usize;
    ret(emu, emu.hle.tls_slots.get(idx).copied().unwrap_or(0));
    HleResult::Retn(4)
}

// BOOL TlsSetValue(DWORD slot, LPVOID value)
// Store a fake TLS value for the requested slot.
fn hle_tls_set_value(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let idx = arg(emu, 0) as usize;
    let value = arg(emu, 1);
    if idx >= emu.hle.tls_slots.len() {
        emu.hle.tls_slots.resize(idx + 1, 0);
    }
    emu.hle.tls_slots[idx] = value;
    ret(emu, 1);
    HleResult::Retn(8)
}

// DWORD FlsAlloc(PFLS_CALLBACK_FUNCTION callback)
// Allocate a fake FLS slot; callback teardown is not observable yet.
fn hle_fls_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let idx = emu.hle.tls_slots.len();
    emu.hle.tls_slots.push(0);
    ret(emu, idx as u32);
    HleResult::Retn(4)
}

// BOOL FlsFree(DWORD slot)
// Clear a fake FLS slot if it exists.
fn hle_fls_free(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let idx = arg(emu, 0) as usize;
    if let Some(slot) = emu.hle.tls_slots.get_mut(idx) {
        *slot = 0;
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// LPVOID FlsGetValue(DWORD slot)
// Return the stored fake FLS value or zero.
fn hle_fls_get_value(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let idx = arg(emu, 0) as usize;
    ret(emu, emu.hle.tls_slots.get(idx).copied().unwrap_or(0));
    HleResult::Retn(4)
}

// BOOL FlsSetValue(DWORD slot, LPVOID value)
// Store a fake FLS value for the requested slot.
fn hle_fls_set_value(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let idx = arg(emu, 0) as usize;
    let value = arg(emu, 1);
    if idx >= emu.hle.tls_slots.len() {
        emu.hle.tls_slots.resize(idx + 1, 0);
    }
    emu.hle.tls_slots[idx] = value;
    ret(emu, 1);
    HleResult::Retn(8)
}

// HANDLE CreateThread(void *sec, SIZE_T stack, LPTHREAD_START_ROUTINE start, void *arg, DWORD flags, DWORD *tid)
// Return a fake worker-thread handle; guest thread scheduling is not active yet.
fn hle_create_thread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let tid_out = arg(emu, 5);
    let handle = emu.hle.alloc_handle(Handle::Thread);
    if tid_out != 0 {
        emu.memory.write_u32(tid_out, handle).hle();
    }
    ret(emu, handle);
    HleResult::Retn(24)
}

// HANDLE CreateEventA(void *sec, BOOL manual, BOOL initial, LPCSTR name)
// Create or open a named fake event handle without kernel wait state.
fn hle_create_event_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.cstr_lossy(arg(emu, 3), 256).unwrap_or_default();
    let handle = create_event_impl(emu, &name);
    ret(emu, handle);
    HleResult::Retn(16)
}

// HANDLE CreateEventW(void *sec, BOOL manual, BOOL initial, LPCWSTR name)
// Create or open a named fake event handle without kernel wait state.
fn hle_create_event_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu
        .memory
        .utf16z_lossy(arg(emu, 3), 256)
        .unwrap_or_default();
    let handle = create_event_impl(emu, &name);
    ret(emu, handle);
    HleResult::Retn(16)
}

// HANDLE OpenEventA(DWORD access, BOOL inherit, LPCSTR name)
// Open an existing named fake event, failing when no such event exists.
fn hle_open_event_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.cstr_lossy(arg(emu, 2), 256).unwrap_or_default();
    let handle = open_event_impl(emu, &name);
    ret(emu, handle);
    HleResult::Retn(12)
}

// HANDLE OpenEventW(DWORD access, BOOL inherit, LPCWSTR name)
// Open an existing named fake event, failing when no such event exists.
fn hle_open_event_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu
        .memory
        .utf16z_lossy(arg(emu, 2), 256)
        .unwrap_or_default();
    let handle = open_event_impl(emu, &name);
    ret(emu, handle);
    HleResult::Retn(12)
}

fn create_event_impl(emu: &mut Emulator, name: &str) -> u32 {
    if let Some(key) = kernel_object_key(name) {
        if emu.hle.named_kernel_objects.contains_key(&key) {
            emu.hle.last_error = 183;
            trace_fs!("CreateEvent name={name:?} -> existing");
        } else {
            emu.hle
                .named_kernel_objects
                .insert(key, NamedKernelObject::Event);
            emu.hle.last_error = 0;
            trace_fs!("CreateEvent name={name:?} -> created");
        }
    } else {
        emu.hle.last_error = 0;
    }
    emu.hle.alloc_handle(Handle::Event)
}

fn open_event_impl(emu: &mut Emulator, name: &str) -> u32 {
    let Some(key) = kernel_object_key(name) else {
        emu.hle.last_error = 87;
        trace_fs!("OpenEvent name={name:?} -> invalid");
        return 0;
    };
    match emu.hle.named_kernel_objects.get(&key) {
        Some(NamedKernelObject::Event) => {
            emu.hle.last_error = 0;
            let handle = emu.hle.alloc_handle(Handle::Event);
            trace_fs!("OpenEvent name={name:?} -> {handle:08x}");
            handle
        }
        Some(NamedKernelObject::FileMapping(_)) => {
            emu.hle.last_error = 6;
            trace_fs!("OpenEvent name={name:?} -> wrong object type");
            0
        }
        None => {
            emu.hle.last_error = 2;
            trace_fs!("OpenEvent name={name:?} -> missing");
            0
        }
    }
}

// HANDLE CreateMutexA(void *sec, BOOL owner, LPCSTR name)
// Allocate a fake mutex handle for single-threaded immediate waits.
fn hle_create_mutex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = emu.hle.alloc_handle(Handle::Mutex);
    ret(emu, h);
    HleResult::Retn(12)
}

// HANDLE CreateMutexW(void *sec, BOOL owner, LPCWSTR name)
// Allocate a fake mutex handle for single-threaded immediate waits.
fn hle_create_mutex_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_create_mutex_a(emu, entry)
}

// HANDLE CreateSemaphoreA(void *sec, LONG initial, LONG max, LPCSTR name)
// Allocate a fake semaphore handle for single-threaded immediate waits.
fn hle_create_semaphore_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = emu.hle.alloc_handle(Handle::Semaphore);
    ret(emu, h);
    HleResult::Retn(16)
}

// HANDLE CreateSemaphoreW(void *sec, LONG initial, LONG max, LPCWSTR name)
// Allocate a fake semaphore handle for single-threaded immediate waits.
fn hle_create_semaphore_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_create_semaphore_a(emu, entry)
}

// DWORD WaitForSingleObject(HANDLE h, DWORD ms)
// Return WAIT_OBJECT_0 immediately for fake synchronization.
fn hle_wait_for_single_object(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    if arg(emu, 1) != 0 {
        flush_gdi_present_if_pending(emu).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// BOOL ReleaseMutex(HANDLE mutex)
// Treat fake single-threaded mutexes as always releasable.
fn hle_release_mutex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL ReleaseSemaphore(HANDLE sem, LONG release, LPLONG previous)
// Treat fake semaphores as released and optionally report count one.
fn hle_release_semaphore(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let previous = arg(emu, 2);
    if previous != 0 {
        emu.memory.write_u32(previous, 1).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// void Sleep(DWORD ms)
// Park live frontends until timeout, but service due WinMM callbacks covered by the sleep interval.
fn hle_sleep(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let ms = arg(emu, 0);
    if dispatch_sleep_mm_timer_callback(emu, entry, ms, 4, 0) {
        return HleResult::Retn(4);
    }
    if emu.has_live_frontend() && ms != 0 {
        emu.refresh_guest_time();
        return HleResult::Wait(HleWaitState::Timeout {
            until_ms: emu.guest_time_ms.saturating_add(ms as u64),
            ret_value: 0,
            arg_bytes: 4,
        });
    }
    flush_gdi_present_if_pending(emu).hle();
    ret(emu, 0);
    HleResult::Retn(4)
}

// DWORD SleepEx(DWORD ms, BOOL alertable)
// Park live frontends until timeout, service due WinMM callbacks, and report no APC completion.
fn hle_sleep_ex(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let ms = arg(emu, 0);
    if dispatch_sleep_mm_timer_callback(emu, entry, ms, 8, 0) {
        return HleResult::Retn(8);
    }
    if emu.has_live_frontend() && ms != 0 {
        emu.refresh_guest_time();
        return HleResult::Wait(HleWaitState::Timeout {
            until_ms: emu.guest_time_ms.saturating_add(ms as u64),
            ret_value: 0,
            arg_bytes: 8,
        });
    }
    flush_gdi_present_if_pending(emu).hle();
    ret(emu, 0);
    HleResult::Retn(8)
}

fn dispatch_sleep_mm_timer_callback(
    emu: &mut Emulator,
    entry: &HleEntry,
    ms: u32,
    hle_arg_bytes: u32,
    hle_return_value: u32,
) -> bool {
    if ms == 0 {
        return false;
    }
    emu.refresh_guest_time();
    let until_ms = emu.guest_time_ms.saturating_add(ms as u64);
    let Some(due_ms) = emu.hle.next_due_mm_timer_ms(until_ms) else {
        return false;
    };
    emu.guest_time_ms = due_ms;
    dispatch_due_mm_timer_callback(emu, entry, hle_arg_bytes, hle_return_value)
}

// BOOL WaitMessage(void)
// Cooperatively wait until the thread message queue has input or app messages.
fn hle_wait_message(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    if !emu.hle.has_messages() {
        emu.reschedule_message_pump().hle();
        if emu.stopped.is_some() {
            ret(emu, 0);
            return HleResult::Retn(0);
        }
    }

    if emu.hle.has_messages() {
        ret(emu, 1);
        return HleResult::Retn(0);
    }

    HleResult::Wait(HleWaitState::Message {
        out: 0,
        hwnd: 0,
        min: 0,
        max: 0,
    })
}

// MMRESULT timeSetEvent(UINT delay, UINT res, LPTIMECALLBACK cb, DWORD_PTR user, UINT event)
// Register a cooperative multimedia callback driven from safe HLE boundaries.
fn hle_time_set_event(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let delay = arg(emu, 0);
    let callback = arg(emu, 2);
    let user = arg(emu, 3);
    let event = arg(emu, 4);
    let id = emu
        .hle
        .set_mm_timer(delay, callback, user, event, emu.guest_time_ms);
    ret(emu, id);
    HleResult::Retn(20)
}

// MMRESULT timeKillEvent(UINT id)
// Remove a cooperative multimedia timer registration.
fn hle_time_kill_event(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let id = arg(emu, 0);
    let removed = emu.hle.kill_mm_timer(id);
    ret(emu, if removed { 0 } else { 97 });
    HleResult::Retn(4)
}

// LONG InterlockedDecrement(LONG volatile *value)
// Atomically decrement a guest 32-bit value in the single-threaded scheduler.
fn hle_interlocked_decrement(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = hle_interlocked_add(emu, u32::MAX);
    ret(emu, value);
    HleResult::Retn(4)
}

// LONG InterlockedIncrement(LONG volatile *value)
// Atomically increment a guest 32-bit value in the single-threaded scheduler.
fn hle_interlocked_increment(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = hle_interlocked_add(emu, 1);
    ret(emu, value);
    HleResult::Retn(4)
}

// LONG InterlockedExchange(LONG volatile *target, LONG value)
// Swap a guest 32-bit value and return the previous value like Wine's i386 stdcall export.
fn hle_interlocked_exchange(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    let value = arg(emu, 1);
    let old = emu.memory.read_u32(ptr).hle();
    emu.memory.write_u32(ptr, value).hle();
    ret(emu, old);
    HleResult::Retn(8)
}

// void GetSystemInfo(LPSYSTEM_INFO info)
// Fill a stable i386 SYSTEM_INFO block using the same fields Wine populates from NT.
fn hle_get_system_info(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u16(out, 0).hle(); // PROCESSOR_ARCHITECTURE_INTEL
        emu.memory.write_u16(out + 2, 0).hle();
        emu.memory.write_u32(out + 4, PAGE_SIZE).hle();
        emu.memory.write_u32(out + 8, 0x0001_0000).hle();
        emu.memory.write_u32(out + 12, 0x7ffe_ffff).hle();
        emu.memory.write_u32(out + 16, 1).hle();
        emu.memory.write_u32(out + 20, 1).hle();
        emu.memory.write_u32(out + 24, 586).hle(); // PROCESSOR_INTEL_PENTIUM
        emu.memory.write_u32(out + 28, ALLOCATION_GRANULARITY).hle();
        emu.memory.write_u16(out + 32, 6).hle();
        emu.memory.write_u16(out + 34, 0).hle();
    }
    HleResult::Retn(4)
}

// BOOL IsBadCodePtr(FARPROC ptr)
// Validate that the pointer lands in currently mapped guest memory.
fn hle_is_bad_code_ptr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    ret(emu, (!emu.memory.is_mapped(ptr, PagePerm::READ)) as u32);
    HleResult::Retn(4)
}

// BOOL IsBadReadPtr(const void *ptr, UINT_PTR size)
// Validate that the probed byte range is currently mapped.
fn hle_is_bad_read_ptr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    let size = arg(emu, 1);
    ret(emu, is_bad_memory_range(emu, ptr, size) as u32);
    HleResult::Retn(8)
}

// BOOL IsBadWritePtr(void *ptr, UINT_PTR size)
// Validate that the probed byte range is currently mapped.
fn hle_is_bad_write_ptr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    let size = arg(emu, 1);
    ret(emu, is_bad_memory_range(emu, ptr, size) as u32);
    HleResult::Retn(8)
}

// HANDLE GetCurrentProcess(void)
// Return the conventional pseudo-handle for the current process.
fn hle_get_current_process(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0xffff_ffff);
    HleResult::Retn(0)
}

// BOOL GetProcessAffinityMask(HANDLE process, DWORD_PTR *process_mask, DWORD_PTR *system_mask)
// Report a single-CPU affinity mask for the emulated process and host system.
fn hle_get_process_affinity_mask(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let process_mask = arg(emu, 1);
    let system_mask = arg(emu, 2);
    if process_mask != 0 {
        emu.memory.write_u32(process_mask, 1).hle();
    }
    if system_mask != 0 {
        emu.memory.write_u32(system_mask, 1).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// DWORD GetCurrentProcessId(void)
// Return a stable nonzero process id for single-process HLE.
fn hle_get_current_process_id(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, HLE_PROCESS_ID);
    HleResult::Retn(0)
}

// HANDLE GetCurrentThread(void)
// Return the Win32 current-thread pseudo handle.
fn hle_get_current_thread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0xffff_fffe);
    HleResult::Retn(0)
}

// DWORD GetCurrentThreadId(void)
// Return a stable nonzero thread id for the single emulated thread.
fn hle_get_current_thread_id(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, HLE_THREAD_ID);
    HleResult::Retn(0)
}

// UINT SetHandleCount(UINT count)
// Accept the obsolete MSVCRT handle-table sizing request and return the count.
fn hle_set_handle_count(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(4)
}

// BOOL SetThreadPriority(HANDLE thread, int priority)
// Accept thread priority changes in the single-threaded HLE scheduler.
fn hle_set_thread_priority(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetPriorityClass(HANDLE process, DWORD priority)
// Accept process priority changes without affecting deterministic scheduling.
fn hle_set_priority_class(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// DWORD GetPriorityClass(HANDLE process)
// Return a normal priority class for the single emulated process.
fn hle_get_priority_class(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0000_0020);
    HleResult::Retn(4)
}

// BOOL CreateProcessA(LPCSTR app, LPSTR cmd, void *psa, void *tsa, BOOL inherit, DWORD flags, void *env, LPCSTR cwd, STARTUPINFOA *si, PROCESS_INFORMATION *pi)
// Decline host process launch requests while clearing PROCESS_INFORMATION.
fn hle_create_process_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let pi = arg(emu, 9);
    if pi != 0 {
        emu.memory.memset(pi, 0, 16).hle();
    }
    emu.hle.last_error = 2;
    ret(emu, 0);
    HleResult::Retn(40)
}

// BOOL DuplicateHandle(HANDLE src_proc, HANDLE src, HANDLE dst_proc, HANDLE *dst, DWORD access, BOOL inherit, DWORD options)
// Duplicate simple pseudo handles by returning the same guest handle value.
fn hle_duplicate_handle(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = arg(emu, 1);
    let dst = arg(emu, 3);
    if dst != 0 {
        emu.memory.write_u32(dst, src).hle();
    }
    ret(emu, 1);
    HleResult::Retn(28)
}

// BOOL GetThreadContext(HANDLE thread, CONTEXT *context)
// Report no real thread context for fake worker threads.
fn hle_get_thread_context(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.last_error = 1;
    ret(emu, 0);
    HleResult::Retn(8)
}

// NTSTATUS NtSetInformationThread(HANDLE thread, THREADINFOCLASS class, PVOID info, ULONG len)
// Accept debugger-hiding and scheduler hints in the single-threaded runtime.
fn hle_nt_set_information_thread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(16)
}

// BOOL ResetEvent(HANDLE event)
// Accept reset for fake event handles.
fn hle_reset_event(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL InitializeCriticalSectionAndSpinCount(CRITICAL_SECTION *cs, DWORD spin)
// Accept critical-section initialization in the single-threaded runtime.
fn hle_initialize_critical_section_and_spin_count(
    emu: &mut Emulator,
    _: &HleEntry,
) -> HleResult {
    initialize_critical_section_memory(emu, arg(emu, 0), arg(emu, 1));
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL InitializeCriticalSectionEx(CRITICAL_SECTION *cs, DWORD spin, DWORD flags)
// Initialize the guest structure and ignore debug-info/spin flags.
fn hle_initialize_critical_section_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    initialize_critical_section_memory(emu, arg(emu, 0), arg(emu, 1));
    ret(emu, 1);
    HleResult::Retn(12)
}

fn initialize_critical_section_memory(emu: &mut Emulator, cs: u32, spin: u32) {
    if cs == 0 {
        return;
    }
    emu.memory.write_u32(cs, 0).hle();
    emu.memory.write_u32(cs + 4, 0xffff_ffff).hle();
    emu.memory.write_u32(cs + 8, 0).hle();
    emu.memory.write_u32(cs + 12, 0).hle();
    emu.memory.write_u32(cs + 16, 0).hle();
    emu.memory.write_u32(cs + 20, spin).hle();
}

// void InitializeSListHead(SLIST_HEADER *head)
// Clear the 32-bit interlocked singly-linked-list header.
fn hle_initialize_slist_head(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let head = arg(emu, 0);
    if head != 0 {
        emu.memory.memset(head, 0, 8).hle();
    }
    HleResult::Retn(4)
}

// PSLIST_ENTRY InterlockedFlushSList(SLIST_HEADER *head)
// Atomically detach the current 32-bit list head and clear the header.
fn hle_interlocked_flush_slist(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let head = arg(emu, 0);
    let first = if head != 0 {
        let first = emu.memory.read_u32(head).hle();
        emu.memory.memset(head, 0, 8).hle();
        first
    } else {
        0
    };
    ret(emu, first);
    HleResult::Retn(4)
}

// PVOID DecodePointer(PVOID ptr)
// Return process-local encoded pointers unchanged.
fn hle_decode_pointer(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(4)
}

// void OutputDebugStringA(LPCSTR text)
// Print debug text only when general HLE tracing is enabled.
fn hle_output_debug_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    if hle_trace_enabled(HLE_TRACE_FS) {
        let text = emu.memory.cstr_lossy(arg(emu, 0), 1024).unwrap_or_default();
        eprintln!("OutputDebugStringA {text:?}");
    }
    HleResult::Retn(4)
}

// BOOL ClearCommBreak(HANDLE file)
// Accept serial-port break clearing for games probing modem support.
fn hle_clear_comm_break(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL SetCommBreak(HANDLE file)
// Accept serial-port break setting for games probing modem support.
fn hle_set_comm_break(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL EscapeCommFunction(HANDLE file, DWORD function)
// Accept serial line-control operations for disabled modem paths.
fn hle_escape_comm_function(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetupComm(HANDLE file, DWORD in_queue, DWORD out_queue)
// Accept serial queue sizing for disabled modem paths.
fn hle_setup_comm(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL PurgeComm(HANDLE file, DWORD flags)
// Accept serial buffer purge requests for disabled modem paths.
fn hle_purge_comm(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL GetCommModemStatus(HANDLE file, DWORD *status)
// Report no modem status bits for fake serial devices.
fn hle_get_comm_modem_status(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL ClearCommError(HANDLE file, DWORD *errors, COMSTAT *stat)
// Report no serial errors or queued bytes.
fn hle_clear_comm_error(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let errors = arg(emu, 1);
    let stat = arg(emu, 2);
    if errors != 0 {
        emu.memory.write_u32(errors, 0).hle();
    }
    if stat != 0 {
        emu.memory.memset(stat, 0, 12).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL GetCommState(HANDLE file, DCB *dcb)
// Fill a minimal 9600-8N1 DCB for modem capability probes.
fn hle_get_comm_state(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dcb = arg(emu, 1);
    if dcb != 0 {
        let size = emu.memory.read_u32(dcb).unwrap_or(28).clamp(4, 128);
        emu.memory.memset(dcb, 0, size).hle();
        emu.memory.write_u32(dcb, size).hle();
        if size >= 8 {
            emu.memory.write_u32(dcb + 4, 9600).hle();
        }
        if size >= 28 {
            emu.memory.write_u8(dcb + 26, 8).hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetCommState(HANDLE file, DCB *dcb)
// Accept serial settings for disabled modem paths.
fn hle_set_comm_state(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetCommTimeouts(HANDLE file, COMMTIMEOUTS *timeouts)
// Accept serial timeout settings for disabled modem paths.
fn hle_set_comm_timeouts(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

fn hle_interlocked_add(emu: &mut Emulator, delta: u32) -> u32 {
    let ptr = arg(emu, 0);
    let value = emu.memory.read_u32(ptr).hle().wrapping_add(delta);
    emu.memory.write_u32(ptr, value).hle();
    value
}

fn is_bad_memory_range(emu: &Emulator, ptr: u32, size: u32) -> bool {
    if size == 0 {
        return false;
    }
    let Some(last) = ptr.checked_add(size - 1) else {
        return true;
    };
    !emu.memory.is_mapped(ptr, PagePerm::READ) || !emu.memory.is_mapped(last, PagePerm::READ)
}

// BOOL TerminateThread(HANDLE thread, DWORD exit_code)
// Accept termination of fake worker threads without touching the main task.
fn hle_terminate_thread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// void RtlUnwind(PVOID frame, PVOID target, PEXCEPTION_RECORD record, PVOID value)
// Treat structured-exception unwind as complete for startup cleanup paths.
fn hle_rtl_unwind(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(16)
}

// HANDLE GetProcessHeap(void)
// Return a stable fake process heap handle.
fn hle_get_process_heap(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x5000_0000);
    HleResult::Retn(0)
}

// void GetStartupInfoW(STARTUPINFOW *info)
// Fill a minimal startup-info block with SW_SHOWNORMAL defaults.
fn hle_get_startup_info_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let info = arg(emu, 0);
    if info != 0 {
        emu.memory.memset(info, 0, 68).hle();
        emu.memory.write_u32(info, 68).hle();
        emu.memory.write_u32(info + 44, 0x0000_0001).hle();
        emu.memory.write_u16(info + 48, 1).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// void GetStartupInfoA(STARTUPINFOA *info)
// Fill a minimal startup-info block with SW_SHOWNORMAL defaults.
fn hle_get_startup_info_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let info = arg(emu, 0);
    if info != 0 {
        emu.memory.memset(info, 0, 68).hle();
        emu.memory.write_u32(info, 68).hle();
        emu.memory.write_u32(info + 44, 0x0000_0001).hle();
        emu.memory.write_u16(info + 48, 1).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// LANGID GetUserDefaultUILanguage(void)
// Return US English so Notepad does not request RTL process layout.
fn hle_get_user_default_ui_language(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0409);
    HleResult::Retn(0)
}

// BOOL TerminateProcess(HANDLE process, UINT exitCode)
// Treat termination of the current process as an emulator stop.
fn hle_terminate_process(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let code = arg(emu, 1);
    ret(emu, 1);
    emu.stopped = Some(StopReason::ExitProcess(code));
    HleResult::Retn(8)
}

// BOOL GetUserNameA(LPSTR out, DWORD *size)
// Return a stable ANSI user name for shell and settings paths.
fn hle_get_user_name_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let size_ptr = arg(emu, 1);
    let value = "User";
    let needed = value.len() + 1;
    let cap = if size_ptr != 0 {
        emu.memory.read_u32(size_ptr).hle() as usize
    } else {
        0
    };
    if out != 0 && cap != 0 {
        emu.memory.write_cstr(out, value, cap).hle();
    }
    if size_ptr != 0 {
        emu.memory.write_u32(size_ptr, needed as u32).hle();
    }
    if cap < needed && out != 0 {
        emu.hle.last_error = 122;
        ret(emu, 0);
    } else {
        ret(emu, 1);
    }
    HleResult::Retn(8)
}

// DWORD GetWindowThreadProcessId(HWND hwnd, DWORD *pid)
// Return the single emulated GUI thread and process identifiers.
fn hle_get_window_thread_process_id(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let pid_out = arg(emu, 1);
    if pid_out != 0 {
        emu.memory.write_u32(pid_out, 1).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetProcessDefaultLayout(DWORD layout)
// Accept layout changes without altering the fixed framebuffer.
fn hle_set_process_default_layout(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// VOID RaiseException(DWORD code, DWORD flags, DWORD argc, const ULONG_PTR *argv)
// Logically consume software exceptions that legacy runtimes import for error paths.
fn hle_raise_exception(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(16)
}

fn lookup_environment_variable(emu: &Emulator, name: &str) -> Option<String> {
    let mut off = 0u32;
    while off < 4096 {
        let addr = emu.hle.environment_a.wrapping_add(off);
        let text = emu.memory.cstr_lossy(addr, 4096 - off as usize).ok()?;
        if text.is_empty() {
            break;
        }
        off = off.saturating_add(text.len() as u32 + 1);
        if let Some((key, value)) = text.split_once('=') {
            if key.eq_ignore_ascii_case(name) {
                return Some(value.to_string());
            }
        }
    }
    None
}
