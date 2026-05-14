// void InitCommonControls(void)
// Accept common-control registration; built-in window stubs handle controls directly.
fn hle_init_common_controls(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// BOOL InitCommonControlsEx(const INITCOMMONCONTROLSEX *icc)
// Accept common-control registration; built-in window stubs handle controls directly.
fn hle_init_common_controls_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL ImageList_Destroy(HIMAGELIST image_list)
// Accept destruction of unmodeled common-control image lists.
fn hle_image_list_destroy(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// HWND CreateWindowExA(DWORD ex, LPCSTR cls, LPCSTR name, DWORD style, int x, int y, int w, int h, HWND parent, HMENU menu, HINSTANCE inst, void *param)
// Create a tracked ANSI window/control and deliver WM_CREATE for top-level windows.
fn hle_create_window_ex_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_window_ex_common(emu, entry, false)
}

// LRESULT DispatchMessageA(const MSG *msg)
// Transfer control to the guest window procedure or timer callback.
fn hle_dispatch_message_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let msg_ptr = arg(emu, 0);
    let hwnd = emu.memory.read_u32(msg_ptr).hle();
    let msg = emu.memory.read_u32(msg_ptr + 4).hle();
    let wparam = emu.memory.read_u32(msg_ptr + 8).hle();
    let lparam = emu.memory.read_u32(msg_ptr + 12).hle();
    let (proc, callback_lparam) = dispatch_target(emu, hwnd, msg, lparam);
    let message = Message {
        hwnd,
        msg,
        wparam,
        lparam,
    };
    if let Some(value) = dispatch_tracked_control_message(emu, message) {
        note_dispatched_message(emu, message, 0);
        ret(emu, value);
        return HleResult::Retn(4);
    }
    note_dispatched_message(emu, message, proc);
    if emu.trace {
        eprintln!(
            "DispatchMessageA msg={msg:08x} hwnd={hwnd:08x} w={wparam:08x} l={lparam:08x} wndproc={proc:08x}"
        );
    }
    if proc != 0 {
        dispatch_message_tail_call(emu, entry, hwnd, proc, msg, wparam, callback_lparam);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(4)
}

fn dispatch_message_tail_call(
    emu: &mut Emulator,
    entry: &HleEntry,
    hwnd: u32,
    proc: u32,
    msg: u32,
    wparam: u32,
    lparam: u32,
) {
    let hle_esp = emu.cpu.reg(Reg::Esp);
    let ret_addr = emu.memory.read_u32(hle_esp).hle();
    let wnd_esp = hle_esp.wrapping_sub(12);
    emu.memory.write_u32(wnd_esp, ret_addr).hle();
    emu.memory.write_u32(wnd_esp + 4, hwnd).hle();
    emu.memory.write_u32(wnd_esp + 8, msg).hle();
    emu.memory.write_u32(wnd_esp + 12, wparam).hle();
    emu.memory.write_u32(wnd_esp + 16, lparam).hle();
    emu.cpu
        .debug_replace_top_call(entry.addr, proc, ret_addr, wnd_esp + 4, wnd_esp)
        .hle();
    emu.cpu.set_reg(Reg::Esp, wnd_esp);
    emu.cpu.eip = proc;
}

fn note_dispatched_message(emu: &mut Emulator, message: Message, proc: u32) {
    if let Some(report) = emu.hle.note_dispatched_message(message, proc) {
        emit_message_flood_report(emu, report);
    }
}

fn emit_message_flood_report(emu: &Emulator, report: MessageFloodReport) {
    let extra = format!(
        ",\"msg\":{},\"proc\":{},\"cnt\":{},\"hwnd\":{},\"inq\":{},\"appq\":{},\"recent\":[{},{},{},{},{},{},{},{},{},{}]",
        report.msg,
        report.proc,
        report.count,
        report.hwnd,
        report.input_len,
        report.app_len,
        report.recent[0],
        report.recent[1],
        report.recent[2],
        report.recent[3],
        report.recent[4],
        report.recent[5],
        report.recent[6],
        report.recent[7],
        report.recent[8],
        report.recent[9],
    );
    emu.emit_abnormal_report("msg_flood", &extra);
}

// int TranslateAcceleratorA(HWND hwnd, HACCEL accel, LPMSG msg)
// Convert matching accelerator key messages into queued WM_COMMAND messages.
fn hle_translate_accelerator_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    translate_accelerator_impl(emu);
    HleResult::Retn(12)
}

// int TranslateAcceleratorW(HWND hwnd, HACCEL accel, LPMSG msg)
// Convert matching accelerator key messages into queued WM_COMMAND messages.
fn hle_translate_accelerator_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    translate_accelerator_impl(emu);
    HleResult::Retn(12)
}

fn translate_accelerator_impl(emu: &mut Emulator) {
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WM_COMMAND: u32 = 0x0111;

    let hwnd_arg = arg(emu, 0);
    let accel = arg(emu, 1);
    let msg_ptr = arg(emu, 2);
    let Some(table) = emu.hle.accelerator_table(accel).cloned() else {
        ret(emu, 0);
        return;
    };
    if msg_ptr == 0 {
        ret(emu, 0);
        return;
    }
    let hwnd_msg = emu.memory.read_u32(msg_ptr).hle();
    let msg = emu.memory.read_u32(msg_ptr + 4).hle();
    let wparam = emu.memory.read_u32(msg_ptr + 8).hle();
    if msg != WM_KEYDOWN && msg != WM_SYSKEYDOWN {
        ret(emu, 0);
        return;
    }
    if let Some(item) = table
        .items
        .iter()
        .find(|item| accelerator_matches(&emu.hle, **item, wparam))
    {
        let hwnd = if hwnd_arg != 0 { hwnd_arg } else { hwnd_msg };
        let message = Message {
            hwnd,
            msg: WM_COMMAND,
            wparam: item.cmd as u32,
            lparam: 0,
        };
        emu.hle.app_messages.push(message);
        emu.hle.note_queued_message("accelerator", message);
        ret(emu, 1);
    } else {
        ret(emu, 0);
    }
}

// BOOL AdjustWindowRect(LPRECT rect, DWORD style, BOOL menu)
// Expand a client rectangle to the dimensions of the HLE top-level frame.
fn hle_adjust_window_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = arg(emu, 0);
    let style = arg(emu, 1);
    let menu = arg(emu, 2);
    adjust_window_rect_impl(emu, rect, style, menu, 0);
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL AdjustWindowRectEx(LPRECT rect, DWORD style, BOOL menu, DWORD ex_style)
// Expand a client rectangle to match the HLE frame/client conversion.
fn hle_adjust_window_rect_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = arg(emu, 0);
    let style = arg(emu, 1);
    let menu = arg(emu, 2);
    let ex_style = arg(emu, 3);
    adjust_window_rect_impl(emu, rect, style, menu, ex_style);
    ret(emu, 1);
    HleResult::Retn(16)
}

fn adjust_window_rect_impl(emu: &mut Emulator, rect: u32, style: u32, menu: u32, _ex_style: u32) {
    if rect != 0 && style_has_hle_frame(style) {
        let mut value = read_gdi_rect(emu, rect);
        value.0 = value.0.saturating_sub(DIALOG_BORDER);
        value.1 = value.1.saturating_sub(DIALOG_TITLE_HEIGHT);
        value.2 = value.2.saturating_add(DIALOG_BORDER);
        if menu != 0 {
            value.1 = value.1.saturating_sub(MENU_BAR_HEIGHT);
        }
        value.3 = value.3.saturating_add(DIALOG_BORDER);
        write_gdi_rect(emu, rect, value);
    }
}

