const DEFAULT_FILETIME: u64 = 133_485_408_000_000_000;
const TICKS_PER_MILLISECOND: u64 = 10_000;
const TICKS_PER_SECOND: u64 = 10_000_000;
const SECONDS_PER_DAY: u64 = 86_400;
const DAYS_1601_TO_1970: i64 = 134_774;

// BOOL AreFileApisANSI(void)
// Report ANSI file APIs; OEM mode is not distinguished by the HLE path layer.
fn hle_are_file_apis_ansi(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(0)
}

// void SetFileApisToANSI(void)
// Keep file APIs in the default ANSI-compatible mode.
fn hle_set_file_apis_to_ansi(_: &mut Emulator, _: &HleEntry) -> HleResult {
    HleResult::Retn(0)
}

// void SetFileApisToOEM(void)
// Accept OEM mode requests without changing path decoding yet.
fn hle_set_file_apis_to_oem(_: &mut Emulator, _: &HleEntry) -> HleResult {
    HleResult::Retn(0)
}

// HANDLE CreateFileA(LPCSTR name, DWORD access, DWORD share, void *sec, DWORD create, DWORD flags, HANDLE template)
// Translate the guest path and open a host file handle.
fn hle_create_file_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = arg(emu, 0);
    let raw_name = emu.memory.cstr_lossy(name, 1024).unwrap_or_default();
    let access = arg(emu, 1);
    let creation = arg(emu, 4);
    create_file_impl(emu, "CreateFileA", &raw_name, access, creation)
}

fn create_file_impl(
    emu: &mut Emulator,
    api_name: &str,
    raw_name: &str,
    access: u32,
    creation: u32,
) -> HleResult {
    match emu.hle.open_file_handle(raw_name, access, creation) {
        FileOpen::Opened(h) => {
            trace_fs!(
                "{api_name} name={raw_name:?} access={access:08x} create={creation} -> handle={h:08x}"
            );
            if emu.should_trace() {
                eprintln!("{api_name} {raw_name:?} -> {h:08x}");
            }
            ret(emu, h);
        }
        FileOpen::Failed(last_error) => {
            emu.hle.last_error = last_error;
            trace_fs!(
                "{api_name} name={raw_name:?} access={access:08x} create={creation} -> failed last_error={last_error}"
            );
            ret(emu, INVALID_HANDLE_VALUE);
        }
    }
    HleResult::Retn(28)
}

