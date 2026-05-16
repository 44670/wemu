// int __getmainargs(int *argc, char ***argv, char ***env, int glob, void *start)
// Build argc/argv/envp arrays from the emulated command line.
fn hle_crt_getmainargs(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let argc_out = arg(emu, 0);
    let argv_out = arg(emu, 1);
    let env_out = arg(emu, 2);
    let command_line = emu
        .memory
        .cstr_lossy(emu.hle.command_line_a, 4096)
        .unwrap_or_default();
    let mut parts: Vec<String> = command_line
        .split_whitespace()
        .map(|part| part.trim_matches('"').to_string())
        .collect();
    if parts.is_empty() {
        parts.push("C:\\wemu.exe".to_string());
    }

    let argv = emu.hle.alloc(
        &mut emu.memory,
        (parts.len() as u32 + 1) * 4,
        PagePerm::READ | PagePerm::WRITE,
    ).hle();
    for (idx, part) in parts.iter().enumerate() {
        let ptr = emu.hle.alloc(
            &mut emu.memory,
            part.len() as u32 + 1,
            PagePerm::READ | PagePerm::WRITE,
        ).hle();
        emu.memory.write_cstr(ptr, part, part.len() + 1).hle();
        emu.memory.write_u32(argv + idx as u32 * 4, ptr).hle();
    }
    emu.memory.write_u32(argv + parts.len() as u32 * 4, 0).hle();

    let envp = emu
        .hle
        .alloc(&mut emu.memory, 8, PagePerm::READ | PagePerm::WRITE).hle();
    emu.memory.write_u32(envp, emu.hle.environment_a).hle();
    emu.memory.write_u32(envp + 4, 0).hle();

    if argc_out != 0 {
        emu.memory.write_u32(argc_out, parts.len() as u32).hle();
    }
    if argv_out != 0 {
        emu.memory.write_u32(argv_out, argv).hle();
    }
    if env_out != 0 {
        emu.memory.write_u32(env_out, envp).hle();
    }
    ret(emu, 0);
    HleResult::Retn(0)
}

// int __wgetmainargs(int *argc, wchar_t ***argv, wchar_t ***env, int glob, void *start)
// Build wide argc/argv arrays from the emulated UTF-16 command line.
fn hle_crt_wgetmainargs(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let argc_out = arg(emu, 0);
    let argv_out = arg(emu, 1);
    let env_out = arg(emu, 2);
    let command_line = emu
        .memory
        .utf16z_lossy(emu.hle.command_line_w, 4096)
        .unwrap_or_else(|_| "C:\\wemu.exe".to_string());
    let mut parts: Vec<String> = command_line
        .split_whitespace()
        .map(|part| part.trim_matches('"').to_string())
        .collect();
    if parts.is_empty() {
        parts.push("C:\\wemu.exe".to_string());
    }

    let argv = emu
        .hle
        .alloc(
            &mut emu.memory,
            (parts.len() as u32 + 1) * 4,
            PagePerm::READ | PagePerm::WRITE,
        )
        .hle();
    for (idx, part) in parts.iter().enumerate() {
        let chars = part.encode_utf16().count() as u32 + 1;
        let ptr = emu
            .hle
            .alloc(&mut emu.memory, chars * 2, PagePerm::READ | PagePerm::WRITE)
            .hle();
        emu.memory.write_utf16z(ptr, part, chars as usize).hle();
        emu.memory.write_u32(argv + idx as u32 * 4, ptr).hle();
    }
    emu.memory
        .write_u32(argv + parts.len() as u32 * 4, 0)
        .hle();

    let envp = emu
        .hle
        .alloc(&mut emu.memory, 4, PagePerm::READ | PagePerm::WRITE)
        .hle();
    emu.memory.write_u32(envp, 0).hle();

    if argc_out != 0 {
        emu.memory.write_u32(argc_out, parts.len() as u32).hle();
    }
    if argv_out != 0 {
        emu.memory.write_u32(argv_out, argv).hle();
    }
    if env_out != 0 {
        emu.memory.write_u32(env_out, envp).hle();
    }
    ret(emu, 0);
    HleResult::Retn(0)
}

// char **__initenv(void)
// Return the CRT initial environment pointer.
fn hle_crt_initenv(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.environment_a);
    HleResult::Retn(0)
}

// wchar_t **__winitenv(void)
// Return a null wide environment pointer for CRT startup.
fn hle_crt_winitenv(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// wchar_t *_wcmdln(void)
// Return the UTF-16 command line pointer if the import was called as a function.
fn hle_crt_wcmdln(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.command_line_w);
    HleResult::Retn(0)
}

// int msvcrt_stub_ret_zero(...)
// Return zero for CRT initialization/control stubs.
fn hle_crt_ret_zero(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// int isdigit(int c)
// Classify ASCII decimal digits for ctype callers.
fn hle_crt_isdigit(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ch = (arg(emu, 0) & 0xff) as u8;
    ret(emu, ch.is_ascii_digit() as u32);
    HleResult::Retn(0)
}

// int isalnum(int c)
// Classify ASCII letters and decimal digits for ctype callers.
fn hle_crt_isalnum(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ch = (arg(emu, 0) & 0xff) as u8;
    ret(emu, ch.is_ascii_alphanumeric() as u32);
    HleResult::Retn(0)
}

// int isspace(int c)
// Classify ASCII whitespace for ctype callers.
fn hle_crt_isspace(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ch = (arg(emu, 0) & 0xff) as u8;
    ret(emu, ch.is_ascii_whitespace() as u32);
    HleResult::Retn(0)
}

// int _purecall(void)
// Return zero if an old runtime imports but never meaningfully uses this abort hook.
fn hle_crt_purecall(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// void exit(int code)
// Stop the emulator with the process exit code.
fn hle_crt_exit(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let code = if entry.name == "abort" {
        3
    } else {
        arg(emu, 0)
    };
    ret(emu, code);
    emu.stopped = Some(StopReason::ExitProcess(code));
    HleResult::Retn(0)
}

// EXCEPTION_DISPOSITION _except_handler3(EXCEPTION_RECORD *rec, void *frame, CONTEXT *ctx, void *dispatcher)
// Continue SEH search; real handler execution is only needed after a guest exception is raised.
fn hle_crt_except_handler3(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(0)
}

// int _XcptFilter(unsigned long code, EXCEPTION_POINTERS *ptr)
// Use the CRT default of continuing exception search when no signal handler is installed.
fn hle_crt_xcpt_filter(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// uintptr_t msvcrt_return_arg0(uintptr_t arg0)
// Return the first argument for simple CRT callback registration stubs.
fn hle_crt_return_arg0(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(0)
}

// int *__p__commode(void)
// Return a stable pointer to the CRT commode global.
fn hle_crt_p_commode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    if emu.hle.crt_commode == 0 {
        emu.hle.crt_commode =
            emu.hle
                .alloc(&mut emu.memory, 4, PagePerm::READ | PagePerm::WRITE).hle();
        emu.memory.write_u32(emu.hle.crt_commode, 0).hle();
    }
    ret(emu, emu.hle.crt_commode);
    HleResult::Retn(0)
}

// int *__p__fmode(void)
// Return a stable pointer to the CRT file mode global.
fn hle_crt_p_fmode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    if emu.hle.crt_fmode == 0 {
        emu.hle.crt_fmode = emu
            .hle
            .alloc(&mut emu.memory, 4, PagePerm::READ | PagePerm::WRITE).hle();
        emu.memory.write_u32(emu.hle.crt_fmode, 0).hle();
    }
    ret(emu, emu.hle.crt_fmode);
    HleResult::Retn(0)
}

// FILE *_iob(void)
// Return a fake three-entry CRT stdio table.
fn hle_crt_iob(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    if emu.hle.crt_iob == 0 {
        emu.hle.crt_iob =
            emu.hle
                .alloc(&mut emu.memory, 3 * 32, PagePerm::READ | PagePerm::WRITE).hle();
    }
    ret(emu, emu.hle.crt_iob);
    HleResult::Retn(0)
}

// void *malloc(size_t size)
// Allocate zeroed guest heap memory and return its address.
fn hle_crt_malloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let size = arg(emu, 0).max(1);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE).hle();
    ret(emu, ptr);
    HleResult::Retn(0)
}

