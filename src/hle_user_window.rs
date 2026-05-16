const HLE_DESKTOP_WINDOW: u32 = 0x0001_0000;

// BOOL ShowWindow(HWND hWnd, int nCmdShow)
// Mark a window visible and synchronously activate visible top-level windows.
fn hle_show_window(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    const WM_ACTIVATE: u32 = 0x0006;
    const WM_SETFOCUS: u32 = 0x0007;
    const WM_ACTIVATEAPP: u32 = 0x001c;
    const WA_ACTIVE: u32 = 1;

    let hwnd = arg(emu, 0);
    let hwnd = if hwnd == 0 { 0x0002_0001 } else { hwnd };
    let cmd = arg(emu, 1);
    let mut was_visible = false;
    let proc = emu.hle.window(hwnd).map(|window| window.proc).unwrap_or(0);
    let is_child = emu
        .hle
        .window_mut(hwnd)
        .map(|window| {
            was_visible = window.visible;
            window.visible = cmd != 0;
            if cmd == 0 {
                window.invalid_rect = None;
                window.erase_pending = false;
            }
            window.parent != 0
        })
        .unwrap_or(false);
    if cmd == 0 {
        remove_pending_paint_message(emu, hwnd);
    }
    render_hle_windows(emu);
    if cmd != 0 {
        invalidate_window_rect(emu, hwnd, 0, true);
        queue_paint_message(emu, hwnd, "ShowWindow");
    }
    if is_child {
        ret(emu, was_visible as u32);
        return HleResult::Retn(8);
    }
    if show_window_cmd_activates(cmd) && proc != 0 {
        let old_focus = emu.hle.focus_window;
        let old_active = top_level_hwnd_for(emu, old_focus).unwrap_or(0);
        emu.hle.focus_window = hwnd;
        let mut messages = Vec::with_capacity(3);
        if old_active == 0 {
            messages.push(Message {
                hwnd,
                msg: WM_ACTIVATEAPP,
                wparam: 1,
                lparam: 0,
            });
        }
        messages.push(Message {
            hwnd,
            msg: WM_ACTIVATE,
            wparam: WA_ACTIVE,
            lparam: old_active,
        });
        if old_focus != hwnd {
            // Focus and activation are sent messages, not posted messages. Some
            // games poll and discard posted messages while waiting for their
            // first active window, so these callbacks must run before returning.
            messages.push(Message {
                hwnd,
                msg: WM_SETFOCUS,
                wparam: old_focus,
                lparam: 0,
            });
        }
        dispatch_window_proc_message_chain(emu, entry, proc, messages, was_visible as u32, 8);
        return HleResult::Retn(8);
    }
    if cmd != 0 {
        emu.hle.focus_window = hwnd;
    } else if emu.hle.focus_window == hwnd {
        emu.hle.focus_window = 0;
    }
    ret(emu, was_visible as u32);
    HleResult::Retn(8)
}

fn show_window_cmd_activates(cmd: u32) -> bool {
    const SW_HIDE: u32 = 0;
    const SW_SHOWNOACTIVATE: u32 = 4;
    const SW_SHOWMINNOACTIVE: u32 = 7;
    const SW_SHOWNA: u32 = 8;

    !matches!(
        cmd,
        SW_HIDE | SW_SHOWNOACTIVATE | SW_SHOWMINNOACTIVE | SW_SHOWNA
    )
}

// BOOL PostMessageA(HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Queue an application message for the HLE message pump.
fn hle_post_message_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    post_message_impl(emu, "PostMessageA");
    HleResult::Retn(16)
}

// BOOL PostThreadMessageA(DWORD threadId, UINT msg, WPARAM w, LPARAM l)
// Queue a message without an HWND for the single emulated GUI thread.
fn hle_post_thread_message_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    post_thread_message_impl(emu, "PostThreadMessageA");
    HleResult::Retn(16)
}

// BOOL PostThreadMessageW(DWORD threadId, UINT msg, WPARAM w, LPARAM l)
// Queue the same thread message because the payload is not string-encoded.
fn hle_post_thread_message_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    post_thread_message_impl(emu, "PostThreadMessageW");
    HleResult::Retn(16)
}

fn post_message_impl(emu: &mut Emulator, source: &'static str) {
    let hwnd = arg(emu, 0);
    let msg = arg(emu, 1);
    let wparam = arg(emu, 2);
    let lparam = arg(emu, 3);
    if !post_message_target_valid(emu, hwnd) {
        emu.hle.last_error = 1400; // ERROR_INVALID_WINDOW_HANDLE
        trace_gdi!(
            "user PostMessageA invalid hwnd={hwnd:08x} msg={msg:08x} w={wparam:08x} l={lparam:08x}"
        );
        ret(emu, 0);
        return;
    }
    if msg == 0x00f5 {
        if let Some(command) = emu.hle.command_from_click(hwnd) {
            emu.hle.app_messages.push(command);
            emu.hle.note_queued_message("BM_CLICK", command);
            ret(emu, 1);
            return;
        }
    }
    let message = Message {
        hwnd,
        msg,
        wparam,
        lparam,
    };
    emu.hle.app_messages.push(message);
    emu.hle.note_queued_message(source, message);
    ret(emu, 1);
}

fn post_thread_message_impl(emu: &mut Emulator, source: &'static str) {
    let msg = arg(emu, 1);
    let wparam = arg(emu, 2);
    let lparam = arg(emu, 3);
    let message = Message {
        hwnd: 0,
        msg,
        wparam,
        lparam,
    };
    emu.hle.app_messages.push(message);
    emu.hle.note_queued_message(source, message);
    ret(emu, 1);
}

// BOOL InvalidateRect(HWND hwnd, const RECT *rect, BOOL erase)
// Mark the requested client rectangle dirty and queue a coalesced WM_PAINT.
fn hle_invalidate_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let rect = arg(emu, 1);
    let erase = arg(emu, 2) != 0;
    let hwnd = if hwnd == 0 { 0x0002_0001 } else { hwnd };
    invalidate_window_rect(emu, hwnd, rect, erase);
    queue_paint_message(emu, hwnd, "InvalidateRect");
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL ValidateRect(HWND hwnd, const RECT *rect)
// Validate the tracked update region and remove any pending WM_PAINT.
fn hle_validate_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    if hwnd == 0 {
        let hwnds = emu.hle.windows.keys().copied().collect::<Vec<_>>();
        for hwnd in hwnds {
            validate_window_paint(emu, hwnd);
        }
    } else {
        validate_window_paint(emu, hwnd);
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL GetUpdateRect(HWND hwnd, LPRECT rect, BOOL erase)
// Report the tracked invalid client rectangle without validating it.
fn hle_get_update_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = if arg(emu, 0) == 0 {
        0x0002_0001
    } else {
        arg(emu, 0)
    };
    let rect = arg(emu, 1);
    let update = emu.hle.window(hwnd).and_then(|window| window.invalid_rect);
    if let (Some(update), true) = (update, rect != 0) {
        write_gdi_rect(emu, rect, (update.left, update.top, update.right, update.bottom));
    }
    ret(emu, update.is_some() as u32);
    HleResult::Retn(12)
}

// int GetUpdateRgn(HWND hwnd, HRGN rgn, BOOL erase)
// Copy the tracked invalid client rectangle into a fake region and report its type.
fn hle_get_update_rgn(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = if arg(emu, 0) == 0 {
        0x0002_0001
    } else {
        arg(emu, 0)
    };
    let rgn = arg(emu, 1);
    let update = emu.hle.window(hwnd).and_then(|window| window.invalid_rect);
    let rect = update.unwrap_or(WindowRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    });
    if rgn != 0 {
        emu.hle.gdi_regions.insert(rgn, rect);
    }
    ret(emu, region_result(rect));
    HleResult::Retn(12)
}

// BOOL IntersectRect(LPRECT dst, const RECT *a, const RECT *b)
// Write the intersection rectangle and return whether it is non-empty.
fn hle_intersect_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let a = arg(emu, 1);
    let b = arg(emu, 2);
    let a = read_gdi_rect(emu, a);
    let b = read_gdi_rect(emu, b);
    let rect = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    let non_empty = rect.2 > rect.0 && rect.3 > rect.1;
    if dst != 0 {
        if non_empty {
            write_gdi_rect(emu, dst, rect);
        } else {
            write_gdi_rect(emu, dst, (0, 0, 0, 0));
        }
    }
    ret(emu, non_empty as u32);
    HleResult::Retn(12)
}

