// HFONT CreateFontA(int h, int w, int esc, int orient, int weight, DWORD italic, DWORD underline, DWORD strike, DWORD charset, DWORD out, DWORD clip, DWORD quality, DWORD pitch, LPCSTR face)
// Create a fake font handle with enough height information for text extent and raster HLE.
fn hle_create_font_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let raw_height = arg(emu, 0) as i32;
    let height = raw_height.unsigned_abs().max(1);
    let handle = emu.hle.create_gdi_font(height);
    ret(emu, handle);
    HleResult::Retn(56)
}

fn font_resource_impl(emu: &mut Emulator, path: &str, add: bool) {
    trace_gdi!(
        "gdi {}FontResource path={path:?}",
        if add { "Add" } else { "Remove" }
    );
    ret(emu, 1);
}

// int AddFontResourceA(LPCSTR path)
// Pretend legacy game font files were registered; built-in text rasterization remains active.
fn hle_add_font_resource_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let path = emu.memory.cstr_lossy(arg(emu, 0), 260).unwrap_or_default();
    font_resource_impl(emu, &path, true);
    HleResult::Retn(4)
}

// int AddFontResourceW(LPCWSTR path)
// Pretend legacy game font files were registered; built-in text rasterization remains active.
fn hle_add_font_resource_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let path = emu
        .memory
        .utf16z_lossy(arg(emu, 0), 260)
        .unwrap_or_default();
    font_resource_impl(emu, &path, true);
    HleResult::Retn(4)
}

// BOOL RemoveFontResourceA(LPCSTR path)
// Accept removal of fake registered legacy game font files.
fn hle_remove_font_resource_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let path = emu.memory.cstr_lossy(arg(emu, 0), 260).unwrap_or_default();
    font_resource_impl(emu, &path, false);
    HleResult::Retn(4)
}

// BOOL RemoveFontResourceW(LPCWSTR path)
// Accept removal of fake registered legacy game font files.
fn hle_remove_font_resource_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let path = emu
        .memory
        .utf16z_lossy(arg(emu, 0), 260)
        .unwrap_or_default();
    font_resource_impl(emu, &path, false);
    HleResult::Retn(4)
}

// HGDIOBJ SelectObject(HDC hdc, HGDIOBJ obj)
// Select tracked fonts or bitmaps into a DC and return the previous selected object.
fn hle_select_object(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let object = arg(emu, 1);
    let mut old = 0;
    if emu.hle.gdi_fonts.contains_key(&object) {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_font;
            dc.selected_font = object;
        }
        trace_gdi!("gdi SelectObject font hdc={hdc:08x} obj={object:08x} old={old:08x}");
    } else if object == GDI_STOCK_BITMAP {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_bitmap;
            dc.selected_bitmap = object;
            dc.surface = 0;
        }
        trace_gdi!(
            "gdi SelectObject stock-bitmap hdc={hdc:08x} obj={object:08x} old={old:08x}"
        );
    } else if let Some(bitmap) = emu.hle.gdi_bitmaps.get(&object).copied() {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_bitmap;
            dc.selected_bitmap = object;
            dc.surface = bitmap.surface;
        }
        trace_gdi!(
            "gdi SelectObject bitmap hdc={hdc:08x} obj={object:08x} old={old:08x} surf={:08x}",
            bitmap.surface
        );
    } else if emu.hle.gdi_brushes.contains_key(&object) || is_sys_color_brush(object) {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_brush;
            dc.selected_brush = object;
        }
        trace_gdi!("gdi SelectObject brush hdc={hdc:08x} obj={object:08x} old={old:08x}");
    } else if stock_brush_colorref(object).is_some() {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_brush;
            dc.selected_brush = object;
        }
        trace_gdi!("gdi SelectObject stock-brush hdc={hdc:08x} obj={object:08x} old={old:08x}");
    } else if emu.hle.gdi_pens.contains_key(&object) {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_pen;
            dc.selected_pen = object;
        }
        trace_gdi!("gdi SelectObject pen hdc={hdc:08x} obj={object:08x} old={old:08x}");
    } else if stock_pen_colorref(object).is_some() {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_pen;
            dc.selected_pen = object;
        }
        trace_gdi!("gdi SelectObject stock-pen hdc={hdc:08x} obj={object:08x} old={old:08x}");
    } else if is_stock_font(object) {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_font;
            dc.selected_font = object;
        }
        trace_gdi!("gdi SelectObject stock-font hdc={hdc:08x} obj={object:08x} old={old:08x}");
    } else {
        trace_gdi!("gdi SelectObject unknown hdc={hdc:08x} obj={object:08x}");
    }
    ret(emu, old);
    HleResult::Retn(8)
}

// int wsprintfW(LPWSTR out, LPCWSTR fmt, ...)
// Format the simple wide status strings used by Notepad.
fn hle_wsprintf_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let fmt = emu
        .memory
        .utf16z_lossy(arg(emu, 1), 512)
        .unwrap_or_default();
    let mut args = 2;
    let mut text = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            text.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => text.push('%'),
            Some('d') | Some('u') => {
                text.push_str(&(arg(emu, args) as i32).to_string());
                args += 1;
            }
            Some('s') => {
                text.push_str(
                    &emu.memory
                        .utf16z_lossy(arg(emu, args), 512)
                        .unwrap_or_default(),
                );
                args += 1;
            }
            Some(other) => {
                text.push('%');
                text.push(other);
            }
            None => break,
        }
    }
    if out != 0 {
        emu.memory
            .write_utf16z(out, &text, text.encode_utf16().count() + 1)
            .hle();
    }
    ret(emu, text.encode_utf16().count() as u32);
    HleResult::Retn(0)
}