// BOOL ReadFile(HANDLE h, void *buf, DWORD len, DWORD *read, void *overlapped)
// Read host file bytes into guest memory and report the byte count.
fn hle_read_file(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let buf = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let read_out = arg(emu, 3);
    let mut tmp = vec![0; len];
    let read = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => match file.read(&mut tmp) {
            FileReadResult::Ready(result) => result.map_err(Error::Io).hle(),
            FileReadResult::Pending { key, offset, len } => {
                if len == 0 {
                    if read_out != 0 {
                        emu.memory.write_u32(read_out, 0).hle();
                    }
                    ret(emu, 1);
                    return HleResult::Retn(20);
                }
                let request_id = emu.hle.begin_vfs_read(&key, offset, len);
                return HleResult::Wait(HleWaitState::VfsRead {
                    request_id,
                    buf,
                    read_out,
                    ret_transferred: false,
                    ret_item_size: 1,
                    fail_value: 0,
                    arg_bytes: 20,
                });
            }
        },
        _ => {
            emu.hle.last_error = 6;
            ret(emu, 0);
            return HleResult::Retn(20);
        }
    };
    emu.memory.write_bytes(buf, &tmp[..read]).hle();
    if read_out != 0 {
        emu.memory.write_u32(read_out, read as u32).hle();
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// BOOL WriteFile(HANDLE h, const void *buf, DWORD len, DWORD *written, void *overlapped)
// Write guest bytes to a host file or accept writes to fake handles.
fn hle_write_file(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let buf = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let written_out = arg(emu, 3);
    let data = emu.memory.read_bytes(buf, len).hle();
    let written = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => match file.write(data) {
            FileWriteResult::Ready(Ok(written)) => written,
            FileWriteResult::Ready(Err(err)) => {
                emu.hle.last_error = if err.kind() == std::io::ErrorKind::PermissionDenied {
                    5
                } else {
                    6
                };
                ret(emu, 0);
                return HleResult::Retn(20);
            }
            FileWriteResult::Pending { key, offset, data } => {
                emu.hle.note_async_vfs_write(&key, offset, data.len());
                let request_id = emu.hle.begin_vfs_write(&key, offset, data);
                return HleResult::Wait(HleWaitState::VfsWrite {
                    request_id,
                    written_out,
                    ret_transferred: false,
                    ret_item_size: 1,
                    fail_value: 0,
                    arg_bytes: 20,
                });
            }
        },
        _ => len,
    };
    if written_out != 0 {
        emu.memory.write_u32(written_out, written as u32).hle();
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// BOOL WriteConsoleW(HANDLE out, const WCHAR *buf, DWORD chars, DWORD *written, void *reserved)
// Append UTF-16 console text to the captured CRT output buffer.
fn hle_write_console_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let buf = arg(emu, 1);
    let chars = arg(emu, 2);
    let written_out = arg(emu, 3);
    let text = read_wide_counted(emu, buf, chars);
    emu.hle.crt_output.push_str(&text);
    if written_out != 0 {
        emu.memory.write_u32(written_out, chars).hle();
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// BOOL ReadConsoleW(HANDLE in, WCHAR *buf, DWORD chars, DWORD *read, void *reserved)
// Report end-of-input for console reads without blocking the emulator.
fn hle_read_console_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let read_out = arg(emu, 3);
    if read_out != 0 {
        emu.memory.write_u32(read_out, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// BOOL CloseHandle(HANDLE h)
// Close a fake HLE handle slot if it exists.
fn hle_close_handle(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let closed = emu.hle.close_handle(h) as u32;
    ret(emu, closed);
    HleResult::Retn(4)
}

// DWORD GetFileSize(HANDLE h, DWORD *high)
// Return the host file length without changing the file position.
fn hle_get_file_size(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let high = arg(emu, 1);
    let pos_and_len = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => Some((file.pos, file.size())),
        _ => None,
    };
    if let Some((_, len)) = pos_and_len {
        if high != 0 {
            emu.memory.write_u32(high, (len >> 32) as u32).hle();
        }
        ret(emu, len as u32);
    } else {
        ret(emu, INVALID_HANDLE_VALUE);
    }
    HleResult::Retn(8)
}

// DWORD GetFileType(HANDLE h)
// Report disk files for known file handles and unknown for others.
fn hle_get_file_type(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let value = match emu.hle.handle_mut(h) {
        Some(Handle::File(_)) => 1, // FILE_TYPE_DISK
        _ => 0,
    };
    ret(emu, value);
    HleResult::Retn(4)
}

// DWORD SetFilePointer(HANDLE h, LONG dist, LONG *high, DWORD method)
// Seek a host file and return the low 32 bits of the new offset.
fn hle_set_file_pointer(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let low = arg(emu, 1);
    let high_ptr = arg(emu, 2);
    let method = arg(emu, 3);
    let high = if high_ptr != 0 {
        Some(emu.memory.read_u32(high_ptr).hle())
    } else {
        None
    };
    let dist = set_file_pointer_distance(low, high);
    let pos = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => file.seek(dist, method).map_err(Error::Io).hle(),
        _ => {
            ret(emu, INVALID_HANDLE_VALUE);
            return HleResult::Retn(16);
        }
    };
    if high_ptr != 0 {
        emu.memory.write_u32(high_ptr, (pos >> 32) as u32).hle();
    }
    ret(emu, pos as u32);
    HleResult::Retn(16)
}

fn set_file_pointer_distance(low: u32, high: Option<u32>) -> i64 {
    match high {
        // With lpDistanceToMoveHigh, Win32 treats the pair as a signed 64-bit offset.
        Some(high) => ((high as i32 as i64) << 32) | low as i64,
        // Without it, the low parameter is a signed 32-bit LONG.
        None => low as i32 as i64,
    }
}

// BOOL SetFilePointerEx(HANDLE h, LARGE_INTEGER dist, LARGE_INTEGER *new_pos, DWORD method)
// Seek a host or virtual file and optionally report the new 64-bit offset.
fn hle_set_file_pointer_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let dist = arg(emu, 1) as i32 as i64;
    let new_pos = arg(emu, 2);
    let method = arg(emu, 3);
    let pos = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => file.seek(dist, method).map_err(Error::Io).hle(),
        _ => {
            emu.hle.last_error = 6;
            ret(emu, 0);
            return HleResult::Retn(16);
        }
    };
    if new_pos != 0 {
        emu.memory.write_u32(new_pos, pos as u32).hle();
        emu.memory.write_u32(new_pos + 4, (pos >> 32) as u32).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL SetEndOfFile(HANDLE h)
// Truncate or extend the current file at its current seek position.
fn hle_set_end_of_file(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let ok = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) if file.writable => {
            file.set_end().map_err(Error::Io).hle();
            true
        }
        Some(Handle::File(_)) => false,
        _ => false,
    };
    if !ok {
        emu.hle.last_error = 6;
    }
    ret(emu, ok as u32);
    HleResult::Retn(4)
}

// BOOL FlushFileBuffers(HANDLE h)
// Flush host file buffers for real file handles.
fn hle_flush_file_buffers(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    if let Some(Handle::File(file)) = emu.hle.handle_mut(h) {
        if let FileBackend::Host(host) = &mut file.backend {
            host.flush().map_err(Error::Io).hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL GetOverlappedResult(HANDLE h, OVERLAPPED *ov, DWORD *transferred, BOOL wait)
// Complete synchronous fake file I/O immediately with zero pending bytes.
fn hle_get_overlapped_result(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let transferred = arg(emu, 2);
    if transferred != 0 {
        emu.memory.write_u32(transferred, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL DeviceIoControl(HANDLE h, DWORD code, void *in, DWORD in_len, void *out, DWORD out_len, DWORD *ret, OVERLAPPED *ov)
// Reject device-specific controls while reporting no output bytes.
fn hle_device_io_control(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let returned = arg(emu, 6);
    if returned != 0 {
        emu.memory.write_u32(returned, 0).hle();
    }
    emu.hle.last_error = 1;
    ret(emu, 0);
    HleResult::Retn(32)
}

// HFILE _lopen(LPCSTR path, int mode)
// Open a guest path through the mounted filesystem and return an HFILE handle.
fn hle_lopen(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let mode = arg(emu, 1) & 0x3;
    let access = match mode {
        1 => 0x4000_0000,
        2 => 0xc000_0000,
        _ => 0x8000_0000,
    };
    let h = open_compat_file(emu, &raw, access, 3);
    ret(emu, h);
    HleResult::Retn(8)
}

// HFILE _lcreat(LPCSTR path, int attrs)
// Create or truncate a guest path and return an HFILE handle.
fn hle_lcreat(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let h = open_compat_file(emu, &raw, 0x4000_0000, 2);
    ret(emu, h);
    HleResult::Retn(8)
}

// HFILE OpenFile(LPCSTR path, OFSTRUCT *info, UINT style)
// Open or create a legacy compatibility file and fill minimal OFSTRUCT metadata.
fn hle_open_file(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const OF_WRITE: u32 = 0x0001;
    const OF_READWRITE: u32 = 0x0002;
    const OF_CREATE: u32 = 0x1000;

    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let info = arg(emu, 1);
    let style = arg(emu, 2);
    let access = match style & 0x0003 {
        OF_WRITE => 0x4000_0000,
        OF_READWRITE => 0xc000_0000,
        _ => 0x8000_0000,
    };
    let creation = if (style & OF_CREATE) != 0 { 2 } else { 3 };
    let h = open_compat_file(emu, &raw, access, creation);
    if info != 0 {
        emu.memory.write_u8(info, 136).hle();
        emu.memory.write_u8(info + 1, 1).hle();
        emu.memory
            .write_u16(info + 2, if h == INVALID_HANDLE_VALUE { 2 } else { 0 })
            .hle();
        emu.memory.write_u16(info + 4, 0).hle();
        emu.memory.write_u16(info + 6, 0).hle();
        emu.memory.write_cstr(info + 8, &raw, 128).hle();
    }
    ret(emu, h);
    HleResult::Retn(12)
}

// UINT _lread(HFILE h, void *buf, UINT len)
// Read bytes from a compatibility file handle into guest memory.
fn hle_lread(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let buf = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let mut tmp = vec![0; len];
    let read = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => match file.read(&mut tmp) {
            FileReadResult::Ready(result) => result.map_err(Error::Io).hle(),
            FileReadResult::Pending { key, offset, len } => {
                if len == 0 {
                    ret(emu, 0);
                    return HleResult::Retn(12);
                }
                let request_id = emu.hle.begin_vfs_read(&key, offset, len);
                return HleResult::Wait(HleWaitState::VfsRead {
                    request_id,
                    buf,
                    read_out: 0,
                    ret_transferred: true,
                    ret_item_size: 1,
                    fail_value: u32::MAX,
                    arg_bytes: 12,
                });
            }
        },
        _ => {
            emu.hle.last_error = 6;
            ret(emu, INVALID_HANDLE_VALUE);
            return HleResult::Retn(12);
        }
    };
    emu.memory.write_bytes(buf, &tmp[..read]).hle();
    ret(emu, read as u32);
    HleResult::Retn(12)
}

// UINT _lwrite(HFILE h, const void *buf, UINT len)
// Write bytes from guest memory to a compatibility file handle.
fn hle_lwrite(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let buf = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let data = emu.memory.read_bytes(buf, len).hle();
    let written = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => match file.write(data) {
            FileWriteResult::Ready(Ok(written)) => written,
            FileWriteResult::Ready(Err(err)) => {
                emu.hle.last_error = if err.kind() == std::io::ErrorKind::PermissionDenied {
                    5
                } else {
                    6
                };
                ret(emu, INVALID_HANDLE_VALUE);
                return HleResult::Retn(12);
            }
            FileWriteResult::Pending { key, offset, data } => {
                emu.hle.note_async_vfs_write(&key, offset, data.len());
                let request_id = emu.hle.begin_vfs_write(&key, offset, data);
                return HleResult::Wait(HleWaitState::VfsWrite {
                    request_id,
                    written_out: 0,
                    ret_transferred: true,
                    ret_item_size: 1,
                    fail_value: u32::MAX,
                    arg_bytes: 12,
                });
            }
        },
        _ => len,
    };
    ret(emu, written as u32);
    HleResult::Retn(12)
}

// LONG _llseek(HFILE h, LONG offset, int origin)
// Seek a compatibility file handle and return the low 32-bit position.
fn hle_llseek(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let offset = arg(emu, 1) as i32 as i64;
    let origin = arg(emu, 2);
    let pos = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => file.seek(offset, origin).map_err(Error::Io).hle() as u32,
        _ => INVALID_HANDLE_VALUE,
    };
    ret(emu, pos);
    HleResult::Retn(12)
}

// HFILE _lclose(HFILE h)
// Close a compatibility file handle and return zero on success.
fn hle_lclose(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let closed = emu.hle.close_handle(arg(emu, 0));
    ret(emu, if closed { 0 } else { INVALID_HANDLE_VALUE });
    HleResult::Retn(4)
}

// DWORD GetLogicalDrives(void)
// Return a bitmask of currently mounted guest drives.
fn hle_get_logical_drives(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let mut mask = 0u32;
    for index in 0..26 {
        if emu.hle.drive_mounted_at(index) {
            mask |= 1 << index;
        }
    }
    ret(emu, mask);
    HleResult::Retn(0)
}

// DWORD GetLogicalDriveStringsA(DWORD len, LPSTR out)
// Write a multi-string of mounted guest drive roots.
fn hle_get_logical_drive_strings_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let len = arg(emu, 0) as usize;
    let out = arg(emu, 1);
    let mut bytes = Vec::new();
    for index in 0..26 {
        if emu.hle.drive_mounted_at(index) {
            bytes.push(b'A' + index as u8);
            bytes.extend_from_slice(b":\\\0");
        }
    }
    bytes.push(0);

    if out != 0 && len >= bytes.len() {
        emu.memory.write_bytes(out, &bytes).hle();
        ret(emu, bytes.len().saturating_sub(1) as u32);
    } else {
        ret(emu, bytes.len() as u32);
    }
    HleResult::Retn(8)
}

// BOOL DeleteFileA(LPCSTR name)
// Translate and remove a host file, returning Win32-style success.
fn hle_delete_file_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let deleted = delete_file_impl(&mut emu.hle, &raw) as u32;
    ret(emu, deleted);
    HleResult::Retn(4)
}

// BOOL DeleteFileW(LPCWSTR name)
// Translate and remove a host or virtual file from a wide guest path.
fn hle_delete_file_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.utf16z_lossy(arg(emu, 0), 1024).unwrap_or_default();
    let deleted = delete_file_impl(&mut emu.hle, &raw) as u32;
    ret(emu, deleted);
    HleResult::Retn(4)
}

fn delete_file_impl(hle: &mut Hle, raw: &str) -> bool {
    hle.delete_file_path(raw)
}

// BOOL MoveFileA(LPCSTR old, LPCSTR new)
// Translate paths and rename a host file.
fn hle_move_file_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw_from = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let raw_to = emu.memory.cstr_lossy(arg(emu, 1), 1024).hle();
    let moved = move_file_impl(&mut emu.hle, &raw_from, Some(&raw_to), 0);
    ret(emu, moved as u32);
    HleResult::Retn(8)
}

