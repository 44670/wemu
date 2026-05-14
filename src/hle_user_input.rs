// ATOM RegisterClassA(const WNDCLASSA *wc)
// Capture the class window procedure and return a fake atom.
fn hle_register_class_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let class = arg(emu, 0);
    let atom = register_window_class_common(emu, class, 4, 16, 32, 36, false);
    ret(emu, atom);
    HleResult::Retn(4)
}

// ATOM RegisterClassW(const WNDCLASSW *wc)
// Capture the class window procedure and return a fake atom.
fn hle_register_class_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let class = arg(emu, 0);
    let atom = register_window_class_common(emu, class, 4, 16, 32, 36, true);
    ret(emu, atom);
    HleResult::Retn(4)
}

// BOOL TranslateMessage(const MSG *msg)
// Accept keyboard translation without generating extra messages.
fn hle_translate_message(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// HWND SetCapture(HWND hwnd)
// Track the window that should receive mouse messages during a drag.
fn hle_set_capture(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let old = emu.hle.capture_window;
    emu.hle.capture_window = hwnd;
    ret(emu, old);
    HleResult::Retn(4)
}

// BOOL ReleaseCapture(void)
// Clear the tracked mouse capture owner.
fn hle_release_capture(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.capture_window = 0;
    ret(emu, 1);
    HleResult::Retn(0)
}

// HWND GetCapture(void)
// Return the current tracked mouse-capture owner.
fn hle_get_capture(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.capture_window);
    HleResult::Retn(0)
}

// BOOL GetKeyboardState(PBYTE keys)
// Copy the frontend-driven virtual-key state table.
fn hle_get_keyboard_state(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        for key in 0..256u32 {
            emu.memory
                .write_u8(out + key, emu.hle.keyboard_state_byte(key))
                .hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// SHORT GetKeyState(int key)
// Report the current frontend-driven virtual-key state.
fn hle_get_key_state(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let key = arg(emu, 0);
    let consume_press = entry.name == "GetAsyncKeyState";
    let value = emu.hle.key_state_word(key, consume_press);
    ret(emu, value);
    HleResult::Retn(4)
}

// BOOL GetKeyboardLayoutNameA(LPSTR out)
// Return the stable US English keyboard layout identifier.
fn hle_get_keyboard_layout_name_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_cstr(out, "00000409", 9).hle();
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL GetKeyboardLayoutNameW(LPWSTR out)
// Return the stable US English keyboard layout identifier.
fn hle_get_keyboard_layout_name_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_utf16z(out, "00000409", 9).hle();
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// UINT MapVirtualKeyA(UINT code, UINT map_type)
// Translate common Win32 virtual keys, scan codes, and unshifted character values.
fn hle_map_virtual_key_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    map_virtual_key_impl(emu);
    HleResult::Retn(8)
}

// UINT MapVirtualKeyW(UINT code, UINT map_type)
// Translate common Win32 virtual keys, scan codes, and unshifted character values.
fn hle_map_virtual_key_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    map_virtual_key_impl(emu);
    HleResult::Retn(8)
}

// int GetKeyNameTextA(LONG lparam, LPSTR out, int cch)
// Write an ANSI key name derived from the scan-code bits in a key-message lParam.
fn hle_get_key_name_text_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = key_name_text(arg(emu, 0));
    let out = arg(emu, 1);
    let cch = arg(emu, 2) as usize;
    let written = if out != 0 && cch != 0 {
        emu.memory.write_cstr(out, &name, cch).hle()
    } else {
        0
    };
    ret(emu, written);
    HleResult::Retn(12)
}

// int GetKeyNameTextW(LONG lparam, LPWSTR out, int cch)
// Write a UTF-16 key name derived from the scan-code bits in a key-message lParam.
fn hle_get_key_name_text_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = key_name_text(arg(emu, 0));
    let out = arg(emu, 1);
    let cch = arg(emu, 2) as usize;
    let written = if out != 0 && cch != 0 {
        emu.memory.write_utf16z(out, &name, cch).hle()
    } else {
        0
    };
    ret(emu, written);
    HleResult::Retn(12)
}