// BOOL UpdateWindow(HWND hwnd)
// Synchronously send WM_PAINT when the tracked update region is non-empty.
fn hle_update_window(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let hwnd = if hwnd == 0 { 0x0002_0001 } else { hwnd };
    let paint_proc = emu
        .hle
        .window(hwnd)
        .filter(|window| window.invalid_rect.is_some())
        .map(|window| window.proc)
        .filter(|proc| *proc != 0);
    if let Some(proc) = paint_proc {
        dispatch_window_proc_callback(emu, entry, hwnd, proc, 0x000f, 0, 0, 1, 4);
        return HleResult::Retn(4);
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL RedrawWindow(HWND hwnd, const RECT *rect, HRGN rgn, UINT flags)
// Track invalid client regions and queue WM_PAINT for requested redraws.
fn hle_redraw_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const RDW_INVALIDATE: u32 = 0x0001;
    const RDW_UPDATENOW: u32 = 0x0100;

    let hwnd = arg(emu, 0);
    let rect = arg(emu, 1);
    let flags = arg(emu, 3);
    let hwnd = if hwnd == 0 { 0x0002_0001 } else { hwnd };
    if (flags & RDW_INVALIDATE) != 0 {
        invalidate_window_rect(emu, hwnd, rect, (flags & 0x0004) != 0);
    }
    if (flags & (RDW_INVALIDATE | RDW_UPDATENOW)) != 0
        && emu
            .hle
            .window(hwnd)
            .map_or(true, |window| window.invalid_rect.is_some())
    {
        queue_paint_message(emu, hwnd, "RedrawWindow");
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL ExitWindowsEx(UINT flags, DWORD reason)
// Acknowledge shutdown requests without terminating the host or emulator.
fn hle_exit_windows_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// DWORD WaitForInputIdle(HANDLE process, DWORD milliseconds)
// Report the single emulated GUI thread as already input-idle.
fn hle_wait_for_input_idle(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// int ToAscii(UINT vk, UINT scan, const BYTE *state, LPWORD out, UINT flags)
// Convert simple alphanumeric virtual keys for legacy keyboard probes.
fn hle_to_ascii(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let vk = arg(emu, 0);
    let out = arg(emu, 3);
    if out != 0 {
        if let Some(ch) = ascii_for_virtual_key(vk) {
            emu.memory.write_u16(out, ch as u16).hle();
            ret(emu, 1);
        } else {
            emu.memory.write_u16(out, 0).hle();
            ret(emu, 0);
        }
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(20)
}

// UINT DdeInitializeA(DWORD *instance, void *callback, DWORD flags, DWORD reserved)
// Initialize a fake DDE instance for old shell/game lobby probes.
fn hle_dde_initialize_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u32(out, 0x5200_0001).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// BOOL DdeUninitialize(DWORD instance)
// Accept teardown of fake DDE instances.
fn hle_dde_uninitialize(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// HSZ DdeCreateStringHandleA(DWORD instance, LPCSTR string, int codepage)
// Return a stable nonzero fake string handle.
fn hle_dde_create_string_handle_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x5200_0100);
    HleResult::Retn(12)
}

// DWORD DdeQueryStringA(DWORD instance, HSZ string, LPSTR out, DWORD max, int codepage)
// Return an empty string for fake DDE string handles.
fn hle_dde_query_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    let max = arg(emu, 3) as usize;
    if out != 0 && max != 0 {
        emu.memory.write_u8(out, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(20)
}

// HCONV DdeConnect(DWORD instance, HSZ service, HSZ topic, void *context)
// Report no DDE server connection.
fn hle_dde_connect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(16)
}

// BOOL DdeDisconnect(HCONV conv)
// Accept disconnect for fake or absent DDE conversations.
fn hle_dde_disconnect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// HDDEDATA DdeNameService(DWORD instance, HSZ service, HSZ reserved, UINT cmd)
// Accept registration/unregistration while returning no data handle.
fn hle_dde_name_service(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(16)
}

// BYTE *DdeAccessData(HDDEDATA data, DWORD *size)
// Report empty DDE data.
fn hle_dde_access_data(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let size = arg(emu, 1);
    if size != 0 {
        emu.memory.write_u32(size, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// BOOL DdeUnaccessData(HDDEDATA data)
// Accept release of fake DDE data.
fn hle_dde_unaccess_data(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// HDDEDATA DdeClientTransaction(BYTE *data, DWORD len, HCONV conv, HSZ item, UINT fmt, UINT type, DWORD timeout, DWORD *result)
// Report no DDE transaction result for absent shell/game lobby peers.
fn hle_dde_client_transaction(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let result = arg(emu, 7);
    if result != 0 {
        emu.memory.write_u32(result, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(32)
}

fn ascii_for_virtual_key(vk: u32) -> Option<u8> {
    match vk {
        0x30..=0x39 | 0x41..=0x5a => Some(vk as u8),
        0x20 => Some(b' '),
        0x0d => Some(b'\r'),
        0x09 => Some(b'\t'),
        _ => None,
    }
}

// HMONITOR MonitorFromRect(LPCRECT rect, DWORD flags)
// Return a stable primary-monitor handle for single-display HLE.
fn hle_monitor_from_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x5300_0001);
    HleResult::Retn(8)
}

// BOOL GetMonitorInfoW(HMONITOR monitor, LPMONITORINFO info)
// Fill monitor and work-area rectangles from the backend framebuffer.
fn hle_get_monitor_info_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let info = arg(emu, 1);
    if info != 0 {
        let right = emu.backend.width();
        let bottom = emu.backend.height();
        emu.memory.write_u32(info + 4, 0).hle();
        emu.memory.write_u32(info + 8, 0).hle();
        emu.memory.write_u32(info + 12, right).hle();
        emu.memory.write_u32(info + 16, bottom).hle();
        emu.memory.write_u32(info + 20, 0).hle();
        emu.memory.write_u32(info + 24, 0).hle();
        emu.memory.write_u32(info + 28, right).hle();
        emu.memory.write_u32(info + 32, bottom).hle();
        emu.memory.write_u32(info + 36, 1).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL PtInRect(const RECT *rect, POINT pt)
// Test a by-value POINT against the half-open RECT bounds.
fn hle_pt_in_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = read_gdi_rect(emu, arg(emu, 0));
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    ret(
        emu,
        (x >= rect.0 && x < rect.2 && y >= rect.1 && y < rect.3) as u32,
    );
    HleResult::Retn(12)
}

// BOOL CopyRect(LPRECT dst, const RECT *src)
// Copy RECT coordinates between guest buffers.
fn hle_copy_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    if dst != 0 && src != 0 {
        let rect = read_gdi_rect(emu, src);
        write_gdi_rect(emu, dst, rect);
        ret(emu, 1);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// BOOL OffsetRect(LPRECT rect, int dx, int dy)
// Offset RECT coordinates in place.
fn hle_offset_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect_ptr = arg(emu, 0);
    if rect_ptr != 0 {
        let mut rect = read_gdi_rect(emu, rect_ptr);
        let dx = arg(emu, 1) as i32;
        let dy = arg(emu, 2) as i32;
        rect.0 = rect.0.saturating_add(dx);
        rect.1 = rect.1.saturating_add(dy);
        rect.2 = rect.2.saturating_add(dx);
        rect.3 = rect.3.saturating_add(dy);
        write_gdi_rect(emu, rect_ptr, rect);
        ret(emu, 1);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// BOOL SetRectEmpty(LPRECT rect)
// Set a RECT to the conventional empty all-zero coordinates.
fn hle_set_rect_empty(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = arg(emu, 0);
    if rect != 0 {
        write_gdi_rect(emu, rect, (0, 0, 0, 0));
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL IsRectEmpty(const RECT *rect)
// Return true when a RECT has no positive area.
fn hle_is_rect_empty(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = read_gdi_rect(emu, arg(emu, 0));
    ret(emu, (rect.2 <= rect.0 || rect.3 <= rect.1) as u32);
    HleResult::Retn(4)
}

// BOOL EqualRect(const RECT *a, const RECT *b)
// Compare two RECT buffers coordinate-for-coordinate.
fn hle_equal_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let a = read_gdi_rect(emu, arg(emu, 0));
    let b = read_gdi_rect(emu, arg(emu, 1));
    ret(emu, (a == b) as u32);
    HleResult::Retn(8)
}

// BOOL UnionRect(LPRECT dst, const RECT *a, const RECT *b)
// Write the bounding rectangle of two non-empty source RECTs.
fn hle_union_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let a = read_gdi_rect(emu, arg(emu, 1));
    let b = read_gdi_rect(emu, arg(emu, 2));
    let a_empty = a.2 <= a.0 || a.3 <= a.1;
    let b_empty = b.2 <= b.0 || b.3 <= b.1;
    let rect = match (a_empty, b_empty) {
        (true, true) => (0, 0, 0, 0),
        (true, false) => b,
        (false, true) => a,
        (false, false) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
    };
    if dst != 0 {
        write_gdi_rect(emu, dst, rect);
    }
    ret(emu, (!a_empty || !b_empty) as u32);
    HleResult::Retn(12)
}

// BOOL ClientToScreen(HWND hwnd, LPPOINT point)
// Translate a client point to the tracked absolute screen coordinate space.
fn hle_client_to_screen(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let point = arg(emu, 1);
    if point != 0 {
        let (left, top, _, _) = window_client_area(emu, hwnd);
        let x = emu.memory.read_u32(point).hle() as i32;
        let y = emu.memory.read_u32(point + 4).hle() as i32;
        emu.memory
            .write_u32(point, x.saturating_add(left) as u32)
            .hle();
        emu.memory
            .write_u32(point + 4, y.saturating_add(top) as u32)
            .hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL ScreenToClient(HWND hwnd, LPPOINT point)
// Translate an absolute screen point to the tracked client coordinate space.
fn hle_screen_to_client(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let point = arg(emu, 1);
    if point != 0 {
        let (left, top, _, _) = window_client_area(emu, hwnd);
        let x = emu.memory.read_u32(point).hle() as i32;
        let y = emu.memory.read_u32(point + 4).hle() as i32;
        emu.memory
            .write_u32(point, x.saturating_sub(left) as u32)
            .hle();
        emu.memory
            .write_u32(point + 4, y.saturating_sub(top) as u32)
            .hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// int MapWindowPoints(HWND from, HWND to, LPPOINT points, UINT count)
// Translate points between tracked window client coordinate spaces.
fn hle_map_window_points(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let from = arg(emu, 0);
    let to = arg(emu, 1);
    let points = arg(emu, 2);
    let count = arg(emu, 3);
    let Some((from_x, from_y)) = map_window_origin(emu, from) else {
        ret(emu, 0);
        return HleResult::Retn(16);
    };
    let Some((to_x, to_y)) = map_window_origin(emu, to) else {
        ret(emu, 0);
        return HleResult::Retn(16);
    };
    let dx = from_x.saturating_sub(to_x);
    let dy = from_y.saturating_sub(to_y);
    if points != 0 {
        for index in 0..count {
            let point = points.wrapping_add(index.saturating_mul(8));
            let x = emu.memory.read_u32(point).hle() as i32;
            let y = emu.memory.read_u32(point + 4).hle() as i32;
            emu.memory.write_u32(point, x.saturating_add(dx) as u32).hle();
            emu.memory
                .write_u32(point + 4, y.saturating_add(dy) as u32)
                .hle();
        }
    }
    ret(emu, make_map_window_points_result(dx, dy));
    HleResult::Retn(16)
}

// int GetSystemMetrics(int index)
// Return framebuffer dimensions and small border metrics.
fn hle_get_system_metrics(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let index = arg(emu, 0);
    let value = match index {
        0 => emu.backend.width(),              // SM_CXSCREEN
        1 => emu.backend.height(),             // SM_CYSCREEN
        4 => DIALOG_TITLE_HEIGHT as u32,       // SM_CYCAPTION
        5 | 7 | 32 => DIALOG_BORDER as u32,    // SM_CXBORDER/CXDLGFRAME/CXFRAME
        6 | 8 | 33 => DIALOG_BORDER as u32,    // SM_CYBORDER/CYDLGFRAME/CYFRAME
        15 => MENU_BAR_HEIGHT as u32,          // SM_CYMENU
        _ => 0,
    };
    ret(emu, value);
    HleResult::Retn(4)
}

// BOOL SetScrollRange(HWND hwnd, int bar, int min, int max, BOOL redraw)
// Track scrollbar range state for apps that manage owner-drawn scrollers.
fn hle_set_scroll_range(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let bar = arg(emu, 1);
    let min = arg(emu, 2) as i32;
    let max = arg(emu, 3) as i32;
    let state = emu.hle.scroll_states.entry((hwnd, bar)).or_default();
    state.min = min;
    state.max = max;
    state.pos = clamp_scroll_pos(state.pos, min, max);
    ret(emu, 1);
    HleResult::Retn(20)
}

// int SetScrollPos(HWND hwnd, int bar, int pos, BOOL redraw)
// Store the scrollbar position and return the previous tracked value.
fn hle_set_scroll_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let bar = arg(emu, 1);
    let pos = arg(emu, 2) as i32;
    let state = emu.hle.scroll_states.entry((hwnd, bar)).or_default();
    let previous = state.pos;
    state.pos = clamp_scroll_pos(pos, state.min, state.max);
    ret(emu, previous as u32);
    HleResult::Retn(16)
}

// BOOL PatBlt(HDC hdc, int x, int y, int w, int h, DWORD rop)
// Fill a DC rectangle using the selected brush for common raster ops.
fn hle_pat_blt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const BLACKNESS: u32 = 0x0000_0042;
    const WHITENESS: u32 = 0x00ff_0062;
    const PATCOPY: u32 = 0x00f0_0021;

    let hdc = arg(emu, 0);
    let dc = gdi_dc_or_default(emu, hdc);
    if dc.surface != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        let rect = RectI {
            left: (arg(emu, 1) as i32).saturating_add(dc.origin_x),
            top: (arg(emu, 2) as i32).saturating_add(dc.origin_y),
            right: (arg(emu, 1) as i32)
                .saturating_add(dc.origin_x)
                .saturating_add(arg(emu, 3) as i32),
            bottom: (arg(emu, 2) as i32)
                .saturating_add(dc.origin_y)
                .saturating_add(arg(emu, 4) as i32),
        };
        match arg(emu, 5) {
            BLACKNESS => draw_gdi_rect_colorref(emu, surface, rect, 0x0000_0000),
            WHITENESS => draw_gdi_rect_colorref(emu, surface, rect, 0x00ff_ffff),
            PATCOPY => draw_gdi_rect_brush(emu, surface, rect, dc.selected_brush),
            _ => draw_gdi_rect_brush(emu, surface, rect, dc.selected_brush),
        }
    }
    ret(emu, 1);
    HleResult::Retn(24)
}

// COLORREF GetSysColor(int index)
// Return stable classic Windows system colors.
fn hle_get_sys_color(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, sys_colorref(arg(emu, 0)));
    HleResult::Retn(4)
}

// BOOL SetSysColors(int count, const INT *elements, const COLORREF *colors)
// Accept palette/theme updates while keeping the built-in classic colors.
fn hle_set_sys_colors(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(12)
}

// int DrawTextA(HDC hdc, LPCSTR text, int count, LPRECT rect, UINT format)
// Calculate text bounds for DT_CALCRECT and raster clipped bitmap glyphs otherwise.
fn hle_draw_text_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const DT_CENTER: u32 = 0x0001;
    const DT_RIGHT: u32 = 0x0002;
    const DT_VCENTER: u32 = 0x0004;
    const DT_BOTTOM: u32 = 0x0008;
    const DT_SINGLELINE: u32 = 0x0020;
    const DT_NOCLIP: u32 = 0x0100;
    const DT_CALCRECT: u32 = 0x0400;

    let hdc = arg(emu, 0);
    let text = arg(emu, 1);
    let count = arg(emu, 2);
    let rect_ptr = arg(emu, 3);
    let format = arg(emu, 4);
    let bytes = gdi_text_bytes(emu, text, count);
    let dc = gdi_dc_or_default(emu, hdc);
    let multiline = (format & DT_SINGLELINE) == 0;
    let layout = gdi_text_layout(emu, dc, &bytes, multiline);

    let mut rect = if rect_ptr != 0 {
        read_gdi_rect(emu, rect_ptr)
    } else {
        (0, 0, emu.backend.width() as i32, emu.backend.height() as i32)
    };
    if (format & DT_CALCRECT) != 0 {
        rect.2 = rect.0.saturating_add(layout.width);
        rect.3 = rect.1.saturating_add(layout.height);
        if rect_ptr != 0 {
            write_gdi_rect(emu, rect_ptr, rect);
        }
        ret(emu, layout.height as u32);
        return HleResult::Retn(20);
    }

    let rect_width = rect.2.saturating_sub(rect.0).max(0);
    let rect_height = rect.3.saturating_sub(rect.1).max(0);
    let mut y = rect.1;
    if !multiline && (format & DT_VCENTER) != 0 {
        y = rect.1 + (rect_height.saturating_sub(layout.height) / 2);
    } else if !multiline && (format & DT_BOTTOM) != 0 {
        y = rect.3.saturating_sub(layout.height);
    }
    let clip = if (format & DT_NOCLIP) != 0 {
        None
    } else {
        Some(rect)
    };
    for line in &layout.lines {
        let mut x = rect.0;
        if (format & DT_CENTER) != 0 {
            x = rect.0 + (rect_width.saturating_sub(line.width) / 2);
        } else if (format & DT_RIGHT) != 0 {
            x = rect.2.saturating_sub(line.width);
        }
        draw_gdi_text(
            emu,
            dc,
            &bytes[line.start..line.end],
            x,
            y,
            layout.metrics,
            clip,
        );
        y = y.saturating_add(layout.metrics.height);
    }
    ret(emu, layout.height as u32);
    HleResult::Retn(20)
}

// int DrawTextExA(HDC hdc, LPSTR text, int count, LPRECT rect, UINT format, DRAWTEXTPARAMS *params)
// Draw through the DrawTextA path and ignore optional tab/left-margin parameters.
fn hle_draw_text_ex_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let result = hle_draw_text_a(emu, entry);
    let _ = result;
    HleResult::Retn(24)
}

// BOOL ExtTextOutA(HDC hdc, int x, int y, UINT options, RECT *rect, LPCSTR text, UINT count, INT *dx)
// Fill opaque bounds when requested and draw clipped ANSI bitmap glyphs.
fn hle_ext_text_out_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const ETO_OPAQUE: u32 = 0x0002;
    const ETO_CLIPPED: u32 = 0x0004;

    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let options = arg(emu, 3);
    let rect_ptr = arg(emu, 4);
    let text = arg(emu, 5);
    let count = arg(emu, 6);
    let bytes = gdi_text_bytes(emu, text, count);
    let dc = gdi_dc_or_default(emu, hdc);
    let rect = if rect_ptr != 0 {
        Some(read_gdi_rect(emu, rect_ptr))
    } else {
        None
    };
    if dc.surface != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        if (options & ETO_OPAQUE) != 0 {
            if let Some(rect) = rect {
                draw_gdi_glyph_rect(
                    emu,
                    surface,
                    dc.bk_color,
                    rect.0 + dc.origin_x,
                    rect.1 + dc.origin_y,
                    rect.2.saturating_sub(rect.0),
                    rect.3.saturating_sub(rect.1),
                    None,
                );
            }
        }
    }
    let clip = if (options & ETO_CLIPPED) != 0 {
        rect
    } else {
        None
    };
    let layout = gdi_text_layout(emu, dc, &bytes, false);
    draw_gdi_text(emu, dc, &bytes, x, y, layout.metrics, clip);
    update_text_current_pos(emu, hdc, dc, &layout);
    ret(emu, 1);
    HleResult::Retn(32)
}

// int MessageBoxA(HWND hwnd, LPCSTR text, LPCSTR caption, UINT type)
// Trace the message text and return IDOK.
fn hle_message_box_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let text = arg(emu, 1);
    let caption = arg(emu, 2);
    if emu.should_trace() {
        eprintln!(
            "MessageBoxA {:?}: {:?}",
            emu.memory.cstr_lossy(caption, 512).unwrap_or_default(),
            emu.memory.cstr_lossy(text, 2048).unwrap_or_default()
        );
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL MessageBeep(UINT type)
// Accept notification beeps without sound output.
fn hle_message_beep(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL FlashWindow(HWND hwnd, BOOL invert)
// Accept taskbar/caption flash requests without visual blinking.
fn hle_flash_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let active = top_level_hwnd_for(emu, emu.hle.focus_window) == top_level_hwnd_for(emu, hwnd);
    ret(emu, active as u32);
    HleResult::Retn(8)
}

// BOOL DrawMenuBar(HWND hwnd)
// Report success; menu pixels are produced by the tracked window renderer.
fn hle_draw_menu_bar(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    render_hle_windows(emu);
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL SystemParametersInfoA(UINT action, UINT param, PVOID out, UINT flags)
// Return stable defaults for system settings probes.
fn hle_system_parameters_info_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    system_parameters_info_impl(emu);
    HleResult::Retn(16)
}

// BOOL SystemParametersInfoW(UINT action, UINT param, PVOID out, UINT flags)
// Return stable defaults for system settings probes.
fn hle_system_parameters_info_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    system_parameters_info_impl(emu);
    HleResult::Retn(16)
}

fn system_parameters_info_impl(emu: &mut Emulator) {
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 1);
}

// BOOL GetMessageA(MSG *msg, HWND hwnd, UINT min, UINT max)
// Block cooperatively until a queued message or frontend stop exists.
fn hle_get_message_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let filter = MessageFilter::new(arg(emu, 1), arg(emu, 2), arg(emu, 3));

    if !emu.hle.has_matching_message(filter) {
        emu.poll_frontend_events_no_timers().hle();
        if emu.stopped.is_some() {
            ret(emu, 0);
            return HleResult::Retn(16);
        }
    }

    if let Some(message) = poll_message(
        emu,
        out,
        filter,
        true,
        "GetMessage-input",
        "GetMessage-app",
    ) {
        ret(emu, if message.msg == 0x0012 { 0 } else { 1 });
        return HleResult::Retn(16);
    }

    emu.hle.note_peek_message("GetMessage-none", 1, None);
    HleResult::Wait(HleWaitState::Message {
        out,
        filter,
    })
}

fn poll_message(
    emu: &mut Emulator,
    out: u32,
    filter: MessageFilter,
    remove: bool,
    input_source: &'static str,
    app_source: &'static str,
) -> Option<Message> {
    let queued = emu.hle.matching_queued_message(filter)?;
    let source = match queued.kind {
        MessageQueueKind::Input => input_source,
        MessageQueueKind::App => app_source,
    };
    emu.hle
        .note_peek_message(source, remove as u32, Some(queued.message));
    if out != 0 {
        write_msg(emu, out, queued.message);
    }
    emu.hle.note_generated_paint_delivered(queued.message);
    if remove {
        emu.hle.remove_queued_message(queued, source);
    }
    Some(queued.message)
}

// void PostQuitMessage(int code)
// Queue WM_QUIT with the requested exit code.
fn hle_post_quit_message(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let code = arg(emu, 0);
    let message = Message {
        hwnd: 0,
        msg: 0x0012, // WM_QUIT
        wparam: code,
        lparam: 0,
    };
    emu.hle.app_messages.push(message);
    emu.hle.note_queued_message("PostQuitMessage", message);
    ret(emu, 0);
    HleResult::Retn(4)
}

// UINT_PTR SetTimer(HWND hwnd, UINT_PTR id, UINT elapse, TIMERPROC proc)
// Register or update a fake millisecond timer.
fn hle_set_timer(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let requested = arg(emu, 1);
    let elapse = arg(emu, 2);
    let proc = arg(emu, 3);
    let target = emu.delay_target(elapse);
    let id = emu.hle.set_timer(
        hwnd,
        requested,
        proc,
        emu.guest_time_ms,
        emu.current_scheduler_frame(),
        target,
    );
    trace_gdi!(
        "user SetTimer hwnd={hwnd:08x} requested={requested:08x} elapse={elapse} proc={proc:08x} -> {id:08x}"
    );
    ret(emu, id);
    HleResult::Retn(16)
}

// BOOL KillTimer(HWND hwnd, UINT_PTR id)
// Remove a fake timer registration.
fn hle_kill_timer(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let id = arg(emu, 1);
    emu.hle.kill_timer(hwnd, id);
    ret(emu, 1);
    HleResult::Retn(8)
}

// HDC BeginPaint(HWND hwnd, PAINTSTRUCT *ps)
// Validate the window update region and return a fake screen HDC.
fn hle_begin_paint(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let ps = arg(emu, 1);
    let hdc = create_gdi_screen_dc(emu, hwnd);
    if ps != 0 {
        let paint = take_window_invalid_rect(emu, hwnd).unwrap_or_else(|| {
            let client = window_client_rect(emu, hwnd);
            WindowRect {
                left: client.0,
                top: client.1,
                right: client.2,
                bottom: client.3,
            }
        });
        let needs_erase = take_window_erase_pending(emu, hwnd);
        let erased = if needs_erase {
            erase_window_background(emu, hwnd, paint)
        } else {
            false
        };
        emu.memory.write_u32(ps, hdc).hle();
        emu.memory
            .write_u32(ps + 4, (needs_erase && !erased) as u32)
            .hle();
        emu.memory.write_u32(ps + 8, paint.left as u32).hle();
        emu.memory.write_u32(ps + 12, paint.top as u32).hle();
        emu.memory.write_u32(ps + 16, paint.right as u32).hle();
        emu.memory.write_u32(ps + 20, paint.bottom as u32).hle();
        trace_gdi!(
            "user BeginPaint hwnd={hwnd:08x} hdc={hdc:08x} paint=({},{}..{},{}) erase={needs_erase}",
            paint.left,
            paint.top,
            paint.right,
            paint.bottom,
        );
        remove_pending_paint_message(emu, hwnd);
    }
    ret(emu, hdc);
    HleResult::Retn(8)
}

// HRESULT DirectPlayCreate(GUID *guid, IDirectPlay **out, IUnknown *outer)
// Report unavailable DirectPlay and clear the output interface.
fn hle_direct_play_create(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const E_NOTIMPL: u32 = 0x8000_4001;
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, E_NOTIMPL);
    HleResult::Retn(12)
}

// HRESULT DirectPlayEnumerateA(LPDPENUMDPCALLBACKA cb, void *ctx)
// Succeed with an empty provider list so network play is simply unavailable.
fn hle_direct_play_enumerate_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT DirectPlayLobbyCreateA(GUID *guid, IDirectPlayLobbyA **out, IUnknown *outer, void *data, DWORD size)
// Report unavailable DirectPlay lobby support and clear the output interface.
fn hle_direct_play_lobby_create_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const E_NOTIMPL: u32 = 0x8000_4001;
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, E_NOTIMPL);
    HleResult::Retn(20)
}

// BOOL GdiFlush(void)
// Report success because drawing writes directly into tracked surfaces.
fn hle_gdi_flush(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(0)
}

// HGLOBAL GlobalAlloc(UINT flags, SIZE_T size)
// Allocate a direct HLE memory handle; movable blocks are represented by pointers.
fn hle_global_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 0);
    let size = arg(emu, 1).max(1);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    if (flags & 0x0040) != 0 {
        emu.memory.memset(ptr, 0, size).hle();
    }
    ret(emu, ptr);
    HleResult::Retn(8)
}