// BOOL TrackPopupMenu(HMENU menu, UINT flags, int x, int y, int reserved, HWND hwnd, const RECT *rect)
// Draw a tracked popup menu; item clicks later post WM_COMMAND to the owner.
fn hle_track_popup_menu(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const TPM_RETURNCMD: u32 = 0x0000_0100;
    let menu = arg(emu, 0);
    let flags = arg(emu, 1);
    let x = arg(emu, 2) as i32;
    let y = arg(emu, 3) as i32;
    let hwnd = arg(emu, 5);
    if let Some(popup) = build_popup_menu(emu, menu, hwnd, x, y) {
        emu.hle.active_popup_menu = Some(popup);
        render_active_popup_menu_overlay(emu);
        ret(emu, if (flags & TPM_RETURNCMD) != 0 { 0 } else { 1 });
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(28)
}

// BOOL OpenClipboard(HWND owner)
// Accept clipboard access; clipboard payloads are not modeled.
fn hle_open_clipboard(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL CloseClipboard(void)
// Accept closing the fake clipboard.
fn hle_close_clipboard(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(0)
}

// HANDLE GetClipboardData(UINT format)
// Report no clipboard payload for deterministic frontend runs.
fn hle_get_clipboard_data(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// UINT GetCaretBlinkTime(void)
// Return a stable default blink interval in milliseconds.
fn hle_get_caret_blink_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 500);
    HleResult::Retn(0)
}

// HWND CreateWindowExW(DWORD ex, LPCWSTR cls, LPCWSTR name, DWORD style, int x, int y, int w, int h, HWND parent, HMENU menu, HINSTANCE inst, void *param)
// Create a tracked wide window/control and deliver WM_CREATE for top-level windows.
fn hle_create_window_ex_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_window_ex_common(emu, entry, true)
}

// HWND CreateDialogIndirectParamA(HINSTANCE inst, LPCDLGTEMPLATE tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Create a tracked indirect-template dialog and synchronously deliver WM_INITDIALOG.
fn hle_create_dialog_indirect_param_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_dialog_indirect_common(emu, entry, true)
}

// HWND CreateDialogIndirectParamW(HINSTANCE inst, LPCDLGTEMPLATE tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Create a tracked indirect-template dialog and synchronously deliver WM_INITDIALOG.
fn hle_create_dialog_indirect_param_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_dialog_indirect_common(emu, entry, true)
}

// INT_PTR DialogBoxIndirectParamA(HINSTANCE inst, LPCDLGTEMPLATE tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Create a tracked modal-style dialog and return after the WM_INITDIALOG callback.
fn hle_dialog_box_indirect_param_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_dialog_indirect_common(emu, entry, false)
}

// INT_PTR DialogBoxIndirectParamW(HINSTANCE inst, LPCDLGTEMPLATE tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Create a tracked modal-style dialog and return after the WM_INITDIALOG callback.
fn hle_dialog_box_indirect_param_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_dialog_indirect_common(emu, entry, false)
}

// BOOL EnumChildWindows(HWND parent, WNDENUMPROC proc, LPARAM param)
// Accept child-window enumeration requests for currently tracked controls.
fn hle_enum_child_windows(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL CheckRadioButton(HWND dlg, int first, int last, int check)
// Accept radio-group state changes until per-control check state is modeled.
fn hle_check_radio_button(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL CheckDlgButton(HWND dlg, int id, UINT check)
// Accept checkbox state changes until per-control check state is modeled.
fn hle_check_dlg_button(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(12)
}

// UINT IsDlgButtonChecked(HWND dlg, int id)
// Report unchecked by default until per-control check state is modeled.
fn hle_is_dlg_button_checked(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// HMENU GetMenu(HWND hwnd)
// Return the tracked menu handle attached to a top-level window.
fn hle_get_menu(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let menu = emu
        .hle
        .window(hwnd)
        .and_then(|window| {
            (window.parent == 0 && emu.hle.menus.contains_key(&window.id)).then_some(window.id)
        })
        .unwrap_or(0);
    ret(emu, menu);
    HleResult::Retn(4)
}

// BOOL SetMenu(HWND hwnd, HMENU menu)
// Attach or remove a tracked menu handle on a top-level window.
fn hle_set_menu(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let menu = arg(emu, 1);
    if menu != 0 && !emu.hle.menus.contains_key(&menu) {
        ret(emu, 0);
        return HleResult::Retn(8);
    }
    let ok = emu
        .hle
        .window_mut(hwnd)
        .map(|window| {
            if window.parent != 0 {
                return false;
            }
            window.id = menu;
            true
        })
        .unwrap_or(false);
    if ok {
        render_hle_windows(emu);
    }
    ret(emu, ok as u32);
    HleResult::Retn(8)
}

// BOOL GetMenuItemRect(HWND hwnd, HMENU menu, UINT item, RECT *rect)
// Return screen-space rectangles for tracked menu-bar or active popup items.
fn hle_get_menu_item_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd_arg = arg(emu, 0);
    let menu = arg(emu, 1);
    let index = arg(emu, 2) as usize;
    let rect = arg(emu, 3);
    if rect == 0 {
        ret(emu, 0);
        return HleResult::Retn(16);
    }
    let hwnd = if hwnd_arg != 0 {
        hwnd_arg
    } else {
        emu.hle
            .windows
            .values()
            .find(|window| window.parent == 0 && window.id == menu)
            .map(|window| window.hwnd)
            .unwrap_or(0)
    };
    let item_rect = menu_bar_item_rect(emu, hwnd, menu, index).or_else(|| {
        emu.hle
            .active_popup_menu
            .as_ref()
            .filter(|popup| popup.owner == hwnd)
            .and_then(|popup| popup.items.get(index))
            .map(|item| item.rect)
    });
    if let Some(item_rect) = item_rect {
        write_gdi_rect(
            emu,
            rect,
            (
                item_rect.left,
                item_rect.top,
                item_rect.right,
                item_rect.bottom,
            ),
        );
        ret(emu, 1);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(16)
}

fn menu_bar_item_rect(
    emu: &Emulator,
    hwnd: u32,
    menu_handle: u32,
    target_index: usize,
) -> Option<WindowRect> {
    let window = emu.hle.window(hwnd)?;
    if emu.hle.window_menu_handle(window)? != menu_handle {
        return None;
    }
    let bar = emu.hle.menu_bar_rect_for_window(window)?;
    let menu = emu.hle.menu(menu_handle)?;
    let mut left = bar.left + 4;
    for (index, item) in menu.items.iter().enumerate() {
        if item.separator {
            if index == target_index {
                return None;
            }
            continue;
        }
        let width = menu_bar_item_width(&item.text);
        let rect = WindowRect {
            left,
            top: bar.top,
            right: (left + width).min(bar.right),
            bottom: bar.bottom,
        };
        if index == target_index {
            return (!rect.is_empty()).then_some(rect);
        }
        left += width;
        if left >= bar.right {
            break;
        }
    }
    None
}

// HMENU GetSubMenu(HMENU menu, int pos)
// Return the submenu handle at the requested zero-based menu position.
fn hle_get_sub_menu(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let menu = arg(emu, 0);
    let pos = arg(emu, 1) as usize;
    let submenu = emu
        .hle
        .menu(menu)
        .and_then(|menu| menu.items.get(pos))
        .map(|item| item.submenu)
        .unwrap_or(0);
    ret(emu, submenu);
    HleResult::Retn(8)
}

// BOOL DeleteMenu(HMENU menu, UINT pos, UINT flags)
// Remove a tracked menu item by position or command identifier.
fn hle_delete_menu(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let removed = remove_menu_item(emu, arg(emu, 0), arg(emu, 1), arg(emu, 2));
    ret(emu, removed as u32);
    HleResult::Retn(12)
}

// BOOL RemoveMenu(HMENU menu, UINT pos, UINT flags)
// Detach a tracked menu item by position or command identifier.
fn hle_remove_menu(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let removed = remove_menu_item(emu, arg(emu, 0), arg(emu, 1), arg(emu, 2));
    ret(emu, removed as u32);
    HleResult::Retn(12)
}

// HWND CreateStatusWindowW(LONG style, LPCWSTR text, HWND parent, UINT id)
// Return a fake child status-bar handle.
fn hle_create_status_window_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let parent = arg(emu, 2);
    let hwnd = emu.hle.alloc_window_handle(parent);
    ret(emu, hwnd);
    HleResult::Retn(16)
}

// int GetClassNameA(HWND hwnd, LPSTR out, int max)
// Copy the tracked class name for a dialog or control as ANSI.
fn hle_get_class_name_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let out = arg(emu, 1);
    let max = arg(emu, 2) as usize;
    let class_name = emu
        .hle
        .window(hwnd)
        .map(|window| window.class_name.clone())
        .unwrap_or_default();
    if out != 0 && max != 0 {
        emu.memory.write_cstr(out, &class_name, max).hle();
    }
    ret(emu, class_name.len().min(max.saturating_sub(1)) as u32);
    HleResult::Retn(12)
}

// int GetClassNameW(HWND hwnd, LPWSTR out, int max)
// Copy the tracked class name for a dialog or control.
fn hle_get_class_name_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let out = arg(emu, 1);
    let max = arg(emu, 2) as usize;
    let class_name = emu
        .hle
        .window(hwnd)
        .map(|window| window.class_name.clone())
        .unwrap_or_default();
    if out != 0 && max != 0 {
        emu.memory.write_utf16z(out, &class_name, max).hle();
    }
    ret(
        emu,
        class_name
            .encode_utf16()
            .count()
            .min(max.saturating_sub(1)) as u32,
    );
    HleResult::Retn(12)
}