// void *calloc(size_t count, size_t size)
// Allocate zeroed guest heap memory for count*size bytes.
fn hle_crt_calloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let count = arg(emu, 0);
    let size = arg(emu, 1);
    let total = count.saturating_mul(size).max(1);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, total, PagePerm::READ | PagePerm::WRITE).hle();
    emu.memory.memset(ptr, 0, total).hle();
    ret(emu, ptr);
    HleResult::Retn(0)
}

// void free(void *ptr)
// Release HLE guest heap allocations; unknown pointers are accepted.
fn hle_crt_free(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    emu.hle.free_alloc(&mut emu.memory, ptr).hle();
    ret(emu, 0);
    HleResult::Retn(0)
}

// void *realloc(void *ptr, size_t size)
// Allocate a resized block, preserving bytes known from the old HLE allocation.
fn hle_crt_realloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let old = arg(emu, 0);
    let size = arg(emu, 1);
    if old == 0 {
        let ptr =
            emu.hle
                .alloc(&mut emu.memory, size.max(1), PagePerm::READ | PagePerm::WRITE).hle();
        ret(emu, ptr);
        return HleResult::Retn(0);
    }
    if size == 0 {
        emu.hle.free_alloc(&mut emu.memory, old).hle();
        ret(emu, 0);
        return HleResult::Retn(0);
    }
    let copy_len = emu.hle.alloc_size(old).unwrap_or(0).min(size);
    let bytes = if copy_len != 0 {
        emu.memory
            .read_bytes(old, copy_len as usize)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let new = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    if !bytes.is_empty() {
        emu.memory.write_bytes(new, &bytes).hle();
    }
    emu.hle.free_alloc(&mut emu.memory, old).hle();
    ret(emu, new);
    HleResult::Retn(0)
}

// void *memcpy(void *dst, const void *src, size_t len)
// Copy guest bytes and return the destination pointer.
fn hle_crt_memcpy(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let bytes = emu.memory.read_bytes(src, len).hle();
    emu.memory.write_bytes(dst, &bytes).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// void *memmove(void *dst, const void *src, size_t len)
// Copy possibly-overlapping guest bytes and return the destination pointer.
fn hle_crt_memmove(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let bytes = emu.memory.read_bytes(src, len).hle();
    emu.memory.write_bytes(dst, &bytes).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// void *memset(void *dst, int value, size_t len)
// Fill guest memory with one byte value and return the destination pointer.
fn hle_crt_memset(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let value = (arg(emu, 1) & 0xff) as u8;
    let len = arg(emu, 2);
    emu.memory.memset(dst, value, len).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// char *strcpy(char *dst, const char *src)
// Copy a NUL-terminated guest string and return dst.
fn hle_crt_strcpy(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let s = emu.memory.cstr_lossy(src, 4096).hle();
    emu.memory.write_cstr(dst, &s, s.len() + 1).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// char *_strdup(const char *src)
// Allocate and copy a NUL-terminated guest string.
fn hle_crt_strdup(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = emu.memory.cstr_lossy(arg(emu, 0), 4096).hle();
    let dst = emu
        .hle
        .alloc(&mut emu.memory, s.len() as u32 + 1, PagePerm::READ | PagePerm::WRITE)
        .hle();
    emu.memory.write_cstr(dst, &s, s.len() + 1).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// char *_strupr(char *s)
// Uppercase an ASCII guest string in place.
fn hle_crt_strupr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    let len = c_strlen(&emu.memory, ptr, 1 << 20).hle();
    for i in 0..len {
        let addr = ptr + i as u32;
        let b = emu.memory.read_u8(addr).hle().to_ascii_uppercase();
        emu.memory.write_u8(addr, b).hle();
    }
    ret(emu, ptr);
    HleResult::Retn(0)
}

// size_t strlen(const char *s)
// Count guest string bytes before the NUL terminator.
fn hle_crt_strlen(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = arg(emu, 0);
    ret(emu, c_strlen(&emu.memory, s, 1 << 20).hle() as u32);
    HleResult::Retn(0)
}

// int strcmp(const char *a, const char *b)
// Compare guest strings using byte ordering.
fn hle_crt_strcmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let lhs = emu.memory.cstr_lossy(arg(emu, 0), 4096).hle();
    let rhs = emu.memory.cstr_lossy(arg(emu, 1), 4096).hle();
    ret(emu, ord_to_crt(lhs.as_bytes().cmp(rhs.as_bytes())) as u32);
    HleResult::Retn(0)
}

// int strncmp(const char *a, const char *b, size_t n)
// Compare at most n guest string bytes.
fn hle_crt_strncmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let n = arg(emu, 2) as usize;
    let lhs = emu.memory.cstr_lossy(arg(emu, 0), n).hle();
    let rhs = emu.memory.cstr_lossy(arg(emu, 1), n).hle();
    ret(emu, ord_to_crt(lhs.as_bytes().cmp(rhs.as_bytes())) as u32);
    HleResult::Retn(0)
}

// int _stricmp(const char *a, const char *b)
// Compare guest strings using ASCII case-insensitive byte ordering.
fn hle_crt_stricmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let lhs = read_c_bytes(&emu.memory, arg(emu, 0), 1 << 20).hle();
    let rhs = read_c_bytes(&emu.memory, arg(emu, 1), 1 << 20).hle();
    ret(emu, ascii_casecmp(&lhs, &rhs) as u32);
    HleResult::Retn(0)
}

// int _strnicmp(const char *a, const char *b, size_t n)
// Compare at most n guest string bytes with ASCII case-insensitive ordering.
fn hle_crt_strnicmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let n = arg(emu, 2) as usize;
    let lhs = read_c_bytes(&emu.memory, arg(emu, 0), n).hle();
    let rhs = read_c_bytes(&emu.memory, arg(emu, 1), n).hle();
    ret(emu, ascii_casecmp(&lhs, &rhs) as u32);
    HleResult::Retn(0)
}

// FILE *fopen(const char *path, const char *mode)
// Open a guest path as a CRT stream backed by the existing file-handle table.
fn hle_crt_fopen(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let mode = emu.memory.cstr_lossy(arg(emu, 1), 16).hle();
    let (access, creation, append) = crt_file_open_mode(&mode);
    let h = open_compat_file(emu, &raw, access, creation);
    if h == INVALID_HANDLE_VALUE {
        ret(emu, 0);
        return HleResult::Retn(0);
    }
    if append {
        seek_crt_stream(emu, h, 0, 2);
    }
    ret(emu, h);
    HleResult::Retn(0)
}