// HGLOBAL GlobalReAlloc(HGLOBAL mem, SIZE_T size, UINT flags)
// Allocate a replacement direct handle and copy the requested byte count best-effort.
fn hle_global_re_alloc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let old = arg(emu, 0);
    let size = arg(emu, 1).max(1);
    let flags = arg(emu, 2);
    let ptr = emu
        .hle
        .alloc(&mut emu.memory, size, PagePerm::READ | PagePerm::WRITE)
        .hle();
    if (flags & 0x0040) != 0 {
        emu.memory.memset(ptr, 0, size).hle();
    }
    if old != 0 {
        if let Ok(bytes) = emu.memory.read_bytes(old, size.min(4096) as usize) {
            emu.memory.write_bytes(ptr, &bytes).hle();
        }
        emu.hle.free_alloc(&mut emu.memory, old).hle();
    }
    ret(emu, ptr);
    HleResult::Retn(12)
}

// LPVOID GlobalLock(HGLOBAL mem)
// Return the direct pointer used as the global memory handle.
fn hle_global_lock(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(4)
}

// BOOL GlobalUnlock(HGLOBAL mem)
// Accept unlock for direct global handles.
fn hle_global_unlock(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// HGLOBAL GlobalHandle(LPCVOID mem)
// Return the direct pointer used as the global memory handle.
fn hle_global_handle(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(4)
}

// SIZE_T GlobalSize(HGLOBAL mem)
// Return the tracked allocation size for direct global handles.
fn hle_global_size(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.alloc_size(arg(emu, 0)).unwrap_or(0));
    HleResult::Retn(4)
}

// LPVOID LocalLock(HLOCAL mem)
// Return the same pointer because local handles are direct allocations.
fn hle_local_lock(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(4)
}

// BOOL LocalUnlock(HLOCAL mem)
// Accept unlock for direct local allocations.
fn hle_local_unlock(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// DWORD FormatMessageW(DWORD flags, LPCVOID src, DWORD id, DWORD lang, LPWSTR out, DWORD size, va_list *args)
// Write a compact generic diagnostic string or allocate one when requested.
fn hle_format_message_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const FORMAT_MESSAGE_ALLOCATE_BUFFER: u32 = 0x0000_0100;
    let flags = arg(emu, 0);
    let message_id = arg(emu, 2);
    let out = arg(emu, 4);
    let size = arg(emu, 5) as usize;
    let text = format!("Error {message_id}");
    let written = text.encode_utf16().count() as u32;
    if out != 0 {
        if (flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) != 0 {
            let chars = written + 1;
            let ptr = emu
                .hle
                .alloc(&mut emu.memory, chars * 2, PagePerm::READ | PagePerm::WRITE)
                .hle();
            emu.memory.write_utf16z(ptr, &text, chars as usize).hle();
            emu.memory.write_u32(out, ptr).hle();
        } else if size != 0 {
            emu.memory.write_utf16z(out, &text, size).hle();
        }
    }
    ret(emu, written);
    HleResult::Retn(28)
}

// DWORD FormatMessageA(DWORD flags, LPCVOID src, DWORD id, DWORD lang, LPSTR out, DWORD size, va_list *args)
// Write a compact generic ANSI diagnostic string or allocate one when requested.
fn hle_format_message_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const FORMAT_MESSAGE_ALLOCATE_BUFFER: u32 = 0x0000_0100;
    let flags = arg(emu, 0);
    let message_id = arg(emu, 2);
    let out = arg(emu, 4);
    let size = arg(emu, 5) as usize;
    let text = format!("Error {message_id}");
    let written = text.len() as u32;
    if out != 0 {
        if (flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) != 0 {
            let ptr = emu
                .hle
                .alloc(
                    &mut emu.memory,
                    written + 1,
                    PagePerm::READ | PagePerm::WRITE,
                )
                .hle();
            emu.memory
                .write_cstr(ptr, &text, (written + 1) as usize)
                .hle();
            emu.memory.write_u32(out, ptr).hle();
        } else if size != 0 {
            emu.memory.write_cstr(out, &text, size).hle();
        }
    }
    ret(emu, written);
    HleResult::Retn(28)
}

// UINT RegisterWindowMessageA(LPCSTR name)
// Register an ANSI message name and return its stable application-defined ID.
fn hle_register_window_message_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.cstr_lossy(arg(emu, 0), 256).unwrap_or_default();
    let message = register_window_message_impl(emu, &name);
    ret(emu, message);
    HleResult::Retn(4)
}

// UINT RegisterWindowMessageW(LPCWSTR name)
// Register a UTF-16 message name and return its stable application-defined ID.
fn hle_register_window_message_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = emu.memory.utf16z_lossy(arg(emu, 0), 256).unwrap_or_default();
    let message = register_window_message_impl(emu, &name);
    ret(emu, message);
    HleResult::Retn(4)
}

fn register_window_message_impl(emu: &mut Emulator, name: &str) -> u32 {
    emu.hle.register_window_message(name)
}

// BOOL EnableWindow(HWND hwnd, BOOL enable)
// Update tracked enabled state and return the previous state.
fn hle_enable_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let enable = arg(emu, 1) != 0;
    let old = emu
        .hle
        .window_mut(hwnd)
        .map(|window| {
            let old = window.enabled;
            window.enabled = enable;
            old
        })
        .unwrap_or(false);
    render_hle_windows(emu);
    ret(emu, old as u32);
    HleResult::Retn(8)
}

// HWND FindWindowA(LPCSTR class_name, LPCSTR window_name)
// Report no existing matching top-level window by default.
fn hle_find_window_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// BOOL PostMessageW(HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Queue an application message for the HLE message pump.
fn hle_post_message_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    post_message_impl(emu, "PostMessageW");
    HleResult::Retn(16)
}

// LRESULT DispatchMessageW(const MSG *msg)
// Dispatch wide messages through the same single-window HLE callback path.
fn hle_dispatch_message_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_dispatch_message_a(emu, entry)
}

// LRESULT DefWindowProcA(HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Apply default ANSI USER message side effects not handled by guest code.
fn hle_def_window_proc_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = def_window_proc_impl(emu);
    ret(emu, value);
    HleResult::Retn(16)
}

// LRESULT DefWindowProcW(HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Apply the same default USER side effects for wide window procedures.
fn hle_def_window_proc_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = def_window_proc_impl(emu);
    ret(emu, value);
    HleResult::Retn(16)
}

fn def_window_proc_impl(emu: &mut Emulator) -> u32 {
    const WM_PAINT: u32 = 0x000f;
    const WM_ERASEBKGND: u32 = 0x0014;
    const WM_SETCURSOR: u32 = 0x0020;
    const WM_NCCREATE: u32 = 0x0081;

    let hwnd = arg(emu, 0);
    let msg = arg(emu, 1);
    match msg {
        WM_NCCREATE | WM_ERASEBKGND | WM_SETCURSOR => 1,
        WM_PAINT => {
            validate_window_paint(emu, hwnd);
            0
        }
        _ => 0,
    }
}

// BOOL GetMessageW(MSG *msg, HWND hwnd, UINT min, UINT max)
// Block cooperatively until a queued message or frontend stop exists.
fn hle_get_message_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_get_message_a(emu, entry)
}

// int MessageBoxW(HWND hwnd, LPCWSTR text, LPCWSTR caption, UINT type)
// Trace wide message text when requested and return IDOK.
fn hle_message_box_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    if emu.should_trace() {
        eprintln!(
            "MessageBoxW {:?}: {:?}",
            emu.memory
                .utf16z_lossy(arg(emu, 2), 512)
                .unwrap_or_default(),
            emu.memory
                .utf16z_lossy(arg(emu, 1), 2048)
                .unwrap_or_default()
        );
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// HDC GetDC(HWND hwnd)
// Return a fake HDC tied to the SDL-presented GDI screen surface.
fn hle_get_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    get_dc_impl(emu);
    HleResult::Retn(4)
}

// HWND GetDesktopWindow(void)
// Return a stable fake desktop HWND that maps to the full frontend surface.
fn hle_get_desktop_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, HLE_DESKTOP_WINDOW);
    HleResult::Retn(0)
}

// HDC GetWindowDC(HWND hwnd)
// Return a fake HDC tied to the SDL-presented GDI screen surface.
fn hle_get_window_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    get_dc_impl(emu);
    HleResult::Retn(4)
}

fn get_dc_impl(emu: &mut Emulator) {
    let hwnd = arg(emu, 0);
    let hdc = create_gdi_screen_dc(emu, hwnd);
    let dc = gdi_dc_or_default(emu, hdc);
    trace_gdi!(
        "user GetDC hwnd={hwnd:08x} -> hdc={hdc:08x} surface={:08x} origin={},{}",
        dc.surface,
        dc.origin_x,
        dc.origin_y,
    );
    ret(emu, hdc);
}

// int ReleaseDC(HWND hwnd, HDC hdc)
// Present and release a fake screen HDC.
fn hle_release_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 1);
    present_and_drop_gdi_dc(emu, hdc);
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL EndPaint(HWND hwnd, const PAINTSTRUCT *ps)
// Present and release the HDC stored in the paint structure.
fn hle_end_paint(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let ps = arg(emu, 1);
    if ps != 0 {
        let hdc = emu.memory.read_u32(ps).unwrap_or(0);
        present_and_drop_gdi_dc(emu, hdc);
    }
    if let Some((first, chain)) = owner_draw_children_after_parent_paint(emu, hwnd) {
        if dispatch_owner_draw_button_from_hle(emu, entry, first, 0x0001, 0, 1, 8, chain) {
            return HleResult::Retn(8);
        }
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// int FillRect(HDC hdc, const RECT *rect, HBRUSH brush)
// Fill a rectangle on a surface-backed DC with a tracked or system brush.
fn hle_fill_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let rect_ptr = arg(emu, 1);
    let brush = arg(emu, 2);
    if dc.surface != 0 && rect_ptr != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        let rect = read_gdi_rect(emu, rect_ptr);
        draw_gdi_rect_brush(
            emu,
            surface,
            RectI {
                left: rect.0.saturating_add(dc.origin_x),
                top: rect.1.saturating_add(dc.origin_y),
                right: rect.2.saturating_add(dc.origin_x),
                bottom: rect.3.saturating_add(dc.origin_y),
            },
            brush,
        );
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// HFONT CreateFontIndirectW(const LOGFONTW *lf)
// Create a fake font handle from the LOGFONT height.
fn hle_create_font_indirect_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let lf = arg(emu, 0);
    let raw_height = if lf != 0 {
        emu.memory.read_u32(lf).unwrap_or(16) as i32
    } else {
        16
    };
    let height = raw_height.unsigned_abs().max(1);
    let font = emu.hle.create_gdi_font(height);
    ret(emu, font);
    HleResult::Retn(4)
}

// HFONT CreateFontIndirectA(const LOGFONTA *lf)
// Create a fake font handle from the LOGFONT height.
fn hle_create_font_indirect_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let lf = arg(emu, 0);
    let raw_height = if lf != 0 {
        emu.memory.read_u32(lf).unwrap_or(16) as i32
    } else {
        16
    };
    let height = raw_height.unsigned_abs().max(1);
    let font = emu.hle.create_gdi_font(height);
    ret(emu, font);
    HleResult::Retn(4)
}

// BOOL DestroyWindow(HWND hwnd)
// Queue WM_DESTROY for the main window and accept child destruction.
fn hle_destroy_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let proc = emu.hle.window(hwnd).map(|window| window.proc).unwrap_or(0);
    if let Some(window) = emu.hle.window_mut(hwnd) {
        window.visible = false;
        window.invalid_rect = None;
        window.erase_pending = false;
    }
    remove_pending_paint_message(emu, hwnd);
    render_hle_windows(emu);
    if proc != 0 {
        let message = Message {
            hwnd,
            msg: 0x0002,
            wparam: 0,
            lparam: 0,
        };
        emu.hle.app_messages.push(message);
        emu.hle.note_queued_message("DestroyWindow", message);
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// HWND GetParent(HWND hwnd)
// Return the tracked parent window handle.
fn hle_get_parent(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let parent = emu.hle.window(hwnd).map(|window| window.parent).unwrap_or(0);
    ret(emu, parent);
    HleResult::Retn(4)
}

// int GetDlgCtrlID(HWND hwnd)
// Return the tracked child-window/menu identifier assigned at creation.
fn hle_get_dlg_ctrl_id(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let id = emu.hle.window(hwnd).map(|window| window.id).unwrap_or(0);
    ret(emu, id);
    HleResult::Retn(4)
}

// HWND GetTopWindow(HWND hwnd)
// Return the topmost tracked child; NULL/desktop enumerates top-level windows.
fn hle_get_top_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let parent = if hwnd == 0 || hwnd == HLE_DESKTOP_WINDOW {
        0
    } else {
        hwnd
    };
    let result = topmost_child_window(emu, parent).unwrap_or(0);
    trace_gdi!("user GetTopWindow hwnd={hwnd:08x} parent={parent:08x} -> {result:08x}");
    ret(emu, result);
    HleResult::Retn(4)
}

// HWND GetWindow(HWND hwnd, UINT cmd)
// Traverse the tracked HWND parent/child/sibling graph using creation order as z-order.
fn hle_get_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const GW_HWNDFIRST: u32 = 0;
    const GW_HWNDLAST: u32 = 1;
    const GW_HWNDNEXT: u32 = 2;
    const GW_HWNDPREV: u32 = 3;
    const GW_OWNER: u32 = 4;
    const GW_CHILD: u32 = 5;
    const GW_ENABLEDPOPUP: u32 = 6;

    let hwnd = arg(emu, 0);
    let cmd = arg(emu, 1);
    let result = match cmd {
        GW_HWNDFIRST => sibling_window(emu, hwnd, true),
        GW_HWNDLAST => sibling_window(emu, hwnd, false),
        GW_HWNDNEXT => adjacent_sibling_window(emu, hwnd, false),
        GW_HWNDPREV => adjacent_sibling_window(emu, hwnd, true),
        GW_OWNER => 0,
        GW_CHILD => {
            let parent = if hwnd == 0 || hwnd == HLE_DESKTOP_WINDOW {
                0
            } else {
                hwnd
            };
            topmost_child_window(emu, parent).unwrap_or(0)
        }
        GW_ENABLEDPOPUP => emu
            .hle
            .window(hwnd)
            .filter(|window| window.enabled)
            .map(|window| window.hwnd)
            .unwrap_or(0),
        _ => 0,
    };
    trace_gdi!("user GetWindow hwnd={hwnd:08x} cmd={cmd} -> {result:08x}");
    ret(emu, result);
    HleResult::Retn(8)
}

// HWND SetParent(HWND child, HWND new_parent)
// Reparent a tracked HWND and return its previous parent.
fn hle_set_parent(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let child = arg(emu, 0);
    let new_parent = match arg(emu, 1) {
        HLE_DESKTOP_WINDOW => 0,
        hwnd => hwnd,
    };
    let old_parent = emu
        .hle
        .window_mut(child)
        .map(|window| {
            let old_parent = window.parent;
            window.parent = new_parent;
            old_parent
        })
        .unwrap_or(0);
    ret(emu, old_parent);
    HleResult::Retn(8)
}