fn map_virtual_key_impl(emu: &mut Emulator) {
    const MAPVK_VK_TO_VSC: u32 = 0;
    const MAPVK_VSC_TO_VK: u32 = 1;
    const MAPVK_VK_TO_CHAR: u32 = 2;
    const MAPVK_VSC_TO_VK_EX: u32 = 3;
    const MAPVK_VK_TO_VSC_EX: u32 = 4;

    let code = arg(emu, 0);
    let map_type = arg(emu, 1);
    let value = match map_type {
        MAPVK_VK_TO_VSC | MAPVK_VK_TO_VSC_EX => vk_to_scan_code(code),
        MAPVK_VSC_TO_VK | MAPVK_VSC_TO_VK_EX => scan_code_to_vk(code),
        MAPVK_VK_TO_CHAR => vk_to_unshifted_char(code),
        _ => 0,
    };
    ret(emu, value);
}

fn key_name_text(lparam: u32) -> String {
    let scan_code = (lparam >> 16) & 0xff;
    let extended = (lparam & 0x0100_0000) != 0;
    let vk = scan_code_to_vk(scan_code);
    let name = match vk {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0d if extended => "Num Enter",
        0x0d => "Enter",
        0x10 => "Shift",
        0x11 => "Ctrl",
        0x12 => "Alt",
        0x13 => "Pause",
        0x14 => "Caps Lock",
        0x1b => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2d => "Insert",
        0x2e => "Delete",
        0x60 => "Num 0",
        0x61 => "Num 1",
        0x62 => "Num 2",
        0x63 => "Num 3",
        0x64 => "Num 4",
        0x65 => "Num 5",
        0x66 => "Num 6",
        0x67 => "Num 7",
        0x68 => "Num 8",
        0x69 => "Num 9",
        0x6a => "Num *",
        0x6b => "Num +",
        0x6d => "Num -",
        0x6e => "Num .",
        0x6f => "Num /",
        0x70..=0x7b => return format!("F{}", vk - 0x6f),
        0x90 => "Num Lock",
        0x91 => "Scroll Lock",
        0xba => ";",
        0xbb => "=",
        0xbc => ",",
        0xbd => "-",
        0xbe => ".",
        0xbf => "/",
        0xc0 => "`",
        0xdb => "[",
        0xdc => "\\",
        0xdd => "]",
        0xde => "'",
        0x30..=0x39 | 0x41..=0x5a => return char::from_u32(vk).unwrap_or('?').to_string(),
        _ => "",
    };
    name.to_string()
}

fn vk_to_scan_code(vk: u32) -> u32 {
    match vk {
        0x08 => 0x0e,
        0x09 => 0x0f,
        0x0d => 0x1c,
        0x10 => 0x2a,
        0x11 => 0x1d,
        0x12 => 0x38,
        0x13 => 0x45,
        0x14 => 0x3a,
        0x1b => 0x01,
        0x20 => 0x39,
        0x21 => 0x49,
        0x22 => 0x51,
        0x23 => 0x4f,
        0x24 => 0x47,
        0x25 => 0x4b,
        0x26 => 0x48,
        0x27 => 0x4d,
        0x28 => 0x50,
        0x2d => 0x52,
        0x2e => 0x53,
        0x30..=0x39 => [0x0b, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a]
            [(vk - 0x30) as usize],
        0x41..=0x5a => [
            0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32,
            0x31, 0x18, 0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
        ][(vk - 0x41) as usize],
        0x60..=0x69 => [0x52, 0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47, 0x48, 0x49]
            [(vk - 0x60) as usize],
        0x6a => 0x37,
        0x6b => 0x4e,
        0x6d => 0x4a,
        0x6e => 0x53,
        0x6f => 0x35,
        0x70..=0x79 => 0x3b + (vk - 0x70),
        0x7a => 0x57,
        0x7b => 0x58,
        0x90 => 0x45,
        0x91 => 0x46,
        0xba => 0x27,
        0xbb => 0x0d,
        0xbc => 0x33,
        0xbd => 0x0c,
        0xbe => 0x34,
        0xbf => 0x35,
        0xc0 => 0x29,
        0xdb => 0x1a,
        0xdc => 0x2b,
        0xdd => 0x1b,
        0xde => 0x28,
        _ => 0,
    }
}