// int fclose(FILE *stream)
// Close a CRT stream backed by an emulated file handle.
fn hle_crt_fclose(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let stream = arg(emu, 0);
    let value = if emu.hle.close_handle(stream) { 0 } else { -1i32 };
    ret(emu, value as u32);
    HleResult::Retn(0)
}

// size_t fread(void *dst, size_t size, size_t count, FILE *stream)
// Read complete items from a CRT stream backed by an emulated file handle.
fn hle_crt_fread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let size = arg(emu, 1);
    let count = arg(emu, 2);
    let stream = arg(emu, 3);
    let bytes = size.saturating_mul(count);
    if dst == 0 || size == 0 || count == 0 || bytes == 0 {
        ret(emu, 0);
        return HleResult::Retn(0);
    }
    let mut tmp = vec![0; bytes as usize];
    let read = match emu.hle.handle_mut(stream) {
        Some(Handle::File(file)) => match file.read(&mut tmp) {
            FileReadResult::Ready(result) => result.map_err(Error::Io).hle(),
            FileReadResult::Pending { key, offset, len } => {
                if len == 0 {
                    ret(emu, 0);
                    return HleResult::Retn(0);
                }
                let request_id = emu.hle.begin_vfs_read(&key, offset, len);
                return HleResult::Wait(HleWaitState::VfsRead {
                    request_id,
                    buf: dst,
                    read_out: 0,
                    ret_transferred: true,
                    ret_item_size: size,
                    fail_value: 0,
                    arg_bytes: 0,
                });
            }
        },
        _ => 0,
    };
    if read != 0 {
        emu.memory.write_bytes(dst, &tmp[..read]).hle();
    }
    ret(emu, (read as u32) / size);
    HleResult::Retn(0)
}

// int fseek(FILE *stream, long offset, int origin)
// Seek a CRT stream and return zero on success.
fn hle_crt_fseek(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let stream = arg(emu, 0);
    let offset = arg(emu, 1) as i32 as i64;
    let origin = arg(emu, 2);
    let ok = seek_crt_stream(emu, stream, offset, origin).is_some();
    ret(emu, if ok { 0 } else { -1i32 as u32 });
    HleResult::Retn(0)
}

// long ftell(FILE *stream)
// Return the current byte offset of a CRT stream.
fn hle_crt_ftell(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let stream = arg(emu, 0);
    let pos = seek_crt_stream(emu, stream, 0, 1).unwrap_or(-1);
    ret(emu, pos as u32);
    HleResult::Retn(0)
}

// char *strstr(const char *s, const char *needle)
// Return the first byte substring match in a NUL-terminated string.
fn hle_crt_strstr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = arg(emu, 0);
    let needle = arg(emu, 1);
    if s == 0 || needle == 0 {
        ret(emu, 0);
        return HleResult::Retn(0);
    }
    let haystack = read_c_bytes(&emu.memory, s, 1 << 20).hle();
    let needle_bytes = read_c_bytes(&emu.memory, needle, 1 << 20).hle();
    if needle_bytes.is_empty() {
        ret(emu, s);
        return HleResult::Retn(0);
    }
    let found = haystack
        .windows(needle_bytes.len())
        .position(|window| window == needle_bytes.as_slice())
        .map(|index| s + index as u32)
        .unwrap_or(0);
    ret(emu, found);
    HleResult::Retn(0)
}

// size_t wcslen(const wchar_t *s)
// Count UTF-16 code units before the NUL terminator.
fn hle_crt_wcslen(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, wcs_len(&emu.memory, arg(emu, 0), 1 << 20).hle() as u32);
    HleResult::Retn(0)
}

// wchar_t *wcscpy(wchar_t *dst, const wchar_t *src)
// Copy a NUL-terminated UTF-16 string and return dst.
fn hle_crt_wcscpy(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let len = wcs_len(&emu.memory, src, 1 << 20).hle();
    copy_wide_units(emu, dst, src, len + 1);
    ret(emu, dst);
    HleResult::Retn(0)
}

// wchar_t *wcscat(wchar_t *dst, const wchar_t *src)
// Append a NUL-terminated UTF-16 string and return dst.
fn hle_crt_wcscat(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let dst_len = wcs_len(&emu.memory, dst, 1 << 20).hle();
    let src_len = wcs_len(&emu.memory, src, 1 << 20).hle();
    copy_wide_units(emu, dst + (dst_len as u32 * 2), src, src_len + 1);
    ret(emu, dst);
    HleResult::Retn(0)
}

// int wcscmp(const wchar_t *a, const wchar_t *b)
// Compare two NUL-terminated UTF-16 strings.
fn hle_crt_wcscmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let lhs = emu.memory.utf16z_lossy(arg(emu, 0), 4096).hle();
    let rhs = emu.memory.utf16z_lossy(arg(emu, 1), 4096).hle();
    ret(emu, ord_to_crt(lhs.cmp(&rhs)) as u32);
    HleResult::Retn(0)
}

// int wcsncmp(const wchar_t *a, const wchar_t *b, size_t n)
// Compare up to n UTF-16 code units.
fn hle_crt_wcsncmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let n = arg(emu, 2) as usize;
    let lhs = read_wide_units(&emu.memory, arg(emu, 0), n).hle();
    let rhs = read_wide_units(&emu.memory, arg(emu, 1), n).hle();
    ret(emu, ord_to_crt(lhs.cmp(&rhs)) as u32);
    HleResult::Retn(0)
}

// int _wcsnicmp(const wchar_t *a, const wchar_t *b, size_t n)
// Compare up to n UTF-16 code units without ASCII case sensitivity.
fn hle_crt_wcsnicmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let n = arg(emu, 2) as usize;
    let mut lhs = read_wide_units(&emu.memory, arg(emu, 0), n).hle();
    let mut rhs = read_wide_units(&emu.memory, arg(emu, 1), n).hle();
    for unit in &mut lhs {
        *unit = ascii_upper_w(*unit);
    }
    for unit in &mut rhs {
        *unit = ascii_upper_w(*unit);
    }
    ret(emu, ord_to_crt(lhs.cmp(&rhs)) as u32);
    HleResult::Retn(0)
}

// int _wcsicmp(const wchar_t *a, const wchar_t *b)
// Compare UTF-16 strings without ASCII case sensitivity.
fn hle_crt_wcsicmp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let mut lhs = read_wide_units(&emu.memory, arg(emu, 0), 1 << 20).hle();
    let mut rhs = read_wide_units(&emu.memory, arg(emu, 1), 1 << 20).hle();
    for unit in &mut lhs {
        *unit = ascii_upper_w(*unit);
    }
    for unit in &mut rhs {
        *unit = ascii_upper_w(*unit);
    }
    ret(emu, ord_to_crt(lhs.cmp(&rhs)) as u32);
    HleResult::Retn(0)
}