// BOOL IsChild(HWND parent, HWND child)
// Walk tracked parent links to test descendant ownership.
fn hle_is_child(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let parent = arg(emu, 0);
    let child = arg(emu, 1);
    let parent = if parent == HLE_DESKTOP_WINDOW {
        0
    } else {
        parent
    };
    let mut current = emu.hle.window(child).map(|window| window.parent).unwrap_or(0);
    for _ in 0..emu.hle.windows.len() {
        if current == 0 {
            break;
        }
        if current == parent {
            ret(emu, 1);
            return HleResult::Retn(8);
        }
        current = emu
            .hle
            .window(current)
            .map(|window| window.parent)
            .unwrap_or(0);
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HWND GetLastActivePopup(HWND hwnd)
// Return the owner itself because modal popup ownership is not separately modeled.
fn hle_get_last_active_popup(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(4)
}

// BOOL IsIconic(HWND hwnd)
// Report that tracked windows are never minimized in the fixed framebuffer.
fn hle_is_iconic(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// BOOL IsZoomed(HWND hwnd)
// Match Wine by reporting whether the tracked window style has WS_MAXIMIZE set.
fn hle_is_zoomed(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const WS_MAXIMIZE: u32 = 0x0100_0000;
    let zoomed = emu
        .hle
        .window(arg(emu, 0))
        .map(|window| (window.style & WS_MAXIMIZE) != 0)
        .unwrap_or(false);
    ret(emu, zoomed as u32);
    HleResult::Retn(4)
}

// BOOL IsWindow(HWND hwnd)
// Report whether the handle is tracked by the HLE window manager.
fn hle_is_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.window(arg(emu, 0)).is_some() as u32);
    HleResult::Retn(4)
}

// BOOL GetClientRect(HWND hwnd, RECT *rect)
// Return the tracked window client rectangle in local coordinates.
fn hle_get_client_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let rect = arg(emu, 1);
    if rect != 0 {
        write_gdi_rect(emu, rect, window_client_rect(emu, hwnd));
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL GetWindowRect(HWND hwnd, RECT *rect)
// Return the tracked absolute window rectangle.
fn hle_get_window_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let rect = arg(emu, 1);
    if rect != 0 {
        let value = emu
            .hle
            .window(hwnd)
            .map(|window| {
                (
                    window.rect.left,
                    window.rect.top,
                    window.rect.right,
                    window.rect.bottom,
                )
            })
            .unwrap_or_else(|| {
                if hwnd == 0x0002_0001 || hwnd == 0 || hwnd == HLE_DESKTOP_WINDOW {
                    (0, 0, emu.backend.width() as i32, emu.backend.height() as i32)
                } else {
                    (
                        0,
                        emu.backend.height() as i32 - 22,
                        emu.backend.width() as i32,
                        emu.backend.height() as i32,
                    )
                }
            });
        write_gdi_rect(emu, rect, value);
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL GetWindowPlacement(HWND hwnd, WINDOWPLACEMENT *placement)
// Fill a normal visible window placement.
fn hle_get_window_placement(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let placement = arg(emu, 1);
    if placement != 0 {
        emu.memory.memset(placement, 0, 44).hle();
        emu.memory.write_u32(placement, 44).hle();
        emu.memory.write_u32(placement + 8, 1).hle();
        emu.memory.write_u32(placement + 28, 0).hle();
        emu.memory.write_u32(placement + 32, 0).hle();
        emu.memory
            .write_u32(placement + 36, emu.backend.width())
            .hle();
        emu.memory
            .write_u32(placement + 40, emu.backend.height())
            .hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetWindowPlacement(HWND hwnd, const WINDOWPLACEMENT *placement)
// Accept placement updates without resizing the fixed logical framebuffer.
fn hle_set_window_placement(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL MoveWindow(HWND hwnd, int x, int y, int w, int h, BOOL repaint)
// Accept child layout changes and optionally queue a paint.
fn hle_move_window(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let w = (arg(emu, 3) as i32).max(1);
    let h = (arg(emu, 4) as i32).max(1);
    let parent = emu.hle.window(hwnd).map(|window| window.parent).unwrap_or(0);
    let origin = if parent != 0 {
        let (left, top, _, _) = window_client_area(emu, parent);
        (left, top)
    } else {
        (0, 0)
    };
    let mut resized = false;
    if let Some(window) = emu.hle.window_mut(hwnd) {
        window.rect = WindowRect {
            left: origin.0.saturating_add(x),
            top: origin.1.saturating_add(y),
            right: origin.0.saturating_add(x).saturating_add(w),
            bottom: origin.1.saturating_add(y).saturating_add(h),
        };
        resized = true;
    }
    render_hle_windows(emu);
    let repaint = arg(emu, 5) != 0;
    if repaint {
        remove_pending_paint_message(emu, hwnd);
    }
    if resized {
        queue_size_message(emu, hwnd, "MoveWindow");
    }
    if repaint {
        invalidate_window_rect(emu, hwnd, 0, true);
        queue_paint_message(emu, hwnd, "MoveWindow");
    }
    ret(emu, 1);
    HleResult::Retn(24)
}

// BOOL SetWindowPos(HWND hwnd, HWND after, int x, int y, int cx, int cy, UINT flags)
// Update tracked window position/size and queue repaint unless redraw is suppressed.
fn hle_set_window_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let x = arg(emu, 2) as i32;
    let y = arg(emu, 3) as i32;
    let w = arg(emu, 4) as i32;
    let h = arg(emu, 5) as i32;
    let flags = arg(emu, 6);
    set_window_pos_impl(emu, hwnd, x, y, w, h, flags, "SetWindowPos");
    ret(emu, 1);
    HleResult::Retn(28)
}

// HDWP BeginDeferWindowPos(int count)
// Return a stable batch handle; DeferWindowPos applies entries immediately.
fn hle_begin_defer_window_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x5301_0001);
    HleResult::Retn(4)
}

// HDWP DeferWindowPos(HDWP hdwp, HWND hwnd, HWND after, int x, int y, int cx, int cy, UINT flags)
// Apply the deferred position immediately and keep the batch handle alive.
fn hle_defer_window_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdwp = arg(emu, 0);
    if hdwp == 0 {
        ret(emu, 0);
        return HleResult::Retn(32);
    }
    let hwnd = arg(emu, 1);
    let x = arg(emu, 3) as i32;
    let y = arg(emu, 4) as i32;
    let w = arg(emu, 5) as i32;
    let h = arg(emu, 6) as i32;
    let flags = arg(emu, 7);
    set_window_pos_impl(emu, hwnd, x, y, w, h, flags, "DeferWindowPos");
    ret(emu, hdwp);
    HleResult::Retn(32)
}

// BOOL EndDeferWindowPos(HDWP hdwp)
// Report success for non-null batches; entries were applied by DeferWindowPos.
fn hle_end_defer_window_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, (arg(emu, 0) != 0) as u32);
    HleResult::Retn(4)
}

fn set_window_pos_impl(
    emu: &mut Emulator,
    hwnd: u32,
    req_x: i32,
    req_y: i32,
    raw_w: i32,
    raw_h: i32,
    flags: u32,
    source: &'static str,
) {
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOREDRAW: u32 = 0x0008;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const SWP_HIDEWINDOW: u32 = 0x0080;

    let parent = emu.hle.window(hwnd).map(|window| window.parent).unwrap_or(0);
    let origin = if parent != 0 {
        let (left, top, _, _) = window_client_area(emu, parent);
        (left, top)
    } else {
        (0, 0)
    };
    let req_w = raw_w.max(1);
    let req_h = raw_h.max(1);
    let mut resized = false;
    if let Some(window) = emu.hle.window_mut(hwnd) {
        let current_w = window.rect.right.saturating_sub(window.rect.left).max(1);
        let current_h = window.rect.bottom.saturating_sub(window.rect.top).max(1);
        let left = if (flags & SWP_NOMOVE) != 0 {
            window.rect.left
        } else {
            origin.0.saturating_add(req_x)
        };
        let top = if (flags & SWP_NOMOVE) != 0 {
            window.rect.top
        } else {
            origin.1.saturating_add(req_y)
        };
        let width = if (flags & SWP_NOSIZE) != 0 || raw_w <= 0 {
            current_w
        } else {
            req_w
        };
        let height = if (flags & SWP_NOSIZE) != 0 || raw_h <= 0 {
            current_h
        } else {
            req_h
        };
        window.rect = WindowRect {
            left,
            top,
            right: left.saturating_add(width),
            bottom: top.saturating_add(height),
        };
        resized = true;
        if (flags & SWP_SHOWWINDOW) != 0 {
            window.visible = true;
        }
        if (flags & SWP_HIDEWINDOW) != 0 {
            window.visible = false;
        }
    }
    render_hle_windows(emu);
    if (flags & SWP_NOREDRAW) == 0 {
        remove_pending_paint_message(emu, hwnd);
    }
    if resized {
        queue_size_message(emu, hwnd, source);
    }
    if (flags & SWP_NOREDRAW) == 0 {
        invalidate_window_rect(emu, hwnd, 0, true);
        queue_paint_message(emu, hwnd, source);
    }
}

// LONG GetWindowLongW(HWND hwnd, int index)
// Return tracked style, id, parent, procedure, and user-data fields.
fn hle_get_window_long_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let index = arg(emu, 1) as i32;
    let value = emu
        .hle
        .window(hwnd)
        .map(|window| match index {
            GWL_WNDPROC => window.proc,
            GWL_HWNDPARENT => window.parent,
            GWL_ID => window.id,
            GWL_STYLE => window.style,
            GWL_EXSTYLE => window.ex_style,
            GWLP_USERDATA => window.user_data,
            index if index >= 0 => window.extra.get(&index).copied().unwrap_or(0),
            _ => 0,
        })
        .unwrap_or(0);
    ret(emu, value);
    HleResult::Retn(8)
}

// LONG GetWindowLongA(HWND hwnd, int index)
// Return the same tracked fields as the wide entry point.
fn hle_get_window_long_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_get_window_long_w(emu, entry)
}

fn get_window_word_value(window: &HleWindow, index: i32) -> u16 {
    const GWW_HINSTANCE: i32 = -6;
    match index {
        GWW_HINSTANCE => 0x0040,
        GWL_HWNDPARENT => window.parent as u16,
        GWL_ID => window.id as u16,
        index if index >= 0 => window.extra.get(&index).copied().unwrap_or(0) as u16,
        _ => 0,
    }
}

// WORD GetWindowWord(HWND hwnd, int index)
// Return tracked 16-bit parent/id/extra window words for Win16-era callers.
fn hle_get_window_word(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let index = arg(emu, 1) as i32;
    let value = emu
        .hle
        .window(hwnd)
        .map(|window| get_window_word_value(window, index) as u32)
        .unwrap_or(0);
    trace_gdi!("user GetWindowWord hwnd={hwnd:08x} index={index} -> {value:04x}");
    ret(emu, value);
    HleResult::Retn(8)
}

// LONG SetWindowLongW(HWND hwnd, int index, LONG value)
// Update tracked style/proc/user-data fields and return the previous value.
fn hle_set_window_long_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let index = arg(emu, 1) as i32;
    let value = arg(emu, 2);
    let mut queue_initial_paint = false;
    let old = emu
        .hle
        .window_mut(hwnd)
        .map(|window| {
            let old = match index {
                GWL_WNDPROC => window.proc,
                GWL_HWNDPARENT => window.parent,
                GWL_ID => window.id,
                GWL_STYLE => window.style,
                GWL_EXSTYLE => window.ex_style,
                GWLP_USERDATA => window.user_data,
                index if index >= 0 => window.extra.get(&index).copied().unwrap_or(0),
                _ => 0,
            };
            match index {
                GWL_WNDPROC => {
                    queue_initial_paint =
                        old == 0 && value != 0 && window.visible && window.invalid_rect.is_none();
                    window.proc = value;
                }
                GWL_HWNDPARENT => window.parent = value,
                GWL_ID => window.id = value,
                GWL_STYLE => {
                    window.style = value;
                    window.enabled = (value & WS_DISABLED) == 0;
                    // Fullscreen games rewrite style bits without intending to hide the HWND.
                    // ShowWindow/SetWindowPos own tracked visibility.
                }
                GWL_EXSTYLE => window.ex_style = value,
                GWLP_USERDATA => window.user_data = value,
                index if index >= 0 => {
                    window.extra.insert(index, value);
                }
                _ => {}
            }
            old
        })
        .unwrap_or(0);
    trace_gdi!("user SetWindowLong hwnd={hwnd:08x} index={index} value={value:08x} -> {old:08x}");
    render_hle_windows(emu);
    if queue_initial_paint {
        invalidate_window_rect(emu, hwnd, 0, true);
        queue_paint_message(emu, hwnd, "SetWindowLong");
    }
    ret(emu, old);
    HleResult::Retn(12)
}

// LONG SetWindowLongA(HWND hwnd, int index, LONG value)
// Update the same tracked fields as the wide entry point.
fn hle_set_window_long_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_set_window_long_w(emu, entry)
}

// WORD SetWindowWord(HWND hwnd, int index, WORD value)
// Update tracked 16-bit parent/id/extra window words and return the previous value.
fn hle_set_window_word(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const GWW_HINSTANCE: i32 = -6;
    let hwnd = arg(emu, 0);
    let index = arg(emu, 1) as i32;
    let value = arg(emu, 2) & 0xffff;
    let old = emu
        .hle
        .window_mut(hwnd)
        .map(|window| {
            let old = get_window_word_value(window, index) as u32;
            match index {
                GWW_HINSTANCE => {}
                GWL_HWNDPARENT => window.parent = value,
                GWL_ID => window.id = value,
                index if index >= 0 => {
                    window.extra.insert(index, value);
                }
                _ => {}
            }
            old
        })
        .unwrap_or(0);
    trace_gdi!("user SetWindowWord hwnd={hwnd:08x} index={index} value={value:04x} -> {old:04x}");
    ret(emu, old);
    HleResult::Retn(12)
}

// LONG SetClassLongA(HWND hwnd, int index, LONG value)
// Accept class metadata changes; per-window subclassing is handled by SetWindowLong.
fn hle_set_class_long_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(12)
}

// DWORD GetClassLongA(HWND hwnd, int index)
// Return tracked per-window class metadata for common negative GCL indexes.
fn hle_get_class_long_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let index = arg(emu, 1) as i32;
    let value = emu
        .hle
        .window(hwnd)
        .map(|window| match index {
            -24 => window.proc,
            -10 => window.background_brush,
            _ => 0,
        })
        .unwrap_or(0);
    ret(emu, value);
    HleResult::Retn(8)
}

// LONG ChangeDisplaySettingsA(DEVMODEA *mode, DWORD flags)
// Accept mode changes; DirectDraw owns actual framebuffer mode selection.
fn hle_change_display_settings_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// BOOL EnumThreadWindows(DWORD thread_id, WNDENUMPROC callback, LPARAM lparam)
// Succeed with an empty enumeration to avoid nested callbacks for startup probes.
fn hle_enum_thread_windows(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(12)
}

// UINT GetDoubleClickTime(void)
// Return the classic Windows default double-click interval.
fn hle_get_double_click_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 500);
    HleResult::Retn(0)
}

// LRESULT SendMessageA(HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Emulate common ANSI window messages with tracked window text state.
fn hle_send_message_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let msg = arg(emu, 1);
    let wparam = arg(emu, 2);
    let lparam = arg(emu, 3);
    let value = match msg {
        0x000c => {
            let text = emu.memory.cstr_lossy(lparam, 4096).unwrap_or_default();
            if let Some(window) = emu.hle.window_mut(hwnd) {
                window.text = text;
            }
            render_hle_windows(emu);
            1
        }
        0x000d => {
            let text = emu
                .hle
                .window(hwnd)
                .map(|window| window.text.clone())
                .unwrap_or_default();
            if lparam != 0 && wparam != 0 {
                emu.memory.write_cstr(lparam, &text, wparam as usize).hle();
            }
            text.len().min((wparam as usize).saturating_sub(1)) as u32
        }
        0x000e => emu
            .hle
            .window(hwnd)
            .map(|window| window.text.len() as u32)
            .unwrap_or(0),
        0x0100 | 0x0101 | 0x0102 | 0x0201 | 0x0202 => {
            dispatch_tracked_control_message(
                emu,
                Message {
                    hwnd,
                    msg,
                    wparam,
                    lparam,
                },
            )
            .unwrap_or(0)
        }
        _ => {
            let entry = HleEntry {
                addr: 0,
                dll: "user32.dll",
                name: "SendMessageW",
                callback: hle_send_message_w,
            };
            let _ = hle_send_message_w(emu, &entry);
            emu.cpu.reg(Reg::Eax)
        }
    };
    ret(emu, value);
    HleResult::Retn(16)
}