fn scan_code_to_vk(scan_code: u32) -> u32 {
    match scan_code & 0xff {
        0x01 => 0x1b,
        0x02..=0x0a => 0x31 + ((scan_code & 0xff) - 0x02),
        0x0b => 0x30,
        0x0c => 0xbd,
        0x0d => 0xbb,
        0x0e => 0x08,
        0x0f => 0x09,
        0x10 => 0x51,
        0x11 => 0x57,
        0x12 => 0x45,
        0x13 => 0x52,
        0x14 => 0x54,
        0x15 => 0x59,
        0x16 => 0x55,
        0x17 => 0x49,
        0x18 => 0x4f,
        0x19 => 0x50,
        0x1a => 0xdb,
        0x1b => 0xdd,
        0x1c => 0x0d,
        0x1d => 0x11,
        0x1e => 0x41,
        0x1f => 0x53,
        0x20 => 0x44,
        0x21 => 0x46,
        0x22 => 0x47,
        0x23 => 0x48,
        0x24 => 0x4a,
        0x25 => 0x4b,
        0x26 => 0x4c,
        0x27 => 0xba,
        0x28 => 0xde,
        0x29 => 0xc0,
        0x2a => 0x10,
        0x2b => 0xdc,
        0x2c => 0x5a,
        0x2d => 0x58,
        0x2e => 0x43,
        0x2f => 0x56,
        0x30 => 0x42,
        0x31 => 0x4e,
        0x32 => 0x4d,
        0x33 => 0xbc,
        0x34 => 0xbe,
        0x35 => 0xbf,
        0x38 => 0x12,
        0x39 => 0x20,
        0x3a => 0x14,
        0x3b..=0x44 => 0x70 + ((scan_code & 0xff) - 0x3b),
        0x45 => 0x90,
        0x46 => 0x91,
        0x47 => 0x24,
        0x48 => 0x26,
        0x49 => 0x21,
        0x4b => 0x25,
        0x4d => 0x27,
        0x4f => 0x23,
        0x50 => 0x28,
        0x51 => 0x22,
        0x52 => 0x2d,
        0x53 => 0x2e,
        0x57 => 0x7a,
        0x58 => 0x7b,
        _ => 0,
    }
}

fn vk_to_unshifted_char(vk: u32) -> u32 {
    match vk {
        0x30..=0x39 | 0x41..=0x5a => vk,
        0x60..=0x69 => vk - 0x60 + b'0' as u32,
        0x6a => b'*' as u32,
        0x6b => b'+' as u32,
        0x6d => b'-' as u32,
        0x6e => b'.' as u32,
        0x6f => b'/' as u32,
        0xba => b';' as u32,
        0xbb => b'=' as u32,
        0xbc => b',' as u32,
        0xbd => b'-' as u32,
        0xbe => b'.' as u32,
        0xbf => b'/' as u32,
        0xc0 => b'`' as u32,
        0xdb => b'[' as u32,
        0xdc => b'\\' as u32,
        0xdd => b']' as u32,
        0xde => b'\'' as u32,
        _ => 0,
    }
}