// wchar_t *wcsncpy(wchar_t *dst, const wchar_t *src, size_t n)
// Copy at most n UTF-16 units, padding with NULs, and return dst.
fn hle_crt_wcsncpy(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let n = arg(emu, 2) as usize;
    let units = read_wide_units(&emu.memory, src, n).hle();
    for i in 0..n {
        let value = units.get(i).copied().unwrap_or(0);
        emu.memory
            .write_u16(dst + (i as u32 * 2), value)
            .hle();
    }
    ret(emu, dst);
    HleResult::Retn(0)
}

// wchar_t *wcsrchr(const wchar_t *s, wchar_t ch)
// Return the last matching UTF-16 code unit in a NUL-terminated string.
fn hle_crt_wcsrchr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = arg(emu, 0);
    let ch = (arg(emu, 1) & 0xffff) as u16;
    let len = wcs_len(&emu.memory, s, 1 << 20).hle();
    let mut found = if ch == 0 { s + (len as u32 * 2) } else { 0 };
    for i in 0..len {
        if emu.memory.read_u16(s + (i as u32 * 2)).hle() == ch {
            found = s + (i as u32 * 2);
        }
    }
    ret(emu, found);
    HleResult::Retn(0)
}

// wchar_t *wcschr(const wchar_t *s, wchar_t ch)
// Return the first matching UTF-16 code unit in a NUL-terminated string.
fn hle_crt_wcschr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = arg(emu, 0);
    let ch = (arg(emu, 1) & 0xffff) as u16;
    let len = wcs_len(&emu.memory, s, 1 << 20).hle();
    if ch == 0 {
        ret(emu, s + (len as u32 * 2));
        return HleResult::Retn(0);
    }
    for i in 0..len {
        if emu.memory.read_u16(s + (i as u32 * 2)).hle() == ch {
            ret(emu, s + (i as u32 * 2));
            return HleResult::Retn(0);
        }
    }
    ret(emu, 0);
    HleResult::Retn(0)
}

// wchar_t *wcsstr(const wchar_t *s, const wchar_t *needle)
// Return the first UTF-16 substring match in a NUL-terminated string.
fn hle_crt_wcsstr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = arg(emu, 0);
    let needle = arg(emu, 1);
    let haystack_units = read_wide_units(&emu.memory, s, 1 << 20).hle();
    let needle_units = read_wide_units(&emu.memory, needle, 1 << 20).hle();
    if needle_units.is_empty() {
        ret(emu, s);
        return HleResult::Retn(0);
    }
    let found = haystack_units
        .windows(needle_units.len())
        .position(|window| window == needle_units.as_slice())
        .map(|index| s + (index as u32 * 2))
        .unwrap_or(0);
    ret(emu, found);
    HleResult::Retn(0)
}

// wint_t towupper(wint_t ch)
// Uppercase ASCII UTF-16 code units and leave other values unchanged.
fn hle_crt_towupper(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, ascii_upper_w((arg(emu, 0) & 0xffff) as u16) as u32);
    HleResult::Retn(0)
}

// int _vsnwprintf(wchar_t *dst, size_t count, const wchar_t *fmt, va_list ap)
// Format common wide CRT integer/string/character conversions from a va_list.
fn hle_crt_vsnwprintf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let count = arg(emu, 1) as usize;
    let fmt = emu
        .memory
        .utf16z_lossy(arg(emu, 2), 4096)
        .unwrap_or_default();
    let ap = arg(emu, 3);
    let text = format_wide_va_list(emu, &fmt, VaSource::Memory { addr: ap }).hle();
    if dst != 0 && count != 0 {
        emu.memory.write_utf16z(dst, &text, count).hle();
    }
    ret(emu, text.encode_utf16().count() as u32);
    HleResult::Retn(0)
}

// int swscanf(const wchar_t *src, const wchar_t *fmt, ...)
// Scan common wide integer and floating-point conversions into guest pointers.
fn hle_crt_swscanf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = emu
        .memory
        .utf16z_lossy(arg(emu, 0), 4096)
        .unwrap_or_default();
    let fmt = emu
        .memory
        .utf16z_lossy(arg(emu, 1), 512)
        .unwrap_or_default();
    let assigned = scan_wide_input(emu, &src, &fmt, VaSource::Stack { next_word: 2 }).hle();
    ret(emu, assigned);
    HleResult::Retn(0)
}

// int sscanf(const char *src, const char *fmt, ...)
// Scan common ANSI integer, floating-point, string, and character conversions.
fn hle_crt_sscanf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = emu.memory.cstr_lossy(arg(emu, 0), 4096).unwrap_or_default();
    let fmt = emu.memory.cstr_lossy(arg(emu, 1), 512).unwrap_or_default();
    let assigned = scan_ansi_input(emu, &src, &fmt, VaSource::Stack { next_word: 2 }).hle();
    ret(emu, assigned);
    HleResult::Retn(0)
}

// int _finite(double x)
// Return whether the double argument is finite.
fn hle_crt_finite(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg_f64(emu, 0).is_finite() as u32);
    HleResult::Retn(0)
}

// int _isnan(double x)
// Return whether the double argument is NaN.
fn hle_crt_isnan(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg_f64(emu, 0).is_nan() as u32);
    HleResult::Retn(0)
}

// long _ftol(void)
// Pop ST0, truncate it to a signed integer, and return the low 32 bits in EAX.
fn hle_crt_ftol(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = emu.cpu.pop_x87_trunc_i64();
    ret(emu, value as u32);
    emu.cpu.set_reg(Reg::Edx, (value >> 32) as u32);
    HleResult::Retn(0)
}

// double _CIacos(void)
// Replace ST0 with acos(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_acos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::acos);
    HleResult::Retn(0)
}

// double _CIasin(void)
// Replace ST0 with asin(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_asin(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::asin);
    HleResult::Retn(0)
}

// double _CIatan(void)
// Replace ST0 with atan(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_atan(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::atan);
    HleResult::Retn(0)
}

// double _CIcos(void)
// Replace ST0 with cos(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_cos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::cos);
    HleResult::Retn(0)
}

// double _CIexp(void)
// Replace ST0 with exp(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_exp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::exp);
    HleResult::Retn(0)
}

// double _CIlog(void)
// Replace ST0 with log(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_log(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::ln);
    HleResult::Retn(0)
}

// double _CIlog10(void)
// Replace ST0 with log10(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_log10(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::log10);
    HleResult::Retn(0)
}

// double _CIsin(void)
// Replace ST0 with sin(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_sin(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::sin);
    HleResult::Retn(0)
}

// double _CIsqrt(void)
// Replace ST0 with sqrt(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_sqrt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::sqrt);
    HleResult::Retn(0)
}

// double _CItan(void)
// Replace ST0 with tan(ST0) for MSVC intrinsic math calls.
fn hle_crt_ci_tan(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.cpu.map_x87_top(f64::tan);
    HleResult::Retn(0)
}

// double acos(double x)
// Return acos(x) on the x87 stack.
fn hle_crt_acos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).acos());
    HleResult::Retn(0)
}

// double asin(double x)
// Return asin(x) on the x87 stack.
fn hle_crt_asin(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).asin());
    HleResult::Retn(0)
}

// double atan(double x)
// Return atan(x) on the x87 stack.
fn hle_crt_atan(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).atan());
    HleResult::Retn(0)
}

// double cos(double x)
// Return cos(x) on the x87 stack.
fn hle_crt_cos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).cos());
    HleResult::Retn(0)
}