// BOOL MoveFileExA(LPCSTR old, LPCSTR new, DWORD flags)
// Translate paths and rename/delete host files with replace-existing support.
fn hle_move_file_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw_from = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let new_addr = arg(emu, 1);
    let flags = arg(emu, 2);
    if new_addr == 0 {
        let moved = move_file_impl(&mut emu.hle, &raw_from, None, flags);
        ret(emu, moved as u32);
        return HleResult::Retn(12);
    }
    let raw_to = emu.memory.cstr_lossy(new_addr, 1024).hle();
    let moved = move_file_impl(&mut emu.hle, &raw_from, Some(&raw_to), flags);
    ret(emu, moved as u32);
    HleResult::Retn(12)
}

fn move_file_impl(hle: &mut Hle, raw_from: &str, raw_to: Option<&str>, flags: u32) -> bool {
    hle.move_file_path(raw_from, raw_to, flags)
}

// BOOL CreateDirectoryA(LPCSTR path, LPSECURITY_ATTRIBUTES attrs)
// Create a translated host directory under the mounted filesystem.
fn hle_create_directory_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    if emu.hle.virtual_fs_enabled() {
        ret(emu, 1);
        return HleResult::Retn(8);
    }
    let path = emu.hle.translate_raw_path(&raw).hle();
    let ok = fs::create_dir(&path).is_ok();
    if !ok {
        emu.hle.last_error = if path.exists() { 183 } else { 3 };
    }
    ret(emu, ok as u32);
    HleResult::Retn(8)
}

// BOOL RemoveDirectoryA(LPCSTR path)
// Remove an empty translated host directory.
fn hle_remove_directory_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    if emu.hle.virtual_fs_enabled() {
        ret(emu, 1);
        return HleResult::Retn(4);
    }
    let path = emu.hle.translate_raw_path(&raw).hle();
    ret(emu, fs::remove_dir(path).is_ok() as u32);
    HleResult::Retn(4)
}

// BOOL SetFileAttributesA(LPCSTR path, DWORD attrs)
// Accept attribute changes for existing translated files and directories.
fn hle_set_file_attributes_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let ok = emu.hle.set_file_attributes_path(&raw) as u32;
    ret(emu, ok);
    HleResult::Retn(8)
}

// HANDLE FindFirstFileA(LPCSTR pattern, WIN32_FIND_DATAA *out)
// Enumerate host and virtual directory entries matching a DOS wildcard pattern.
fn hle_find_first_file_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let out = arg(emu, 1);
    find_first_file_impl(emu, "FindFirstFileA", &raw, out, false, 8)
}

fn find_first_file_impl(
    emu: &mut Emulator,
    api_name: &str,
    raw: &str,
    out: u32,
    wide: bool,
    retn: u32,
) -> HleResult {
    let Some(found) = emu.hle.find_file_entries(raw) else {
        trace_fs!("{api_name} pattern={raw:?} -> none last_error=2");
        ret(emu, INVALID_HANDLE_VALUE);
        return HleResult::Retn(retn);
    };
    let FindEntriesResult {
        dir_raw,
        pattern,
        host_dir,
        entries,
    } = found;
    if wide {
        write_find_data_w(emu, out, &entries[0]);
    } else {
        write_find_data_a(emu, out, &entries[0]);
    }
    let handle = emu.hle.alloc_handle(Handle::Find { entries, index: 0 });
    if let Some(dir) = host_dir.as_ref() {
        trace_fs!(
            "{api_name} pattern={raw:?} host_dir={dir:?} match={:?} -> handle={handle:08x}",
            pattern
        );
    } else {
        trace_fs!(
            "{api_name} pattern={raw:?} dir={:?} match={:?} -> handle={handle:08x}",
            dir_raw, pattern
        );
    }
    ret(emu, handle);
    HleResult::Retn(retn)
}

// HANDLE FindFirstFileExW(LPCWSTR pattern, FINDEX_INFO_LEVELS level, void *out, FINDEX_SEARCH_OPS search, void *filter, DWORD flags)
// Enumerate host and virtual directory entries matching a wide DOS wildcard pattern.
fn hle_find_first_file_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.utf16z_lossy(arg(emu, 0), 1024).hle();
    let out = arg(emu, 2);
    find_first_file_impl(emu, "FindFirstFileExW", &raw, out, true, 24)
}