// BOOL IsWindowEnabled(HWND hwnd)
// Return the tracked enabled state for a window/control.
fn hle_is_window_enabled(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let enabled = emu
        .hle
        .window(arg(emu, 0))
        .map(|window| window.enabled)
        .unwrap_or(true);
    ret(emu, enabled as u32);
    HleResult::Retn(4)
}

// BOOL IsWindowVisible(HWND hwnd)
// Return the tracked visible state for a window/control.
fn hle_is_window_visible(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let visible = emu
        .hle
        .window(arg(emu, 0))
        .map(|window| window.visible)
        .unwrap_or(false);
    ret(emu, visible as u32);
    HleResult::Retn(4)
}

// int GetWindowTextLengthW(HWND hwnd)
// Return the tracked UTF-16 caption/control length.
fn hle_get_window_text_length_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let len = emu
        .hle
        .window(hwnd)
        .map(|window| window.text.encode_utf16().count())
        .unwrap_or(0);
    ret(emu, len as u32);
    HleResult::Retn(4)
}

// int GetWindowTextLengthA(HWND hwnd)
// Return the tracked ANSI caption/control length.
fn hle_get_window_text_length_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let len = emu
        .hle
        .window(hwnd)
        .map(|window| window.text.len())
        .unwrap_or(0);
    ret(emu, len as u32);
    HleResult::Retn(4)
}

// int GetWindowTextW(HWND hwnd, LPWSTR out, int max)
// Copy the tracked UTF-16 caption/control text.
fn hle_get_window_text_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let out = arg(emu, 1);
    let max = arg(emu, 2) as usize;
    let text = emu
        .hle
        .window(hwnd)
        .map(|window| window.text.clone())
        .unwrap_or_default();
    if out != 0 && max != 0 {
        emu.memory.write_utf16z(out, &text, max).hle();
    }
    ret(emu, text.encode_utf16().count().min(max.saturating_sub(1)) as u32);
    HleResult::Retn(12)
}

// int GetWindowTextA(HWND hwnd, LPSTR out, int max)
// Copy the tracked caption/control text as ANSI.
fn hle_get_window_text_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let out = arg(emu, 1);
    let max = arg(emu, 2) as usize;
    let text = emu
        .hle
        .window(hwnd)
        .map(|window| window.text.clone())
        .unwrap_or_default();
    if out != 0 && max != 0 {
        emu.memory.write_cstr(out, &text, max).hle();
    }
    ret(emu, text.len().min(max.saturating_sub(1)) as u32);
    HleResult::Retn(12)
}

// BOOL SetWindowTextW(HWND hwnd, LPCWSTR text)
// Update tracked caption/control text and refresh the GDI frame.
fn hle_set_window_text_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let text = emu.memory.utf16z_lossy(arg(emu, 1), 4096).unwrap_or_default();
    if let Some(window) = emu.hle.window_mut(hwnd) {
        window.text = text;
    }
    render_hle_windows(emu);
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetWindowTextA(HWND hwnd, LPCSTR text)
// Update tracked caption/control text from ANSI and refresh the GDI frame.
fn hle_set_window_text_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let text = emu.memory.cstr_lossy(arg(emu, 1), 4096).unwrap_or_default();
    if let Some(window) = emu.hle.window_mut(hwnd) {
        window.text = text;
    }
    render_hle_windows(emu);
    ret(emu, 1);
    HleResult::Retn(8)
}

// LRESULT SendDlgItemMessageW(HWND dlg, int id, UINT msg, WPARAM w, LPARAM l)
// Route dialog-item messages through the generic wide SendMessage stub.
fn hle_send_dlg_item_message_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    let id = arg(emu, 1);
    let msg = arg(emu, 2);
    let wparam = arg(emu, 3);
    let lparam = arg(emu, 4);
    let hwnd = emu
        .hle
        .control_by_id(dlg, id)
        .map(|window| window.hwnd)
        .unwrap_or(0);
    let esp = emu.cpu.reg(Reg::Esp);
    emu.memory.write_u32(esp + 4, hwnd).hle();
    emu.memory.write_u32(esp + 8, msg).hle();
    emu.memory.write_u32(esp + 12, wparam).hle();
    emu.memory.write_u32(esp + 16, lparam).hle();
    let result = hle_send_message_w(emu, entry);
    emu.cpu.set_reg(Reg::Esp, esp);
    let value = emu.cpu.reg(Reg::Eax);
    ret(emu, value);
    let _ = result;
    HleResult::Retn(20)
}

// LRESULT SendDlgItemMessageA(HWND dlg, int id, UINT msg, WPARAM w, LPARAM l)
// Route dialog-item messages through the generic ANSI SendMessage stub.
fn hle_send_dlg_item_message_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    let id = arg(emu, 1);
    let msg = arg(emu, 2);
    let wparam = arg(emu, 3);
    let lparam = arg(emu, 4);
    let hwnd = emu
        .hle
        .control_by_id(dlg, id)
        .map(|window| window.hwnd)
        .unwrap_or(0);
    let esp = emu.cpu.reg(Reg::Esp);
    emu.memory.write_u32(esp + 4, hwnd).hle();
    emu.memory.write_u32(esp + 8, msg).hle();
    emu.memory.write_u32(esp + 12, wparam).hle();
    emu.memory.write_u32(esp + 16, lparam).hle();
    let result = hle_send_message_a(emu, entry);
    emu.cpu.set_reg(Reg::Esp, esp);
    let value = emu.cpu.reg(Reg::Eax);
    ret(emu, value);
    let _ = result;
    HleResult::Retn(20)
}