// double cosh(double x)
// Return cosh(x) on the x87 stack.
fn hle_crt_cosh(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).cosh());
    HleResult::Retn(0)
}

// double ceil(double x)
// Return ceil(x) on the x87 stack.
fn hle_crt_ceil(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).ceil());
    HleResult::Retn(0)
}

// double exp(double x)
// Return exp(x) on the x87 stack.
fn hle_crt_exp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).exp());
    HleResult::Retn(0)
}

// double floor(double x)
// Return floor(x) on the x87 stack.
fn hle_crt_floor(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).floor());
    HleResult::Retn(0)
}

// double fmod(double x, double y)
// Return x modulo y on the x87 stack.
fn hle_crt_fmod(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0) % arg_f64(emu, 2));
    HleResult::Retn(0)
}

// double log(double x)
// Return natural log(x) on the x87 stack.
fn hle_crt_log(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).ln());
    HleResult::Retn(0)
}

// double log10(double x)
// Return log10(x) on the x87 stack.
fn hle_crt_log10(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).log10());
    HleResult::Retn(0)
}

// double modf(double x, double *iptr)
// Store the integer part and return the fractional part on the x87 stack.
fn hle_crt_modf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = arg_f64(emu, 0);
    let out = arg(emu, 2);
    let int = value.trunc();
    emu.memory.write_bytes(out, &int.to_le_bytes()).hle();
    ret_f64(emu, value - int);
    HleResult::Retn(0)
}

// double pow(double x, double y)
// Return x raised to y on the x87 stack.
fn hle_crt_pow(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).powf(arg_f64(emu, 2)));
    HleResult::Retn(0)
}

// double sin(double x)
// Return sin(x) on the x87 stack.
fn hle_crt_sin(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).sin());
    HleResult::Retn(0)
}

// double sinh(double x)
// Return sinh(x) on the x87 stack.
fn hle_crt_sinh(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).sinh());
    HleResult::Retn(0)
}

// double sqrt(double x)
// Return sqrt(x) on the x87 stack.
fn hle_crt_sqrt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).sqrt());
    HleResult::Retn(0)
}

// double tan(double x)
// Return tan(x) on the x87 stack.
fn hle_crt_tan(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).tan());
    HleResult::Retn(0)
}

// double tanh(double x)
// Return tanh(x) on the x87 stack.
fn hle_crt_tanh(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret_f64(emu, arg_f64(emu, 0).tanh());
    HleResult::Retn(0)
}

// char *_itoa(int value, char *dst, int radix)
// Convert a signed integer to an ANSI string and return dst.
fn hle_crt_itoa(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = arg(emu, 0) as i32;
    let dst = arg(emu, 1);
    let radix = arg(emu, 2);
    let text = format_radix_i32(value, radix);
    emu.memory.write_cstr(dst, &text, text.len() + 1).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// char *_ltoa(long value, char *dst, int radix)
// Convert a signed long to an ANSI string and return dst.
fn hle_crt_ltoa(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = arg(emu, 0) as i32;
    let dst = arg(emu, 1);
    let radix = arg(emu, 2);
    let text = format_radix_i32(value, radix);
    emu.memory.write_cstr(dst, &text, text.len() + 1).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// char *_ultoa(unsigned long value, char *dst, int radix)
// Convert an unsigned long to an ANSI string and return dst.
fn hle_crt_ultoa(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = arg(emu, 0);
    let dst = arg(emu, 1);
    let radix = arg(emu, 2);
    let text = format_radix_u32(value, radix);
    emu.memory.write_cstr(dst, &text, text.len() + 1).hle();
    ret(emu, dst);
    HleResult::Retn(0)
}

// int atoi(const char *s)
// Parse a guest decimal string into a signed integer.
fn hle_crt_atoi(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = emu.memory.cstr_lossy(arg(emu, 0), 128).hle();
    let value = parse_c_decimal_i32_prefix(&s);
    ret(emu, value as u32);
    HleResult::Retn(0)
}

// long atol(const char *s)
// Parse a guest decimal string into a signed long.
fn hle_crt_atol(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = emu.memory.cstr_lossy(arg(emu, 0), 128).hle();
    let value = parse_c_decimal_i32_prefix(&s);
    ret(emu, value as u32);
    HleResult::Retn(0)
}

// time_t time(time_t *out)
// Return deterministic fake time and optionally store it.
fn hle_crt_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    emu.hle.fake_time = emu.hle.fake_time.wrapping_add(3);
    if out != 0 {
        emu.memory.write_u32(out, emu.hle.fake_time).hle();
    }
    ret(emu, emu.hle.fake_time);
    HleResult::Retn(0)
}

// void srand(unsigned seed)
// Seed the deterministic CRT pseudo-random generator.
fn hle_crt_srand(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.rand_seed = arg(emu, 0);
    ret(emu, 0);
    HleResult::Retn(0)
}

// int rand(void)
// Return the next MSVCRT-style 15-bit pseudo-random value.
fn hle_crt_rand(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.rand_seed = emu
        .hle
        .rand_seed
        .wrapping_mul(214013)
        .wrapping_add(2531011);
    ret(emu, (emu.hle.rand_seed >> 16) & 0x7fff);
    HleResult::Retn(0)
}

// int printf(const char *fmt, ...)
// Append formatted CRT output using the limited formatter.
fn hle_crt_printf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let fmt = arg(emu, 0);
    let text = format_c_output(emu, fmt, VaSource::Stack { next_word: 1 }).hle();
    ret(emu, text.len() as u32);
    emu.hle.crt_output.push_str(&text);
    HleResult::Retn(0)
}

// int sprintf(char *dst, const char *fmt, ...)
// Format CRT output into a guest ANSI buffer.
fn hle_crt_sprintf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let fmt = arg(emu, 1);
    let text = format_c_output(emu, fmt, VaSource::Stack { next_word: 2 }).hle();
    if dst != 0 {
        emu.memory.write_cstr(dst, &text, text.len() + 1).hle();
    }
    ret(emu, text.len() as u32);
    HleResult::Retn(0)
}

// int vsprintf(char *dst, const char *fmt, va_list args)
// Format CRT output into a guest ANSI buffer from a guest va_list.
fn hle_crt_vsprintf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let fmt = arg(emu, 1);
    let args = arg(emu, 2);
    let text = format_c_output(emu, fmt, VaSource::Memory { addr: args }).hle();
    if dst != 0 {
        emu.memory.write_cstr(dst, &text, text.len() + 1).hle();
    }
    ret(emu, text.len() as u32);
    HleResult::Retn(0)
}

// int fprintf(FILE *stream, const char *fmt, ...)
// Append formatted CRT output while ignoring the fake stream.
fn hle_crt_fprintf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let stream = arg(emu, 0);
    let fmt = arg(emu, 1);
    let text = format_c_output(emu, fmt, VaSource::Stack { next_word: 2 }).hle();
    ret(emu, text.len() as u32);
    write_crt_stream(emu, stream, text.as_bytes());
    HleResult::Retn(0)
}

// int vfprintf(FILE *stream, const char *fmt, va_list ap)
// Append formatted CRT output from a guest va_list.
fn hle_crt_vfprintf(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let stream = arg(emu, 0);
    let fmt = arg(emu, 1);
    let args = arg(emu, 2);
    let text = format_c_output(emu, fmt, VaSource::Memory { addr: args }).hle();
    ret(emu, text.len() as u32);
    write_crt_stream(emu, stream, text.as_bytes());
    HleResult::Retn(0)
}