// BOOL FindNextFileA(HANDLE find, WIN32_FIND_DATAA *out)
// Return the next directory entry from a tracked find handle.
fn hle_find_next_file_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let out = arg(emu, 1);
    let entry = match emu.hle.handle_mut(h) {
        Some(Handle::Find { entries, index }) => {
            *index += 1;
            entries.get(*index).map(|entry| FindEntry {
                name: entry.name.clone(),
                attrs: entry.attrs,
                size: entry.size,
            })
        }
        _ => None,
    };
    if let Some(entry) = entry {
        write_find_data_a(emu, out, &entry);
        trace_fs!(
            "FindNextFileA handle={h:08x} -> name={:?} attrs={:08x} size={}",
            entry.name, entry.attrs, entry.size
        );
        ret(emu, 1);
    } else {
        emu.hle.last_error = 18;
        trace_fs!(
            "FindNextFileA handle={h:08x} -> none last_error=18"
        );
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// BOOL FindNextFileW(HANDLE find, WIN32_FIND_DATAW *out)
// Return the next directory entry from a tracked find handle as wide text.
fn hle_find_next_file_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let out = arg(emu, 1);
    let entry = match emu.hle.handle_mut(h) {
        Some(Handle::Find { entries, index }) => {
            *index += 1;
            entries.get(*index).map(|entry| FindEntry {
                name: entry.name.clone(),
                attrs: entry.attrs,
                size: entry.size,
            })
        }
        _ => None,
    };
    if let Some(entry) = entry {
        write_find_data_w(emu, out, &entry);
        trace_fs!(
            "FindNextFileW handle={h:08x} -> name={:?} attrs={:08x} size={}",
            entry.name, entry.attrs, entry.size
        );
        ret(emu, 1);
    } else {
        emu.hle.last_error = 18;
        trace_fs!("FindNextFileW handle={h:08x} -> none last_error=18");
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// BOOL FindClose(HANDLE find)
// Close a tracked directory enumeration handle.
fn hle_find_close(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let closed = emu.hle.close_handle(h) as u32;
    ret(emu, closed);
    HleResult::Retn(4)
}

// DWORD GetFileAttributesA(LPCSTR name)
// Return minimal directory/archive attributes for translated paths.
fn hle_get_file_attributes_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let attrs = get_file_attributes_impl(&mut emu.hle, "GetFileAttributesA", &raw);
    ret(emu, attrs);
    HleResult::Retn(4)
}

fn get_file_attributes_impl(hle: &mut Hle, api_name: &str, raw: &str) -> u32 {
    match get_file_attribute_info_impl(hle, api_name, raw) {
        Some((attrs, _)) => attrs,
        None => INVALID_HANDLE_VALUE,
    }
}

fn get_file_attribute_info_impl(hle: &mut Hle, api_name: &str, raw: &str) -> Option<(u32, u64)> {
    match hle.file_attribute_info(raw) {
        Some((attrs, size)) => {
            trace_fs!("{api_name} name={raw:?} -> attrs={attrs:08x} size={size}");
            Some((attrs, size))
        }
        None => {
            trace_fs!("{api_name} name={raw:?} -> failed last_error=2");
            None
        }
    }
}

// DWORD GetFullPathNameA(LPCSTR name, DWORD len, LPSTR buf, LPSTR *filepart)
// Expand a guest path into the output buffer.
fn hle_get_full_path_name_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src_addr = arg(emu, 0);
    let len = arg(emu, 1) as usize;
    let dst = arg(emu, 2);
    let file_part = arg(emu, 3);
    let src = emu.memory.cstr_lossy(src_addr, 1024).hle();
    let full = emu.hle.full_guest_path(&src);
    if dst != 0 && len != 0 {
        emu.memory.write_cstr(dst, &full, len).hle();
    }
    if file_part != 0 {
        let slash = full.rfind('\\').map(|i| i + 1).unwrap_or(0);
        emu.memory
            .write_u32(file_part, dst.wrapping_add(slash as u32)).hle();
    }
    ret(emu, full.len() as u32);
    HleResult::Retn(16)
}

// DWORD GetCurrentDirectoryA(DWORD len, LPSTR buf)
// Return the current guest drive/path string.
fn hle_get_current_directory_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let len = arg(emu, 0) as usize;
    let dst = arg(emu, 1);
    let cwd = emu.hle.cwd_display();
    if dst != 0 && len != 0 {
        emu.memory.write_cstr(dst, &cwd, len).hle();
    }
    ret(emu, cwd.len() as u32);
    HleResult::Retn(8)
}

// DWORD GetCurrentDirectoryW(DWORD len, LPWSTR buf)
// Return the current guest drive/path string as UTF-16.
fn hle_get_current_directory_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let len = arg(emu, 0) as usize;
    let dst = arg(emu, 1);
    let cwd = emu.hle.cwd_display();
    if dst != 0 && len != 0 {
        emu.memory.write_utf16z(dst, &cwd, len).hle();
    }
    ret(emu, cwd.encode_utf16().count() as u32);
    HleResult::Retn(8)
}

// BOOL SetCurrentDirectoryA(LPCSTR path)
// Update the guest current drive/path using DOS path normalization.
fn hle_set_current_directory_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu.memory.cstr_lossy(arg(emu, 0), 1024).hle();
    let key = emu.hle.guest_path_key(&raw);
    let mut chars = key.chars();
    let drive = chars.next().unwrap_or_else(|| emu.hle.cwd_drive());
    let path = key
        .split_once(':')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| "\\".to_string());
    emu.hle.set_cwd(drive, path);
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL WritePrivateProfileStringA(LPCSTR section, LPCSTR key, LPCSTR value, LPCSTR file)
// Accept legacy INI writes without persistent profile storage.
fn hle_write_private_profile_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(16)
}

// UINT GetProfileIntA(LPCSTR section, LPCSTR key, INT default)
// Return the caller-supplied default for legacy WIN.INI reads.
fn hle_get_profile_int_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 2));
    HleResult::Retn(12)
}

// UINT GetPrivateProfileIntA(LPCSTR section, LPCSTR key, INT default, LPCSTR file)
// Return the caller-supplied default for legacy INI reads.
fn hle_get_private_profile_int_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 2));
    HleResult::Retn(16)
}

// UINT GetPrivateProfileIntW(LPCWSTR section, LPCWSTR key, INT default, LPCWSTR file)
// Return the caller-supplied default for legacy INI reads.
fn hle_get_private_profile_int_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 2));
    HleResult::Retn(16)
}

// DWORD GetPrivateProfileStringA(LPCSTR section, LPCSTR key, LPCSTR default, LPSTR out, DWORD size, LPCSTR file)
// Copy the caller-supplied default for legacy INI string reads.
fn hle_get_private_profile_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let default_ptr = arg(emu, 2);
    let out = arg(emu, 3);
    let size = arg(emu, 4) as usize;
    let value = if default_ptr != 0 {
        emu.memory.cstr_lossy(default_ptr, 4096).hle()
    } else {
        String::new()
    };
    if out != 0 && size != 0 {
        emu.memory.write_cstr(out, &value, size).hle();
    }
    ret(emu, value.len().min(size.saturating_sub(1)) as u32);
    HleResult::Retn(24)
}