// LRESULT SendMessageW(HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Emulate common edit/status messages with conservative return values.
fn hle_send_message_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 0);
    let msg = arg(emu, 1);
    let wparam = arg(emu, 2);
    let lparam = arg(emu, 3);
    let value = match msg {
        0x000c => {
            let text = emu.memory.utf16z_lossy(lparam, 4096).unwrap_or_default();
            if let Some(window) = emu.hle.window_mut(hwnd) {
                window.text = text;
            }
            render_hle_windows(emu);
            1
        }
        0x000d => {
            let text = emu
                .hle
                .window(hwnd)
                .map(|window| window.text.clone())
                .unwrap_or_default();
            if lparam != 0 && wparam != 0 {
                emu.memory.write_utf16z(lparam, &text, wparam as usize).hle();
            }
            text.encode_utf16()
                .count()
                .min((wparam as usize).saturating_sub(1)) as u32
        }
        0x000e => emu
            .hle
            .window(hwnd)
            .map(|window| window.text.encode_utf16().count() as u32)
            .unwrap_or(0),
        0x0100 | 0x0101 | 0x0102 | 0x0201 | 0x0202 => {
            dispatch_tracked_control_message(
                emu,
                Message {
                    hwnd,
                    msg,
                    wparam,
                    lparam,
                },
            )
            .unwrap_or(0)
        }
        0x00f5 => {
            if let Some(command) = emu.hle.command_from_click(hwnd) {
                emu.hle.app_messages.push(command);
                emu.hle.note_queued_message("BM_CLICK", command);
            }
            0
        }
        0x0030 => 0,
        0x00b0 => {
            if wparam != 0 {
                emu.memory.write_u32(wparam, 0).hle();
            }
            if lparam != 0 {
                emu.memory.write_u32(lparam, 0).hle();
            }
            0
        }
        0x00b8 | 0x00c6 => 0,
        0x00ba => 1,
        0x00bb | 0x00c9 => 0,
        0x0143 => 0,
        0x0147 => 0,
        0x014e => wparam,
        0x0404 | 0x040b => 1,
        _ => 0,
    };
    ret(emu, value);
    HleResult::Retn(16)
}

// LRESULT CallWindowProcW(WNDPROC proc, HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Return zero when no previous subclass procedure is available.
fn hle_call_window_proc_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let proc = arg(emu, 0);
    if proc == 0 {
        ret(emu, 0);
        return HleResult::Retn(20);
    }
    let hle_esp = emu.cpu.reg(Reg::Esp);
    let ret_addr = emu.memory.read_u32(hle_esp).hle();
    let wnd_esp = hle_esp.wrapping_add(4);
    for i in 0..4 {
        emu.memory
            .write_u32(wnd_esp + 4 + i * 4, arg(emu, i + 1))
            .hle();
    }
    emu.memory.write_u32(wnd_esp, ret_addr).hle();
    emu.cpu
        .debug_replace_top_call(entry.addr, proc, ret_addr, wnd_esp + 4, wnd_esp)
        .hle();
    emu.cpu.set_reg(Reg::Esp, wnd_esp);
    emu.cpu.eip = proc;
    HleResult::Retn(20)
}

// LRESULT CallWindowProcA(WNDPROC proc, HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Transfer to the supplied guest window procedure with ANSI message payloads unchanged.
fn hle_call_window_proc_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_call_window_proc_w(emu, entry)
}

// LRESULT __wemu_button_wndproc(HWND hwnd, UINT msg, WPARAM w, LPARAM l)
// Emulate the built-in Button class default proc for owner-draw paint dispatch.
fn hle_button_wndproc(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    const WM_NCCREATE: u32 = 0x0081;
    const WM_CREATE: u32 = 0x0001;
    const WM_ENABLE: u32 = 0x000a;
    const WM_PAINT: u32 = 0x000f;
    const WM_ERASEBKGND: u32 = 0x0014;
    const WM_GETDLGCODE: u32 = 0x0087;
    const WM_PRINTCLIENT: u32 = 0x0318;
    const BM_CLICK: u32 = 0x00f5;
    const DLGC_BUTTON: u32 = 0x2000;
    const ODA_DRAWENTIRE: u32 = 0x0001;

    let hwnd = arg(emu, 0);
    let msg = arg(emu, 1);
    let wparam = arg(emu, 2);
    match msg {
        WM_NCCREATE => ret(emu, 1),
        WM_CREATE => ret(emu, 0),
        WM_GETDLGCODE => ret(emu, DLGC_BUTTON),
        WM_ERASEBKGND => ret(emu, 1),
        BM_CLICK => {
            if let Some(command) = emu.hle.command_from_click(hwnd) {
                emu.hle.app_messages.push(command);
                emu.hle.note_queued_message("BM_CLICK", command);
            }
            ret(emu, 0);
        }
        WM_ENABLE | WM_PAINT | WM_PRINTCLIENT => {
            if msg == WM_PAINT && wparam == 0 {
                take_window_invalid_rect(emu, hwnd);
                take_window_erase_pending(emu, hwnd);
                remove_pending_paint_message(emu, hwnd);
            }
            if button_window_is_ownerdraw(emu, hwnd)
                && dispatch_owner_draw_button_from_hle(
                    emu,
                    entry,
                    hwnd,
                    ODA_DRAWENTIRE,
                    wparam,
                    0,
                    16,
                    None,
                )
            {
                return HleResult::Retn(16);
            }
            ret(emu, 0);
        }
        _ => ret(emu, 0),
    }
    HleResult::Retn(16)
}

fn button_window_is_ownerdraw(emu: &Emulator, hwnd: u32) -> bool {
    emu.hle.window(hwnd).is_some_and(|window| {
        window.control_kind == HleControlKind::Button && button_style_is_ownerdraw(window.style)
    })
}

fn owner_draw_children_after_parent_paint(
    emu: &Emulator,
    parent: u32,
) -> Option<(u32, Option<OwnerDrawChain>)> {
    let mut children = emu
        .hle
        .windows
        .values()
        .filter(|window| {
            window.parent == parent
                && window.visible
                && !window.ddraw_owned
                && window.invalid_rect.is_some()
                && window.control_kind == HleControlKind::Button
                && button_style_is_ownerdraw(window.style)
        })
        .map(|window| (window.rect.top, window.rect.left, window.hwnd))
        .collect::<Vec<_>>();
    children.sort_unstable();
    let mut children = children
        .into_iter()
        .map(|(_, _, hwnd)| hwnd)
        .collect::<Vec<_>>();
    if children.is_empty() {
        return None;
    }
    let first = children.remove(0);
    let chain = (!children.is_empty()).then_some(OwnerDrawChain {
        children,
        next_index: 0,
    });
    Some((first, chain))
}

fn dispatch_owner_draw_button_from_hle(
    emu: &mut Emulator,
    entry: &HleEntry,
    hwnd: u32,
    action: u32,
    paint_hdc: u32,
    hle_return_value: u32,
    hle_arg_bytes: u32,
    chain: Option<OwnerDrawChain>,
) -> bool {
    let hle_esp = emu.cpu.reg(Reg::Esp);
    let original_ret = emu.memory.read_u32(hle_esp).hle();
    let original_ret_esp = hle_esp.wrapping_add(hle_arg_bytes);
    dispatch_owner_draw_button_at(
        emu,
        entry.addr,
        hwnd,
        action,
        paint_hdc,
        hle_return_value,
        original_ret,
        original_ret_esp,
        chain,
        false,
    )
}

fn dispatch_next_owner_draw_child_after_async(
    emu: &mut Emulator,
    entry: &HleEntry,
    mut chain: OwnerDrawChain,
    hle_return_value: u32,
) -> bool {
    while chain.next_index < chain.children.len() {
        let hwnd = chain.children[chain.next_index];
        chain.next_index += 1;
        let next_chain = (chain.next_index < chain.children.len()).then_some(chain);
        let original_ret_esp = emu.cpu.reg(Reg::Esp);
        let original_ret = emu.memory.read_u32(original_ret_esp).hle();
        return dispatch_owner_draw_button_at(
            emu,
            entry.addr,
            hwnd,
            0x0001,
            0,
            hle_return_value,
            original_ret,
            original_ret_esp,
            next_chain,
            true,
        );
    }
    false
}

fn dispatch_owner_draw_button_at(
    emu: &mut Emulator,
    source_addr: u32,
    hwnd: u32,
    action: u32,
    paint_hdc: u32,
    hle_return_value: u32,
    original_ret: u32,
    original_ret_esp: u32,
    chain: Option<OwnerDrawChain>,
    synthetic_debug_frame: bool,
) -> bool {
    const ODT_BUTTON: u32 = 4;

    let Some(window) = emu.hle.window(hwnd).cloned() else {
        return false;
    };
    let parent = if window.parent != 0 {
        window.parent
    } else {
        hwnd
    };
    let Some(parent_proc) = emu
        .hle
        .window(parent)
        .map(|window| window.proc)
        .filter(|proc| *proc != 0)
    else {
        return false;
    };
    take_window_invalid_rect(emu, hwnd);
    take_window_erase_pending(emu, hwnd);
    remove_pending_paint_message(emu, hwnd);
    let hdc = if paint_hdc != 0 {
        paint_hdc
    } else {
        create_gdi_screen_dc(emu, hwnd)
    };
    let draw_item = emu
        .hle
        .alloc(&mut emu.memory, 48, PagePerm::READ | PagePerm::WRITE)
        .hle();
    let (left, top, right, bottom) = window_client_rect(emu, hwnd);
    let state = if window.enabled { 0 } else { 0x0004 };
    emu.memory.write_u32(draw_item, ODT_BUTTON).hle();
    emu.memory.write_u32(draw_item + 4, window.id).hle();
    emu.memory.write_u32(draw_item + 8, 0).hle();
    emu.memory.write_u32(draw_item + 12, action).hle();
    emu.memory.write_u32(draw_item + 16, state).hle();
    emu.memory.write_u32(draw_item + 20, hwnd).hle();
    emu.memory.write_u32(draw_item + 24, hdc).hle();
    emu.memory.write_u32(draw_item + 28, left as u32).hle();
    emu.memory.write_u32(draw_item + 32, top as u32).hle();
    emu.memory.write_u32(draw_item + 36, right as u32).hle();
    emu.memory.write_u32(draw_item + 40, bottom as u32).hle();
    emu.memory.write_u32(draw_item + 44, 0).hle();

    let callback_esp = original_ret_esp.wrapping_sub(20);
    let async_return = emu.hle.async_return_thunk();
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, parent).hle();
    emu.memory.write_u32(callback_esp + 8, 0x002b).hle();
    emu.memory.write_u32(callback_esp + 12, window.id).hle();
    emu.memory.write_u32(callback_esp + 16, draw_item).hle();
    emu.memory.write_u32(callback_esp + 20, original_ret).hle();
    let hdc_to_drop = if paint_hdc == 0 { hdc } else { 0 };
    emu.hle.push_owner_draw_button_callback_return(
        hle_return_value,
        hdc_to_drop,
        draw_item,
        chain,
    );
    // Wine and ReactOS implement BS_OWNERDRAW by having the default Button
    // window proc send WM_DRAWITEM to the parent. This is a non-tail callback:
    // after the parent paints, wemu!__async_return must restore the original
    // Button-proc return instead of returning to this HLE thunk.
    if synthetic_debug_frame {
        emu.cpu
            .debug_push_synthetic_call(
                source_addr,
                parent_proc,
                async_return,
                callback_esp + 4,
                callback_esp,
            )
            .hle();
    } else {
        emu.cpu
            .debug_replace_top_call(
                source_addr,
                parent_proc,
                async_return,
                callback_esp + 4,
                callback_esp,
            )
            .hle();
    }
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = parent_proc;
    true
}

// int DrawTextW(HDC hdc, LPCWSTR text, int count, LPRECT rect, UINT format)
// Draw lossy wide text through the existing DrawTextA layout path.
fn hle_draw_text_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let text = wide_text_bytes(emu, arg(emu, 1), arg(emu, 2));
    let rect_ptr = arg(emu, 3);
    let format = arg(emu, 4);
    let scratch = emu
        .hle
        .alloc(
            &mut emu.memory,
            text.len() as u32 + 1,
            PagePerm::READ | PagePerm::WRITE,
        )
        .hle();
    emu.memory.write_bytes(scratch, &text).hle();
    emu.memory.write_u8(scratch + text.len() as u32, 0).hle();
    let esp = emu.cpu.reg(Reg::Esp);
    emu.memory.write_u32(esp + 8, scratch).hle();
    emu.memory.write_u32(esp + 12, text.len() as u32).hle();
    emu.memory.write_u32(esp + 16, rect_ptr).hle();
    emu.memory.write_u32(esp + 20, format).hle();
    let result = hle_draw_text_a(emu, &HleEntry {
        addr: 0,
        dll: "user32.dll",
        name: "DrawTextA",
        callback: hle_draw_text_a,
    });
    emu.cpu.set_reg(Reg::Esp, esp);
    emu.hle.free_alloc(&mut emu.memory, scratch).hle();
    let _ = hdc;
    result
}