// int puts(const char *s)
// Append a guest string plus newline to CRT output.
fn hle_crt_puts(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let s = arg(emu, 0);
    let mut text = emu.memory.cstr_lossy(s, 4096).unwrap_or_default();
    text.push('\n');
    ret(emu, text.len() as u32);
    emu.hle.crt_output.push_str(&text);
    HleResult::Retn(0)
}

// int putchar(int ch)
// Append one byte to CRT output and return it.
fn hle_crt_putchar(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ch = (arg(emu, 0) & 0xff) as u8 as char;
    emu.hle.crt_output.push(ch);
    ret(emu, ch as u32);
    HleResult::Retn(0)
}

// int fputc(int ch, FILE *stream)
// Append one byte to CRT output and ignore the fake stream.
fn hle_crt_fputc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ch = (arg(emu, 0) & 0xff) as u8;
    let stream = arg(emu, 1);
    write_crt_stream(emu, stream, &[ch]);
    ret(emu, ch as u32);
    HleResult::Retn(0)
}

// size_t fwrite(const void *ptr, size_t size, size_t count, FILE *stream)
// Append raw bytes to CRT output and return the written item count.
fn hle_crt_fwrite(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    let size = arg(emu, 1);
    let count = arg(emu, 2);
    let stream = arg(emu, 3);
    let bytes = size.saturating_mul(count);
    if ptr != 0 && bytes != 0 {
        let data = emu.memory.read_bytes(ptr, bytes as usize).hle();
        match emu.hle.handle_mut(stream) {
            Some(Handle::File(file)) => match file.write(data) {
                FileWriteResult::Ready(result) => {
                    let written = result.map_err(Error::Io).hle();
                    ret(emu, (written as u32) / size);
                    return HleResult::Retn(0);
                }
                FileWriteResult::Pending { key, offset, data } => {
                    emu.hle.note_async_vfs_write(&key, offset, data.len());
                    let request_id = emu.hle.begin_vfs_write(&key, offset, data);
                    return HleResult::Wait(HleWaitState::VfsWrite {
                        request_id,
                        written_out: 0,
                        ret_transferred: true,
                        ret_item_size: size,
                        fail_value: 0,
                        arg_bytes: 0,
                    });
                }
            },
            _ => {
                write_crt_stream(emu, stream, &data);
            }
        }
    }
    ret(emu, count);
    HleResult::Retn(0)
}

enum VaSource {
    Stack { next_word: u32 },
    Memory { addr: u32 },
}

fn arg_f64(emu: &Emulator, word_index: u32) -> f64 {
    let lo = arg(emu, word_index) as u64;
    let hi = arg(emu, word_index + 1) as u64;
    f64::from_bits(lo | (hi << 32))
}

fn ret_f64(emu: &mut Emulator, value: f64) {
    emu.cpu.push_x87_return(value);
}

impl VaSource {
    fn next_u32(&mut self, emu: &Emulator) -> Result<u32> {
        match self {
            VaSource::Stack { next_word } => {
                let value = arg(emu, *next_word);
                *next_word += 1;
                Ok(value)
            }
            VaSource::Memory { addr } => {
                let value = emu.memory.read_u32(*addr)?;
                *addr = (*addr).wrapping_add(4);
                Ok(value)
            }
        }
    }

    fn next_f64(&mut self, emu: &Emulator) -> Result<f64> {
        let lo = self.next_u32(emu)? as u64;
        let hi = self.next_u32(emu)? as u64;
        Ok(f64::from_bits(lo | (hi << 32)))
    }
}

fn format_c_output(emu: &Emulator, fmt_addr: u32, mut args: VaSource) -> Result<String> {
    let fmt = emu.memory.cstr_lossy(fmt_addr, 4096)?;
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }

        let mut left = false;
        let mut zero = false;
        loop {
            match chars.peek().copied() {
                Some('-') => {
                    left = true;
                    chars.next();
                }
                Some('0') => {
                    zero = true;
                    chars.next();
                }
                Some('+') | Some(' ') | Some('#') => {
                    chars.next();
                }
                _ => break,
            }
        }

        let width = parse_decimal(&mut chars);
        let precision = if chars.peek() == Some(&'.') {
            chars.next();
            Some(parse_decimal(&mut chars).unwrap_or(0))
        } else {
            None
        };
        while matches!(chars.peek(), Some('h') | Some('l') | Some('L')) {
            chars.next();
        }

        let Some(spec) = chars.next() else {
            break;
        };
        let piece = match spec {
            'd' | 'i' => (args.next_u32(emu)? as i32).to_string(),
            'u' => args.next_u32(emu)?.to_string(),
            'x' => format!("{:x}", args.next_u32(emu)?),
            'X' => format!("{:X}", args.next_u32(emu)?),
            'p' => format!("{:08x}", args.next_u32(emu)?),
            'c' => ((args.next_u32(emu)? & 0xff) as u8 as char).to_string(),
            's' => {
                let ptr = args.next_u32(emu)?;
                let mut s = emu.memory.cstr_lossy(ptr, 4096)?;
                if let Some(limit) = precision {
                    s.truncate(limit);
                }
                s
            }
            'f' | 'F' => {
                let precision = precision.unwrap_or(6);
                format!("{:.*}", precision, args.next_f64(emu)?)
            }
            other => {
                let mut s = String::from("%");
                s.push(other);
                s
            }
        };
        out.push_str(&apply_width(piece, width, left, zero));
    }
    Ok(out)
}

fn format_wide_va_list(emu: &Emulator, fmt: &str, mut args: VaSource) -> Result<String> {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }

        let mut left = false;
        let mut zero = false;
        let mut plus = false;
        loop {
            match chars.peek().copied() {
                Some('-') => {
                    left = true;
                    chars.next();
                }
                Some('0') => {
                    zero = true;
                    chars.next();
                }
                Some('+') => {
                    plus = true;
                    chars.next();
                }
                Some(' ') | Some('#') => {
                    chars.next();
                }
                _ => break,
            }
        }

        let width = parse_printf_width(&mut chars, &mut args, emu)?;
        let precision = if chars.peek() == Some(&'.') {
            chars.next();
            parse_printf_width(&mut chars, &mut args, emu)?.or(Some(0))
        } else {
            None
        };
        while matches!(chars.peek(), Some('h') | Some('l') | Some('L')) {
            chars.next();
        }

        let Some(spec) = chars.next() else {
            break;
        };
        let piece = match spec {
            'd' | 'i' => {
                let value = args.next_u32(emu)? as i32;
                if plus && value >= 0 {
                    format!("+{value}")
                } else {
                    value.to_string()
                }
            }
            'u' => args.next_u32(emu)?.to_string(),
            'x' => format!("{:x}", args.next_u32(emu)?),
            'X' => format!("{:X}", args.next_u32(emu)?),
            'p' => format!("{:08x}", args.next_u32(emu)?),
            'c' | 'C' => {
                let unit = (args.next_u32(emu)? & 0xffff) as u16;
                char::from_u32(unit as u32).unwrap_or('?').to_string()
            }
            's' => {
                let ptr = args.next_u32(emu)?;
                let mut s = emu.memory.utf16z_lossy(ptr, 4096)?;
                if let Some(limit) = precision {
                    s.truncate(limit);
                }
                s
            }
            'S' => {
                let ptr = args.next_u32(emu)?;
                let mut s = emu.memory.cstr_lossy(ptr, 4096)?;
                if let Some(limit) = precision {
                    s.truncate(limit);
                }
                s
            }
            'f' | 'F' => {
                let precision = precision.unwrap_or(6);
                format!("{:.*}", precision, args.next_f64(emu)?)
            }
            other => {
                let mut s = String::from("%");
                s.push(other);
                s
            }
        };
        out.push_str(&apply_width(piece, width, left, zero));
    }
    Ok(out)
}