// DWORD GetPrivateProfileStringW(LPCWSTR section, LPCWSTR key, LPCWSTR default, LPWSTR out, DWORD size, LPCWSTR file)
// Copy the caller-supplied default for legacy INI string reads.
fn hle_get_private_profile_string_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let default_ptr = arg(emu, 2);
    let out = arg(emu, 3);
    let size = arg(emu, 4) as usize;
    let value = if default_ptr != 0 {
        emu.memory.utf16z_lossy(default_ptr, 4096).hle()
    } else {
        String::new()
    };
    let written = if out != 0 && size != 0 {
        emu.memory.write_utf16z(out, &value, size).hle()
    } else {
        0
    };
    ret(emu, written);
    HleResult::Retn(24)
}

// UINT GetSystemDirectoryA(LPSTR out, UINT size)
// Return the fake guest system directory used by legacy runtime DLLs.
fn hle_get_system_directory_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let size = arg(emu, 1) as usize;
    let path = "C:\\WINDOWS\\SYSTEM32";
    if out != 0 && size != 0 {
        emu.memory.write_cstr(out, path, size).hle();
    }
    ret(emu, path.len() as u32);
    HleResult::Retn(8)
}

// UINT GetWindowsDirectoryA(LPSTR out, UINT size)
// Return the fake guest Windows directory used by the process environment.
fn hle_get_windows_directory_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let size = arg(emu, 1) as usize;
    let path = "C:\\WINDOWS";
    if out != 0 && size != 0 {
        emu.memory.write_cstr(out, path, size).hle();
    }
    ret(emu, path.len() as u32);
    HleResult::Retn(8)
}

// UINT GetWindowsDirectoryW(LPWSTR out, UINT size)
// Return the fake guest Windows directory used by the process environment.
fn hle_get_windows_directory_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let size = arg(emu, 1) as usize;
    let path = "C:\\WINDOWS";
    if out != 0 && size != 0 {
        emu.memory.write_utf16z(out, path, size).hle();
    }
    ret(emu, path.encode_utf16().count() as u32);
    HleResult::Retn(8)
}

// UINT GetDriveTypeA(LPCSTR root)
// Report mounted drive devices, including app DB cdrom mounts.
fn hle_get_drive_type_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let root = arg(emu, 0);
    let s = emu.memory.cstr_lossy(root, 260).hle().to_ascii_uppercase();
    let drive = s.as_bytes().first().copied().unwrap_or(b'C') as char;
    let value = emu.hle.drive_type(drive).unwrap_or(1);
    ret(emu, value);
    HleResult::Retn(4)
}

// BOOL GetVolumeInformationA(LPCSTR root, LPSTR vol, DWORD vol_len, LPDWORD serial, LPDWORD max_name, LPDWORD flags, LPSTR fs, DWORD fs_len)
// Report stable FAT-style metadata for mounted guest drives.
fn hle_get_volume_information_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let root = arg(emu, 0);
    let volume_name = arg(emu, 1);
    let volume_name_len = arg(emu, 2) as usize;
    let serial = arg(emu, 3);
    let max_component_len = arg(emu, 4);
    let fs_flags = arg(emu, 5);
    let fs_name = arg(emu, 6);
    let fs_name_len = arg(emu, 7) as usize;
    let drive = guest_root_drive_a(emu, root);
    let volume = emu.hle.drive_volume_name(drive);

    if volume_name != 0 && volume_name_len != 0 {
        emu.memory
            .write_cstr(volume_name, volume, volume_name_len)
            .hle();
    }
    if serial != 0 {
        emu.memory.write_u32(serial, 0x5745_4d55).hle();
    }
    if max_component_len != 0 {
        emu.memory.write_u32(max_component_len, 255).hle();
    }
    if fs_flags != 0 {
        emu.memory.write_u32(fs_flags, 0x0000_0002).hle();
    }
    if fs_name != 0 && fs_name_len != 0 {
        emu.memory.write_cstr(fs_name, "FAT32", fs_name_len).hle();
    }
    ret(emu, 1);
    HleResult::Retn(32)
}

// BOOL GetDiskFreeSpaceA(LPCSTR root, LPDWORD sectors, LPDWORD bytes, LPDWORD free, LPDWORD total)
// Return deterministic nonzero capacity values for mounted guest drives.
fn hle_get_disk_free_space_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let sectors_per_cluster = arg(emu, 1);
    let bytes_per_sector = arg(emu, 2);
    let free_clusters = arg(emu, 3);
    let total_clusters = arg(emu, 4);
    if sectors_per_cluster != 0 {
        emu.memory.write_u32(sectors_per_cluster, 8).hle();
    }
    if bytes_per_sector != 0 {
        emu.memory.write_u32(bytes_per_sector, 512).hle();
    }
    if free_clusters != 0 {
        emu.memory.write_u32(free_clusters, 0x0002_0000).hle();
    }
    if total_clusters != 0 {
        emu.memory.write_u32(total_clusters, 0x0004_0000).hle();
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

fn guest_root_drive_a(emu: &Emulator, root: u32) -> char {
    if root == 0 {
        return emu.hle.cwd_drive();
    }
    let raw = emu.memory.cstr_lossy(root, 260).unwrap_or_default();
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        bytes[0].to_ascii_uppercase() as char
    } else {
        emu.hle.cwd_drive()
    }
}

// DWORD GetModuleFileNameA(HMODULE mod, LPSTR buf, DWORD len)
// Return the guest executable path string.
fn hle_get_module_file_name_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let name = emu.hle.module_file_name.as_str();
    if dst != 0 && len != 0 {
        emu.memory.write_cstr(dst, name, len).hle();
    }
    ret(emu, name.len() as u32);
    HleResult::Retn(12)
}

// DWORD GetModuleFileNameW(HMODULE mod, LPWSTR buf, DWORD len)
// Return the guest executable path as UTF-16.
fn hle_get_module_file_name_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    let name = emu.hle.module_file_name.as_str();
    if dst != 0 && len != 0 {
        emu.memory.write_utf16z(dst, name, len).hle();
    }
    ret(emu, name.encode_utf16().count() as u32);
    HleResult::Retn(12)
}

// BOOL WinHelpA(HWND hwnd, LPCSTR file, UINT command, ULONG_PTR data)
// Accept help requests without launching an external viewer.
fn hle_win_help_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(16)
}

