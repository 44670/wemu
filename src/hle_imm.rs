// HIMC ImmGetContext(HWND hwnd)
// Return a stable fake input-method context for programs that probe IME state.
fn hle_imm_get_context(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x5200_3000);
    HleResult::Retn(4)
}

// HIMC ImmAssociateContext(HWND hwnd, HIMC himc)
// Accept context association changes and report the previous fake context.
fn hle_imm_associate_context(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x5200_3000);
    HleResult::Retn(8)
}

// BOOL ImmReleaseContext(HWND hwnd, HIMC himc)
// Accept release of the fake input-method context.
fn hle_imm_release_context(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL ImmSetOpenStatus(HIMC himc, BOOL open)
// Accept open-status changes without maintaining IME composition state.
fn hle_imm_set_open_status(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL ImmNotifyIME(HIMC himc, DWORD action, DWORD index, DWORD value)
// Accept IME notifications and leave composition state empty.
fn hle_imm_notify_ime(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(16)
}