// int wsprintfA(LPSTR out, LPCSTR fmt, ...)
// Format ANSI printf-style output from variadic stack arguments.
fn hle_wsprintf_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let fmt = arg(emu, 1);
    let text = format_c_output(emu, fmt, VaSource::Stack { next_word: 2 }).hle();
    if out != 0 {
        emu.memory.write_cstr(out, &text, text.len() + 1).hle();
    }
    ret(emu, text.len() as u32);
    HleResult::Retn(0)
}

// int wvsprintfA(LPSTR out, LPCSTR fmt, va_list args)
// Format simple ANSI %d/%u/%s sequences from a guest va_list.
fn hle_wvsprintf_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let fmt = emu.memory.cstr_lossy(arg(emu, 1), 512).unwrap_or_default();
    let mut va = arg(emu, 2);
    let mut text = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            text.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => text.push('%'),
            Some('d') | Some('u') => {
                let value = emu.memory.read_u32(va).hle();
                va = va.wrapping_add(4);
                text.push_str(&(value as i32).to_string());
            }
            Some('s') => {
                let ptr = emu.memory.read_u32(va).hle();
                va = va.wrapping_add(4);
                text.push_str(&emu.memory.cstr_lossy(ptr, 512).unwrap_or_default());
            }
            Some(other) => {
                text.push('%');
                text.push(other);
            }
            None => break,
        }
    }
    if out != 0 {
        emu.memory.write_cstr(out, &text, text.len() + 1).hle();
    }
    ret(emu, text.len() as u32);
    HleResult::Retn(12)
}

// BOOL TextOutW(HDC hdc, int x, int y, LPCWSTR text, int count)
// Draw lossy wide text through the existing GDI placeholder rasterizer.
fn hle_text_out_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let text = wide_text_bytes(emu, arg(emu, 3), arg(emu, 4));
    let dc = gdi_dc_or_default(emu, hdc);
    let layout = gdi_text_layout(emu, dc, &text, false);
    draw_gdi_text(emu, dc, &text, x, y, layout.metrics, None);
    update_text_current_pos(emu, hdc, dc, &layout);
    ret(emu, 1);
    HleResult::Retn(20)
}

// int GetDeviceCaps(HDC hdc, int index)
// Return basic display metrics for font and layout calculations.
fn hle_get_device_caps(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let index = arg(emu, 1);
    let value = match index {
        8 => emu.backend.width(),  // HORZRES
        10 => emu.backend.height(), // VERTRES
        12 => 16,                   // BITSPIXEL
        14 => 1,                    // PLANES
        24 => 256,                  // NUMCOLORS
        38 => 0x2b81,               // RASTERCAPS: BitBlt, DIB, palette, stretch DIB/Blt
        88 | 90 => 96,              // LOGPIXELSX/Y
        104 => 256,                 // SIZEPALETTE
        108 => 8,                   // COLORRES
        _ => 0,
    };
    ret(emu, value);
    HleResult::Retn(8)
}

// BOOL GetTextExtentPoint32W(HDC hdc, LPCWSTR text, int count, SIZE *size)
// Fill simple monospace text extents for wide GDI layout.
fn hle_get_text_extent_point32_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let text = wide_text_bytes(emu, arg(emu, 1), arg(emu, 2));
    let out = arg(emu, 3);
    let dc = gdi_dc_or_default(emu, hdc);
    let layout = gdi_text_layout(emu, dc, &text, false);
    if out != 0 {
        emu.memory.write_u32(out, layout.width as u32).hle();
        emu.memory.write_u32(out + 4, layout.height as u32).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL GetTextExtentPoint32A(HDC hdc, LPCSTR text, int count, SIZE *size)
// Fill simple monospace text extents for ANSI GDI layout.
fn hle_get_text_extent_point32_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let text = gdi_text_bytes(emu, arg(emu, 1), arg(emu, 2));
    let out = arg(emu, 3);
    let dc = gdi_dc_or_default(emu, hdc);
    let layout = gdi_text_layout(emu, dc, &text, false);
    if out != 0 {
        emu.memory.write_u32(out, layout.width as u32).hle();
        emu.memory.write_u32(out + 4, layout.height as u32).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL GetTextExtentPointA(HDC hdc, LPCSTR text, int count, SIZE *size)
// Alias the Win9x-era ANSI name to the 32-bit extent implementation.
fn hle_get_text_extent_point_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    hle_get_text_extent_point32_a(emu, entry)
}

// BOOL GetTextMetricsA(HDC hdc, TEXTMETRICA *tm)
// Fill minimal font metrics for legacy ANSI GDI layout.
fn hle_get_text_metrics_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let out = arg(emu, 1);
    let dc = gdi_dc_or_default(emu, hdc);
    let height = gdi_font_height(emu, dc).max(1) as u32;
    if out != 0 {
        emu.memory.memset(out, 0, 56).hle();
        emu.memory.write_u32(out, height).hle();
        emu.memory.write_u32(out + 20, (height / 2).max(1)).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

fn gdi_font_height(emu: &Emulator, dc: GdiDc) -> i32 {
    emu.hle
        .gdi_fonts
        .get(&dc.selected_font)
        .map(|font| font.height as i32)
        .unwrap_or(16)
        .clamp(1, 200)
}