// BOOL GetTextMetricsW(HDC hdc, TEXTMETRICW *tm)
// Fill minimal font metrics for printing and edit-control layout.
fn hle_get_text_metrics_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let out = arg(emu, 1);
    let dc = gdi_dc_or_default(emu, hdc);
    let height = gdi_font_height(emu, dc).max(1) as u32;
    if out != 0 {
        emu.memory.memset(out, 0, 60).hle();
        emu.memory.write_u32(out, height).hle();
        emu.memory.write_u32(out + 20, (height / 2).max(1)).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// HWND GetDlgItem(HWND dlg, int id)
// Return a tracked child control handle by dialog id.
fn hle_get_dlg_item(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    let id = arg(emu, 1);
    let hwnd = emu
        .hle
        .control_by_id(dlg, id)
        .map(|window| window.hwnd)
        .unwrap_or(0);
    ret(emu, hwnd);
    HleResult::Retn(8)
}

// UINT GetDlgItemInt(HWND dlg, int id, BOOL *translated, BOOL signed)
// Return zero and report successful translation.
fn hle_get_dlg_item_int(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let translated = arg(emu, 2);
    if translated != 0 {
        emu.memory.write_u32(translated, 1).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// UINT GetDlgItemTextW(HWND dlg, int id, LPWSTR out, int max)
// Copy tracked dialog-control text.
fn hle_get_dlg_item_text_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    let id = arg(emu, 1);
    let out = arg(emu, 2);
    let max = arg(emu, 3) as usize;
    let text = emu
        .hle
        .control_by_id(dlg, id)
        .map(|window| window.text.clone())
        .unwrap_or_default();
    if out != 0 && max != 0 {
        emu.memory.write_utf16z(out, &text, max).hle();
    }
    ret(emu, text.encode_utf16().count().min(max.saturating_sub(1)) as u32);
    HleResult::Retn(16)
}

// UINT GetDlgItemTextA(HWND dlg, int id, LPSTR out, int max)
// Copy tracked dialog-control text as an ANSI string.
fn hle_get_dlg_item_text_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    let id = arg(emu, 1);
    let out = arg(emu, 2);
    let max = arg(emu, 3) as usize;
    let text = emu
        .hle
        .control_by_id(dlg, id)
        .map(|window| window.text.clone())
        .unwrap_or_default();
    if out != 0 && max != 0 {
        emu.memory.write_cstr(out, &text, max).hle();
    }
    ret(emu, text.len().min(max.saturating_sub(1)) as u32);
    HleResult::Retn(16)
}

// BOOL SetDlgItemInt(HWND dlg, int id, UINT value, BOOL signed)
// Accept integer control updates.
fn hle_set_dlg_item_int(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL SetDlgItemTextW(HWND dlg, int id, LPCWSTR text)
// Update tracked dialog-control text and redraw the HLE window tree.
fn hle_set_dlg_item_text_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    let id = arg(emu, 1);
    let text = emu.memory.utf16z_lossy(arg(emu, 2), 4096).unwrap_or_default();
    if let Some(window) = emu.hle.control_by_id_mut(dlg, id) {
        window.text = text;
    }
    render_hle_windows(emu);
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL SetDlgItemTextA(HWND dlg, int id, LPCSTR text)
// Update tracked dialog-control text from an ANSI string and redraw.
fn hle_set_dlg_item_text_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    let id = arg(emu, 1);
    let text = emu.memory.cstr_lossy(arg(emu, 2), 4096).unwrap_or_default();
    if let Some(window) = emu.hle.control_by_id_mut(dlg, id) {
        window.text = text;
    }
    render_hle_windows(emu);
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL EndDialog(HWND dlg, INT_PTR result)
// Hide a tracked dialog and its children, then redraw the HLE window tree.
fn hle_end_dialog(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dlg = arg(emu, 0);
    for window in emu.hle.windows.values_mut() {
        if window.hwnd == dlg || window.parent == dlg {
            window.visible = false;
        }
    }
    render_hle_windows(emu);
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL IsDialogMessageA(HWND dlg, LPMSG msg)
// Let the normal message loop dispatch mouse and command messages for dialogs.
fn hle_is_dialog_message_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// BOOL IsDialogMessageW(HWND dlg, LPMSG msg)
// Let the normal message loop dispatch mouse and command messages for dialogs.
fn hle_is_dialog_message_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_is_dialog_message_a(emu, entry)
}

// INT_PTR DialogBoxParamW(HINSTANCE inst, LPCWSTR tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Return IDCANCEL for modal dialogs that are not implemented.
fn hle_dialog_box_param_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 2);
    HleResult::Retn(20)
}

fn dispatch_tracked_control_message(emu: &mut Emulator, message: Message) -> Option<u32> {
    let kind = emu.hle.window(message.hwnd)?.control_kind;
    match kind {
        HleControlKind::Edit => dispatch_edit_message(emu, message),
        _ => None,
    }
}

fn accelerator_matches(hle: &Hle, accel: HleAccelerator, wparam: u32) -> bool {
    if (accel.flags & ACCEL_FVIRTKEY) != 0 {
        if (accel.key as u32) != (wparam & 0xffff) {
            return false;
        }
    } else {
        let key = accel.key as u8;
        if !key.eq_ignore_ascii_case(&((wparam & 0xff) as u8)) {
            return false;
        }
    }
    accelerator_modifier_matches(hle, 0x10, (accel.flags & ACCEL_FSHIFT) != 0)
        && accelerator_modifier_matches(hle, 0x11, (accel.flags & ACCEL_FCONTROL) != 0)
        && accelerator_modifier_matches(hle, 0x12, (accel.flags & ACCEL_FALT) != 0)
}

fn accelerator_modifier_matches(hle: &Hle, vk: u32, required: bool) -> bool {
    !required || hle.keyboard_state_byte(vk) != 0
}

fn build_popup_menu(emu: &Emulator, menu: u32, owner: u32, x: i32, y: i32) -> Option<HlePopupMenu> {
    build_popup_menu_from_hle(
        &emu.hle,
        menu,
        owner,
        x,
        y,
        emu.backend.width() as i32,
        emu.backend.height() as i32,
    )
}

fn build_popup_menu_from_hle(
    hle: &Hle,
    menu: u32,
    owner: u32,
    x: i32,
    y: i32,
    screen_w: i32,
    screen_h: i32,
) -> Option<HlePopupMenu> {
    const ITEM_HEIGHT: i32 = 18;
    const SEPARATOR_HEIGHT: i32 = 7;
    const MIN_WIDTH: i32 = 96;
    const MAX_WIDTH: i32 = 260;
    const H_MARGIN: i32 = 8;
    const CHECK_WIDTH: i32 = 16;
    const ARROW_WIDTH: i32 = 14;

    let menu = hle.menu(menu)?;
    let mut width = MIN_WIDTH;
    for item in &menu.items {
        if item.separator {
            continue;
        }
        let text_width = menu_display_text(&item.text).chars().count() as i32 * 8;
        width = width.max(text_width + H_MARGIN * 2 + CHECK_WIDTH + ARROW_WIDTH);
    }
    width = width.min(MAX_WIDTH);
    let height = menu.items.iter().fold(4, |acc, item| {
        acc + if item.separator {
            SEPARATOR_HEIGHT
        } else {
            ITEM_HEIGHT
        }
    });
    let screen_w = screen_w.max(1);
    let screen_h = screen_h.max(1);
    let left = x.min(screen_w.saturating_sub(width + 1)).max(0);
    let top = y.min(screen_h.saturating_sub(height + 1)).max(0);
    let mut row = top + 2;
    let mut popup_items = Vec::with_capacity(menu.items.len());
    for item in &menu.items {
        let item_height = if item.separator {
            SEPARATOR_HEIGHT
        } else {
            ITEM_HEIGHT
        };
        let rect = WindowRect {
            left: left + 2,
            top: row,
            right: left + width - 2,
            bottom: row + item_height,
        };
        popup_items.push(HlePopupMenuItem {
            id: item.id,
            submenu: item.submenu,
            text: item.text.clone(),
            rect,
            separator: item.separator,
            enabled: item.enabled,
            checked: item.checked,
        });
        row += item_height;
    }
    Some(HlePopupMenu {
        owner,
        rect: WindowRect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        },
        items: popup_items,
    })
}

