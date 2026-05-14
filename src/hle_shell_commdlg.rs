// BOOL Shell_NotifyIconA(DWORD message, NOTIFYICONDATAA *data)
// Accept tray icon add/update/delete requests without shell UI.
fn hle_shell_notify_icon_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// int ShellAboutA(HWND hwnd, LPCSTR app, LPCSTR other, HICON icon)
// Accept About-box requests without opening host UI.
fn hle_shell_about_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(16)
}

// HINSTANCE ShellExecuteA(HWND hwnd, LPCSTR op, LPCSTR file, LPCSTR params, LPCSTR dir, INT show)
// Pretend the shell accepted the launch while keeping host shell access contained.
fn hle_shell_execute_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 33);
    HleResult::Retn(24)
}

// HINSTANCE FindExecutableA(LPCSTR file, LPCSTR dir, LPSTR result)
// Report no shell association for guest-side documents and URLs.
fn hle_find_executable_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory.write_u8(out, 0).hle();
    }
    ret(emu, 31);
    HleResult::Retn(12)
}

// DWORD CommDlgExtendedError(void)
// Report no pending common-dialog extended error.
fn hle_comm_dlg_extended_error(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// BOOL SHGetSpecialFolderPathW(HWND hwnd, LPWSTR out, int csidl, BOOL create)
// Return stable guest shell folders, following Wine's BOOL wrapper over SHGetFolderPathW.
fn hle_sh_get_special_folder_path_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    let csidl = arg(emu, 2) & 0x00ff;
    let Some(path) = special_folder_path(csidl) else {
        emu.hle.last_error = 2;
        ret(emu, 0);
        return HleResult::Retn(16);
    };
    if out != 0 {
        emu.memory.write_utf16z(out, path, 260).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

fn special_folder_path(csidl: u32) -> Option<&'static str> {
    match csidl {
        0x0000 => Some("C:\\Documents and Settings\\wemu\\Desktop"),
        0x0002 => Some("C:\\Documents and Settings\\wemu\\Start Menu\\Programs"),
        0x0005 => Some("C:\\Documents and Settings\\wemu\\My Documents"),
        0x0006 => Some("C:\\Documents and Settings\\wemu\\Favorites"),
        0x0007 => Some("C:\\Documents and Settings\\wemu\\Start Menu\\Programs\\Startup"),
        0x0008 => Some("C:\\Documents and Settings\\wemu\\Recent"),
        0x0009 => Some("C:\\Documents and Settings\\wemu\\SendTo"),
        0x000b => Some("C:\\Documents and Settings\\wemu\\Start Menu"),
        0x000d => Some("C:\\Documents and Settings\\wemu\\My Documents\\My Music"),
        0x0010 => Some("C:\\Documents and Settings\\wemu\\Desktop"),
        0x001a => Some("C:\\Documents and Settings\\wemu\\Application Data"),
        0x001c => Some("C:\\Documents and Settings\\wemu\\Local Settings\\Application Data"),
        0x0023 => Some("C:\\Documents and Settings\\All Users\\Application Data"),
        0x0024 => Some("C:\\WINDOWS"),
        0x0025 => Some("C:\\WINDOWS\\SYSTEM32"),
        0x0026 => Some("C:\\Program Files"),
        0x0027 => Some("C:\\Documents and Settings\\wemu\\My Documents\\My Pictures"),
        0x0028 => Some("C:\\Documents and Settings\\wemu"),
        0x002b => Some("C:\\Program Files\\Common Files"),
        _ => None,
    }
}

// HRESULT CoInitialize(void *reserved)
// Accept COM initialization for shell integration probes.
fn hle_co_initialize(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// void CoUninitialize(void)
// Accept COM teardown for shell integration probes.
fn hle_co_uninitialize(_emu: &mut Emulator, _: &HleEntry) -> HleResult {
    HleResult::Retn(0)
}

// HRESULT OleInitialize(void *reserved)
// Accept OLE initialization for shell integration probes.
fn hle_ole_initialize(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// void OleUninitialize(void)
// Accept OLE teardown for shell integration probes.
fn hle_ole_uninitialize(_emu: &mut Emulator, _: &HleEntry) -> HleResult {
    HleResult::Retn(0)
}

// HRESULT CoCreateInstance(REFCLSID clsid, IUnknown *outer, DWORD context, REFIID iid, void **out)
// Report unavailable COM classes while clearing the output pointer.
fn hle_co_create_instance(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 4);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 0x8004_0154);
    HleResult::Retn(20)
}