fn parse_printf_width<I>(
    chars: &mut std::iter::Peekable<I>,
    args: &mut VaSource,
    emu: &Emulator,
) -> Result<Option<usize>>
where
    I: Iterator<Item = char>,
{
    if chars.peek() == Some(&'*') {
        chars.next();
        return Ok(Some(args.next_u32(emu)? as usize));
    }
    Ok(parse_decimal(chars))
}

fn scan_wide_input(
    emu: &mut Emulator,
    src: &str,
    fmt: &str,
    mut args: VaSource,
) -> Result<u32> {
    let mut input = src.trim_start();
    let mut assigned = 0u32;
    let mut chars = fmt.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            input = input.trim_start();
            continue;
        }
        if ch != '%' {
            if input.starts_with(ch) {
                input = &input[ch.len_utf8()..];
                continue;
            }
            break;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            if input.starts_with('%') {
                input = &input[1..];
                continue;
            }
            break;
        }
        while matches!(chars.peek(), Some('I' | '6' | '4' | 'h' | 'l' | 'L')) {
            chars.next();
        }
        let Some(spec) = chars.next() else {
            break;
        };
        input = input.trim_start();
        let token_len = scan_token_len(input);
        if token_len == 0 {
            break;
        }
        let token = &input[..token_len];
        let out = args.next_u32(emu)?;
        let ok = match spec {
            'd' | 'i' => {
                if let Ok(value) = token.parse::<i32>() {
                    emu.memory.write_u32(out, value as u32)?;
                    true
                } else {
                    false
                }
            }
            'u' => {
                if let Ok(value) = token.parse::<u32>() {
                    emu.memory.write_u32(out, value)?;
                    true
                } else {
                    false
                }
            }
            'x' | 'X' => parse_u64_radix(token, 16)
                .map(|value| write_guest_u64(emu, out, value))
                .transpose()?
                .is_some(),
            'o' => parse_u64_radix(token, 8)
                .map(|value| write_guest_u64(emu, out, value))
                .transpose()?
                .is_some(),
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                if let Ok(value) = token.parse::<f64>() {
                    emu.memory.write_bytes(out, &value.to_le_bytes())?;
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if !ok {
            break;
        }
        assigned = assigned.saturating_add(1);
        input = &input[token_len..];
    }
    Ok(assigned)
}

fn scan_ansi_input(
    emu: &mut Emulator,
    src: &str,
    fmt: &str,
    mut args: VaSource,
) -> Result<u32> {
    let mut input = src;
    let mut assigned = 0u32;
    let mut chars = fmt.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            input = input.trim_start();
            continue;
        }
        if ch != '%' {
            if input.starts_with(ch) {
                input = &input[ch.len_utf8()..];
                continue;
            }
            break;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            if input.starts_with('%') {
                input = &input[1..];
                continue;
            }
            break;
        }

        let suppressed = if chars.peek() == Some(&'*') {
            chars.next();
            true
        } else {
            false
        };
        let width = parse_decimal(&mut chars);
        let mut long_count = 0;
        while matches!(chars.peek(), Some('h' | 'l' | 'L')) {
            if chars.peek() == Some(&'l') {
                long_count += 1;
            }
            chars.next();
        }
        let Some(spec) = chars.next() else {
            break;
        };

        if spec != 'c' && spec != '[' {
            input = input.trim_start();
        }
        let out = if suppressed { 0 } else { args.next_u32(emu)? };
        let consumed = match spec {
            'd' | 'i' | 'u' | 'x' | 'X' | 'o' => {
                let radix = match spec {
                    'x' | 'X' => 16,
                    'o' => 8,
                    _ => 10,
                };
                let Some((token, len)) = scan_int_prefix(input, width, radix, spec == 'u') else {
                    break;
                };
                if !suppressed {
                    let value = if spec == 'u' {
                        token.parse::<u32>().map(|v| v as i64).ok()
                    } else if spec == 'x' || spec == 'X' || spec == 'o' {
                        parse_u64_radix(token, radix).map(|v| v as i64)
                    } else {
                        token.parse::<i32>().map(|v| v as i64).ok()
                    };
                    let Some(value) = value else {
                        break;
                    };
                    emu.memory.write_u32(out, value as u32)?;
                    assigned = assigned.saturating_add(1);
                }
                len
            }
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                let Some((token, len)) = scan_float_prefix(input, width) else {
                    break;
                };
                let Ok(value) = token.parse::<f64>() else {
                    break;
                };
                if !suppressed {
                    if long_count != 0 || spec == 'F' {
                        emu.memory.write_bytes(out, &value.to_le_bytes())?;
                    } else {
                        emu.memory.write_bytes(out, &(value as f32).to_le_bytes())?;
                    }
                    assigned = assigned.saturating_add(1);
                }
                len
            }
            's' => {
                let len = scan_token_len(input).min(width.unwrap_or(usize::MAX));
                if len == 0 {
                    break;
                }
                if !suppressed {
                    emu.memory.write_bytes(out, input[..len].as_bytes())?;
                    emu.memory.write_u8(out + len as u32, 0)?;
                    assigned = assigned.saturating_add(1);
                }
                len
            }
            'c' => {
                let len = width.unwrap_or(1).min(input.len());
                if len == 0 {
                    break;
                }
                if !suppressed {
                    emu.memory.write_bytes(out, input[..len].as_bytes())?;
                    assigned = assigned.saturating_add(1);
                }
                len
            }
            _ => break,
        };
        input = &input[consumed..];
    }
    Ok(assigned)
}

fn scan_token_len(input: &str) -> usize {
    input
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(input.len())
}

fn scan_int_prefix(
    input: &str,
    width: Option<usize>,
    radix: u32,
    unsigned: bool,
) -> Option<(&str, usize)> {
    let limit = width.unwrap_or(input.len()).min(input.len());
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    if !unsigned && pos < limit && matches!(bytes[pos], b'+' | b'-') {
        pos += 1;
    }
    if radix == 16 && pos + 1 < limit && bytes[pos] == b'0' && matches!(bytes[pos + 1], b'x' | b'X') {
        pos += 2;
    }
    let digits_start = pos;
    while pos < limit && (bytes[pos] as char).is_digit(radix) {
        pos += 1;
    }
    (pos > digits_start).then_some((&input[..pos], pos))
}