fn menu_bar_item_width(text: &str) -> i32 {
    let text_width = menu_display_text(text).chars().count() as i32 * 8;
    (text_width + 18).max(24)
}

fn menu_display_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '&' if chars.peek() == Some(&'&') => {
                out.push('&');
                chars.next();
            }
            '&' => {}
            '\t' => out.push_str("    "),
            _ => out.push(ch),
        }
    }
    out
}

fn remove_menu_item(emu: &mut Emulator, menu: u32, position: u32, flags: u32) -> bool {
    const MF_BYPOSITION: u32 = 0x0400;

    let Some(menu) = emu.hle.menus.get_mut(&menu) else {
        return false;
    };
    let index = if (flags & MF_BYPOSITION) != 0 {
        ((position as usize) < menu.items.len()).then_some(position as usize)
    } else {
        menu.items.iter().position(|item| item.id == position)
    };
    let Some(index) = index else {
        return false;
    };
    menu.items.remove(index);
    emu.hle.active_popup_menu = None;
    true
}

fn dialog_unit_x(value: i16) -> i32 {
    value as i32 * 2
}

fn dialog_unit_y(value: i16) -> i32 {
    value as i32 * 2
}

fn is_clickable_button_style(style: u32) -> bool {
    const BS_GROUPBOX: u32 = 0x0007;
    (style & 0x000f) != BS_GROUPBOX
}

fn button_style_is_groupbox(style: u32) -> bool {
    const BS_GROUPBOX: u32 = 0x0007;
    (style & 0x000f) == BS_GROUPBOX
}

fn button_style_is_ownerdraw(style: u32) -> bool {
    const BS_OWNERDRAW: u32 = 0x000b;
    (style & 0x000f) == BS_OWNERDRAW
}

fn parse_accelerator_table(
    emu: &Emulator,
    addr: u32,
    size: u32,
) -> Option<HleAcceleratorTable> {
    const ACCEL_LAST: u16 = 0x0080;
    let mut off = 0;
    let mut items = Vec::new();
    while off + 8 <= size {
        let entry = addr.wrapping_add(off);
        let flags = emu.memory.read_u16(entry).ok()?;
        let key = emu.memory.read_u16(entry + 2).ok()?;
        let cmd = emu.memory.read_u16(entry + 4).ok()?;
        items.push(HleAccelerator {
            flags: flags & !ACCEL_LAST,
            key,
            cmd,
        });
        off += 8;
        if (flags & ACCEL_LAST) != 0 {
            break;
        }
    }
    (!items.is_empty()).then_some(HleAcceleratorTable { items })
}

fn parse_standard_menu_template(emu: &mut Emulator, addr: u32, size: u32) -> Option<u32> {
    let mut r = TemplateReader::new(addr, size);
    let version = r.read_u16(&emu.memory)?;
    let header_size = r.read_u16(&emu.memory)?;
    if version != 0 {
        return None;
    }
    r.skip(header_size as u32)?;
    parse_standard_menu_items(emu, &mut r)
}

fn parse_standard_menu_items(emu: &mut Emulator, r: &mut TemplateReader) -> Option<u32> {
    const MF_GRAYED: u32 = 0x0001;
    const MF_DISABLED: u32 = 0x0002;
    const MF_CHECKED: u32 = 0x0008;
    const MF_POPUP: u32 = 0x0010;
    const MF_END: u32 = 0x0080;

    let mut items = Vec::new();
    loop {
        let flags = r.read_u16(&emu.memory)? as u32;
        let is_popup = (flags & MF_POPUP) != 0;
        let is_end = (flags & MF_END) != 0;
        let (id, text, submenu) = if is_popup {
            let text = read_menu_utf16z(&emu.memory, r)?;
            let submenu = parse_standard_menu_items(emu, r)?;
            (0, text, submenu)
        } else {
            let id = r.read_u16(&emu.memory)? as u32;
            let text = read_menu_utf16z(&emu.memory, r)?;
            (id, text, 0)
        };
        let separator = !is_popup && id == 0 && text.is_empty();
        items.push(HleMenuItem {
            id,
            text,
            submenu,
            separator,
            enabled: (flags & (MF_GRAYED | MF_DISABLED)) == 0,
            checked: (flags & MF_CHECKED) != 0,
        });
        if is_end {
            break;
        }
    }
    Some(emu.hle.alloc_menu_handle(HleMenu { items }))
}

