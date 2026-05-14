// LSTATUS RegCloseKey(HKEY key)
// Accept closing fake registry keys.
fn hle_reg_close_key(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// LSTATUS RegCreateKeyA(HKEY root, LPCSTR subkey, HKEY *out)
// Create a fake registry key for legacy application settings writes.
fn hle_reg_create_key_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    reg_create_key_impl(emu, arg(emu, 2));
    HleResult::Retn(12)
}

// LSTATUS RegCreateKeyW(HKEY root, LPCWSTR subkey, HKEY *out)
// Create a fake registry key for legacy application settings writes.
fn hle_reg_create_key_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    reg_create_key_impl(emu, arg(emu, 2));
    HleResult::Retn(12)
}

fn reg_create_key_impl(emu: &mut Emulator, out: u32) {
    if out != 0 {
        emu.memory.write_u32(out, 0x5100_0000).hle();
    }
    ret(emu, 0);
}

// LSTATUS RegCreateKeyExA(HKEY root, LPCSTR subkey, DWORD reserved, LPSTR class, DWORD opts, REGSAM sam, void *sec, HKEY *out, DWORD *disp)
// Create a fake registry key for legacy application settings writes.
fn hle_reg_create_key_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 7);
    let disp = arg(emu, 8);
    if out != 0 {
        emu.memory.write_u32(out, 0x5100_0000).hle();
    }
    if disp != 0 {
        emu.memory.write_u32(disp, 1).hle();
    }
    ret(emu, 0);
    HleResult::Retn(36)
}

// LSTATUS RegOpenKeyW(HKEY root, LPCWSTR subkey, HKEY *out)
// Report missing user settings so Notepad falls back to defaults.
fn hle_reg_open_key_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 2);
    HleResult::Retn(12)
}

// LSTATUS RegOpenKeyA(HKEY root, LPCSTR subkey, HKEY *out)
// Report missing user settings so applications keep their built-in defaults.
fn hle_reg_open_key_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 2);
    HleResult::Retn(12)
}

// LSTATUS RegOpenKeyExW(HKEY root, LPCWSTR subkey, DWORD opts, REGSAM sam, HKEY *out)
// Report missing user settings so applications keep their built-in defaults.
fn hle_reg_open_key_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 4);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 2);
    HleResult::Retn(20)
}

// LSTATUS RegOpenKeyExA(HKEY root, LPCSTR subkey, DWORD opts, REGSAM sam, HKEY *out)
// Report missing user settings so applications keep their built-in defaults.
fn hle_reg_open_key_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 4);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 2);
    HleResult::Retn(20)
}

// LSTATUS RegCreateKeyExW(HKEY root, LPCWSTR subkey, DWORD reserved, LPWSTR class, DWORD opts, REGSAM sam, void *sec, HKEY *out, DWORD *disp)
// Create a fake registry key for shutdown settings writes.
fn hle_reg_create_key_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 7);
    let disp = arg(emu, 8);
    if out != 0 {
        emu.memory.write_u32(out, 0x5100_0000).hle();
    }
    if disp != 0 {
        emu.memory.write_u32(disp, 1).hle();
    }
    ret(emu, 0);
    HleResult::Retn(36)
}

// LSTATUS RegQueryValueExW(HKEY key, LPCWSTR name, DWORD *reserved, DWORD *type, BYTE *data, DWORD *cb)
// Report missing values so defaults remain active.
fn hle_reg_query_value_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 2);
    HleResult::Retn(24)
}

// LSTATUS RegQueryValueExA(HKEY key, LPCSTR name, DWORD *reserved, DWORD *type, BYTE *data, DWORD *cb)
// Report missing values so defaults remain active.
fn hle_reg_query_value_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 2);
    HleResult::Retn(24)
}

// LSTATUS RegQueryValueA(HKEY key, LPCSTR subkey, LPSTR data, LONG *cb)
// Report missing values so defaults remain active.
fn hle_reg_query_value_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let cb = arg(emu, 3);
    if cb != 0 {
        emu.memory.write_u32(cb, 0).hle();
    }
    ret(emu, 2);
    HleResult::Retn(16)
}