// BOOL Rectangle(HDC hdc, int left, int top, int right, int bottom)
// Fill and outline the requested rectangle on fake GDI screen/surface DCs.
fn hle_rectangle(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let dc = gdi_dc_or_default(emu, hdc);
    if dc.surface != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        let rect = RectI {
            left: (arg(emu, 1) as i32).saturating_add(dc.origin_x),
            top: (arg(emu, 2) as i32).saturating_add(dc.origin_y),
            right: (arg(emu, 3) as i32).saturating_add(dc.origin_x),
            bottom: (arg(emu, 4) as i32).saturating_add(dc.origin_y),
        };
        draw_gdi_rect_brush(emu, surface, rect, dc.selected_brush);
        draw_gdi_rect_outline(emu, surface, rect, dc);
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// int FrameRect(HDC hdc, const RECT *rect, HBRUSH brush)
// Draw the four one-pixel PATCOPY edges, matching Wine's USER32 helper shape.
fn hle_frame_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let rect_ptr = arg(emu, 1);
    let brush = arg(emu, 2);
    if rect_ptr == 0 || brush == 0 {
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    let rect = read_gdi_rect(emu, rect_ptr);
    let rect = RectI {
        left: rect.0.saturating_add(dc.origin_x),
        top: rect.1.saturating_add(dc.origin_y),
        right: rect.2.saturating_add(dc.origin_x),
        bottom: rect.3.saturating_add(dc.origin_y),
    };
    if rect.width() <= 0 || rect.height() <= 0 {
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    if dc.surface != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        draw_gdi_rect_brush(
            emu,
            surface,
            RectI {
                left: rect.left,
                top: rect.top,
                right: rect.left + 1,
                bottom: rect.bottom,
            },
            brush,
        );
        draw_gdi_rect_brush(
            emu,
            surface,
            RectI {
                left: rect.right - 1,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            brush,
        );
        draw_gdi_rect_brush(
            emu,
            surface,
            RectI {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.top + 1,
            },
            brush,
        );
        draw_gdi_rect_brush(
            emu,
            surface,
            RectI {
                left: rect.left,
                top: rect.bottom - 1,
                right: rect.right,
                bottom: rect.bottom,
            },
            brush,
        );
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL InvertRect(HDC hdc, const RECT *rect)
// Invert destination pixels like Wine's PatBlt(DSTINVERT) implementation.
fn hle_invert_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let rect_ptr = arg(emu, 1);
    if rect_ptr != 0 && dc.surface != 0 {
        let rect = read_gdi_rect(emu, rect_ptr);
        let surface = read_surface_info(emu, dc.surface).hle();
        invert_gdi_rect(
            emu,
            surface,
            RectI {
                left: rect.0.saturating_add(dc.origin_x),
                top: rect.1.saturating_add(dc.origin_y),
                right: rect.2.saturating_add(dc.origin_x),
                bottom: rect.3.saturating_add(dc.origin_y),
            },
        );
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// HRGN CreateRectRgn(int left, int top, int right, int bottom)
// Allocate a tracked rectangle region handle.
fn hle_create_rect_rgn(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = emu.hle.create_gdi_region(WindowRect {
        left: arg(emu, 0) as i32,
        top: arg(emu, 1) as i32,
        right: arg(emu, 2) as i32,
        bottom: arg(emu, 3) as i32,
    });
    ret(emu, handle);
    HleResult::Retn(16)
}

// HRGN CreateRectRgnIndirect(const RECT *rect)
// Allocate a tracked rectangle region handle from a RECT.
fn hle_create_rect_rgn_indirect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = arg(emu, 0);
    let rect = if rect != 0 {
        let (left, top, right, bottom) = read_gdi_rect(emu, rect);
        WindowRect {
            left,
            top,
            right,
            bottom,
        }
    } else {
        WindowRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }
    };
    let handle = emu.hle.create_gdi_region(rect);
    ret(emu, handle);
    HleResult::Retn(4)
}

// int CombineRgn(HRGN dst, HRGN src1, HRGN src2, int mode)
// Combine tracked rectangle regions using simple copy/union/intersection bounds.
fn hle_combine_rgn(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const RGN_AND: u32 = 1;
    const RGN_COPY: u32 = 5;
    let dst = arg(emu, 0);
    let src1 = arg(emu, 1);
    let src2 = arg(emu, 2);
    let mode = arg(emu, 3);
    let a = emu.hle.gdi_regions.get(&src1).copied().unwrap_or(WindowRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    });
    let b = emu.hle.gdi_regions.get(&src2).copied().unwrap_or(a);
    let rect = match mode {
        RGN_AND => WindowRect {
            left: a.left.max(b.left),
            top: a.top.max(b.top),
            right: a.right.min(b.right),
            bottom: a.bottom.min(b.bottom),
        },
        RGN_COPY => a,
        _ => a.union(b),
    };
    if dst != 0 {
        emu.hle.gdi_regions.insert(dst, rect);
    }
    ret(emu, region_result(rect));
    HleResult::Retn(16)
}

// BOOL SetRectRgn(HRGN rgn, int left, int top, int right, int bottom)
// Update a tracked rectangle region.
fn hle_set_rect_rgn(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rgn = arg(emu, 0);
    if rgn != 0 {
        emu.hle.gdi_regions.insert(
            rgn,
            WindowRect {
                left: arg(emu, 1) as i32,
                top: arg(emu, 2) as i32,
                right: arg(emu, 3) as i32,
                bottom: arg(emu, 4) as i32,
            },
        );
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// BOOL SetRect(RECT *rect, int left, int top, int right, int bottom)
// Write the requested rectangle coordinates.
fn hle_set_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = arg(emu, 0);
    if rect != 0 {
        write_gdi_rect(
            emu,
            rect,
            (
                arg(emu, 1) as i32,
                arg(emu, 2) as i32,
                arg(emu, 3) as i32,
                arg(emu, 4) as i32,
            ),
        );
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// BOOL InflateRect(RECT *rect, int dx, int dy)
// Expand or contract a rectangle in place.
fn hle_inflate_rect(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rect = arg(emu, 0);
    let dx = arg(emu, 1) as i32;
    let dy = arg(emu, 2) as i32;
    if rect != 0 {
        let (left, top, right, bottom) = read_gdi_rect(emu, rect);
        write_gdi_rect(emu, rect, (left - dx, top - dy, right + dx, bottom + dy));
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

fn create_window_ex_common(emu: &mut Emulator, entry: &HleEntry, wide: bool) -> HleResult {
    const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
    const WH_CBT: i32 = 5;

    let ex_style = arg(emu, 0);
    let class_ptr = arg(emu, 1);
    let name_ptr = arg(emu, 2);
    let style = arg(emu, 3);
    let x = arg(emu, 4) as i32;
    let y = arg(emu, 5) as i32;
    let w = arg(emu, 6) as i32;
    let h = arg(emu, 7) as i32;
    let parent = arg(emu, 8);
    let mut id = arg(emu, 9);
    let inst = arg(emu, 10);
    let create_param = arg(emu, 11);
    let hwnd = emu.hle.alloc_window_handle(parent);
    let class_atom = if class_ptr < 0x10000 { class_ptr } else { 0 };
    let class_name = if class_ptr < 0x10000 {
        String::new()
    } else if wide {
        emu.memory
            .utf16z_lossy(class_ptr, 128)
            .unwrap_or_default()
            .to_ascii_lowercase()
    } else {
        emu.memory
            .cstr_lossy(class_ptr, 128)
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let proc = emu
        .hle
        .window_proc_for_class(&class_name, class_atom)
        .unwrap_or_else(|| {
            if class_name.eq_ignore_ascii_case("button") {
                emu.hle.button_wndproc_thunk
            } else if parent == 0 {
                emu.hle.window_proc
            } else {
                0
            }
        });
    if parent == 0 && id == 0 {
        id = emu
            .hle
            .window_menu_for_class(&class_name, class_atom)
            .unwrap_or(0);
    }
    let text = if name_ptr < 0x10000 {
        String::new()
    } else if wide {
        emu.memory.utf16z_lossy(name_ptr, 256).unwrap_or_default()
    } else {
        emu.memory.cstr_lossy(name_ptr, 256).unwrap_or_default()
    };
    let rect = if parent == 0 {
        let default_w = (emu.backend.width() as i32).max(320);
        let default_h = (emu.backend.height() as i32).max(240);
        let width = if w == CW_USEDEFAULT || w <= 0 {
            default_w.min(emu.backend.width() as i32)
        } else {
            w
        };
        let height = if h == CW_USEDEFAULT || h <= 0 {
            default_h.min(emu.backend.height() as i32)
        } else {
            h
        };
        let left = if x == CW_USEDEFAULT {
            ((emu.backend.width() as i32).saturating_sub(width) / 2).max(0)
        } else {
            x.max(0)
        };
        let top = if y == CW_USEDEFAULT {
            ((emu.backend.height() as i32).saturating_sub(height) / 3).max(0)
        } else {
            y.max(0)
        };
        WindowRect {
            left,
            top,
            right: left.saturating_add(width.max(1)),
            bottom: top.saturating_add(height.max(1)),
        }
    } else {
        let (client_left, client_top, _, _) = window_client_area(emu, parent);
        WindowRect {
            left: client_left.saturating_add(x.max(0)),
            top: client_top.saturating_add(y.max(0)),
            right: client_left
                .saturating_add(x.max(0))
                .saturating_add(w.max(1)),
            bottom: client_top
                .saturating_add(y.max(0))
                .saturating_add(h.max(1)),
        }
    };
    let create_param_head = if create_param >= 0x10000 {
        emu.memory.read_u32(create_param).unwrap_or(0)
    } else {
        0
    };
    trace_gdi!(
        "user CreateWindowEx hwnd={hwnd:08x} parent={parent:08x} class={class_name:?} text={text:?} style={style:08x} ex={ex_style:08x} rect=({},{}..{},{}) menu/id={id:08x} inst={inst:08x} param={create_param:08x} param0={create_param_head:08x} proc={proc:08x}",
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
    );
    emu.hle.register_window(HleWindow {
        hwnd,
        parent,
        id,
        class_name: class_name.clone(),
        text,
        rect,
        style,
        ex_style,
        proc,
        user_data: 0,
        extra: std::collections::HashMap::new(),
        enabled: (style & WS_DISABLED) == 0,
        visible: (style & WS_VISIBLE) != 0,
        control_kind: dialog_control_kind(&class_name),
        background_brush: emu.hle.window_background_for_class(&class_name, class_atom),
        invalid_rect: None,
        erase_pending: false,
        last_generated_paint_frame: 0,
        ddraw_owned: false,
    });
    ensure_gdi_screen_surface(emu);
    render_hle_windows(emu);
    queue_size_message(emu, hwnd, "CreateWindowEx");
    if (style & WS_VISIBLE) != 0 && proc != 0 {
        invalidate_window_rect(emu, hwnd, 0, true);
        queue_paint_message(emu, hwnd, "CreateWindowEx");
    }
    let create_args = CreateWindowArgs {
        ex_style,
        class_ptr,
        name_ptr,
        style,
        x,
        y,
        w,
        h,
        parent,
        menu: id,
        inst,
        param: create_param,
    };
    if let Some(hook) = emu.hle.latest_windows_hook(WH_CBT) {
        let continuation = if proc != 0 && !class_name.eq_ignore_ascii_case("edit") {
            Some(CreateWindowContinuation {
                hwnd,
                proc,
                msg: 0x0081,
                next_msg: Some(0x0001),
                args: create_args,
            })
        } else {
            None
        };
        dispatch_cbt_create_window_hook_callback(
            emu,
            entry,
            hook,
            hwnd,
            create_args,
            continuation,
        );
    } else if proc != 0 && !class_name.eq_ignore_ascii_case("edit") {
        dispatch_create_window_callback(
            emu,
            entry,
            hwnd,
            proc,
            0x0081,
            Some(0x0001),
            create_args,
        );
    } else {
        ret(emu, hwnd);
    }
    HleResult::Retn(48)
}

fn register_window_class_common(
    emu: &mut Emulator,
    class: u32,
    proc_offset: u32,
    inst_offset: u32,
    menu_offset: u32,
    name_offset: u32,
    wide: bool,
) -> u32 {
    if class == 0 {
        return 0;
    }
    let proc = emu.memory.read_u32(class + proc_offset).unwrap_or(0);
    let inst = emu.memory.read_u32(class + inst_offset).unwrap_or(0);
    let background = emu
        .memory
        .read_u32(class + menu_offset.saturating_sub(4))
        .unwrap_or(0);
    let menu_ptr = emu.memory.read_u32(class + menu_offset).unwrap_or(0);
    let name_ptr = emu.memory.read_u32(class + name_offset).unwrap_or(0);
    let name = read_window_class_name(emu, name_ptr, wide);
    emu.hle.window_proc = proc;
    let atom = emu.hle.register_window_class(&name, proc);
    let menu = read_resource_key(emu, menu_ptr, wide)
        .and_then(|key| load_menu_resource(emu, inst, &key))
        .unwrap_or(0);
    emu.hle.set_window_class_menu(&name, atom, menu);
    emu.hle.set_window_class_background(&name, atom, background);
    if emu.should_trace() {
        eprintln!("RegisterClass name={name:?} wndproc={proc:08x} menu={menu:08x} background={background:08x} atom={atom:04x}");
    }
    atom
}

// BOOL GetClassInfoA(HINSTANCE inst, LPCSTR class_name, LPWNDCLASSA out)
// Return the tracked WNDCLASSA fields for registered window classes.
fn hle_get_class_info_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let inst = arg(emu, 0);
    let name_ptr = arg(emu, 1);
    let out = arg(emu, 2);
    let atom = if name_ptr < 0x10000 { name_ptr } else { 0 };
    let name = read_window_class_name(emu, name_ptr, false);
    let Some(proc) = emu.hle.window_proc_for_class(&name, atom) else {
        emu.hle.last_error = 1411;
        ret(emu, 0);
        return HleResult::Retn(12);
    };

    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
        emu.memory.write_u32(out + 4, proc).hle();
        emu.memory.write_u32(out + 8, 0).hle();
        emu.memory.write_u32(out + 12, 0).hle();
        emu.memory.write_u32(out + 16, inst).hle();
        emu.memory.write_u32(out + 20, 0).hle();
        emu.memory.write_u32(out + 24, 0).hle();
        emu.memory
            .write_u32(out + 28, emu.hle.window_background_for_class(&name, atom))
            .hle();
        emu.memory.write_u32(out + 32, 0).hle();
        emu.memory.write_u32(out + 36, name_ptr).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

fn read_window_class_name(emu: &Emulator, name_ptr: u32, wide: bool) -> String {
    if name_ptr == 0 {
        String::new()
    } else if name_ptr < 0x10000 {
        format!("#{:x}", name_ptr)
    } else if wide {
        emu.memory
            .utf16z_lossy(name_ptr, 128)
            .unwrap_or_default()
            .to_ascii_lowercase()
    } else {
        emu.memory
            .cstr_lossy(name_ptr, 128)
            .unwrap_or_default()
            .to_ascii_lowercase()
    }
}

fn dispatch_edit_message(emu: &mut Emulator, message: Message) -> Option<u32> {
    const WM_SETFOCUS: u32 = 0x0007;
    const WM_KILLFOCUS: u32 = 0x0008;
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const WM_CHAR: u32 = 0x0102;
    const WM_MOUSEMOVE: u32 = 0x0200;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const WM_LBUTTONUP: u32 = 0x0202;
    const VK_BACK: u32 = 0x08;

    match message.msg {
        WM_SETFOCUS | WM_KILLFOCUS | WM_KEYUP | WM_MOUSEMOVE | WM_LBUTTONUP => Some(0),
        WM_LBUTTONDOWN => {
            emu.hle.focus_window = message.hwnd;
            Some(0)
        }
        WM_KEYDOWN if message.wparam == VK_BACK => {
            edit_backspace(emu, message.hwnd);
            Some(0)
        }
        WM_KEYDOWN => Some(0),
        WM_CHAR => {
            edit_insert_char(emu, message.hwnd, message.wparam);
            Some(0)
        }
        _ => None,
    }
}

fn clamp_scroll_pos(pos: i32, min: i32, max: i32) -> i32 {
    let low = min.min(max);
    let high = min.max(max);
    pos.clamp(low, high)
}

fn draw_gdi_glyph_rect(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    colorref: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    clip: Option<(i32, i32, i32, i32)>,
) {
    let mut left = x.max(0).min(surface.width as i32);
    let mut top = y.max(0).min(surface.height as i32);
    let mut right = x
        .saturating_add(width.max(1))
        .max(0)
        .min(surface.width as i32);
    let mut bottom = y
        .saturating_add(height.max(1))
        .max(0)
        .min(surface.height as i32);
    if let Some((clip_left, clip_top, clip_right, clip_bottom)) = clip {
        left = left.max(clip_left);
        top = top.max(clip_top);
        right = right.min(clip_right);
        bottom = bottom.min(clip_bottom);
    }
    if right <= left || bottom <= top {
        return;
    }

    let bpp = surface.bytes_per_pixel() as usize;
    let pixel = gdi_pixel_bytes(colorref, surface.bpp);
    let mut row = vec![0u8; (right - left) as usize * bpp];
    for chunk in row.chunks_mut(bpp) {
        chunk.copy_from_slice(&pixel[..bpp]);
    }
    for row_y in top..bottom {
        let addr = surface
            .buffer
            .wrapping_add(row_y as u32 * surface.pitch)
            .wrapping_add(left as u32 * bpp as u32);
        emu.memory.write_bytes(addr, &row).hle();
    }
}

fn draw_gdi_rect_colorref(emu: &mut Emulator, surface: SurfaceInfo, rect: RectI, colorref: u32) {
    if rect.width() <= 0 || rect.height() <= 0 {
        return;
    }
    draw_gdi_glyph_rect(
        emu,
        surface,
        colorref,
        rect.left,
        rect.top,
        rect.width(),
        rect.height(),
        None,
    );
    mark_gdi_screen_surface_dirty(emu, surface);
}

fn invert_gdi_rect(emu: &mut Emulator, surface: SurfaceInfo, rect: RectI) {
    let left = rect.left.max(0).min(surface.width as i32);
    let top = rect.top.max(0).min(surface.height as i32);
    let right = rect.right.max(left).min(surface.width as i32);
    let bottom = rect.bottom.max(top).min(surface.height as i32);
    let bpp = surface.bytes_per_pixel();
    for y in top..bottom {
        for x in left..right {
            let addr = surface
                .buffer
                .wrapping_add(y as u32 * surface.pitch)
                .wrapping_add(x as u32 * bpp);
            for offset in 0..bpp {
                let byte = emu.memory.read_u8(addr + offset).hle();
                emu.memory.write_u8(addr + offset, !byte).hle();
            }
        }
    }
    mark_gdi_screen_surface_dirty(emu, surface);
}

fn draw_gdi_rect_brush(emu: &mut Emulator, surface: SurfaceInfo, rect: RectI, brush: u32) {
    if let Some(color) = stock_brush_colorref(brush) {
        if let Some(color) = color {
            draw_gdi_rect_colorref(emu, surface, rect, color);
        }
        return;
    }
    if is_sys_color_brush(brush) {
        draw_gdi_rect_colorref(emu, surface, rect, sys_colorref(brush & 0xff));
        return;
    }
    if (1..=32).contains(&brush) {
        draw_gdi_rect_colorref(emu, surface, rect, sys_colorref(brush - 1));
        return;
    }
    if let Some(brush) = emu.hle.gdi_brushes.get(&brush).copied() {
        if brush.bitmap != 0 {
            if let Some(bitmap) = emu.hle.gdi_bitmaps.get(&brush.bitmap).copied() {
                if let Ok(src) = read_surface_info(emu, bitmap.surface) {
                    tile_surface_rect(emu, surface, src, rect);
                    return;
                }
            }
        }
        draw_gdi_rect_colorref(emu, surface, rect, brush.color);
    } else {
        draw_gdi_rect_colorref(emu, surface, rect, 0x00ff_ffff);
    }
}

fn draw_gdi_rect_outline(emu: &mut Emulator, surface: SurfaceInfo, rect: RectI, dc: GdiDc) {
    if rect.width() <= 0 || rect.height() <= 0 {
        return;
    }
    let Some(color) = selected_pen_colorref(emu, dc) else {
        return;
    };
    let right = rect.right.saturating_sub(1);
    let bottom = rect.bottom.saturating_sub(1);
    draw_gdi_line(
        emu,
        surface,
        rect.left,
        rect.top,
        right,
        rect.top,
        color,
        dc.rop2,
    );
    draw_gdi_line(emu, surface, rect.left, bottom, right, bottom, color, dc.rop2);
    draw_gdi_line(
        emu,
        surface,
        rect.left,
        rect.top,
        rect.left,
        bottom,
        color,
        dc.rop2,
    );
    draw_gdi_line(emu, surface, right, rect.top, right, bottom, color, dc.rop2);
}

fn read_gdi_rect(emu: &Emulator, rect: u32) -> (i32, i32, i32, i32) {
    (
        emu.memory.read_u32(rect).hle() as i32,
        emu.memory.read_u32(rect + 4).hle() as i32,
        emu.memory.read_u32(rect + 8).hle() as i32,
        emu.memory.read_u32(rect + 12).hle() as i32,
    )
}

fn write_gdi_rect(emu: &mut Emulator, rect: u32, value: (i32, i32, i32, i32)) {
    emu.memory.write_u32(rect, value.0 as u32).hle();
    emu.memory.write_u32(rect + 4, value.1 as u32).hle();
    emu.memory.write_u32(rect + 8, value.2 as u32).hle();
    emu.memory.write_u32(rect + 12, value.3 as u32).hle();
}

fn window_has_hle_frame(window: &HleWindow) -> bool {
    style_has_hle_frame(window.style)
}

pub(crate) fn dispatch_due_mm_timer_interrupt(emu: &mut Emulator) -> bool {
    let Some(timer) = emu
        .hle
        .take_due_mm_timer(emu.guest_time_ms, emu.current_scheduler_frame())
    else {
        return false;
    };
    let saved_cpu = emu.cpu.clone();
    let original_esp = saved_cpu.reg(Reg::Esp);
    let callback_esp = original_esp.wrapping_sub(24);
    let async_return = emu.hle.async_return_thunk();
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, timer.id).hle();
    emu.memory.write_u32(callback_esp + 8, 0).hle();
    emu.memory.write_u32(callback_esp + 12, timer.user).hle();
    emu.memory.write_u32(callback_esp + 16, 0).hle();
    emu.memory.write_u32(callback_esp + 20, 0).hle();
    emu.hle.push_async_cpu_callback_return(saved_cpu);
    emu.cpu
        .debug_push_synthetic_call(
            emu.cpu.eip,
            timer.callback,
            async_return,
            original_esp,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = timer.callback;
    true
}

fn dispatch_window_proc_callback(
    emu: &mut Emulator,
    entry: &HleEntry,
    hwnd: u32,
    proc: u32,
    msg: u32,
    wparam: u32,
    lparam: u32,
    hle_return_value: u32,
    hle_arg_bytes: u32,
) {
    dispatch_window_proc_message_callback(
        emu,
        entry,
        proc,
        Message {
            hwnd,
            msg,
            wparam,
            lparam,
        },
        hle_return_value,
        hle_arg_bytes,
        None,
    );
}

fn dispatch_window_proc_message_chain(
    emu: &mut Emulator,
    entry: &HleEntry,
    proc: u32,
    mut messages: Vec<Message>,
    hle_return_value: u32,
    hle_arg_bytes: u32,
) {
    if messages.is_empty() {
        ret(emu, hle_return_value);
        return;
    }
    let message = messages.remove(0);
    let continuation = (!messages.is_empty()).then_some(WindowProcMessageChain { proc, messages });
    dispatch_window_proc_message_callback(
        emu,
        entry,
        proc,
        message,
        hle_return_value,
        hle_arg_bytes,
        continuation,
    );
}

fn dispatch_window_proc_message_callback(
    emu: &mut Emulator,
    entry: &HleEntry,
    proc: u32,
    message: Message,
    hle_return_value: u32,
    hle_arg_bytes: u32,
    continuation: Option<WindowProcMessageChain>,
) {
    let hle_esp = emu.cpu.reg(Reg::Esp);
    let original_ret = emu.memory.read_u32(hle_esp).hle();
    let callback_esp = hle_esp.wrapping_add(hle_arg_bytes).wrapping_sub(20);
    let async_return = emu.hle.async_return_thunk();
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, message.hwnd).hle();
    emu.memory.write_u32(callback_esp + 8, message.msg).hle();
    emu.memory.write_u32(callback_esp + 12, message.wparam).hle();
    emu.memory.write_u32(callback_esp + 16, message.lparam).hle();
    emu.memory.write_u32(callback_esp + 20, original_ret).hle();
    if let Some(continuation) = continuation {
        emu.hle
            .push_window_proc_message_chain_return(hle_return_value, continuation);
    } else {
        emu.hle.push_hle_callback_return(hle_return_value);
    }
    emu.cpu
        .debug_replace_top_call(
            entry.addr,
            proc,
            async_return,
            callback_esp + 4,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = proc;
}

fn dispatch_window_proc_message_chain_after_async(
    emu: &mut Emulator,
    entry: &HleEntry,
    mut chain: WindowProcMessageChain,
    hle_return_value: u32,
) {
    if chain.messages.is_empty() {
        let esp = emu.cpu.reg(Reg::Esp);
        let ret_addr = emu.memory.read_u32(esp).hle();
        ret(emu, hle_return_value);
        emu.cpu.set_reg(Reg::Esp, esp.wrapping_add(4));
        emu.cpu.eip = ret_addr;
        return;
    }
    let message = chain.messages.remove(0);
    let proc = emu
        .hle
        .window(message.hwnd)
        .map(|window| window.proc)
        .filter(|proc| *proc != 0)
        .unwrap_or(chain.proc);
    let continuation = (!chain.messages.is_empty()).then_some(WindowProcMessageChain {
        proc,
        messages: chain.messages,
    });
    let original_ret_esp = emu.cpu.reg(Reg::Esp);
    let callback_esp = original_ret_esp.wrapping_sub(20);
    let async_return = emu.hle.async_return_thunk();
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, message.hwnd).hle();
    emu.memory.write_u32(callback_esp + 8, message.msg).hle();
    emu.memory.write_u32(callback_esp + 12, message.wparam).hle();
    emu.memory.write_u32(callback_esp + 16, message.lparam).hle();
    if let Some(continuation) = continuation {
        emu.hle
            .push_window_proc_message_chain_return(hle_return_value, continuation);
    } else {
        emu.hle.push_hle_callback_return(hle_return_value);
    }
    emu.cpu
        .debug_push_synthetic_call(
            entry.addr,
            proc,
            async_return,
            callback_esp + 4,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = proc;
}

pub(crate) fn render_hle_windows(emu: &mut Emulator) {
    emu.hle.mark_hle_windows_dirty();
    if !emu
        .hle
        .windows
        .values()
        .any(|window| window.parent == 0 && window.visible && !window.ddraw_owned)
    {
        return;
    }
    let surf = ensure_gdi_screen_surface(emu);
    let surface = read_surface_info(emu, surf).hle();
    present_surface_if_primary(emu, surface).hle();
}

fn redraw_hle_windows_on_surface_if_dirty(emu: &mut Emulator, surface: SurfaceInfo, surf: u32) {
    if !emu.hle.take_hle_windows_dirty() {
        return;
    }
    draw_hle_windows_on_surface(emu, surface, surf);
    draw_menu_overlays_on_surface(emu, surface, surf);
}

fn draw_hle_windows_on_surface(emu: &mut Emulator, surface: SurfaceInfo, surf: u32) {
    let mut windows = emu
        .hle
        .windows
        .values()
        .filter(|window| window.parent == 0 && window.visible && !window.ddraw_owned)
        .cloned()
        .collect::<Vec<_>>();
    if windows.is_empty() {
        return;
    }
    fill_desktop_around_windows(emu, surface, &windows);
    windows.sort_by_key(|window| window.hwnd);
    for window in windows {
        draw_hle_window(emu, surface, surf, &window);
    }
}

fn fill_desktop_around_windows(emu: &mut Emulator, surface: SurfaceInfo, windows: &[HleWindow]) {
    let mut fill_rects = vec![surface.full_rect()];
    for window in windows {
        let keep = RectI {
            left: window.rect.left,
            top: window.rect.top,
            right: window.rect.right,
            bottom: window.rect.bottom,
        };
        let mut next = Vec::new();
        for rect in fill_rects {
            next.extend(subtract_rect(rect, keep));
        }
        fill_rects = next;
    }
    for rect in fill_rects {
        fill_surface_rect(emu, surface, rect, 0x7bef).hle();
    }
}

fn subtract_rect(rect: RectI, cut: RectI) -> Vec<RectI> {
    let left = rect.left.max(cut.left);
    let top = rect.top.max(cut.top);
    let right = rect.right.min(cut.right);
    let bottom = rect.bottom.min(cut.bottom);
    if left >= right || top >= bottom {
        return vec![rect];
    }
    let mut out = Vec::with_capacity(4);
    push_nonempty_rect(
        &mut out,
        RectI {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: top,
        },
    );
    push_nonempty_rect(
        &mut out,
        RectI {
            left: rect.left,
            top: bottom,
            right: rect.right,
            bottom: rect.bottom,
        },
    );
    push_nonempty_rect(
        &mut out,
        RectI {
            left: rect.left,
            top,
            right: left,
            bottom,
        },
    );
    push_nonempty_rect(
        &mut out,
        RectI {
            left: right,
            top,
            right: rect.right,
            bottom,
        },
    );
    out
}

fn push_nonempty_rect(out: &mut Vec<RectI>, rect: RectI) {
    if rect.width() > 0 && rect.height() > 0 {
        out.push(rect);
    }
}

fn draw_hle_window(emu: &mut Emulator, surface: SurfaceInfo, surf: u32, window: &HleWindow) {
    if window.ddraw_owned {
        return;
    }
    let rect = RectI {
        left: window.rect.left,
        top: window.rect.top,
        right: window.rect.right,
        bottom: window.rect.bottom,
    };
    if window_has_hle_frame(window) {
        let menu_h = emu.hle.menu_bar_height_for_window(window);
        let client = RectI {
            left: rect.left + DIALOG_BORDER,
            top: rect.top + DIALOG_TITLE_HEIGHT + menu_h,
            right: rect.right - DIALOG_BORDER,
            bottom: rect.bottom - DIALOG_BORDER,
        };
        for frame_rect in subtract_rect(rect, client) {
            fill_surface_rect(emu, surface, frame_rect, 0xc618).hle();
        }
        draw_rect_outline(emu, surface, rect, 0x0000, 2);
        let title = RectI {
            left: rect.left + 2,
            top: rect.top + 2,
            right: rect.right - 2,
            bottom: (rect.top + DIALOG_TITLE_HEIGHT - 2).min(rect.bottom),
        };
        fill_surface_rect(emu, surface, title, 0x0010).hle();
        draw_text_left(
            emu,
            surf,
            &window.text,
            title.left + 6,
            title.top + 4,
            title,
            0x00ff_ffff,
        );
        if hle_compositor_owns_client_background(window) {
            fill_hle_window_client_background(emu, surface, window, client);
        }
    }

    draw_menu_bar_for_window(emu, surface, surf, window);
    draw_hle_window_children(emu, surface, surf, window);
}

fn draw_hle_window_children(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    surf: u32,
    window: &HleWindow,
) {
    let mut children = emu
        .hle
        .windows
        .values()
        .filter(|child| child.parent == window.hwnd && child.visible && !child.ddraw_owned)
        .cloned()
        .collect::<Vec<_>>();
    children.sort_by_key(|child| (child.rect.top, child.rect.left, child.hwnd));
    for child in children {
        if window_has_hle_frame(&child) {
            draw_hle_window(emu, surface, surf, &child);
        } else {
            draw_hle_control(emu, surface, surf, &child);
            draw_hle_window_children(emu, surface, surf, &child);
        }
    }
}

fn fill_hle_window_client_background(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    window: &HleWindow,
    client: RectI,
) {
    let mut fill_rects = vec![client];
    let children = emu
        .hle
        .windows
        .values()
        .filter(|child| child.parent == window.hwnd && child.visible)
        .map(|child| {
            (
                child.hwnd,
                RectI {
                    left: child.rect.left,
                    top: child.rect.top,
                    right: child.rect.right,
                    bottom: child.rect.bottom,
                },
            )
        })
        .collect::<Vec<_>>();
    for (_, keep) in children
        .iter()
        .copied()
    {
        let mut next = Vec::new();
        for rect in fill_rects {
            next.extend(subtract_rect(rect, keep));
        }
        fill_rects = next;
    }
    for rect in fill_rects {
        if window.background_brush != 0 {
            draw_gdi_rect_brush(emu, surface, rect, window.background_brush);
        } else {
            fill_surface_rect(emu, surface, rect, 0xc618).hle();
        }
    }
}

fn hle_compositor_owns_client_background(window: &HleWindow) -> bool {
    // Wine erases client areas through WM_ERASEBKGND/BeginPaint and EndPaint only
    // flushes window surfaces. Avoid repainting app-owned clients on every present.
    window.class_name.eq_ignore_ascii_case("#32770")
}

fn fill_framebuffer_rect(emu: &mut Emulator, mut rect: RectI, color: [u8; 4]) {
    let width = emu.backend.width() as i32;
    let height = emu.backend.height() as i32;
    rect.left = rect.left.max(0).min(width);
    rect.top = rect.top.max(0).min(height);
    rect.right = rect.right.max(0).min(width);
    rect.bottom = rect.bottom.max(0).min(height);
    if rect.width() <= 0 || rect.height() <= 0 {
        return;
    }

    let stride = width as usize * 4;
    let fb = emu.backend.framebuffer_mut();
    for y in rect.top..rect.bottom {
        let start = y as usize * stride + rect.left as usize * 4;
        let end = y as usize * stride + rect.right as usize * 4;
        for px in fb[start..end].chunks_mut(4) {
            px.copy_from_slice(&color);
        }
    }
}

fn draw_framebuffer_rect_outline(
    emu: &mut Emulator,
    rect: RectI,
    color: [u8; 4],
    thickness: i32,
) {
    for i in 0..thickness.max(1) {
        fill_framebuffer_rect(
            emu,
            RectI {
                left: rect.left + i,
                top: rect.top + i,
                right: rect.right - i,
                bottom: rect.top + i + 1,
            },
            color,
        );
        fill_framebuffer_rect(
            emu,
            RectI {
                left: rect.left + i,
                top: rect.top + i,
                right: rect.left + i + 1,
                bottom: rect.bottom - i,
            },
            color,
        );
        fill_framebuffer_rect(
            emu,
            RectI {
                left: rect.left + i,
                top: rect.bottom - i - 1,
                right: rect.right - i,
                bottom: rect.bottom - i,
            },
            color,
        );
        fill_framebuffer_rect(
            emu,
            RectI {
                left: rect.right - i - 1,
                top: rect.top + i,
                right: rect.right - i,
                bottom: rect.bottom - i,
            },
            color,
        );
    }
}

fn draw_framebuffer_glyph_rect(
    emu: &mut Emulator,
    color: [u8; 4],
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    clip: Option<(i32, i32, i32, i32)>,
) {
    let mut rect = RectI {
        left: x,
        top: y,
        right: x.saturating_add(width.max(1)),
        bottom: y.saturating_add(height.max(1)),
    };
    if let Some((clip_left, clip_top, clip_right, clip_bottom)) = clip {
        rect.left = rect.left.max(clip_left);
        rect.top = rect.top.max(clip_top);
        rect.right = rect.right.min(clip_right);
        rect.bottom = rect.bottom.min(clip_bottom);
    }
    fill_framebuffer_rect(emu, rect, color);
}

fn draw_rect_outline(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    rect: RectI,
    color: u32,
    thickness: i32,
) {
    for i in 0..thickness.max(1) {
        fill_surface_rect(
            emu,
            surface,
            RectI {
                left: rect.left + i,
                top: rect.top + i,
                right: rect.right - i,
                bottom: rect.top + i + 1,
            },
            color,
        )
        .hle();
        fill_surface_rect(
            emu,
            surface,
            RectI {
                left: rect.left + i,
                top: rect.top + i,
                right: rect.left + i + 1,
                bottom: rect.bottom - i,
            },
            color,
        )
        .hle();
        fill_surface_rect(
            emu,
            surface,
            RectI {
                left: rect.left + i,
                top: rect.bottom - i - 1,
                right: rect.right - i,
                bottom: rect.bottom - i,
            },
            color,
        )
        .hle();
        fill_surface_rect(
            emu,
            surface,
            RectI {
                left: rect.right - i - 1,
                top: rect.top + i,
                right: rect.right - i,
                bottom: rect.bottom - i,
            },
            color,
        )
        .hle();
    }
}

fn window_client_area(emu: &Emulator, hwnd: u32) -> (i32, i32, i32, i32) {
    emu.hle
        .window(hwnd)
        .map(|window| {
            let menu_h = emu.hle.menu_bar_height_for_window(window);
            if window_has_hle_frame(window) {
                (
                    window.rect.left.saturating_add(DIALOG_BORDER),
                    window
                        .rect
                        .top
                        .saturating_add(DIALOG_TITLE_HEIGHT)
                        .saturating_add(menu_h),
                    window.rect.right.saturating_sub(DIALOG_BORDER),
                    window.rect.bottom.saturating_sub(DIALOG_BORDER),
                )
            } else {
                (
                    window.rect.left,
                    window.rect.top.saturating_add(menu_h),
                    window.rect.right,
                    window.rect.bottom,
                )
            }
        })
        .unwrap_or((0, 0, emu.backend.width() as i32, emu.backend.height() as i32))
}

fn map_window_origin(emu: &Emulator, hwnd: u32) -> Option<(i32, i32)> {
    if hwnd == 0 || hwnd == HLE_DESKTOP_WINDOW || hwnd == 0x0002_0001 {
        return Some((0, 0));
    }
    emu.hle
        .window(hwnd)
        .map(|_| {
            let (left, top, _, _) = window_client_area(emu, hwnd);
            (left, top)
        })
}

fn make_map_window_points_result(dx: i32, dy: i32) -> u32 {
    (dx as u16 as u32) | ((dy as u16 as u32) << 16)
}

fn window_client_rect(emu: &Emulator, hwnd: u32) -> (i32, i32, i32, i32) {
    let (left, top, right, bottom) = window_client_area(emu, hwnd);
    (
        0,
        0,
        right.saturating_sub(left).max(0),
        bottom.saturating_sub(top).max(0),
    )
}

fn invalidate_window_rect(emu: &mut Emulator, hwnd: u32, rect_ptr: u32, erase: bool) {
    let Some(rect) = paint_rect_from_arg(emu, hwnd, rect_ptr) else {
        return;
    };
    if let Some(window) = emu.hle.window_mut(hwnd) {
        window.invalid_rect = Some(
            window
                .invalid_rect
                .map(|old| old.union(rect))
                .unwrap_or(rect),
        );
        window.erase_pending |= erase;
        trace_gdi!(
            "user invalidate hwnd={hwnd:08x} rect=({},{}..{},{}) erase={erase}",
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
        );
    }
}

fn take_window_invalid_rect(emu: &mut Emulator, hwnd: u32) -> Option<WindowRect> {
    emu.hle
        .window_mut(hwnd)
        .and_then(|window| window.invalid_rect.take())
}

fn validate_window_paint(emu: &mut Emulator, hwnd: u32) {
    if let Some(window) = emu.hle.window_mut(hwnd) {
        window.invalid_rect = None;
        window.erase_pending = false;
    }
    remove_pending_paint_message(emu, hwnd);
}

fn take_window_erase_pending(emu: &mut Emulator, hwnd: u32) -> bool {
    emu.hle
        .window_mut(hwnd)
        .map(|window| {
            let erase = window.erase_pending;
            window.erase_pending = false;
            erase
        })
        .unwrap_or(false)
}

fn erase_window_background(emu: &mut Emulator, hwnd: u32, rect: WindowRect) -> bool {
    let brush = emu
        .hle
        .window(hwnd)
        .map(|window| window.background_brush)
        .unwrap_or(0);
    if brush == 0 {
        return false;
    }
    let hdc = create_gdi_screen_dc(emu, hwnd);
    let dc = gdi_dc_or_default(emu, hdc);
    if dc.surface == 0 {
        emu.hle.gdi_dcs.remove(&hdc);
        return false;
    }
    let surface = read_surface_info(emu, dc.surface).hle();
    draw_gdi_rect_brush(
        emu,
        surface,
        RectI {
            left: rect.left.saturating_add(dc.origin_x),
            top: rect.top.saturating_add(dc.origin_y),
            right: rect.right.saturating_add(dc.origin_x),
            bottom: rect.bottom.saturating_add(dc.origin_y),
        },
        brush,
    );
    emu.hle.gdi_dcs.remove(&hdc);
    true
}

fn paint_rect_from_arg(emu: &Emulator, hwnd: u32, rect_ptr: u32) -> Option<WindowRect> {
    let client = window_client_rect(emu, hwnd);
    let client_rect = WindowRect {
        left: client.0,
        top: client.1,
        right: client.2,
        bottom: client.3,
    };
    if client_rect.is_empty() {
        return None;
    }
    let raw = if rect_ptr == 0 {
        client_rect
    } else {
        let (left, top, right, bottom) = read_gdi_rect(emu, rect_ptr);
        WindowRect {
            left,
            top,
            right,
            bottom,
        }
    };
    let clipped = WindowRect {
        left: raw.left.max(client_rect.left).min(client_rect.right),
        top: raw.top.max(client_rect.top).min(client_rect.bottom),
        right: raw.right.max(client_rect.left).min(client_rect.right),
        bottom: raw.bottom.max(client_rect.top).min(client_rect.bottom),
    };
    (!clipped.is_empty()).then_some(clipped)
}

fn queue_paint_message(emu: &mut Emulator, hwnd: u32, source: &'static str) {
    if emu
        .hle
        .window(hwnd)
        .is_some_and(|window| !window.visible)
    {
        return;
    }
    if emu
        .hle
        .app_messages
        .iter()
        .any(|message| message.hwnd == hwnd && message.msg == 0x000f)
    {
        return;
    }
    let message = Message {
        hwnd,
        msg: 0x000f,
        wparam: 0,
        lparam: 0,
    };
    // Wine/ReactOS both avoid dispatching a child WM_PAINT while an unpainted
    // ancestor is dirty. Keep the same ordering in our simplified paint queue
    // so parent background work cannot cover a child that already repainted.
    let index = emu
        .hle
        .app_messages
        .iter()
        .position(|old| old.msg == 0x000f && window_is_descendant(emu, old.hwnd, hwnd))
        .unwrap_or(emu.hle.app_messages.len());
    emu.hle.app_messages.insert(index, message);
    emu.hle.note_queued_message(source, message);
}

fn window_is_descendant(emu: &Emulator, mut child: u32, ancestor: u32) -> bool {
    if child == 0 || ancestor == 0 || child == ancestor {
        return false;
    }
    while let Some(window) = emu.hle.window(child) {
        if window.parent == ancestor {
            return true;
        }
        if window.parent == 0 || window.parent == child {
            return false;
        }
        child = window.parent;
    }
    false
}

fn post_message_target_valid(emu: &Emulator, hwnd: u32) -> bool {
    const HWND_BROADCAST: u32 = 0x0000_ffff;
    hwnd == 0 || hwnd == HWND_BROADCAST || emu.hle.window(hwnd).is_some()
}

fn topmost_child_window(emu: &Emulator, parent: u32) -> Option<u32> {
    child_windows_top_to_bottom(emu, parent).into_iter().next()
}

fn sibling_window(emu: &Emulator, hwnd: u32, first: bool) -> u32 {
    let Some(parent) = window_sibling_parent(emu, hwnd) else {
        return 0;
    };
    let siblings = child_windows_top_to_bottom(emu, parent);
    if first {
        siblings.first().copied().unwrap_or(0)
    } else {
        siblings.last().copied().unwrap_or(0)
    }
}

fn adjacent_sibling_window(emu: &Emulator, hwnd: u32, previous: bool) -> u32 {
    if hwnd == 0 || hwnd == HLE_DESKTOP_WINDOW {
        return 0;
    }
    let Some(parent) = window_sibling_parent(emu, hwnd) else {
        return 0;
    };
    let siblings = child_windows_top_to_bottom(emu, parent);
    let Some(index) = siblings.iter().position(|sibling| *sibling == hwnd) else {
        return 0;
    };
    if previous {
        index
            .checked_sub(1)
            .and_then(|index| siblings.get(index))
            .copied()
            .unwrap_or(0)
    } else {
        siblings.get(index + 1).copied().unwrap_or(0)
    }
}

fn window_sibling_parent(emu: &Emulator, hwnd: u32) -> Option<u32> {
    if hwnd == 0 || hwnd == HLE_DESKTOP_WINDOW {
        Some(0)
    } else {
        emu.hle.window(hwnd).map(|window| window.parent)
    }
}

fn child_windows_top_to_bottom(emu: &Emulator, parent: u32) -> Vec<u32> {
    let mut hwnds = emu
        .hle
        .windows
        .values()
        .filter(|window| window.parent == parent)
        .map(|window| window.hwnd)
        .collect::<Vec<_>>();
    // We do not yet keep an explicit z-order list. Window handles are allocated
    // monotonically, so newest/highest HWND is the best generic topmost proxy.
    hwnds.sort_unstable_by(|a, b| b.cmp(a));
    hwnds
}

fn window_fully_obscured_by_higher_top_level(emu: &Emulator, hwnd: u32) -> bool {
    let Some(window) = emu.hle.window(hwnd) else {
        return false;
    };
    if window.parent != 0 || !window.visible || window.rect.is_empty() {
        return false;
    }
    let rank = top_level_z_rank(emu, window.hwnd);
    emu.hle.windows.values().any(|other| {
        top_level_z_rank(emu, other.hwnd) > rank
            && other.parent == 0
            && other.visible
            && !other.rect.is_empty()
            && rect_contains(other.rect, window.rect)
    })
}

fn window_screen_point_occluded_by_higher_top_level(
    emu: &Emulator,
    hwnd: u32,
    x: i32,
    y: i32,
) -> bool {
    let Some(top) = top_level_window_for_hwnd(emu, hwnd) else {
        return false;
    };
    if !top.visible || !top.rect.contains(x, y) {
        return true;
    }
    let rank = top_level_z_rank(emu, top.hwnd);
    emu.hle.windows.values().any(|other| {
        top_level_z_rank(emu, other.hwnd) > rank
            && other.parent == 0
            && other.visible
            && other.rect.contains(x, y)
    })
}

fn top_level_window_for_hwnd<'a>(emu: &'a Emulator, hwnd: u32) -> Option<&'a HleWindow> {
    let mut window = emu.hle.window(hwnd)?;
    while window.parent != 0 {
        window = emu.hle.window(window.parent)?;
    }
    Some(window)
}

fn top_level_z_rank(emu: &Emulator, hwnd: u32) -> u64 {
    let focused = top_level_hwnd_for(emu, emu.hle.focus_window);
    if focused == Some(hwnd) {
        u64::MAX
    } else {
        hwnd as u64
    }
}

fn top_level_hwnd_for(emu: &Emulator, hwnd: u32) -> Option<u32> {
    top_level_window_for_hwnd(emu, hwnd).map(|window| window.hwnd)
}

fn rect_contains(outer: WindowRect, inner: WindowRect) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

fn remove_pending_paint_message(emu: &mut Emulator, hwnd: u32) {
    emu.hle
        .app_messages
        .retain(|message| !(message.hwnd == hwnd && message.msg == 0x000f));
}

fn queue_size_message(emu: &mut Emulator, hwnd: u32, source: &'static str) {
    let Some(window) = emu.hle.window(hwnd) else {
        return;
    };
    if window.proc == 0 {
        return;
    }
    let (left, top, right, bottom) = window_client_area(emu, hwnd);
    let width = right.saturating_sub(left).max(0) as u32;
    let height = bottom.saturating_sub(top).max(0) as u32;
    let message = Message {
        hwnd,
        msg: 0x0005, // WM_SIZE
        wparam: 0,
        lparam: (width & 0xffff) | ((height & 0xffff) << 16),
    };
    emu.hle.app_messages.push(message);
    emu.hle.note_queued_message(source, message);
}

fn dispatch_create_window_callback(
    emu: &mut Emulator,
    entry: &HleEntry,
    hwnd: u32,
    proc: u32,
    msg: u32,
    next_msg: Option<u32>,
    args: CreateWindowArgs,
) {
    let hle_esp = emu.cpu.reg(Reg::Esp);
    let original_ret = emu.memory.read_u32(hle_esp).hle();
    let callback_esp = hle_esp.wrapping_add(28);
    let create_struct = alloc_create_struct(emu);
    let async_return = emu.hle.async_return_thunk();
    trace_gdi!(
        "user CreateWindow callback hwnd={hwnd:08x} proc={proc:08x} msg={msg:04x} next={next_msg:?}"
    );
    write_create_struct(emu, create_struct, args);
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, hwnd).hle();
    emu.memory.write_u32(callback_esp + 8, msg).hle();
    emu.memory.write_u32(callback_esp + 12, 0).hle();
    emu.memory.write_u32(callback_esp + 16, create_struct).hle();
    emu.memory.write_u32(callback_esp + 20, original_ret).hle();
    if let Some(next_msg) = next_msg {
        // Win32 creates a window through several synchronous guest callbacks.
        // Keep the next message on the HLE callback stack until this one returns.
        emu.hle.push_create_window_callback_return(
            hwnd,
            CreateWindowContinuation {
                hwnd,
                proc,
                msg: next_msg,
                next_msg: None,
                args,
            },
        );
    } else {
        emu.hle.push_hle_callback_return(hwnd);
    }
    emu.cpu
        .debug_replace_top_call(
            entry.addr,
            proc,
            async_return,
            callback_esp + 4,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = proc;
}

fn dispatch_cbt_create_window_hook_callback(
    emu: &mut Emulator,
    entry: &HleEntry,
    hook: Hook,
    hwnd: u32,
    args: CreateWindowArgs,
    continuation: Option<CreateWindowContinuation>,
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
    trace_gdi!(
        "user CreateWindow hook hwnd={hwnd:08x} hook={:08x} continuation={}",
        hook.proc,
        continuation.is_some()
    );
    emu.memory.write_u32(cbt_create_wnd, create_struct).hle();
    emu.memory.write_u32(cbt_create_wnd + 4, 0).hle();
    write_create_struct(emu, create_struct, args);
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, HCBT_CREATEWND).hle();
    emu.memory.write_u32(callback_esp + 8, hwnd).hle();
    emu.memory.write_u32(callback_esp + 12, cbt_create_wnd).hle();
    emu.memory.write_u32(callback_esp + 16, original_ret).hle();
    if let Some(continuation) = continuation {
        emu.hle
            .push_create_window_callback_return(hwnd, continuation);
    } else {
        emu.hle.push_hle_callback_return(hwnd);
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

fn dispatch_create_window_callback_after_async(
    emu: &mut Emulator,
    entry: &HleEntry,
    continuation: CreateWindowContinuation,
) {
    // CBT hooks commonly subclass the HWND during creation. The following
    // create callback must use the current proc, not the class proc captured
    // before the non-tail hook returned to HLE.
    let proc = emu
        .hle
        .window(continuation.hwnd)
        .map(|window| window.proc)
        .filter(|proc| *proc != 0)
        .unwrap_or(continuation.proc);
    let next_msg = continuation.next_msg;
    let original_ret_esp = emu.cpu.reg(Reg::Esp);
    let callback_esp = original_ret_esp.wrapping_sub(20);
    let create_struct = alloc_create_struct(emu);
    let async_return = emu.hle.async_return_thunk();
    trace_gdi!(
        "user CreateWindow continuation hwnd={:08x} proc={proc:08x} msg={:04x} next={next_msg:?} param={:08x}",
        continuation.hwnd,
        continuation.msg,
        continuation.args.param
    );
    write_create_struct(emu, create_struct, continuation.args);
    emu.memory.write_u32(callback_esp, async_return).hle();
    emu.memory.write_u32(callback_esp + 4, continuation.hwnd).hle();
    emu.memory.write_u32(callback_esp + 8, continuation.msg).hle();
    emu.memory.write_u32(callback_esp + 12, 0).hle();
    emu.memory.write_u32(callback_esp + 16, create_struct).hle();
    if let Some(next_msg) = next_msg {
        // Non-tail create callbacks chain through wemu!__async_return: after
        // WM_NCCREATE returns, HLE must re-enter the current window proc for WM_CREATE.
        emu.hle.push_create_window_callback_return(
            continuation.hwnd,
            CreateWindowContinuation {
                hwnd: continuation.hwnd,
                proc,
                msg: next_msg,
                next_msg: None,
                args: continuation.args,
            },
        );
    } else {
        emu.hle.push_hle_callback_return(continuation.hwnd);
    }
    // This is a non-tail HLE callback continuation: the CBT hook has already
    // returned to wemu!__async_return, so the original HLE border frame is no
    // longer the top debug frame. Push a synthetic frame for the window proc
    // instead of replacing the caller's normal guest frame.
    emu.cpu
        .debug_push_synthetic_call(
            entry.addr,
            proc,
            async_return,
            callback_esp + 4,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = proc;
}

fn write_create_struct(emu: &mut Emulator, create_struct: u32, args: CreateWindowArgs) {
    emu.memory.write_u32(create_struct, args.param).hle();
    emu.memory.write_u32(create_struct + 4, args.inst).hle();
    emu.memory.write_u32(create_struct + 8, args.menu).hle();
    emu.memory.write_u32(create_struct + 12, args.parent).hle();
    emu.memory.write_u32(create_struct + 16, args.h as u32).hle();
    emu.memory.write_u32(create_struct + 20, args.w as u32).hle();
    emu.memory.write_u32(create_struct + 24, args.y as u32).hle();
    emu.memory.write_u32(create_struct + 28, args.x as u32).hle();
    emu.memory.write_u32(create_struct + 32, args.style).hle();
    emu.memory.write_u32(create_struct + 36, args.name_ptr).hle();
    emu.memory.write_u32(create_struct + 40, args.class_ptr).hle();
    emu.memory.write_u32(create_struct + 44, args.ex_style).hle();
}

fn alloc_create_struct(emu: &mut Emulator) -> u32 {
    // Guest window procs keep the CREATESTRUCT pointer while making nested calls.
    // Allocate stable guest memory instead of placing it below ESP where frames grow.
    emu.hle
        .alloc(&mut emu.memory, 48, PagePerm::READ | PagePerm::WRITE)
        .hle()
}