fn read_menu_utf16z(mem: &Memory, r: &mut TemplateReader) -> Option<String> {
    let mut units = Vec::new();
    while r.remaining() >= 2 {
        let unit = r.read_u16(mem)?;
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Some(String::from_utf16_lossy(&units))
}

fn parse_dialog_template(mem: &Memory, addr: u32, size: u32) -> Option<DialogTemplate> {
    let dlg_ver = mem.read_u16(addr).ok()?;
    let signature = mem.read_u16(addr + 2).ok()?;
    if dlg_ver == 1 && signature == 0xffff {
        parse_dialog_template_ex(mem, addr, size)
    } else {
        parse_dialog_template_standard(mem, addr, size)
    }
}

fn parse_dialog_template_ex(mem: &Memory, addr: u32, size: u32) -> Option<DialogTemplate> {
    let mut r = TemplateReader::new(addr, size);
    let _dlg_ver = r.read_u16(mem)?;
    let _signature = r.read_u16(mem)?;
    let _help_id = r.read_u32(mem)?;
    let ex_style = r.read_u32(mem)?;
    let style = r.read_u32(mem)?;
    let item_count = r.read_u16(mem)? as usize;
    let x = r.read_i16(mem)?;
    let y = r.read_i16(mem)?;
    let cx = r.read_i16(mem)?;
    let cy = r.read_i16(mem)?;
    let _menu = read_template_name(mem, &mut r)?;
    let _class_name = read_template_name(mem, &mut r)?;
    let title = read_template_name(mem, &mut r)?;
    if (style & (DS_SETFONT | DS_SHELLFONT)) != 0 {
        let _point_size = r.read_u16(mem)?;
        let _weight = r.read_u16(mem)?;
        let _italic = r.read_u8(mem)?;
        let _charset = r.read_u8(mem)?;
        let _typeface = read_template_name(mem, &mut r)?;
    }
    let mut controls = Vec::with_capacity(item_count);
    for _ in 0..item_count {
        r.align4();
        let _help_id = r.read_u32(mem)?;
        let ex_style = r.read_u32(mem)?;
        let style = r.read_u32(mem)?;
        let x = r.read_i16(mem)?;
        let y = r.read_i16(mem)?;
        let cx = r.read_i16(mem)?;
        let cy = r.read_i16(mem)?;
        let id = r.read_u32(mem)?;
        let class_name = read_template_name(mem, &mut r)?;
        let text = read_template_name(mem, &mut r)?;
        let extra = r.read_u16(mem)? as u32;
        r.skip(extra)?;
        controls.push(DialogControlTemplate {
            style,
            ex_style,
            id,
            class_name,
            text,
            x,
            y,
            cx,
            cy,
        });
    }
    Some(DialogTemplate {
        style,
        ex_style,
        x,
        y,
        cx,
        cy,
        title,
        controls,
    })
}

fn parse_dialog_template_standard(mem: &Memory, addr: u32, size: u32) -> Option<DialogTemplate> {
    let mut r = TemplateReader::new(addr, size);
    let style = r.read_u32(mem)?;
    let ex_style = r.read_u32(mem)?;
    let item_count = r.read_u16(mem)? as usize;
    let x = r.read_i16(mem)?;
    let y = r.read_i16(mem)?;
    let cx = r.read_i16(mem)?;
    let cy = r.read_i16(mem)?;
    let _menu = read_template_name(mem, &mut r)?;
    let _class_name = read_template_name(mem, &mut r)?;
    let title = read_template_name(mem, &mut r)?;
    if (style & (DS_SETFONT | DS_SHELLFONT)) != 0 {
        let _point_size = r.read_u16(mem)?;
        let _typeface = read_template_name(mem, &mut r)?;
    }
    let mut controls = Vec::with_capacity(item_count);
    for _ in 0..item_count {
        r.align4();
        let style = r.read_u32(mem)?;
        let ex_style = r.read_u32(mem)?;
        let x = r.read_i16(mem)?;
        let y = r.read_i16(mem)?;
        let cx = r.read_i16(mem)?;
        let cy = r.read_i16(mem)?;
        let id = r.read_u16(mem)? as u32;
        let class_name = read_template_name(mem, &mut r)?;
        let text = read_template_name(mem, &mut r)?;
        let extra = r.read_u16(mem)? as u32;
        r.skip(extra)?;
        controls.push(DialogControlTemplate {
            style,
            ex_style,
            id,
            class_name,
            text,
            x,
            y,
            cx,
            cy,
        });
    }
    Some(DialogTemplate {
        style,
        ex_style,
        x,
        y,
        cx,
        cy,
        title,
        controls,
    })
}

fn dialog_control_kind(class_name: &str) -> HleControlKind {
    if class_name.eq_ignore_ascii_case("Button") {
        HleControlKind::Button
    } else if class_name.eq_ignore_ascii_case("Static") {
        HleControlKind::Static
    } else if class_name.eq_ignore_ascii_case("Edit") {
        HleControlKind::Edit
    } else {
        HleControlKind::Window
    }
}

fn load_dialog_template(emu: &Emulator, module: u32, template: u32) -> Option<DialogTemplate> {
    let id = make_int_resource_id(template)?;
    let (addr, size) = find_pe_resource_data(emu, module, RT_DIALOG, id)?;
    parse_dialog_template(&emu.memory, addr, size)
}

fn default_dialog_template() -> DialogTemplate {
    DialogTemplate {
        style: WS_VISIBLE,
        ex_style: 0,
        x: 0x8000u16 as i16,
        y: 0,
        cx: 180,
        cy: 120,
        title: "Dialog".to_string(),
        controls: Vec::new(),
    }
}

fn create_dialog_param_common(
    emu: &mut Emulator,
    entry: &HleEntry,
    returns_hwnd: bool,
) -> HleResult {
    const WH_CBT: i32 = 5;

    let inst = arg(emu, 0);
    let template = arg(emu, 1);
    let parent = arg(emu, 2);
    let proc = arg(emu, 3);
    let param = arg(emu, 4);
    let hwnd = emu.hle.alloc_window_handle(parent);
    ensure_gdi_screen_surface(emu);
    create_dialog_window_from_template(emu, inst, template, hwnd, parent, proc, !returns_hwnd);
    render_hle_windows(emu);
    let result = if returns_hwnd { hwnd } else { hwnd.max(1) };
    if let Some(hook) = emu.hle.latest_windows_hook(WH_CBT) {
        dispatch_dialog_cbt_create_window_hook_callback(
            emu,
            entry,
            hook,
            hwnd,
            inst,
            result,
            (proc != 0).then_some(DialogInitContinuation { hwnd, proc, param }),
        );
    } else if proc != 0 {
        dispatch_dialog_init_callback_with_result(emu, entry, hwnd, proc, param, result);
    } else {
        ret(emu, result);
    }
    HleResult::Retn(20)
}

fn create_dialog_indirect_common(
    emu: &mut Emulator,
    entry: &HleEntry,
    returns_hwnd: bool,
) -> HleResult {
    const WH_CBT: i32 = 5;

    let inst = arg(emu, 0);
    let template_ptr = arg(emu, 1);
    let parent = arg(emu, 2);
    let proc = arg(emu, 3);
    let param = arg(emu, 4);
    let hwnd = emu.hle.alloc_window_handle(parent);
    let template = parse_dialog_template(&emu.memory, template_ptr, 0x0001_0000)
        .unwrap_or_else(default_dialog_template);
    ensure_gdi_screen_surface(emu);
    create_dialog_window_from_template_data(emu, template, hwnd, parent, proc, !returns_hwnd);
    render_hle_windows(emu);
    let result = if returns_hwnd { hwnd } else { hwnd.max(1) };
    if let Some(hook) = emu.hle.latest_windows_hook(WH_CBT) {
        dispatch_dialog_cbt_create_window_hook_callback(
            emu,
            entry,
            hook,
            hwnd,
            inst,
            result,
            (proc != 0).then_some(DialogInitContinuation { hwnd, proc, param }),
        );
    } else if proc != 0 {
        dispatch_dialog_init_callback_with_result(emu, entry, hwnd, proc, param, result);
    } else {
        ret(emu, result);
    }
    HleResult::Retn(20)
}

fn create_dialog_window_from_template(
    emu: &mut Emulator,
    inst: u32,
    template_id: u32,
    hwnd: u32,
    parent: u32,
    proc: u32,
    force_visible: bool,
) {
    let template =
        load_dialog_template(emu, inst, template_id).unwrap_or_else(default_dialog_template);
    create_dialog_window_from_template_data(emu, template, hwnd, parent, proc, force_visible);
}

fn create_dialog_window_from_template_data(
    emu: &mut Emulator,
    template: DialogTemplate,
    hwnd: u32,
    parent: u32,
    proc: u32,
    _force_visible: bool,
) {
    let client_w = dialog_unit_x(template.cx).max(1);
    let client_h = dialog_unit_y(template.cy).max(1);
    let window_w = client_w.saturating_add(DIALOG_BORDER * 2);
    let window_h = client_h.saturating_add(DIALOG_TITLE_HEIGHT + DIALOG_BORDER);
    let default_x = template.x as u16 == 0x8000;
    let left = if default_x {
        ((emu.backend.width() as i32).saturating_sub(window_w) / 2).max(0)
    } else {
        dialog_unit_x(template.x).max(0)
    };
    let top = if default_x {
        ((emu.backend.height() as i32).saturating_sub(window_h) / 3).max(0)
    } else {
        dialog_unit_y(template.y).max(0)
    };
    let client_left = left.saturating_add(DIALOG_BORDER);
    let client_top = top.saturating_add(DIALOG_TITLE_HEIGHT);
    // DialogBox shows modal dialogs immediately, and several old MFC games use
    // CreateDialog templates without WS_VISIBLE but still expect the HLE dialog
    // to become the active UI. Keep resource-created dialogs visible by default.
    let visible = true;
    emu.hle.register_window(HleWindow {
        hwnd,
        parent,
        id: 0,
        class_name: "#32770".to_string(),
        text: template.title.clone(),
        rect: WindowRect {
            left,
            top,
            right: left.saturating_add(window_w),
            bottom: top.saturating_add(window_h),
        },
        style: template.style,
        ex_style: template.ex_style,
        proc,
        user_data: 0,
        extra: std::collections::HashMap::new(),
        enabled: (template.style & WS_DISABLED) == 0,
        visible,
        control_kind: HleControlKind::Window,
        background_brush: 0,
        invalid_rect: None,
        erase_pending: false,
        last_generated_paint_frame: 0,
        ddraw_owned: false,
    });
    for control in template.controls {
        let child = emu.hle.alloc_window_handle(hwnd);
        let x = client_left.saturating_add(dialog_unit_x(control.x));
        let y = client_top.saturating_add(dialog_unit_y(control.y));
        let w = dialog_unit_x(control.cx).max(1);
        let h = dialog_unit_y(control.cy).max(1);
        let control_kind = dialog_control_kind(&control.class_name);
        emu.hle.register_window(HleWindow {
            hwnd: child,
            parent: hwnd,
            id: control.id,
            class_name: control.class_name,
            text: control.text,
            rect: WindowRect {
                left: x,
                top: y,
                right: x.saturating_add(w),
                bottom: y.saturating_add(h),
            },
            style: control.style,
            ex_style: control.ex_style,
            proc: 0,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: (control.style & WS_DISABLED) == 0,
            visible: (control.style & WS_VISIBLE) != 0,
            control_kind,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });
    }
}

fn draw_menu_overlays_on_surface(emu: &mut Emulator, surface: SurfaceInfo, surf: u32) {
    let mut windows = emu
        .hle
        .windows
        .values()
        .filter(|window| window.parent == 0 && window.visible && window.ddraw_owned)
        .cloned()
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| window.hwnd);
    for window in windows {
        draw_menu_bar_for_window(emu, surface, surf, &window);
    }
}