// LSTATUS RegQueryInfoKeyA(HKEY key, LPSTR class, DWORD *class_len, DWORD *reserved, DWORD *subkeys, DWORD *max_subkey_len, DWORD *max_class_len, DWORD *values, DWORD *max_value_name_len, DWORD *max_value_len, DWORD *sec_len, FILETIME *write_time)
// Report an empty volatile key.
fn hle_reg_query_info_key_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    for index in 4..=10 {
        let ptr = arg(emu, index);
        if ptr != 0 {
            emu.memory.write_u32(ptr, 0).hle();
        }
    }
    ret(emu, 0);
    HleResult::Retn(48)
}

// LSTATUS RegSetValueExW(HKEY key, LPCWSTR name, DWORD reserved, DWORD type, const BYTE *data, DWORD cb)
// Accept registry writes without persistent storage.
fn hle_reg_set_value_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(24)
}

// LSTATUS RegSetValueExA(HKEY key, LPCSTR name, DWORD reserved, DWORD type, const BYTE *data, DWORD cb)
// Accept registry writes without persistent storage.
fn hle_reg_set_value_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(24)
}

// LSTATUS RegEnumValueA(HKEY key, DWORD index, LPSTR value, DWORD *value_len, DWORD *reserved, DWORD *type, BYTE *data, DWORD *data_len)
// Report an empty key so legacy option scanners fall back to defaults.
fn hle_reg_enum_value_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const ERROR_NO_MORE_ITEMS: u32 = 259;
    let value_len = arg(emu, 3);
    if value_len != 0 {
        emu.memory.write_u32(value_len, 0).hle();
    }
    ret(emu, ERROR_NO_MORE_ITEMS);
    HleResult::Retn(32)
}

// LSTATUS RegEnumKeyExA(HKEY key, DWORD index, LPSTR name, DWORD *name_len, DWORD *reserved, LPSTR class, DWORD *class_len, FILETIME *time)
// Report no subkeys for the fake registry namespace.
fn hle_reg_enum_key_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const ERROR_NO_MORE_ITEMS: u32 = 259;
    let name_len = arg(emu, 3);
    if name_len != 0 {
        emu.memory.write_u32(name_len, 0).hle();
    }
    ret(emu, ERROR_NO_MORE_ITEMS);
    HleResult::Retn(32)
}

// LSTATUS RegDeleteKeyA(HKEY key, LPCSTR subkey)
// Accept deletion from the fake volatile registry.
fn hle_reg_delete_key_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// LSTATUS RegDeleteValueA(HKEY key, LPCSTR value)
// Accept deletion from the fake volatile registry.
fn hle_reg_delete_value_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// LSTATUS RegDeleteValueW(HKEY key, LPCWSTR value)
// Accept deletion from the fake volatile registry.
fn hle_reg_delete_value_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// LSTATUS RegFlushKey(HKEY key)
// Accept flush for the fake volatile registry.
fn hle_reg_flush_key(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// BOOL OpenProcessToken(HANDLE process, DWORD access, HANDLE *token)
// Return a fake token handle for privilege adjustment probes.
fn hle_open_process_token(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory.write_u32(out, 0x5100_1000).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL LookupPrivilegeValueA(LPCSTR system, LPCSTR name, LUID *luid)
// Return a deterministic fake LUID for privilege names.
fn hle_lookup_privilege_value_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory.write_u32(out, 1).hle();
        emu.memory.write_u32(out + 4, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL AdjustTokenPrivileges(HANDLE token, BOOL disable_all, TOKEN_PRIVILEGES *new_state, DWORD len, TOKEN_PRIVILEGES *prev, DWORD *needed)
// Accept privilege adjustment probes without changing emulator policy.
fn hle_adjust_token_privileges(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let needed = arg(emu, 5);
    if needed != 0 {
        emu.memory.write_u32(needed, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(24)
}