// BOOL GetCursorPos(LPPOINT lpPoint)
// Return the current HLE cursor position for input replay.
fn hle_get_cursor_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let point = arg(emu, 0);
    if point != 0 {
        emu.memory.write_u32(point, emu.hle.cursor_x).hle();
        emu.memory.write_u32(point + 4, emu.hle.cursor_y).hle();
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL SetCursorPos(int X, int Y)
// Move the HLE cursor without generating input messages.
fn hle_set_cursor_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.cursor_x = arg(emu, 0);
    emu.hle.cursor_y = arg(emu, 1);
    ret(emu, 1);
    HleResult::Retn(8)
}

// HHOOK SetWindowsHookExA(int idHook, HOOKPROC proc, HINSTANCE hmod, DWORD tid)
// Record the hook and return a fake handle for USER callbacks that deliver it.
fn hle_set_windows_hook_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let id = arg(emu, 0) as i32;
    let proc = arg(emu, 1);
    let hmod = arg(emu, 2);
    let thread_id = arg(emu, 3);
    let handle = emu.hle.set_windows_hook(id, proc, hmod, thread_id);
    if emu.trace {
        eprintln!(
            "SetWindowsHookExA id={id} proc={proc:08x} hmod={hmod:08x} tid={thread_id:08x} -> {handle:08x}"
        );
    }
    ret(emu, handle);
    HleResult::Retn(16)
}

// BOOL UnhookWindowsHookEx(HHOOK hook)
// Remove a fake hook handle and report whether it was registered.
fn hle_unhook_windows_hook_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = arg(emu, 0);
    let removed = emu.hle.unhook_windows_hook(handle);
    ret(emu, if removed { 1 } else { 0 });
    HleResult::Retn(4)
}

// LRESULT CallNextHookEx(HHOOK hook, int code, WPARAM w, LPARAM l)
// Return zero because no chained hook callbacks are delivered by the HLE.
fn hle_call_next_hook_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(16)
}

// BOOL PeekMessageA(MSG *msg, HWND hwnd, UINT min, UINT max, UINT remove)
// Prefer input messages, then app messages, with SDL/timer rescheduling.
fn hle_peek_message_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let result = peek_message_impl(emu, entry, "PeekMessageA");
    debug_assert!(matches!(result, HleResult::Retn(20)));
    result
}

// BOOL PeekMessageW(MSG *msg, HWND hwnd, UINT min, UINT max, UINT remove)
// Use the same numeric message queue as ANSI; there is no string payload to convert.
fn hle_peek_message_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let result = peek_message_impl(emu, entry, "PeekMessageW");
    debug_assert!(matches!(result, HleResult::Retn(20)));
    result
}

fn peek_message_impl(emu: &mut Emulator, entry: &HleEntry, trace_name: &str) -> HleResult {
    let out = arg(emu, 0);
    let hwnd = arg(emu, 1);
    let min = arg(emu, 2);
    let max = arg(emu, 3);
    let remove = arg(emu, 4);
    if !has_matching_message(emu, hwnd, min, max) {
        emu.reschedule_message_pump().hle();
        if emu.stopped.is_some() {
            emu.hle.note_peek_message("none", remove, None);
            ret(emu, 0);
            return HleResult::Retn(20);
        }
    }
    if !emu.hle.has_input_messages() && dispatch_due_mm_timer_callback(emu, entry, 20, 0) {
        return HleResult::Retn(20);
    }
    let input_index = matching_message_index(emu, &emu.hle.input_messages, hwnd, min, max);
    let app_index = if input_index.is_none() {
        matching_message_index(emu, &emu.hle.app_messages, hwnd, min, max)
    } else {
        None
    };
    let message = input_index
        .map(|index| (true, index, emu.hle.input_messages[index]))
        .or_else(|| app_index.map(|index| (false, index, emu.hle.app_messages[index])));
    if let Some((from_input, index, message)) = message {
        let source = if from_input { "input" } else { "app" };
        emu.hle.note_peek_message(source, remove, Some(message));
        if out != 0 {
            write_msg(emu, out, message);
        }
        emu.hle.note_generated_paint_delivered(message);
        if (remove & 1) != 0 {
            if from_input {
                emu.hle.input_messages.remove(index);
                emu.hle.note_removed_message(source, message);
            } else if message.msg != 0x000f {
                emu.hle.app_messages.remove(index);
                emu.hle.note_removed_message(source, message);
            } else {
            }
        }
        if emu.trace {
            eprintln!(
                "{} -> msg={:08x} hwnd={:08x} w={:08x} l={:08x}",
                trace_name, message.msg, message.hwnd, message.wparam, message.lparam
            );
        }
        ret(emu, 1);
    } else {
        emu.hle.note_peek_message("none", remove, None);
        emu.hle.note_cooperative_idle();
        ret(emu, 0);
    }
    HleResult::Retn(20)
}