fn draw_menu_overlays_on_framebuffer(emu: &mut Emulator) -> bool {
    let drew_popup = emu.hle.active_popup_menu.is_some();
    if drew_popup {
        draw_active_popup_menu_framebuffer(emu);
    }
    drew_popup
}

fn draw_menu_bar_for_window(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    surf: u32,
    window: &HleWindow,
) {
    let Some(bar) = emu.hle.menu_bar_rect_for_window(window) else {
        return;
    };
    let Some(menu_handle) = emu.hle.window_menu_handle(window) else {
        return;
    };
    let Some(menu) = emu.hle.menu(menu_handle).cloned() else {
        return;
    };

    let rect = RectI {
        left: bar.left,
        top: bar.top,
        right: bar.right,
        bottom: bar.bottom,
    };
    fill_surface_rect(emu, surface, rect, 0xc618).hle();
    fill_surface_rect(
        emu,
        surface,
        RectI {
            left: rect.left,
            top: rect.bottom.saturating_sub(1),
            right: rect.right,
            bottom: rect.bottom,
        },
        0x8410,
    )
    .hle();

    let mut left = bar.left + 4;
    for item in menu.items.iter().filter(|item| !item.separator) {
        let width = menu_bar_item_width(&item.text);
        let item_rect = RectI {
            left,
            top: bar.top,
            right: (left + width).min(bar.right),
            bottom: bar.bottom,
        };
        if item_rect.width() <= 0 || item_rect.height() <= 0 {
            break;
        }
        let color = if item.enabled { 0x0000_0000 } else { 0x0080_8080 };
        draw_text_left(
            emu,
            surf,
            &menu_display_text(&item.text),
            item_rect.left + 8,
            item_rect.top + 4,
            item_rect,
            color,
        );
        left += width;
        if left >= bar.right {
            break;
        }
    }
}

fn draw_active_popup_menu_framebuffer(emu: &mut Emulator) {
    let Some(popup) = emu.hle.active_popup_menu.clone() else {
        return;
    };
    let rect = RectI {
        left: popup.rect.left,
        top: popup.rect.top,
        right: popup.rect.right,
        bottom: popup.rect.bottom,
    };
    fill_framebuffer_rect(emu, rect, WIN_FACE);
    draw_framebuffer_button_edge(emu, rect, true);
    for item in &popup.items {
        let item_rect = RectI {
            left: item.rect.left,
            top: item.rect.top,
            right: item.rect.right,
            bottom: item.rect.bottom,
        };
        if item.separator {
            let y = item_rect.top + item_rect.height() / 2;
            fill_framebuffer_rect(
                emu,
                RectI {
                    left: item_rect.left + 4,
                    top: y,
                    right: item_rect.right - 4,
                    bottom: y + 1,
                },
                WIN_SHADOW,
            );
            continue;
        }
        let color = if item.enabled { WIN_TEXT } else { WIN_DISABLED_TEXT };
        if item.checked {
            draw_framebuffer_text_left(
                emu,
                "*",
                item_rect.left + 3,
                item_rect.top + 4,
                item_rect,
                color,
            );
        }
        let text = menu_display_text(&item.text);
        draw_framebuffer_text_left(
            emu,
            &text,
            item_rect.left + 18,
            item_rect.top + 4,
            item_rect,
            color,
        );
        if item.submenu != 0 {
            draw_framebuffer_text_right(emu, ">", item_rect, color);
        }
    }
}