// HWND WINAPIV MCIWndCreateA(HWND parent, HINSTANCE inst, DWORD style, LPCSTR file)
// Skip Video-for-Windows window creation when media playback is unavailable.
fn hle_mci_wnd_create_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// UINT DragQueryFileA(HDROP drop, UINT file, LPSTR out, UINT cch)
// Report an empty drag-file list for shell drop helpers.
fn hle_drag_query_file_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    let cch = arg(emu, 3) as usize;
    if out != 0 && cch != 0 {
        emu.memory.write_u8(out, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// BOOL GetSystemTimeAsFileTime(LPFILETIME out)
// Write a deterministic FILETIME derived from the emulator guest clock.
fn hle_get_system_time_as_file_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    emu.refresh_guest_time();
    let filetime = 116_444_736_000_000_000u64
        + emu.guest_time_ms.saturating_mul(TICKS_PER_MILLISECOND);
    if out != 0 {
        write_filetime(emu, out, filetime);
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// BOOL FileTimeToLocalFileTime(const FILETIME *in, FILETIME *out)
// Copy UTC file time unchanged because timezone offsets are not modeled.
fn hle_file_time_to_local_file_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let input = arg(emu, 0);
    let out = arg(emu, 1);
    if input != 0 && out != 0 {
        write_filetime(emu, out, read_filetime(emu, input));
        ret(emu, 1);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// BOOL DosDateTimeToFileTime(WORD date, WORD time, LPFILETIME out)
// Convert a DOS date/time pair into a Windows FILETIME.
fn hle_dos_date_time_to_file_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let date = arg(emu, 0);
    let time = arg(emu, 1);
    let out = arg(emu, 2);
    if out == 0 {
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    let day = date & 0x1f;
    let month = (date >> 5) & 0x0f;
    let year = ((date >> 9) & 0x7f) as i32 + 1980;
    let second = (time & 0x1f) * 2;
    let minute = (time >> 5) & 0x3f;
    let hour = (time >> 11) & 0x1f;
    if day == 0 || month == 0 || month > 12 || hour > 23 || minute > 59 || second > 59 {
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    let days_since_1601 = days_from_civil(year, month, day) + DAYS_1601_TO_1970;
    let seconds = days_since_1601 as u64 * SECONDS_PER_DAY
        + hour as u64 * 3600
        + minute as u64 * 60
        + second as u64;
    write_filetime(emu, out, seconds.saturating_mul(TICKS_PER_SECOND));
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL FileTimeToDosDateTime(const FILETIME *in, WORD *date, WORD *time)
// Convert a Windows FILETIME into DOS date/time fields.
fn hle_file_time_to_dos_date_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let input = arg(emu, 0);
    let date_out = arg(emu, 1);
    let time_out = arg(emu, 2);
    if input == 0 || date_out == 0 || time_out == 0 {
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    let ticks = read_filetime(emu, input);
    let total_seconds = ticks / TICKS_PER_SECOND;
    let days_since_1601 = (total_seconds / SECONDS_PER_DAY) as i64;
    let seconds_of_day = total_seconds % SECONDS_PER_DAY;
    let (year, month, day) = civil_from_days(days_since_1601 - DAYS_1601_TO_1970);
    let dos_year = year.clamp(1980, 2107) as u32 - 1980;
    let hour = (seconds_of_day / 3600) as u32;
    let minute = ((seconds_of_day / 60) % 60) as u32;
    let second = (seconds_of_day % 60) as u32;
    let dos_date = (dos_year << 9) | ((month as u32) << 5) | day as u32;
    let dos_time = (hour << 11) | (minute << 5) | (second / 2);
    emu.memory.write_u16(date_out, dos_date as u16).hle();
    emu.memory.write_u16(time_out, dos_time as u16).hle();
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL LocalFileTimeToFileTime(const FILETIME *in, FILETIME *out)
// Copy local file time unchanged because timezone offsets are not modeled.
fn hle_local_file_time_to_file_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let input = arg(emu, 0);
    let out = arg(emu, 1);
    if input != 0 && out != 0 {
        write_filetime(emu, out, read_filetime(emu, input));
        ret(emu, 1);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// BOOL SystemTimeToFileTime(const SYSTEMTIME *in, FILETIME *out)
// Convert a SYSTEMTIME structure into a Windows FILETIME tick count.
fn hle_system_time_to_file_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let input = arg(emu, 0);
    let out = arg(emu, 1);
    if input == 0 || out == 0 {
        ret(emu, 0);
        return HleResult::Retn(8);
    }

    let year = emu.memory.read_u16(input).hle() as i32;
    let month = emu.memory.read_u16(input + 2).hle() as u32;
    let day = emu.memory.read_u16(input + 6).hle() as u32;
    let hour = emu.memory.read_u16(input + 8).hle() as u64;
    let minute = emu.memory.read_u16(input + 10).hle() as u64;
    let second = emu.memory.read_u16(input + 12).hle() as u64;
    let millis = emu.memory.read_u16(input + 14).hle() as u64;
    let days_since_1601 = days_from_civil(year, month, day) + DAYS_1601_TO_1970;
    let seconds = days_since_1601 as u64 * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second;
    let filetime = seconds
        .saturating_mul(TICKS_PER_SECOND)
        .saturating_add(millis.saturating_mul(TICKS_PER_MILLISECOND));
    write_filetime(emu, out, filetime);
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL GetFileTime(HANDLE file, LPFILETIME create, LPFILETIME access, LPFILETIME write)
// Return deterministic timestamps for HLE-backed file handles.
fn hle_get_file_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = arg(emu, 0);
    let creation = arg(emu, 1);
    let access = arg(emu, 2);
    let write = arg(emu, 3);
    let valid = matches!(emu.hle.handle_mut(handle), Some(Handle::File(_)));
    if !valid {
        emu.hle.last_error = 6;
        ret(emu, 0);
        return HleResult::Retn(16);
    }
    if creation != 0 {
        write_filetime(emu, creation, DEFAULT_FILETIME);
    }
    if access != 0 {
        write_filetime(emu, access, DEFAULT_FILETIME);
    }
    if write != 0 {
        write_filetime(emu, write, DEFAULT_FILETIME);
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL GetFileInformationByHandle(HANDLE file, BY_HANDLE_FILE_INFORMATION *info)
// Return deterministic metadata for real and virtual file handles.
fn hle_get_file_information_by_handle(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = arg(emu, 0);
    let out = arg(emu, 1);
    let size = match emu.hle.handle_mut(handle) {
        Some(Handle::File(file)) => file.size(),
        _ => {
            emu.hle.last_error = 6;
            ret(emu, 0);
            return HleResult::Retn(8);
        }
    };
    if out != 0 {
        emu.memory.memset(out, 0, 52).hle();
        emu.memory.write_u32(out, 0x80).hle(); // FILE_ATTRIBUTE_NORMAL
        write_filetime(emu, out + 4, DEFAULT_FILETIME);
        write_filetime(emu, out + 12, DEFAULT_FILETIME);
        write_filetime(emu, out + 20, DEFAULT_FILETIME);
        emu.memory.write_u32(out + 28, 1).hle();
        emu.memory.write_u32(out + 32, (size >> 32) as u32).hle();
        emu.memory.write_u32(out + 36, size as u32).hle();
        emu.memory.write_u32(out + 40, 1).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetFileTime(HANDLE file, const FILETIME *create, const FILETIME *access, const FILETIME *write)
// Accept timestamp updates for HLE-backed file handles without mutating host metadata.
fn hle_set_file_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = arg(emu, 0);
    let valid = matches!(emu.hle.handle_mut(handle), Some(Handle::File(_)));
    if !valid {
        emu.hle.last_error = 6;
        ret(emu, 0);
        return HleResult::Retn(16);
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL FileTimeToSystemTime(const FILETIME *ft, SYSTEMTIME *st)
// Convert a Windows FILETIME tick count into a UTC SYSTEMTIME structure.
fn hle_file_time_to_system_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let input = arg(emu, 0);
    let out = arg(emu, 1);
    if input == 0 || out == 0 {
        ret(emu, 0);
        return HleResult::Retn(8);
    }

    let ticks = read_filetime(emu, input);
    let total_seconds = ticks / TICKS_PER_SECOND;
    let days_since_1601 = (total_seconds / SECONDS_PER_DAY) as i64;
    let seconds_of_day = total_seconds % SECONDS_PER_DAY;
    let (year, month, day) = civil_from_days(days_since_1601 - DAYS_1601_TO_1970);
    let day_of_week = ((days_since_1601 + 1).rem_euclid(7)) as u16;
    let millis = ((ticks % TICKS_PER_SECOND) / TICKS_PER_MILLISECOND) as u16;

    emu.memory.write_u16(out, year as u16).hle();
    emu.memory.write_u16(out + 2, month as u16).hle();
    emu.memory.write_u16(out + 4, day_of_week).hle();
    emu.memory.write_u16(out + 6, day as u16).hle();
    emu.memory
        .write_u16(out + 8, (seconds_of_day / 3600) as u16)
        .hle();
    emu.memory
        .write_u16(out + 10, ((seconds_of_day / 60) % 60) as u16)
        .hle();
    emu.memory
        .write_u16(out + 12, (seconds_of_day % 60) as u16)
        .hle();
    emu.memory.write_u16(out + 14, millis).hle();
    ret(emu, 1);
    HleResult::Retn(8)
}

fn read_filetime(emu: &Emulator, addr: u32) -> u64 {
    let low = emu.memory.read_u32(addr).hle() as u64;
    let high = emu.memory.read_u32(addr + 4).hle() as u64;
    (high << 32) | low
}

fn write_filetime(emu: &mut Emulator, addr: u32, filetime: u64) {
    emu.memory.write_u32(addr, filetime as u32).hle();
    emu.memory
        .write_u32(addr + 4, (filetime >> 32) as u32)
        .hle();
}

fn write_file_attribute_data(emu: &mut Emulator, out: u32, attrs: u32, size: u64) {
    if out == 0 {
        return;
    }
    emu.memory.memset(out, 0, 36).hle();
    emu.memory.write_u32(out, attrs).hle();
    write_filetime(emu, out + 4, DEFAULT_FILETIME);
    write_filetime(emu, out + 12, DEFAULT_FILETIME);
    write_filetime(emu, out + 20, DEFAULT_FILETIME);
    emu.memory.write_u32(out + 28, (size >> 32) as u32).hle();
    emu.memory.write_u32(out + 32, size as u32).hle();
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - (month <= 2) as i32;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) as i64
}

// HANDLE CreateFileW(LPCWSTR name, DWORD access, DWORD share, void *sec, DWORD create, DWORD flags, HANDLE template)
// Translate the wide guest path and open a host file handle.
fn hle_create_file_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = arg(emu, 0);
    let raw_name = emu.memory.utf16z_lossy(name, 1024).unwrap_or_default();
    let access = arg(emu, 1);
    let creation = arg(emu, 4);
    create_file_impl(emu, "CreateFileW", &raw_name, access, creation)
}

// HANDLE CreateFileMappingA(HANDLE file, void *sec, DWORD protect, DWORD hi, DWORD lo, LPCSTR name)
// Snapshot a readable file or create a named page-file backed fake mapping.
fn hle_create_file_mapping_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.cstr_lossy(arg(emu, 5), 256).unwrap_or_default();
    let handle = create_file_mapping_common(emu, &name);
    ret(emu, handle);
    HleResult::Retn(24)
}

// HANDLE CreateFileMappingW(HANDLE file, void *sec, DWORD protect, DWORD hi, DWORD lo, LPCWSTR name)
// Snapshot a readable file or create a named page-file backed fake mapping.
fn hle_create_file_mapping_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu
        .memory
        .utf16z_lossy(arg(emu, 5), 256)
        .unwrap_or_default();
    let handle = create_file_mapping_common(emu, &name);
    ret(emu, handle);
    HleResult::Retn(24)
}

// HANDLE OpenFileMappingA(DWORD access, BOOL inherit, LPCSTR name)
// Open an existing named fake file mapping and fail for missing names.
fn hle_open_file_mapping_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.cstr_lossy(arg(emu, 2), 256).unwrap_or_default();
    let handle = open_file_mapping_common(emu, &name);
    ret(emu, handle);
    HleResult::Retn(12)
}

// HANDLE OpenFileMappingW(DWORD access, BOOL inherit, LPCWSTR name)
// Open an existing named fake file mapping and fail for missing names.
fn hle_open_file_mapping_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu
        .memory
        .utf16z_lossy(arg(emu, 2), 256)
        .unwrap_or_default();
    let handle = open_file_mapping_common(emu, &name);
    ret(emu, handle);
    HleResult::Retn(12)
}

// LPVOID MapViewOfFile(HANDLE mapping, DWORD access, DWORD off_hi, DWORD off_lo, SIZE_T bytes)
// Copy fake file-mapping bytes into guest heap memory and return the mapped pointer.
fn hle_map_view_of_file(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let mapping = arg(emu, 0);
    let offset = arg(emu, 3) as usize;
    let requested = arg(emu, 4) as usize;
    let bytes = match emu.hle.handle_mut(mapping) {
        Some(Handle::FileMapping { data }) => {
            let data = data.borrow();
            let start = offset.min(data.len());
            let len = if requested == 0 {
                data.len().saturating_sub(start)
            } else {
                requested.min(data.len().saturating_sub(start))
            };
            data[start..start + len].to_vec()
        }
        _ => Vec::new(),
    };
    let size = bytes.len().max(1) as u32;
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    if !bytes.is_empty() {
        emu.memory.write_bytes(ptr, &bytes).hle();
    }
    ret(emu, ptr);
    HleResult::Retn(20)
}

// BOOL UnmapViewOfFile(LPCVOID base)
// Release fake mapped view memory if it came from the HLE heap.
fn hle_unmap_view_of_file(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let ptr = arg(emu, 0);
    emu.hle.free_alloc(&mut emu.memory, ptr).hle();
    ret(emu, 1);
    HleResult::Retn(4)
}

// DWORD GetFileAttributesW(LPCWSTR name)
// Report host or virtual file attributes for a wide guest path.
fn hle_get_file_attributes_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = arg(emu, 0);
    let raw = emu.memory.utf16z_lossy(name, 1024).unwrap_or_default();
    let attrs = get_file_attributes_impl(&mut emu.hle, "GetFileAttributesW", &raw);
    ret(emu, attrs);
    HleResult::Retn(4)
}

// BOOL GetFileAttributesExW(LPCWSTR name, GET_FILEEX_INFO_LEVELS level, void *data)
// Fill minimal wide file attribute data for host or virtual paths.
fn hle_get_file_attributes_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw = emu
        .memory
        .utf16z_lossy(arg(emu, 0), 1024)
        .unwrap_or_default();
    let out = arg(emu, 2);
    match get_file_attribute_info_impl(&mut emu.hle, "GetFileAttributesExW", &raw) {
        Some((attrs, size)) => {
            write_file_attribute_data(emu, out, attrs, size);
            ret(emu, 1);
        }
        None => {
            ret(emu, 0);
        }
    }
    HleResult::Retn(12)
}

// DWORD GetFullPathNameW(LPCWSTR name, DWORD len, LPWSTR out, LPWSTR *filepart)
// Normalize a wide path through the mounted guest path rules.
fn hle_get_full_path_name_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = arg(emu, 0);
    let len = arg(emu, 1) as usize;
    let out = arg(emu, 2);
    let file_part = arg(emu, 3);
    let raw = emu.memory.utf16z_lossy(name, 1024).unwrap_or_default();
    let key = emu.hle.guest_path_key(&raw);
    if out != 0 && len != 0 {
        emu.memory.write_utf16z(out, &key, len).hle();
    }
    if file_part != 0 {
        let slash = key.rfind('\\').map(|idx| idx + 1).unwrap_or(0);
        emu.memory
            .write_u32(file_part, out.wrapping_add((slash as u32) * 2))
            .hle();
    }
    ret(emu, key.encode_utf16().count() as u32);
    HleResult::Retn(16)
}

fn log_create_file(line: impl AsRef<str>) {
    let line = line.as_ref();
    #[cfg(target_arch = "wasm32")]
    unsafe {
        wemu_console_log(line.as_ptr(), line.len());
    }
    #[cfg(not(target_arch = "wasm32"))]
    println!("{line}");
}

fn open_compat_file(emu: &mut Emulator, raw: &str, access: u32, creation: u32) -> u32 {
    match emu.hle.open_file_handle(raw, access, creation) {
        FileOpen::Opened(h) => h,
        FileOpen::Failed(last_error) => {
            emu.hle.last_error = last_error;
            INVALID_HANDLE_VALUE
        }
    }
}

fn create_file_mapping_common(emu: &mut Emulator, name: &str) -> u32 {
    let h = arg(emu, 0);
    if let Some(key) = kernel_object_key(name) {
        match emu.hle.named_kernel_objects.get(&key) {
            Some(NamedKernelObject::FileMapping(data)) => {
                emu.hle.last_error = 183;
                let handle = emu.hle.alloc_handle(Handle::FileMapping { data: data.clone() });
                trace_fs!("CreateFileMapping name={name:?} -> existing {handle:08x}");
                return handle;
            }
            Some(NamedKernelObject::Event) => {
                emu.hle.last_error = 6;
                trace_fs!("CreateFileMapping name={name:?} -> wrong object type");
                return 0;
            }
            None => {}
        }
    }
    let data = match emu.hle.handle_mut(h) {
        Some(Handle::File(file)) => file.mapping_bytes(),
        _ => {
            let size = arg(emu, 4);
            vec![0; size.min(0x0100_0000) as usize]
        }
    };
    let data = Rc::new(RefCell::new(data));
    if let Some(key) = kernel_object_key(name) {
        emu.hle
            .named_kernel_objects
            .insert(key, NamedKernelObject::FileMapping(data.clone()));
        emu.hle.last_error = 0;
        trace_fs!("CreateFileMapping name={name:?} -> created");
    } else {
        emu.hle.last_error = 0;
    }
    emu.hle.alloc_handle(Handle::FileMapping { data })
}

fn open_file_mapping_common(emu: &mut Emulator, name: &str) -> u32 {
    let Some(key) = kernel_object_key(name) else {
        emu.hle.last_error = 87;
        trace_fs!("OpenFileMapping name={name:?} -> invalid");
        return 0;
    };
    match emu.hle.named_kernel_objects.get(&key) {
        Some(NamedKernelObject::FileMapping(data)) => {
            emu.hle.last_error = 0;
            let handle = emu.hle.alloc_handle(Handle::FileMapping { data: data.clone() });
            trace_fs!("OpenFileMapping name={name:?} -> {handle:08x}");
            handle
        }
        Some(NamedKernelObject::Event) => {
            emu.hle.last_error = 6;
            trace_fs!("OpenFileMapping name={name:?} -> wrong object type");
            0
        }
        None => {
            emu.hle.last_error = 2;
            trace_fs!("OpenFileMapping name={name:?} -> missing");
            0
        }
    }
}

#[cfg(test)]
mod kernel_file_tests {
    use super::{
        delete_file_impl, get_file_attributes_impl, move_file_impl, set_file_pointer_distance,
        INVALID_HANDLE_VALUE,
    };
    use super::{FileOpen, Hle};
    use std::fs;

    #[test]
    fn set_file_pointer_null_high_uses_signed_long_distance() {
        assert_eq!(set_file_pointer_distance(0xffff_fffc, None), -4);
        assert_eq!(set_file_pointer_distance(0x0000_0004, None), 4);
    }

    #[test]
    fn set_file_pointer_high_pointer_uses_signed_i64_pair() {
        assert_eq!(
            set_file_pointer_distance(0xffff_fffc, Some(0xffff_ffff)),
            -4
        );
        assert_eq!(
            set_file_pointer_distance(0xffff_fffc, Some(0)),
            0xffff_fffc
        );
    }

    #[test]
    fn attributes_and_delete_share_virtual_and_host_paths() {
        let root =
            std::env::temp_dir().join(format!("wemu-vfs-attrs-delete-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("host.dat"), b"host").unwrap();

        let mut hle = Hle::new();
        hle.set_drive_mount('C', root.clone());
        assert_eq!(
            get_file_attributes_impl(&mut hle, "TestAttrs", "C:\\host.dat"),
            0x80
        );
        assert!(delete_file_impl(&mut hle, "C:\\host.dat"));

        let mut hle = Hle::new();
        hle.add_virtual_file("C:\\Data\\Virtual.DAT", b"vfs");
        assert_eq!(
            get_file_attributes_impl(&mut hle, "TestAttrs", "C:\\Data\\Virtual.DAT"),
            0x80
        );
        assert!(delete_file_impl(&mut hle, "C:\\Data\\Virtual.DAT"));
        assert_eq!(
            get_file_attributes_impl(&mut hle, "TestAttrs", "C:\\Data\\Virtual.DAT"),
            INVALID_HANDLE_VALUE
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn move_file_impl_handles_host_replace_and_virtual_delete() {
        let root = std::env::temp_dir().join(format!("wemu-vfs-move-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("from.dat"), b"from").unwrap();
        fs::write(root.join("to.dat"), b"to").unwrap();

        let mut hle = Hle::new();
        hle.set_drive_mount('C', root.clone());
        assert!(move_file_impl(
            &mut hle,
            "C:\\from.dat",
            Some("C:\\to.dat"),
            1
        ));
        assert_eq!(fs::read(root.join("to.dat")).unwrap(), b"from");

        let mut hle = Hle::new();
        hle.add_virtual_file("C:\\Data\\from.dat", b"vfs");
        assert!(move_file_impl(
            &mut hle,
            "C:\\Data\\from.dat",
            Some("C:\\Data\\to.dat"),
            0
        ));
        assert_eq!(
            get_file_attributes_impl(&mut hle, "TestAttrs", "C:\\Data\\to.dat"),
            0x80
        );
        assert!(move_file_impl(&mut hle, "C:\\Data\\to.dat", None, 0));
        assert_eq!(
            get_file_attributes_impl(&mut hle, "TestAttrs", "C:\\Data\\to.dat"),
            INVALID_HANDLE_VALUE
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn open_file_handle_uses_host_candidates_and_creation_dispositions() {
        let root = std::env::temp_dir().join(format!("wemu-vfs-open-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Mixed.DAT"), b"host").unwrap();

        let mut hle = Hle::new();
        hle.set_drive_mount('C', root.clone());
        match hle.open_file_handle("C:\\mixed.dat", 0x8000_0000, 3) {
            FileOpen::Opened(_) => {}
            FileOpen::Failed(err) => panic!("case-insensitive host open failed: {err}"),
        }
        match hle.open_file_handle("C:\\Mixed.DAT", 0x4000_0000, 1) {
            FileOpen::Failed(80) => {}
            _ => panic!("CREATE_NEW existing file did not fail with ERROR_FILE_EXISTS"),
        }

        let _ = fs::remove_dir_all(root);
    }
}