fn scan_float_prefix(input: &str, width: Option<usize>) -> Option<(&str, usize)> {
    let limit = width.unwrap_or(input.len()).min(input.len());
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    if pos < limit && matches!(bytes[pos], b'+' | b'-') {
        pos += 1;
    }
    let mut saw_digit = false;
    while pos < limit && bytes[pos].is_ascii_digit() {
        saw_digit = true;
        pos += 1;
    }
    if pos < limit && bytes[pos] == b'.' {
        pos += 1;
        while pos < limit && bytes[pos].is_ascii_digit() {
            saw_digit = true;
            pos += 1;
        }
    }
    if saw_digit && pos < limit && matches!(bytes[pos], b'e' | b'E') {
        let exp = pos;
        pos += 1;
        if pos < limit && matches!(bytes[pos], b'+' | b'-') {
            pos += 1;
        }
        let exp_digits = pos;
        while pos < limit && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        if pos == exp_digits {
            pos = exp;
        }
    }
    saw_digit.then_some((&input[..pos], pos))
}

fn parse_u64_radix(token: &str, radix: u32) -> Option<u64> {
    let token = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
        .unwrap_or(token);
    u64::from_str_radix(token, radix).ok()
}

fn write_guest_u64(emu: &mut Emulator, out: u32, value: u64) -> Result<()> {
    emu.memory.write_u32(out, value as u32)?;
    emu.memory.write_u32(out + 4, (value >> 32) as u32)?;
    Ok(())
}

fn parse_decimal<I>(chars: &mut std::iter::Peekable<I>) -> Option<usize>
where
    I: Iterator<Item = char>,
{
    let mut value = 0usize;
    let mut seen = false;
    while let Some(ch) = chars.peek().copied() {
        let Some(digit) = ch.to_digit(10) else {
            break;
        };
        seen = true;
        value = value.saturating_mul(10).saturating_add(digit as usize);
        chars.next();
    }
    seen.then_some(value)
}

fn apply_width(mut text: String, width: Option<usize>, left: bool, zero: bool) -> String {
    let Some(width) = width else {
        return text;
    };
    let len = text.chars().count();
    if len >= width {
        return text;
    }
    let pad = std::iter::repeat(if zero { '0' } else { ' ' })
        .take(width - len)
        .collect::<String>();
    if left {
        text.push_str(&pad);
        text
    } else {
        format!("{pad}{text}")
    }
}

fn c_strlen(mem: &Memory, addr: u32, max_len: usize) -> Result<usize> {
    for i in 0..max_len {
        if mem.read_u8(addr.wrapping_add(i as u32))? == 0 {
            return Ok(i);
        }
    }
    Ok(max_len)
}

fn read_c_bytes(mem: &Memory, addr: u32, max_len: usize) -> Result<Vec<u8>> {
    let len = c_strlen(mem, addr, max_len)?;
    mem.read_bytes(addr, len)
}

fn ascii_casecmp(lhs: &[u8], rhs: &[u8]) -> i32 {
    for (a, b) in lhs
        .iter()
        .map(u8::to_ascii_lowercase)
        .zip(rhs.iter().map(u8::to_ascii_lowercase))
    {
        if a != b {
            return a as i32 - b as i32;
        }
    }
    lhs.len() as i32 - rhs.len() as i32
}

fn crt_file_open_mode(mode: &str) -> (u32, u32, bool) {
    let read_write = mode.contains('+');
    let access = if read_write {
        0xc000_0000
    } else if mode.starts_with('w') || mode.starts_with('a') {
        0x4000_0000
    } else {
        0x8000_0000
    };
    let creation = if mode.starts_with('w') {
        2
    } else if mode.starts_with('a') {
        4
    } else {
        3
    };
    (access, creation, mode.starts_with('a'))
}

fn write_crt_stream(emu: &mut Emulator, stream: u32, data: &[u8]) -> usize {
    match emu.hle.handle_mut(stream) {
        Some(Handle::File(file)) => file.write_sync(data).unwrap_or(0),
        _ => {
            emu.hle.crt_output.push_str(&String::from_utf8_lossy(data));
            data.len()
        }
    }
}

fn seek_crt_stream(emu: &mut Emulator, stream: u32, offset: i64, origin: u32) -> Option<i64> {
    match emu.hle.handle_mut(stream) {
        Some(Handle::File(file)) => Some(file.seek(offset, origin).map_err(Error::Io).hle() as i64),
        _ => None,
    }
}

fn format_radix_i32(value: i32, radix: u32) -> String {
    if radix == 10 && value < 0 {
        format!("-{}", format_radix_u32(value.unsigned_abs(), radix))
    } else {
        format_radix_u32(value as u32, radix)
    }
}

fn parse_c_decimal_i32_prefix(s: &str) -> i32 {
    let bytes = s.as_bytes();
    let mut index = 0;
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }

    let negative = match bytes.get(index).copied() {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };

    let limit = if negative {
        i32::MAX as i64 + 1
    } else {
        i32::MAX as i64
    };
    let mut value = 0i64;
    let mut has_digit = false;
    while let Some(byte) = bytes.get(index).copied() {
        if !byte.is_ascii_digit() {
            break;
        }
        has_digit = true;
        value = value
            .saturating_mul(10)
            .saturating_add((byte - b'0') as i64)
            .min(limit);
        index += 1;
    }

    if !has_digit {
        return 0;
    }
    if negative {
        if value == limit {
            i32::MIN
        } else {
            -(value as i32)
        }
    } else {
        value as i32
    }
}

fn format_radix_u32(mut value: u32, radix: u32) -> String {
    if !(2..=36).contains(&radix) {
        return String::new();
    }
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while value != 0 {
        let digit = (value % radix) as u8;
        digits.push(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + (digit - 10)
        });
        value /= radix;
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

fn wcs_len(mem: &Memory, addr: u32, max_len: usize) -> Result<usize> {
    if addr == 0 {
        return Ok(0);
    }
    for i in 0..max_len {
        if mem.read_u16(addr.wrapping_add((i * 2) as u32))? == 0 {
            return Ok(i);
        }
    }
    Ok(max_len)
}

fn read_wide_units(mem: &Memory, addr: u32, max_len: usize) -> Result<Vec<u16>> {
    let mut out = Vec::new();
    if addr == 0 {
        return Ok(out);
    }
    for i in 0..max_len {
        let value = mem.read_u16(addr.wrapping_add((i * 2) as u32))?;
        if value == 0 {
            break;
        }
        out.push(value);
    }
    Ok(out)
}

fn copy_wide_units(emu: &mut Emulator, dst: u32, src: u32, count: usize) {
    for i in 0..count {
        let value = emu.memory.read_u16(src.wrapping_add((i * 2) as u32)).hle();
        emu.memory
            .write_u16(dst.wrapping_add((i * 2) as u32), value)
            .hle();
    }
}

fn ascii_upper_w(unit: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&unit) {
        unit - 32
    } else {
        unit
    }
}

fn ord_to_crt(ord: std::cmp::Ordering) -> i32 {
    match ord {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

#[cfg(test)]
mod msvcrt_tests {
    use super::parse_c_decimal_i32_prefix;

    #[test]
    fn atoi_accepts_c_numeric_prefix() {
        assert_eq!(parse_c_decimal_i32_prefix("90 left flipper"), 90);
        assert_eq!(parse_c_decimal_i32_prefix(" \t-12px"), -12);
        assert_eq!(parse_c_decimal_i32_prefix("not a number"), 0);
    }
}