fn draw_framebuffer_button_edge(emu: &mut Emulator, rect: RectI, enabled: bool) {
    let highlight = if enabled { WIN_HIGHLIGHT } else { WIN_FACE };
    let shadow = if enabled { WIN_SHADOW } else { WIN_DISABLED_TEXT };
    fill_framebuffer_rect(
        emu,
        RectI {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.top + 1,
        },
        highlight,
    );
    fill_framebuffer_rect(
        emu,
        RectI {
            left: rect.left,
            top: rect.top,
            right: rect.left + 1,
            bottom: rect.bottom,
        },
        highlight,
    );
    fill_framebuffer_rect(
        emu,
        RectI {
            left: rect.left,
            top: rect.bottom - 1,
            right: rect.right,
            bottom: rect.bottom,
        },
        shadow,
    );
    fill_framebuffer_rect(
        emu,
        RectI {
            left: rect.right - 1,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        shadow,
    );
    draw_framebuffer_rect_outline(emu, rect, WIN_TEXT, 1);
}

fn render_active_popup_menu_overlay(emu: &mut Emulator) {
    if emu.hle.active_popup_menu.is_none() {
        return;
    }
    let surf = ensure_gdi_screen_surface(emu);
    let surface = read_surface_info(emu, surf).hle();
    present_surface_if_primary(emu, surface).hle();
}

fn draw_hle_control(emu: &mut Emulator, surface: SurfaceInfo, surf: u32, control: &HleWindow) {
    if control.ddraw_owned {
        return;
    }
    let rect = RectI {
        left: control.rect.left,
        top: control.rect.top,
        right: control.rect.right,
        bottom: control.rect.bottom,
    };
    match control.control_kind {
        HleControlKind::Button if button_style_is_groupbox(control.style) => {
            draw_rect_outline(emu, surface, rect, 0x8410, 1);
            if !control.text.is_empty() {
                draw_text_left(
                    emu,
                    surf,
                    &control.text,
                    rect.left + 8,
                    rect.top - 1,
                    rect,
                    0x0000_0000,
                );
            }
        }
        HleControlKind::Button if button_style_is_ownerdraw(control.style) => {
            // Owner-draw buttons are painted by the parent in response to
            // WM_DRAWITEM; synthesizing a stock button here would cover it.
        }
        HleControlKind::Button => {
            fill_surface_rect(emu, surface, rect, 0xc618).hle();
            draw_button_edge(emu, surface, rect, control.enabled);
            let color = if control.enabled { 0x0000_0000 } else { 0x0080_8080 };
            draw_text_center(emu, surf, &control.text, rect, color);
        }
        HleControlKind::Static | HleControlKind::Edit => {
            fill_surface_rect(emu, surface, rect, 0xffff).hle();
            draw_inset_edge(emu, surface, rect);
            let color = if control.enabled { 0x0000_0000 } else { 0x0080_8080 };
            if (control.style & 0x0003) == 0x0002 {
                draw_text_right(emu, surf, &control.text, rect, color);
            } else if (control.style & 0x0003) == 0x0001 {
                draw_text_center(emu, surf, &control.text, rect, color);
            } else {
                draw_text_left(emu, surf, &control.text, rect.left + 4, rect.top + 4, rect, color);
            }
        }
        HleControlKind::Window => {
            // USER does not synthesize a client paint for arbitrary custom child
            // classes. Preserve app-drawn pixels here; nested stock controls are
            // drawn by the recursive child walk in the window renderer.
        }
    }
}

fn draw_button_edge(emu: &mut Emulator, surface: SurfaceInfo, rect: RectI, enabled: bool) {
    let highlight = if enabled { 0xffff } else { 0xdedb };
    let shadow = if enabled { 0x8410 } else { 0xad55 };
    fill_surface_rect(
        emu,
        surface,
        RectI {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.top + 1,
        },
        highlight,
    )
    .hle();
    fill_surface_rect(
        emu,
        surface,
        RectI {
            left: rect.left,
            top: rect.top,
            right: rect.left + 1,
            bottom: rect.bottom,
        },
        highlight,
    )
    .hle();
    fill_surface_rect(
        emu,
        surface,
        RectI {
            left: rect.left,
            top: rect.bottom - 1,
            right: rect.right,
            bottom: rect.bottom,
        },
        shadow,
    )
    .hle();
    fill_surface_rect(
        emu,
        surface,
        RectI {
            left: rect.right - 1,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        shadow,
    )
    .hle();
    draw_rect_outline(emu, surface, rect, 0x0000, 1);
}

fn hle_dialog_text_metrics() -> GdiLineMetrics {
    GdiLineMetrics {
        height: 14,
        char_width: 8,
        extra: 0,
    }
}

fn dispatch_dialog_cbt_create_window_hook_callback(
    emu: &mut Emulator,
    entry: &HleEntry,
    hook: Hook,
    hwnd: u32,
    inst: u32,
    result: u32,
    continuation: Option<DialogInitContinuation>,
) {
    const HCBT_CREATEWND: u32 = 3;

    let hle_esp = emu.cpu.reg(Reg::Esp);
    let original_ret = emu.memory.read_u32(hle_esp).hle();
    let callback_esp = hle_esp.wrapping_add(32);
    let cbt_create_wnd = emu
        .hle
        .alloc(&mut emu.memory, 8, PagePerm::READ | PagePerm::WRITE)
        .hle();
    let create_struct = alloc_create_struct(emu);
    let async_return = emu.hle.async_return_thunk();
    let create_args = dialog_create_window_args(emu, hwnd, inst);
    trace_gdi!(
        "user Dialog hook hwnd={hwnd:08x} hook={:08x} continuation={}",
        hook.proc,
        continuation.is_some()
    );
    emu.memory.write_u32(cbt_create_wnd, create_struct).hle();
    emu.memory.write_u32(cbt_create_wnd + 4, 0).hle();
    write_create_struct(emu, create_struct, create_args);
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, HCBT_CREATEWND).hle();
    emu.memory.write_u32(callback_esp + 8, hwnd).hle();
    emu.memory.write_u32(callback_esp + 12, cbt_create_wnd).hle();
    emu.memory.write_u32(callback_esp + 16, original_ret).hle();
    if let Some(continuation) = continuation {
        // MFC installs a WH_CBT hook before CreateDialogIndirectParam so it can
        // attach the CDialog object to the HWND before WM_INITDIALOG. This is a
        // non-tail callback chain: the hook must return to HLE, then HLE enters
        // the dialog proc, and only that second callback returns to the caller.
        emu.hle
            .push_dialog_init_callback_return(result, continuation);
    } else {
        emu.hle.push_hle_callback_return(result);
    }
    emu.cpu
        .debug_replace_top_call(
            entry.addr,
            hook.proc,
            async_return,
            callback_esp + 4,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = hook.proc;
}

fn dialog_create_window_args(emu: &Emulator, hwnd: u32, inst: u32) -> CreateWindowArgs {
    let window = emu.hle.window(hwnd);
    let rect = window.map(|window| window.rect).unwrap_or(WindowRect {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
    });
    CreateWindowArgs {
        ex_style: window.map(|window| window.ex_style).unwrap_or(0),
        class_ptr: 0,
        name_ptr: 0,
        style: window.map(|window| window.style).unwrap_or(0),
        x: rect.left,
        y: rect.top,
        w: rect.right.saturating_sub(rect.left).max(1),
        h: rect.bottom.saturating_sub(rect.top).max(1),
        parent: window.map(|window| window.parent).unwrap_or(0),
        menu: 0,
        inst,
        param: 0,
    }
}

fn dispatch_dialog_init_callback_with_result(
    emu: &mut Emulator,
    entry: &HleEntry,
    hwnd: u32,
    proc: u32,
    param: u32,
    result: u32,
) {
    const WM_INITDIALOG: u32 = 0x0110;

    let hle_esp = emu.cpu.reg(Reg::Esp);
    let original_ret = emu.memory.read_u32(hle_esp).hle();
    let callback_esp = hle_esp;
    let async_return = emu.hle.async_return_thunk();
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, hwnd).hle();
    emu.memory
        .write_u32(callback_esp + 8, WM_INITDIALOG)
        .hle();
    emu.memory.write_u32(callback_esp + 12, 0).hle();
    emu.memory.write_u32(callback_esp + 16, param).hle();
    emu.memory.write_u32(callback_esp + 20, original_ret).hle();
    emu.hle.push_hle_callback_return(result);
    emu.cpu
        .debug_replace_top_call(entry.addr, proc, async_return, callback_esp + 4, callback_esp)
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = proc;
}

fn dispatch_dialog_init_callback_after_async(
    emu: &mut Emulator,
    entry: &HleEntry,
    continuation: DialogInitContinuation,
    result: u32,
) {
    const WM_INITDIALOG: u32 = 0x0110;

    let original_ret_esp = emu.cpu.reg(Reg::Esp);
    let callback_esp = original_ret_esp.wrapping_sub(20);
    let async_return = emu.hle.async_return_thunk();
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, continuation.hwnd).hle();
    emu.memory
        .write_u32(callback_esp + 8, WM_INITDIALOG)
        .hle();
    emu.memory.write_u32(callback_esp + 12, 0).hle();
    emu.memory
        .write_u32(callback_esp + 16, continuation.param)
        .hle();
    emu.hle.push_hle_callback_return(result);
    // The CBT hook returned to wemu!__async_return, so the original API frame
    // is no longer the top debug frame. Enter WM_INITDIALOG as a synthetic
    // non-tail callback and let the next async return finish CreateDialog.
    emu.cpu
        .debug_push_synthetic_call(
            entry.addr,
            continuation.proc,
            async_return,
            callback_esp + 4,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = continuation.proc;
}