// int GetKeyboardType(int typeFlag)
// Return stable IBM-enhanced keyboard metadata.
fn hle_get_keyboard_type(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = match arg(emu, 0) {
        0 => 4,
        1 => 0,
        2 => 12,
        _ => 0,
    };
    ret(emu, value);
    HleResult::Retn(4)
}

// ATOM RegisterClassExW(const WNDCLASSEXW *wc)
// Capture the class window procedure and return a fake atom.
fn hle_register_class_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let class = arg(emu, 0);
    let atom = register_window_class_common(emu, class, 8, 20, 36, 40, true);
    ret(emu, atom);
    HleResult::Retn(4)
}

// ATOM RegisterClassExA(const WNDCLASSEXA *wc)
// Capture the class window procedure and return a fake atom.
fn hle_register_class_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let class = arg(emu, 0);
    let atom = register_window_class_common(emu, class, 8, 20, 36, 40, false);
    ret(emu, atom);
    HleResult::Retn(4)
}

// BOOL UnregisterClassA(LPCSTR class_name, HINSTANCE inst)
// Remove a tracked ANSI window class registration when present.
fn hle_unregister_class_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    unregister_class_impl(emu, false);
    HleResult::Retn(8)
}

// BOOL UnregisterClassW(LPCWSTR class_name, HINSTANCE inst)
// Remove a tracked UTF-16 window class registration when present.
fn hle_unregister_class_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    unregister_class_impl(emu, true);
    HleResult::Retn(8)
}

fn unregister_class_impl(emu: &mut Emulator, wide: bool) {
    let class_ptr = arg(emu, 0);
    if class_ptr != 0 && class_ptr < 0x10000 {
        emu.hle.window_class_atoms.remove(&class_ptr);
        emu.hle.window_class_atom_menus.remove(&class_ptr);
        emu.hle.window_class_atom_backgrounds.remove(&class_ptr);
        ret(emu, 1);
        return;
    }
    let name = read_window_class_name(emu, class_ptr, wide).to_ascii_lowercase();
    if !name.is_empty() {
        emu.hle.window_class_procs.remove(&name);
        emu.hle.window_class_menus.remove(&name);
        emu.hle.window_class_backgrounds.remove(&name);
    }
    ret(emu, 1);
}

// HWND GetFocus(void)
// Return the current tracked focus window.
fn hle_get_focus(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.focus_window);
    HleResult::Retn(0)
}

// HWND SetFocus(HWND hwnd)
// Update the tracked focus window and return the previous focus.
fn hle_set_focus(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let old = emu.hle.focus_window;
    emu.hle.focus_window = hwnd;
    ret(emu, old);
    HleResult::Retn(4)
}

// BOOL SetForegroundWindow(HWND hwnd)
// Mark the requested window as focused in the single-frontend window manager.
fn hle_set_foreground_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.focus_window = arg(emu, 0);
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL BringWindowToTop(HWND hwnd)
// Mark the requested window as focused in the single-frontend window manager.
fn hle_bring_window_to_top(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.focus_window = arg(emu, 0);
    ret(emu, 1);
    HleResult::Retn(4)
}

// HWND GetActiveWindow(void)
// Return the current tracked focus window or the desktop fallback.
fn hle_get_active_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.focus_window);
    HleResult::Retn(0)
}
