// MCIERROR mciSendStringA(LPCSTR cmd, LPSTR ret, UINT ret_len, HWND cb)
// Return simple MCI status text and complete video notifications.
fn hle_mci_send_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const MM_MCINOTIFY: u32 = 0x03b9;
    const MCI_NOTIFY_SUCCESSFUL: u32 = 1;

    let command_ptr = arg(emu, 0);
    let ret_buf = arg(emu, 1);
    let ret_len = arg(emu, 2);
    let hwnd_callback = arg(emu, 3);
    let command = emu.memory.cstr_lossy(command_ptr, 512).hle();
    let lower = command.to_ascii_lowercase();

    if emu.trace {
        eprintln!(
            "mciSendStringA command={command:?} ret_buf={ret_buf:08x} ret_len={ret_len} hwnd={hwnd_callback:08x}"
        );
    }

    if ret_buf != 0 && ret_len != 0 {
        let value = mci_status_reply(&lower);
        emu.memory.write_cstr(ret_buf, value, ret_len as usize).hle();
    }

    // Video startup sequences often wait on MM_MCINOTIFY; background music should not.
    // With no sound backend, completing MIDI instantly creates a tight playlist loop.
    if hwnd_callback != 0 && mci_should_complete_notify(&lower) {
        let message = Message {
            hwnd: hwnd_callback,
            msg: MM_MCINOTIFY,
            wparam: MCI_NOTIFY_SUCCESSFUL,
            lparam: 0,
        };
        emu.hle.app_messages.push(message);
        emu.hle.note_queued_message("mciNotify", message);
    }

    ret(emu, 0);
    HleResult::Retn(16)
}

// MCIDEVICEID mciGetDeviceIDA(LPCSTR device)
// Return a stable fake MCI device id.
fn hle_mci_get_device_id_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// MCIERROR mciSendCommandA(MCIDEVICEID id, UINT msg, DWORD flags, DWORD_PTR params)
// Accept command-style MCI calls without starting real audio playback.
fn hle_mci_send_command_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(16)
}

// BOOL mciGetErrorStringA(MCIERROR err, LPSTR out, UINT len)
// Return a short empty diagnostic string for ignored MCI errors.
fn hle_mci_get_error_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    if out != 0 && len != 0 {
        emu.memory.write_u8(out, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL ICInfo(DWORD fccType, DWORD fccHandler, ICINFO *info)
// Report no installable video codecs for optional intro playback.
fn hle_ic_info(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(12)
}

fn mci_should_complete_notify(command: &str) -> bool {
    command.contains(" notify")
        && !command.starts_with("play mid ")
        && !command.starts_with("play cdtrack ")
}

fn mci_status_reply(command: &str) -> &'static str {
    if command.contains("number of tracks") || command.contains("current track") {
        "1"
    } else if command.contains("mode") {
        "stopped"
    } else {
        "0"
    }
}
