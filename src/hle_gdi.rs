// int SetBkMode(HDC hdc, int mode)
// Remember transparent/opaque mode for the fake DC; text rasterization is transparent.
fn hle_set_bk_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let mode = arg(emu, 1);
    let mut old = 1;
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        old = dc.bk_mode;
        dc.bk_mode = mode;
    }
    ret(emu, old);
    HleResult::Retn(8)
}

// COLORREF SetBkColor(HDC hdc, COLORREF color)
// Remember the current background COLORREF and return the previous value.
fn hle_set_bk_color(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let color = arg(emu, 1);
    let mut old = 0x00ff_ffff;
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        old = dc.bk_color;
        dc.bk_color = color;
    }
    ret(emu, old);
    HleResult::Retn(8)
}

// int SetTextCharacterExtra(HDC hdc, int extra)
// Store the extra glyph spacing used by Rich4's text layout code.
fn hle_set_text_character_extra(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let extra = arg(emu, 1) as i32;
    let mut old = 0;
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        old = dc.text_extra;
        dc.text_extra = extra;
    }
    ret(emu, old as u32);
    HleResult::Retn(8)
}

// COLORREF SetTextColor(HDC hdc, COLORREF color)
// Remember the current COLORREF and return the previous text color.
fn hle_set_text_color(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let color = arg(emu, 1);
    let mut old = 0x00ff_ffff;
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        old = dc.text_color;
        dc.text_color = color;
    }
    ret(emu, old);
    HleResult::Retn(8)
}

// int GetBkMode(HDC hdc)
// Return the tracked DC background mode, or zero for an invalid HDC as Wine does.
fn hle_get_bk_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = emu
        .hle
        .gdi_dcs
        .get(&arg(emu, 0))
        .map(|dc| dc.bk_mode)
        .unwrap_or(0);
    ret(emu, value);
    HleResult::Retn(4)
}

// COLORREF GetTextColor(HDC hdc)
// Return the tracked text COLORREF, or zero for an invalid HDC as Wine does.
fn hle_get_text_color(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = emu
        .hle
        .gdi_dcs
        .get(&arg(emu, 0))
        .map(|dc| dc.text_color)
        .unwrap_or(0);
    ret(emu, value);
    HleResult::Retn(4)
}

// BOOL DeleteObject(HGDIOBJ obj)
// Drop fake GDI objects and release HBITMAP backing storage when deselected.
fn hle_delete_object(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let object = arg(emu, 0);
    if let Some(bitmap) = emu.hle.gdi_bitmaps.get(&object).copied() {
        if emu
            .hle
            .gdi_dcs
            .values()
            .any(|dc| dc.selected_bitmap == object)
        {
            trace_gdi!("gdi DeleteObject bitmap obj={object:08x} selected -> 0");
            ret(emu, 0);
            return HleResult::Retn(4);
        }
        emu.hle.gdi_bitmaps.remove(&object);
        free_surface_allocations(emu, bitmap.surface).hle();
        trace_gdi!(
            "gdi DeleteObject bitmap obj={object:08x} surf={:08x}",
            bitmap.surface
        );
    } else {
        let deleted = emu.hle.gdi_fonts.remove(&object).is_some()
            || emu.hle.gdi_brushes.remove(&object).is_some()
            || emu.hle.gdi_pens.remove(&object).is_some()
            || emu.hle.gdi_palettes.remove(&object).is_some()
            || emu.hle.gdi_regions.remove(&object).is_some();
        trace_gdi!("gdi DeleteObject obj={object:08x} deleted={deleted}");
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// DWORD GetRegionData(HRGN rgn, DWORD count, RGNDATA *out)
// Return a single-rectangle RGNDATA description for tracked rectangle regions.
fn hle_get_region_data(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let rgn = arg(emu, 0);
    let count = arg(emu, 1);
    let out = arg(emu, 2);
    let rect = emu.hle.gdi_regions.get(&rgn).copied().unwrap_or(WindowRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    });
    let non_empty = !rect.is_empty();
    let needed = 32 + if non_empty { 16 } else { 0 };
    if out != 0 && count >= needed {
        emu.memory.write_u32(out, 32).hle();
        emu.memory.write_u32(out + 4, 1).hle(); // RDH_RECTANGLES
        emu.memory.write_u32(out + 8, non_empty as u32).hle();
        emu.memory
            .write_u32(out + 12, if non_empty { 16 } else { 0 })
            .hle();
        write_gdi_rect(emu, out + 16, (rect.left, rect.top, rect.right, rect.bottom));
        if non_empty {
            write_gdi_rect(emu, out + 32, (rect.left, rect.top, rect.right, rect.bottom));
        }
    }
    ret(emu, needed);
    HleResult::Retn(12)
}

// int GetObjectA(HGDIOBJ obj, int cb, LPVOID out)
// Return BITMAP metadata for tracked bitmap handles.
fn hle_get_object_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = get_object_common(emu);
    ret(emu, value);
    HleResult::Retn(12)
}

// int GetObjectW(HGDIOBJ obj, int cb, LPVOID out)
// Return BITMAP metadata for tracked bitmap handles.
fn hle_get_object_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = get_object_common(emu);
    ret(emu, value);
    HleResult::Retn(12)
}

// HDC CreateCompatibleDC(HDC hdc)
// Allocate a memory DC that can draw from a selected bitmap.
fn hle_create_compatible_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = emu.hle.alloc_gdi_handle();
    emu.hle.gdi_dcs.insert(
        handle,
        GdiDc {
            surface: 0,
            hwnd: 0,
            selected_font: 0,
            selected_bitmap: GDI_STOCK_BITMAP,
            selected_brush: stock_object_handle(STOCK_WHITE_BRUSH),
            selected_pen: stock_object_handle(STOCK_BLACK_PEN),
            selected_palette: 0,
            rop2: R2_COPYPEN,
            layout: 0,
            map_mode: MM_TEXT,
            text_align: TA_LEFT | TA_TOP,
            text_extra: 0,
            text_color: 0x00ff_ffff,
            bk_color: 0x00ff_ffff,
            bk_mode: 1,
            origin_x: 0,
            origin_y: 0,
            brush_origin_x: 0,
            brush_origin_y: 0,
            current_x: 0,
            current_y: 0,
        },
    );
    trace_gdi!("gdi CreateCompatibleDC -> hdc={handle:08x}");
    ret(emu, handle);
    HleResult::Retn(4)
}

fn create_ic_impl(emu: &mut Emulator) -> u32 {
    create_gdi_screen_dc(emu, 0)
}

// HDC CreateICA(LPCSTR driver, LPCSTR device, LPCSTR output, const DEVMODEA *init)
// Return a screen-compatible information DC for display capability probes.
fn hle_create_ic_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = create_ic_impl(emu);
    ret(emu, hdc);
    HleResult::Retn(16)
}

// HDC CreateICW(LPCWSTR driver, LPCWSTR device, LPCWSTR output, const DEVMODEW *init)
// Return a screen-compatible information DC for display capability probes.
fn hle_create_ic_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = create_ic_impl(emu);
    ret(emu, hdc);
    HleResult::Retn(16)
}

// BOOL PtVisible(HDC hdc, int x, int y)
// Test whether a logical point lands inside the drawable surface bounds.
fn hle_pt_visible(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let x = (arg(emu, 1) as i32).saturating_add(dc.origin_x);
    let y = (arg(emu, 2) as i32).saturating_add(dc.origin_y);
    ret(emu, gdi_device_point_visible(emu, dc, x, y) as u32);
    HleResult::Retn(12)
}

// BOOL RectVisible(HDC hdc, const RECT *rect)
// Test whether any part of a logical RECT intersects the drawable surface.
fn hle_rect_visible(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let rect = arg(emu, 1);
    let visible = if rect != 0 {
        let (left, top, right, bottom) = read_gdi_rect(emu, rect);
        gdi_device_rect_visible(
            emu,
            dc,
            RectI {
                left: left.saturating_add(dc.origin_x),
                top: top.saturating_add(dc.origin_y),
                right: right.saturating_add(dc.origin_x),
                bottom: bottom.saturating_add(dc.origin_y),
            },
        )
    } else {
        false
    };
    ret(emu, visible as u32);
    HleResult::Retn(8)
}

// HBITMAP CreateCompatibleBitmap(HDC hdc, int width, int height)
// Allocate a tracked bitmap surface using the source DC pixel format.
fn hle_create_compatible_bitmap(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let width = (arg(emu, 1) as i32).max(1) as u32;
    let height = (arg(emu, 2) as i32).max(1) as u32;
    let dc = gdi_dc_or_default(emu, hdc);
    let bpp = if dc.surface != 0 {
        read_surface_info(emu, dc.surface)
            .map(|surface| surface.bpp)
            .unwrap_or(emu.hle.ddraw_bpp)
    } else {
        emu.hle.ddraw_bpp
    }
    .max(8);
    let surface = create_gdi_bitmap_surface_with_format(emu, width, height, bpp).hle();
    let handle = emu.hle.create_gdi_bitmap(surface);
    trace_gdi!(
        "gdi CreateCompatibleBitmap hdc={hdc:08x} {width}x{height}x{bpp} -> {handle:08x}"
    );
    ret(emu, handle);
    HleResult::Retn(12)
}

// HPALETTE CreatePalette(const LOGPALETTE *logpal)
// Copy LOGPALETTE entries into a tracked GDI palette handle.
fn hle_create_palette(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let logpal = arg(emu, 0);
    if logpal == 0 {
        ret(emu, 0);
        return HleResult::Retn(4);
    }
    let count = emu.memory.read_u16(logpal + 2).hle().min(1024) as u32;
    let mut entries = Vec::with_capacity(count as usize);
    for index in 0..count {
        let addr = logpal + 4 + index * 4;
        entries.push([
            emu.memory.read_u8(addr).hle(),
            emu.memory.read_u8(addr + 1).hle(),
            emu.memory.read_u8(addr + 2).hle(),
            emu.memory.read_u8(addr + 3).hle(),
        ]);
    }
    let handle = emu.hle.create_gdi_palette(entries);
    ret(emu, handle);
    HleResult::Retn(4)
}

// BOOL ResizePalette(HPALETTE palette, UINT count)
// Resize tracked palette storage and keep existing entries.
fn hle_resize_palette(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let palette = arg(emu, 0);
    let count = arg(emu, 1).min(1024) as usize;
    if let Some(palette) = emu.hle.gdi_palettes.get_mut(&palette) {
        palette.entries.resize(count, [0, 0, 0, 0]);
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// HPALETTE SelectPalette(HDC hdc, HPALETTE palette, BOOL force_background)
// Select a tracked palette into a DC and return the previous palette.
fn hle_select_palette(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let palette = arg(emu, 1);
    let mut old = 0;
    if palette == 0 || emu.hle.gdi_palettes.contains_key(&palette) {
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            old = dc.selected_palette;
            dc.selected_palette = palette;
        }
    }
    ret(emu, old);
    HleResult::Retn(12)
}

// UINT RealizePalette(HDC hdc)
// Report the selected palette entry count; surfaces are already true-color.
fn hle_realize_palette(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let count = emu
        .hle
        .gdi_dcs
        .get(&hdc)
        .and_then(|dc| emu.hle.gdi_palettes.get(&dc.selected_palette))
        .map(|palette| palette.entries.len() as u32)
        .unwrap_or(0);
    ret(emu, count);
    HleResult::Retn(4)
}

// UINT GetSystemPaletteUse(HDC hdc)
// Report the classic static system-palette policy.
fn hle_get_system_palette_use(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// UINT SetSystemPaletteUse(HDC hdc, UINT use)
// Accept palette policy changes while preserving the classic static policy.
fn hle_set_system_palette_use(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL DeleteDC(HDC hdc)
// Release a tracked DC handle; selected GDI objects keep their own lifetime.
fn hle_delete_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    emu.hle.gdi_dcs.remove(&hdc);
    emu.hle.gdi_dc_saves.remove(&hdc);
    ret(emu, 1);
    HleResult::Retn(4)
}

// int SaveDC(HDC hdc)
// Push a copy of the tracked DC attributes and return the one-based save level.
fn hle_save_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let Some(dc) = emu.hle.gdi_dcs.get(&hdc).copied() else {
        ret(emu, 0);
        return HleResult::Retn(4);
    };
    let stack = emu.hle.gdi_dc_saves.entry(hdc).or_default();
    stack.push(dc);
    let level = stack.len() as u32;
    ret(emu, level);
    HleResult::Retn(4)
}

// BOOL RestoreDC(HDC hdc, int level)
// Restore and pop the requested saved DC level using Win32 positive/negative indexing.
fn hle_restore_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let level = arg(emu, 1) as i32;
    if level == 0 || !emu.hle.gdi_dcs.contains_key(&hdc) {
        ret(emu, 0);
        return HleResult::Retn(8);
    }
    let saved = {
        let Some(stack) = emu.hle.gdi_dc_saves.get_mut(&hdc) else {
            ret(emu, 0);
            return HleResult::Retn(8);
        };
        let len = stack.len();
        let index = if level > 0 {
            let level = level as usize;
            if level == 0 || level > len {
                ret(emu, 0);
                return HleResult::Retn(8);
            }
            level - 1
        } else {
            let back = level.unsigned_abs() as usize;
            if back == 0 || back > len {
                ret(emu, 0);
                return HleResult::Retn(8);
            }
            len - back
        };
        let saved = stack[index];
        stack.truncate(index);
        saved
    };
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        *dc = saved;
        ret(emu, 1);
    } else {
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// BOOL GetDCOrgEx(HDC hdc, LPPOINT point)
// Return the tracked device origin for a fake surface-backed DC.
fn hle_get_dc_org_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let point = arg(emu, 1);
    if point != 0 {
        emu.memory.write_u32(point, dc.origin_x as u32).hle();
        emu.memory.write_u32(point + 4, dc.origin_y as u32).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL SetBrushOrgEx(HDC hdc, int x, int y, LPPOINT old)
// Store the pattern-brush origin and optionally return the previous value.
fn hle_set_brush_org_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let old = arg(emu, 3);
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        if old != 0 {
            emu.memory.write_u32(old, dc.brush_origin_x as u32).hle();
            emu.memory.write_u32(old + 4, dc.brush_origin_y as u32).hle();
        }
        dc.brush_origin_x = x;
        dc.brush_origin_y = y;
    } else if old != 0 {
        emu.memory.write_u32(old, 0).hle();
        emu.memory.write_u32(old + 4, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL MoveToEx(HDC hdc, int x, int y, LPPOINT old)
// Update the current drawing position used by GDI line primitives.
fn hle_move_to_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let old = arg(emu, 3);
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        if old != 0 {
            emu.memory.write_u32(old, dc.current_x as u32).hle();
            emu.memory.write_u32(old + 4, dc.current_y as u32).hle();
        }
        dc.current_x = x;
        dc.current_y = y;
    } else if old != 0 {
        emu.memory.write_u32(old, 0).hle();
        emu.memory.write_u32(old + 4, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// BOOL LineTo(HDC hdc, int x, int y)
// Rasterize a solid pen line from the current position and advance it.
fn hle_line_to(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    if let Some(dc) = emu.hle.gdi_dcs.get(&hdc).copied() {
        if dc.surface != 0 {
            let surface = read_surface_info(emu, dc.surface).hle();
            if let Some(color) = selected_pen_colorref(emu, dc) {
                draw_gdi_line(
                    emu,
                    surface,
                    dc.current_x.saturating_add(dc.origin_x),
                    dc.current_y.saturating_add(dc.origin_y),
                    x.saturating_add(dc.origin_x),
                    y.saturating_add(dc.origin_y),
                    color,
                    dc.rop2,
                );
            }
        }
        if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
            dc.current_x = x;
            dc.current_y = y;
        }
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL BitBlt(HDC dst, int x, int y, int cx, int cy, HDC src, int sx, int sy, DWORD rop)
// Copy pixels between surface-backed DCs, honoring each DC's client origin.
fn hle_bit_blt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst_hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let cx = arg(emu, 3) as i32;
    let cy = arg(emu, 4) as i32;
    let src_hdc = arg(emu, 5);
    let sx = arg(emu, 6) as i32;
    let sy = arg(emu, 7) as i32;
    let rop = arg(emu, 8);
    let dst_dc = gdi_dc_or_default(emu, dst_hdc);
    let src_dc = gdi_dc_or_default(emu, src_hdc);
    trace_gdi!(
        "gdi BitBlt dst_hdc={dst_hdc:08x} dst_surf={:08x} x={x} y={y} cx={cx} cy={cy} src_hdc={src_hdc:08x} src_surf={:08x} sx={sx} sy={sy} rop={rop:08x}",
        dst_dc.surface,
        src_dc.surface,
    );
    if dst_dc.surface != 0 && src_dc.surface != 0 && cx > 0 && cy > 0 {
        let dst = read_surface_info(emu, dst_dc.surface).hle();
        let src = read_surface_info(emu, src_dc.surface).hle();
        let src_rect = RectI {
            left: sx.saturating_add(src_dc.origin_x),
            top: sy.saturating_add(src_dc.origin_y),
            right: sx.saturating_add(src_dc.origin_x).saturating_add(cx),
            bottom: sy.saturating_add(src_dc.origin_y).saturating_add(cy),
        };
        blit_surface_rect_rop(
            emu,
            dst,
            src,
            x.saturating_add(dst_dc.origin_x),
            y.saturating_add(dst_dc.origin_y),
            src_rect,
            rop,
            selected_brush_colorref(emu, dst_dc),
            dst_dc,
        );
    }
    ret(emu, 1);
    HleResult::Retn(36)
}

// BOOL StretchBlt(HDC dst, int x, int y, int cx, int cy, HDC src, int sx, int sy, int sw, int sh, DWORD rop)
// Copy and nearest-neighbor scale pixels between surface-backed DCs.
fn hle_stretch_blt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst_hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let cx = arg(emu, 3) as i32;
    let cy = arg(emu, 4) as i32;
    let src_hdc = arg(emu, 5);
    let sx = arg(emu, 6) as i32;
    let sy = arg(emu, 7) as i32;
    let sw = arg(emu, 8) as i32;
    let sh = arg(emu, 9) as i32;
    let rop = arg(emu, 10);
    let dst_dc = gdi_dc_or_default(emu, dst_hdc);
    let src_dc = gdi_dc_or_default(emu, src_hdc);
    trace_gdi!(
        "gdi StretchBlt dst_hdc={dst_hdc:08x} dst_surf={:08x} x={x} y={y} cx={cx} cy={cy} src_hdc={src_hdc:08x} src_surf={:08x} sx={sx} sy={sy} sw={sw} sh={sh} rop={rop:08x}",
        dst_dc.surface,
        src_dc.surface,
    );
    if dst_dc.surface != 0 && src_dc.surface != 0 && cx > 0 && cy > 0 && sw > 0 && sh > 0 {
        let dst = read_surface_info(emu, dst_dc.surface).hle();
        let src = read_surface_info(emu, src_dc.surface).hle();
        stretch_surface_rect_rop(
            emu,
            dst,
            src,
            RectI {
                left: x.saturating_add(dst_dc.origin_x),
                top: y.saturating_add(dst_dc.origin_y),
                right: x.saturating_add(dst_dc.origin_x).saturating_add(cx),
                bottom: y.saturating_add(dst_dc.origin_y).saturating_add(cy),
            },
            RectI {
                left: sx.saturating_add(src_dc.origin_x),
                top: sy.saturating_add(src_dc.origin_y),
                right: sx.saturating_add(src_dc.origin_x).saturating_add(sw),
                bottom: sy.saturating_add(src_dc.origin_y).saturating_add(sh),
            },
            rop,
            selected_brush_colorref(emu, dst_dc),
            dst_dc,
        );
        mark_gdi_screen_surface_dirty(emu, dst);
    }
    ret(emu, 1);
    HleResult::Retn(44)
}

fn blit_surface_rect_rop(
    emu: &mut Emulator,
    dst: SurfaceInfo,
    src: SurfaceInfo,
    dst_x: i32,
    dst_y: i32,
    src_rect: RectI,
    rop: u32,
    pattern: u32,
    dst_dc: GdiDc,
) {
    let width = src_rect.width();
    let height = src_rect.height();
    if width <= 0 || height <= 0 {
        return;
    }
    if let Some(fill) = gdi_rop_solid_fill_colorref(rop, pattern) {
        draw_gdi_rect_colorref_for_dc(
            emu,
            dst,
            RectI {
                left: dst_x,
                top: dst_y,
                right: dst_x.saturating_add(width),
                bottom: dst_y.saturating_add(height),
            },
            fill,
            dst_dc,
        );
        finish_primary_gdi_update(emu, dst).hle();
        return;
    }
    let mut left = 0;
    let mut top = 0;
    let mut right = width;
    let mut bottom = height;

    left = left.max(0i32.saturating_sub(dst_x));
    top = top.max(0i32.saturating_sub(dst_y));
    right = right.min((dst.width as i32).saturating_sub(dst_x));
    bottom = bottom.min((dst.height as i32).saturating_sub(dst_y));

    left = left.max(0i32.saturating_sub(src_rect.left));
    top = top.max(0i32.saturating_sub(src_rect.top));
    right = right.min((src.width as i32).saturating_sub(src_rect.left));
    bottom = bottom.min((src.height as i32).saturating_sub(src_rect.top));

    if left >= right || top >= bottom {
        return;
    }
    for row in top..bottom {
        let y = dst_y.saturating_add(row);
        let sy = src_rect.top.saturating_add(row);
        for col in left..right {
            let x = dst_x.saturating_add(col);
            let sx = src_rect.left.saturating_add(col);
            if !gdi_dc_draw_point_visible(emu, dst_dc, x, y) {
                continue;
            }
            let src_color = read_surface_pixel_colorref(emu, src, sx, sy).unwrap_or(0);
            let dst_color = read_surface_pixel_colorref(emu, dst, x, y).unwrap_or(0);
            let color = apply_gdi_rop(rop, src_color, dst_color, pattern);
            write_surface_pixel_colorref(emu, dst, x, y, color);
        }
    }
    finish_primary_gdi_update(emu, dst).hle();
}

fn gdi_rop_solid_fill_colorref(rop: u32, pattern: u32) -> Option<u32> {
    const BLACKNESS: u32 = 0x0000_0042;
    const PATCOPY: u32 = 0x00f0_0021;
    const WHITENESS: u32 = 0x00ff_0062;

    let mask = 0x00ff_ffff;
    match rop & mask {
        BLACKNESS => Some(0),
        PATCOPY => Some(pattern),
        WHITENESS => Some(mask),
        _ => None,
    }
}

fn draw_gdi_rect_colorref_for_dc(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    rect: RectI,
    colorref: u32,
    dc: GdiDc,
) {
    if dc.hwnd == 0 {
        draw_gdi_rect_colorref(emu, surface, rect, colorref);
        return;
    }
    let left = rect.left.max(0).min(surface.width as i32);
    let top = rect.top.max(0).min(surface.height as i32);
    let right = rect.right.max(left).min(surface.width as i32);
    let bottom = rect.bottom.max(top).min(surface.height as i32);
    for y in top..bottom {
        for x in left..right {
            if gdi_dc_draw_point_visible(emu, dc, x, y) {
                write_surface_pixel_colorref(emu, surface, x, y, colorref);
            }
        }
    }
}

fn stretch_surface_rect_rop(
    emu: &mut Emulator,
    dst: SurfaceInfo,
    src: SurfaceInfo,
    mut dst_rect: RectI,
    src_rect: RectI,
    rop: u32,
    pattern: u32,
    dst_dc: GdiDc,
) {
    let dst_w = dst_rect.width();
    let dst_h = dst_rect.height();
    let src_w = src_rect.width();
    let src_h = src_rect.height();
    if dst_w <= 0 || dst_h <= 0 || src_w <= 0 || src_h <= 0 {
        return;
    }
    dst_rect.left = dst_rect.left.max(0).min(dst.width as i32);
    dst_rect.top = dst_rect.top.max(0).min(dst.height as i32);
    dst_rect.right = dst_rect.right.max(0).min(dst.width as i32);
    dst_rect.bottom = dst_rect.bottom.max(0).min(dst.height as i32);
    for y in dst_rect.top..dst_rect.bottom {
        let dy = y - dst_rect.top;
        let sy = src_rect.top + dy * src_h / dst_h;
        for x in dst_rect.left..dst_rect.right {
            let dx = x - dst_rect.left;
            let sx = src_rect.left + dx * src_w / dst_w;
            if !gdi_dc_draw_point_visible(emu, dst_dc, x, y) {
                continue;
            }
            let src_color = read_surface_pixel_colorref(emu, src, sx, sy).unwrap_or(0);
            let dst_color = read_surface_pixel_colorref(emu, dst, x, y).unwrap_or(0);
            let color = apply_gdi_rop(rop, src_color, dst_color, pattern);
            write_surface_pixel_colorref(emu, dst, x, y, color);
        }
    }
}

fn selected_brush_colorref(emu: &Emulator, dc: GdiDc) -> u32 {
    if let Some(color) = stock_brush_colorref(dc.selected_brush) {
        return color.unwrap_or(0x00ff_ffff);
    }
    if is_sys_color_brush(dc.selected_brush) {
        return sys_colorref(dc.selected_brush & 0xff);
    }
    emu.hle
        .gdi_brushes
        .get(&dc.selected_brush)
        .map(|brush| brush.color)
        .unwrap_or(0x00ff_ffff)
}

fn apply_gdi_rop(rop: u32, src: u32, dst: u32, pattern: u32) -> u32 {
    const BLACKNESS: u32 = 0x0000_0042;
    const DSTINVERT: u32 = 0x0055_0009;
    const MERGECOPY: u32 = 0x00c0_00ca;
    const MERGEPAINT: u32 = 0x00bb_0226;
    const NOTSRCCOPY: u32 = 0x0033_0008;
    const NOTSRCERASE: u32 = 0x0011_00a6;
    const PATCOPY: u32 = 0x00f0_0021;
    const PATINVERT: u32 = 0x005a_0049;
    const PATPAINT: u32 = 0x00fb_0a09;
    const SRCAND: u32 = 0x0088_00c6;
    const SRCCOPY: u32 = 0x00cc_0020;
    const SRCERASE: u32 = 0x0044_0328;
    const SRCINVERT: u32 = 0x0066_0046;
    const SRCPAINT: u32 = 0x00ee_0086;
    const WHITENESS: u32 = 0x00ff_0062;

    let mask = 0x00ff_ffff;
    match rop & mask {
        BLACKNESS => 0,
        DSTINVERT => !dst & mask,
        MERGECOPY => src & pattern,
        MERGEPAINT => (!src | dst) & mask,
        NOTSRCCOPY => !src & mask,
        NOTSRCERASE => !(src | dst) & mask,
        PATCOPY => pattern,
        PATINVERT => (pattern ^ dst) & mask,
        PATPAINT => (pattern | dst | !src) & mask,
        SRCAND => src & dst,
        SRCCOPY => src,
        SRCERASE => src & !dst & mask,
        SRCINVERT => (src ^ dst) & mask,
        SRCPAINT => src | dst,
        WHITENESS => mask,
        _ => src,
    }
}

fn apply_gdi_rop2(rop2: u32, pen: u32, dst: u32) -> u32 {
    let mask = 0x00ff_ffff;
    match rop2 {
        R2_BLACK => 0,
        R2_NOTMERGEPEN => !(dst | pen) & mask,
        R2_MASKNOTPEN => dst & !pen & mask,
        R2_NOTCOPYPEN => !pen & mask,
        R2_MASKPENNOT => pen & !dst & mask,
        R2_NOT => !dst & mask,
        R2_XORPEN => (dst ^ pen) & mask,
        R2_NOTMASKPEN => !(dst & pen) & mask,
        R2_MASKPEN => dst & pen,
        R2_NOTXORPEN => !(dst ^ pen) & mask,
        R2_NOP => dst,
        R2_MERGENOTPEN => dst | !pen & mask,
        R2_COPYPEN => pen,
        R2_MERGEPENNOT => pen | !dst & mask,
        R2_MERGEPEN => dst | pen,
        R2_WHITE => mask,
        _ => pen,
    }
}

fn selected_pen_colorref(emu: &Emulator, dc: GdiDc) -> Option<u32> {
    if let Some(color) = stock_pen_colorref(dc.selected_pen) {
        return color;
    }
    emu.hle
        .gdi_pens
        .get(&dc.selected_pen)
        .map(|pen| pen.color)
}

fn draw_gdi_line(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    colorref: u32,
    rop2: u32,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        let dst = read_surface_pixel_colorref(emu, surface, x0, y0).unwrap_or(0);
        let color = apply_gdi_rop2(rop2, colorref, dst);
        write_surface_pixel_colorref(emu, surface, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = err.saturating_mul(2);
        if e2 >= dy {
            err = err.saturating_add(dy);
            x0 = x0.saturating_add(sx);
        }
        if e2 <= dx {
            err = err.saturating_add(dx);
            y0 = y0.saturating_add(sy);
        }
    }
    mark_gdi_screen_surface_dirty(emu, surface);
}

// HBRUSH GetSysColorBrush(int index)
// Return a stable stock system-color brush handle.
fn hle_get_sys_color_brush(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let index = arg(emu, 0) & 0xff;
    ret(emu, 0x5000_0000 | index);
    HleResult::Retn(4)
}

// HGDIOBJ GetStockObject(int object)
// Return stable handles for common stock pens, brushes, fonts, and palette objects.
fn hle_get_stock_object(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let object = arg(emu, 0);
    let handle = if is_known_stock_object(object) {
        stock_object_handle(object)
    } else {
        0
    };
    ret(emu, handle);
    HleResult::Retn(4)
}

// HBRUSH CreateSolidBrush(COLORREF color)
// Allocate a tracked solid brush.
fn hle_create_solid_brush(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = emu.hle.create_gdi_brush(arg(emu, 0), 0);
    trace_gdi!("gdi CreateSolidBrush color={:08x} -> {handle:08x}", arg(emu, 0));
    ret(emu, handle);
    HleResult::Retn(4)
}

// HPEN CreatePen(int style, int width, COLORREF color)
// Allocate a tracked pen handle for line and outline drawing.
fn hle_create_pen(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = emu.hle.create_gdi_pen(arg(emu, 0), arg(emu, 1), arg(emu, 2));
    trace_gdi!(
        "gdi CreatePen style={} width={} color={:08x} -> {handle:08x}",
        arg(emu, 0),
        arg(emu, 1),
        arg(emu, 2),
    );
    ret(emu, handle);
    HleResult::Retn(12)
}

// HBRUSH CreatePatternBrush(HBITMAP bitmap)
// Allocate a tracked brush that tiles a bitmap.
fn hle_create_pattern_brush(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let bitmap = arg(emu, 0);
    let handle = if emu.hle.gdi_bitmaps.contains_key(&bitmap) {
        emu.hle.create_gdi_brush(0, bitmap)
    } else {
        0
    };
    ret(emu, handle);
    HleResult::Retn(4)
}

// HBITMAP CreateBitmap(int width, int height, UINT planes, UINT bpp, const void *bits)
// Allocate a tracked bitmap surface and copy simple packed pixel data when present.
fn hle_create_bitmap(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let width = (arg(emu, 0) as i32).max(1) as u32;
    let height = (arg(emu, 1) as i32).max(1) as u32;
    let bits_per_pixel = arg(emu, 2).saturating_mul(arg(emu, 3)).max(1);
    let bits = arg(emu, 4);
    let surface_bpp = if bits_per_pixel <= 8 { 16 } else { bits_per_pixel };
    let surface = create_gdi_bitmap_surface_with_format(emu, width, height, surface_bpp).hle();
    if bits != 0 {
        copy_create_bitmap_bits(emu, surface, bits, width, height, bits_per_pixel);
    }
    let handle = emu.hle.create_gdi_bitmap(surface);
    trace_gdi!(
        "gdi CreateBitmap {width}x{height}x{bits_per_pixel} bits={bits:08x} -> {handle:08x}"
    );
    ret(emu, handle);
    HleResult::Retn(20)
}

// HBITMAP CreateDIBitmap(HDC hdc, const BITMAPINFOHEADER *header, DWORD init, const void *bits, const BITMAPINFO *info, UINT usage)
// Create a compatible bitmap and optionally initialize it from DIB pixel data.
fn hle_create_dibitmap(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const CBM_INIT: u32 = 0x04;

    let hdc = arg(emu, 0);
    let header = arg(emu, 1);
    let init = arg(emu, 2);
    let bits = arg(emu, 3);
    let info = arg(emu, 4);
    let usage = arg(emu, 5);
    let handle = if let Some((width, height)) = dib_header_dimensions(emu, header) {
        let dc = gdi_dc_or_default(emu, hdc);
        let target_bpp = if dc.surface != 0 {
            read_surface_info(emu, dc.surface)
                .map(|surface| surface.bpp)
                .unwrap_or(emu.hle.ddraw_bpp)
        } else {
            emu.hle.ddraw_bpp
        }
        .max(16);
        let surface = create_gdi_bitmap_surface_with_format(emu, width, height, target_bpp).hle();
        if (init & CBM_INIT) != 0 && bits != 0 && info != 0 {
            let dst = read_surface_info(emu, surface).hle();
            if let Some(dib) = read_dib_info(emu, info) {
                let width = width.min(dib.width);
                let height = height.min(dib.height);
                let _ = blit_dib_to_surface(
                    emu,
                    dst,
                    RectI {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    },
                    RectI {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    },
                    bits,
                    info,
                    usage,
                    dc.selected_palette,
                    0,
                    dib.height,
                    None,
                );
            }
        }
        emu.hle.create_gdi_bitmap(surface)
    } else {
        0
    };
    trace_gdi!(
        "gdi CreateDIBitmap hdc={hdc:08x} header={header:08x} init={init:08x} bits={bits:08x} info={info:08x} -> {handle:08x}"
    );
    ret(emu, handle);
    HleResult::Retn(24)
}

// int GetDIBits(HDC hdc, HBITMAP bitmap, UINT start, UINT lines, void *bits, BITMAPINFO *info, UINT usage)
// Return tracked bitmap metadata and copy packed scanlines when requested.
fn hle_get_dibits(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let bitmap = arg(emu, 1);
    let start = arg(emu, 2);
    let lines = arg(emu, 3);
    let bits = arg(emu, 4);
    let info = arg(emu, 5);
    let Some(bitmap) = emu.hle.gdi_bitmaps.get(&bitmap).copied() else {
        ret(emu, 0);
        return HleResult::Retn(28);
    };
    let surface = read_surface_info(emu, bitmap.surface).hle();
    let bpp = surface.bpp.max(8);
    let copied = lines.min(surface.height.saturating_sub(start));
    if info != 0 {
        emu.memory.write_u32(info, 40).hle();
        emu.memory.write_u32(info + 4, surface.width).hle();
        emu.memory.write_u32(info + 8, surface.height).hle();
        emu.memory.write_u16(info + 12, 1).hle();
        emu.memory.write_u16(info + 14, bpp as u16).hle();
        emu.memory.write_u32(info + 16, 0).hle();
        emu.memory
            .write_u32(info + 20, dib_row_stride(surface.width, bpp).unwrap_or(surface.pitch) * surface.height)
            .hle();
        emu.memory.write_u32(info + 24, 0).hle();
        emu.memory.write_u32(info + 28, 0).hle();
        emu.memory.write_u32(info + 32, 0).hle();
        emu.memory.write_u32(info + 36, 0).hle();
    }
    if bits != 0 && copied != 0 {
        if let Some(dst_stride) = dib_row_stride(surface.width, bpp) {
            let src_bpp = surface.bytes_per_pixel();
            let row_bytes = surface.width.saturating_mul(src_bpp).min(dst_stride);
            for line in 0..copied {
                let src_y = surface.height - 1 - start - line;
                let src = surface.buffer + src_y * surface.pitch;
                let dst = bits + line * dst_stride;
                let row = emu.memory.read_bytes(src, row_bytes as usize).hle();
                emu.memory.write_bytes(dst, &row).hle();
                if dst_stride > row_bytes {
                    emu.memory
                        .memset(dst + row_bytes, 0, dst_stride - row_bytes)
                        .hle();
                }
            }
        }
    }
    ret(emu, copied);
    HleResult::Retn(28)
}

// int SetDIBitsToDevice(HDC hdc, int x, int y, DWORD dx, DWORD dy, int sx, int sy, UINT start, UINT lines, const void *bits, const BITMAPINFO *info, UINT color_use)
// Copy packed DIB scanlines into the destination DC without stretching.
fn hle_set_dibits_to_device(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let width = arg(emu, 3) as i32;
    let height = arg(emu, 4) as i32;
    let src_x = arg(emu, 5) as i32;
    let src_y = arg(emu, 6) as i32;
    let start_scan = arg(emu, 7);
    let scan_lines = arg(emu, 8);
    let bits = arg(emu, 9);
    let info = arg(emu, 10);
    let color_use = arg(emu, 11);
    let dc = gdi_dc_or_default(emu, hdc);
    trace_gdi!(
        "gdi SetDIBitsToDevice hdc={hdc:08x} surf={:08x} dst=({x},{y} {width}x{height}) src=({src_x},{src_y}) start={start_scan} lines={scan_lines} bits={bits:08x} info={info:08x} use={color_use}",
        dc.surface,
    );
    let (copied, surface) = if dc.surface != 0 && width > 0 && height > 0 && bits != 0 && info != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        let copied = read_dib_info(emu, info)
            .and_then(|dib| {
                let src_rect = dib_lower_left_src_rect(dib, src_x, src_y, width, height);
                trace_gdi!(
                    "gdi SetDIBitsToDevice mapped_src=({},{}..{},{})",
                    src_rect.left,
                    src_rect.top,
                    src_rect.right,
                    src_rect.bottom,
                );
                blit_dib_to_surface(
                    emu,
                    surface,
                    RectI {
                        left: x.saturating_add(dc.origin_x),
                        top: y.saturating_add(dc.origin_y),
                        right: x.saturating_add(dc.origin_x).saturating_add(width),
                        bottom: y.saturating_add(dc.origin_y).saturating_add(height),
                    },
                    src_rect,
                    bits,
                    info,
                    color_use,
                    dc.selected_palette,
                    start_scan,
                    scan_lines,
                    Some(dc),
                )
            })
            .unwrap_or(0);
        (copied, Some(surface))
    } else {
        (0, None)
    };
    if copied != 0 {
        if let Some(surface) = surface {
            mark_gdi_screen_surface_dirty(emu, surface);
        }
    }
    ret(emu, copied);
    HleResult::Retn(48)
}

// int StretchDIBits(HDC hdc, int x, int y, int dx, int dy, int sx, int sy, int sw, int sh, const void *bits, const BITMAPINFO *info, UINT color_use, DWORD rop)
// Copy packed DIB pixels into the destination DC, nearest-neighbor scaling for SRCCOPY-style use.
fn hle_stretch_dibits(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let dst_w = arg(emu, 3) as i32;
    let dst_h = arg(emu, 4) as i32;
    let src_x = arg(emu, 5) as i32;
    let src_y = arg(emu, 6) as i32;
    let src_w = arg(emu, 7) as i32;
    let src_h = arg(emu, 8) as i32;
    let bits = arg(emu, 9);
    let info = arg(emu, 10);
    let color_use = arg(emu, 11);
    let rop = arg(emu, 12);
    let dc = gdi_dc_or_default(emu, hdc);
    trace_gdi!(
        "gdi StretchDIBits hdc={hdc:08x} surf={:08x} dst=({x},{y} {dst_w}x{dst_h}) src=({src_x},{src_y} {src_w}x{src_h}) bits={bits:08x} info={info:08x} use={color_use} rop={rop:08x}",
        dc.surface,
    );
    let (copied, surface) = if (rop & 0x00ff_ffff) == 0x00cc_0020
        && dc.surface != 0
        && dst_w != 0
        && dst_h != 0
        && src_w != 0
        && src_h != 0
        && bits != 0
        && info != 0
    {
        let surface = read_surface_info(emu, dc.surface).hle();
        let (dst_left, dst_right) = ordered_span(x.saturating_add(dc.origin_x), dst_w);
        let (dst_top, dst_bottom) = ordered_span(y.saturating_add(dc.origin_y), dst_h);
        let copied = read_dib_info(emu, info)
            .and_then(|dib| {
                let src_rect = dib_stretch_src_rect(dib, src_x, src_y, src_w, src_h, dst_w, dst_h);
                trace_gdi!(
                    "gdi StretchDIBits mapped_src=({},{}..{},{})",
                    src_rect.left,
                    src_rect.top,
                    src_rect.right,
                    src_rect.bottom,
                );
                blit_dib_to_surface(
                    emu,
                    surface,
                    RectI {
                        left: dst_left,
                        top: dst_top,
                        right: dst_right,
                        bottom: dst_bottom,
                    },
                    src_rect,
                    bits,
                    info,
                    color_use,
                    dc.selected_palette,
                    0,
                    dib.height,
                    Some(dc),
                )
            })
            .unwrap_or(0);
        (copied, Some(surface))
    } else {
        (0, None)
    };
    if copied != 0 {
        if let Some(surface) = surface {
            mark_gdi_screen_surface_dirty(emu, surface);
        }
    }
    ret(emu, copied);
    HleResult::Retn(52)
}

#[allow(clippy::too_many_arguments)]
fn draw_gdi_text_with_canvas(
    _emu: &mut Emulator,
    _surface: SurfaceInfo,
    _dc: GdiDc,
    _bytes: &[u8],
    _x: i32,
    _y: i32,
    _metrics: GdiLineMetrics,
    _clip: Option<(i32, i32, i32, i32)>,
) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let (clip_left, clip_top, clip_right, clip_bottom) =
            _clip.unwrap_or((0, 0, _surface.width as i32, _surface.height as i32));
        if clip_right <= clip_left || clip_bottom <= clip_top {
            return true;
        }
        return unsafe {
            wemu_canvas_text(
                _surface.buffer,
                _surface.width,
                _surface.height,
                _surface.pitch,
                _surface.bpp,
                _bytes.as_ptr(),
                _bytes.len(),
                _x,
                _y,
                _metrics.height.max(1) as u32,
                _metrics.extra,
                _dc.text_color,
                clip_left,
                clip_top,
                clip_right,
                clip_bottom,
            ) != 0
        };
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

// COLORREF GetPixel(HDC hdc, int x, int y)
// Read one pixel from a surface-backed DC as COLORREF.
fn hle_get_pixel(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let value = if dc.surface != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        read_surface_pixel_colorref(
            emu,
            surface,
            (arg(emu, 1) as i32).saturating_add(dc.origin_x),
            (arg(emu, 2) as i32).saturating_add(dc.origin_y),
        )
        .unwrap_or(0xffff_ffff)
    } else {
        0xffff_ffff
    };
    ret(emu, value);
    HleResult::Retn(12)
}

// COLORREF SetPixel(HDC hdc, int x, int y, COLORREF color)
// Write one pixel to a surface-backed DC and return the color.
fn hle_set_pixel(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dc = gdi_dc_or_default(emu, arg(emu, 0));
    let color = arg(emu, 3);
    if dc.surface != 0 {
        let surface = read_surface_info(emu, dc.surface).hle();
        write_surface_pixel_colorref(
            emu,
            surface,
            (arg(emu, 1) as i32).saturating_add(dc.origin_x),
            (arg(emu, 2) as i32).saturating_add(dc.origin_y),
            color,
        );
    }
    trace_gdi!(
        "gdi SetPixel hdc={:08x} x={} y={} color={color:08x}",
        arg(emu, 0),
        arg(emu, 1) as i32,
        arg(emu, 2) as i32,
    );
    ret(emu, color);
    HleResult::Retn(16)
}

// int SetMapMode(HDC hdc, int mode)
// Track the DC mapping mode and return the previous mode.
fn hle_set_map_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let mode = arg(emu, 1);
    let old = emu
        .hle
        .gdi_dcs
        .get_mut(&hdc)
        .map(|dc| {
            let old = dc.map_mode;
            if (1..=8).contains(&mode) {
                dc.map_mode = mode;
            }
            old
        })
        .unwrap_or(0);
    ret(emu, old);
    HleResult::Retn(8)
}

// int GetMapMode(HDC hdc)
// Return the tracked mapping mode, defaulting to MM_TEXT for unknown DCs.
fn hle_get_map_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let mode = emu
        .hle
        .gdi_dcs
        .get(&arg(emu, 0))
        .map(|dc| dc.map_mode)
        .unwrap_or(MM_TEXT);
    ret(emu, mode);
    HleResult::Retn(4)
}

// UINT SetTextAlign(HDC hdc, UINT align)
// Track text alignment flags and return the previous alignment.
fn hle_set_text_align(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let align = arg(emu, 1);
    let old = emu
        .hle
        .gdi_dcs
        .get_mut(&hdc)
        .map(|dc| {
            let old = dc.text_align;
            dc.text_align = align;
            old
        })
        .unwrap_or(u32::MAX);
    ret(emu, old);
    HleResult::Retn(8)
}

// UINT GetTextAlign(HDC hdc)
// Return the tracked text alignment flags for the DC.
fn hle_get_text_align(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let align = emu
        .hle
        .gdi_dcs
        .get(&arg(emu, 0))
        .map(|dc| dc.text_align)
        .unwrap_or(TA_LEFT | TA_TOP);
    ret(emu, align);
    HleResult::Retn(4)
}

// int SetROP2(HDC hdc, int rop2)
// Set the tracked pen raster operation and return the previous mode.
fn hle_set_rop2(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let mode = arg(emu, 1);
    let old = emu
        .hle
        .gdi_dcs
        .get_mut(&hdc)
        .map(|dc| {
            let old = dc.rop2;
            if (R2_BLACK..=R2_WHITE).contains(&mode) {
                dc.rop2 = mode;
            }
            old
        })
        .unwrap_or(0);
    ret(emu, old);
    HleResult::Retn(8)
}

// DWORD GetLayout(HDC hdc)
// Return the default left-to-right layout for tracked and screen DCs.
fn hle_get_layout(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let layout = emu
        .hle
        .gdi_dcs
        .get(&arg(emu, 0))
        .map(|dc| dc.layout)
        .unwrap_or(0);
    ret(emu, layout);
    HleResult::Retn(4)
}

// DWORD SetLayout(HDC hdc, DWORD layout)
// Track the DC layout flags and return the previous value.
fn hle_set_layout(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let layout = arg(emu, 1);
    let old = emu
        .hle
        .gdi_dcs
        .get_mut(&hdc)
        .map(|dc| {
            let old = dc.layout;
            dc.layout = layout;
            old
        })
        .unwrap_or(0);
    ret(emu, old);
    HleResult::Retn(8)
}

fn wildcard_match_ci(pattern: &str, name: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let p = pattern.as_bytes();
    let n = name.as_bytes();
    let mut dp = vec![false; n.len() + 1];
    dp[0] = true;
    for &pc in p {
        let mut next = vec![false; n.len() + 1];
        if pc == b'*' {
            next[0] = dp[0];
            for i in 1..=n.len() {
                next[i] = dp[i] || next[i - 1];
            }
        } else {
            for i in 1..=n.len() {
                next[i] = dp[i - 1] && (pc == b'?' || pc == n[i - 1]);
            }
        }
        dp = next;
    }
    dp[n.len()]
}

fn get_object_common(emu: &mut Emulator) -> u32 {
    const BITMAP_OBJECT_SIZE: u32 = 24;

    let object = arg(emu, 0);
    let count = arg(emu, 1);
    let out = arg(emu, 2);
    let Some(bitmap) = emu.hle.gdi_bitmaps.get(&object).copied() else {
        return 0;
    };
    if out == 0 {
        return BITMAP_OBJECT_SIZE;
    }
    if count < BITMAP_OBJECT_SIZE {
        return 0;
    }
    let surface = read_surface_info(emu, bitmap.surface).hle();
    emu.memory.write_u32(out, 0).hle();
    emu.memory.write_u32(out + 4, surface.width).hle();
    emu.memory.write_u32(out + 8, surface.height).hle();
    emu.memory.write_u32(out + 12, surface.pitch).hle();
    emu.memory.write_u16(out + 16, 1).hle();
    emu.memory.write_u16(out + 18, surface.bpp as u16).hle();
    emu.memory.write_u32(out + 20, 0).hle();
    BITMAP_OBJECT_SIZE
}

fn gdi_dc_or_default(emu: &Emulator, hdc: u32) -> GdiDc {
    emu.hle.gdi_dcs.get(&hdc).copied().unwrap_or(GdiDc {
        surface: 0,
        hwnd: 0,
        selected_font: 0,
        selected_bitmap: 0,
        selected_brush: stock_object_handle(STOCK_WHITE_BRUSH),
        selected_pen: stock_object_handle(STOCK_BLACK_PEN),
        selected_palette: 0,
        rop2: R2_COPYPEN,
        layout: 0,
        map_mode: MM_TEXT,
        text_align: TA_LEFT | TA_TOP,
        text_extra: 0,
        text_color: 0x00ff_ffff,
        bk_color: 0x00ff_ffff,
        bk_mode: 1,
        origin_x: 0,
        origin_y: 0,
        brush_origin_x: 0,
        brush_origin_y: 0,
        current_x: 0,
        current_y: 0,
    })
}

fn gdi_dc_draw_point_visible(emu: &Emulator, dc: GdiDc, x: i32, y: i32) -> bool {
    dc.hwnd == 0 || !window_screen_point_occluded_by_higher_top_level(emu, dc.hwnd, x, y)
}

fn gdi_device_point_visible(emu: &Emulator, dc: GdiDc, x: i32, y: i32) -> bool {
    dc.surface == 0
        || read_surface_info(emu, dc.surface).is_ok_and(|surface| {
            x >= 0 && y >= 0 && x < surface.width as i32 && y < surface.height as i32
        })
}

fn gdi_device_rect_visible(emu: &Emulator, dc: GdiDc, rect: RectI) -> bool {
    if rect.width() <= 0 || rect.height() <= 0 {
        return false;
    }
    dc.surface == 0
        || read_surface_info(emu, dc.surface).is_ok_and(|surface| {
            rect.left < surface.width as i32
                && rect.top < surface.height as i32
                && rect.right > 0
                && rect.bottom > 0
        })
}

fn gdi_text_bytes(emu: &Emulator, text: u32, count: u32) -> Vec<u8> {
    if text == 0 {
        return Vec::new();
    }
    if count == u32::MAX {
        let mut bytes = Vec::new();
        for index in 0..4096u32 {
            let byte = emu.memory.read_u8(text.wrapping_add(index)).hle();
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }
        return bytes;
    }
    emu.memory
        .read_bytes(text, count.min(4096) as usize)
        .hle()
        .into_iter()
        .take_while(|byte| *byte != 0)
        .collect()
}

fn gdi_text_metrics(emu: &Emulator, dc: GdiDc) -> GdiTextMetrics {
    let height = gdi_font_height(emu, dc);
    let char_width = ((height + 1) / 2).max(4);
    let extra = dc.text_extra.clamp(-char_width + 1, char_width * 4);
    GdiTextMetrics {
        char_width,
        extra,
    }
}

fn gdi_text_layout(emu: &Emulator, dc: GdiDc, bytes: &[u8], multiline: bool) -> GdiTextLayout {
    let raw = gdi_text_metrics(emu, dc);
    let metrics = GdiLineMetrics {
        height: gdi_font_height(emu, dc),
        char_width: raw.char_width,
        extra: raw.extra,
    };
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if multiline && (byte == b'\r' || byte == b'\n') {
            push_gdi_line(&mut lines, bytes, start, index, metrics);
            index += 1;
            if index < bytes.len()
                && ((byte == b'\r' && bytes[index] == b'\n')
                    || (byte == b'\n' && bytes[index] == b'\r'))
            {
                index += 1;
            }
            start = index;
            continue;
        }
        index += if byte >= 0x80 && index + 1 < bytes.len() {
            2
        } else {
            1
        };
    }
    if start < bytes.len() || lines.is_empty() {
        push_gdi_line(&mut lines, bytes, start, bytes.len(), metrics);
    }

    let width = lines.iter().map(|line| line.width).max().unwrap_or(0);
    let height = metrics.height.saturating_mul(lines.len() as i32).max(1);
    GdiTextLayout {
        width,
        height,
        metrics,
        lines,
    }
}

fn push_gdi_line(
    lines: &mut Vec<GdiTextLine>,
    bytes: &[u8],
    start: usize,
    end: usize,
    metrics: GdiLineMetrics,
) {
    lines.push(GdiTextLine {
        start,
        end,
        width: gdi_line_width(&bytes[start..end], metrics.char_width, metrics.extra),
    });
}

fn gdi_line_width(bytes: &[u8], char_width: i32, extra: i32) -> i32 {
    let mut width = 0i32;
    let mut glyphs = 0i32;
    for glyph in gdi_glyphs(bytes, char_width) {
        if glyphs != 0 {
            width = width.saturating_add(extra);
        }
        width = width.saturating_add(glyph.width);
        glyphs += 1;
    }
    width.max(0)
}

fn gdi_glyphs(bytes: &[u8], char_width: i32) -> impl Iterator<Item = GdiGlyph> + '_ {
    let mut index = 0usize;
    std::iter::from_fn(move || {
        if index >= bytes.len() {
            return None;
        }
        let byte = bytes[index];
        if byte >= 0x80 && index + 1 < bytes.len() {
            index += 2;
            Some(GdiGlyph {
                width: char_width * 2,
                visible: true,
                byte: b'?',
            })
        } else {
            index += 1;
            Some(GdiGlyph {
                width: char_width,
                visible: byte != b' ',
                byte,
            })
        }
    })
}

fn draw_gdi_text(
    emu: &mut Emulator,
    dc: GdiDc,
    bytes: &[u8],
    x: i32,
    y: i32,
    metrics: GdiLineMetrics,
    clip: Option<(i32, i32, i32, i32)>,
) {
    if dc.surface == 0 || bytes.is_empty() {
        return;
    }
    let origin_x = if (dc.text_align & TA_UPDATECP) != 0 {
        dc.current_x
    } else {
        x
    };
    let origin_y = if (dc.text_align & TA_UPDATECP) != 0 {
        dc.current_y
    } else {
        y
    };
    let mut x = origin_x.saturating_add(dc.origin_x);
    let mut y = origin_y.saturating_add(dc.origin_y);
    let width = gdi_line_width(bytes, metrics.char_width, metrics.extra);
    match dc.text_align & (TA_RIGHT | TA_CENTER) {
        TA_CENTER => x = x.saturating_sub(width / 2),
        TA_RIGHT => x = x.saturating_sub(width),
        _ => {}
    }
    match dc.text_align & (TA_BOTTOM | TA_BASELINE) {
        TA_BASELINE => y = y.saturating_sub(metrics.height.saturating_sub(2)),
        TA_BOTTOM => y = y.saturating_sub(metrics.height),
        _ => {}
    }
    let clip = clip.map(|rect| {
        (
            rect.0.saturating_add(dc.origin_x),
            rect.1.saturating_add(dc.origin_y),
            rect.2.saturating_add(dc.origin_x),
            rect.3.saturating_add(dc.origin_y),
        )
    });
    let surface = read_surface_info(emu, dc.surface).hle();
    if draw_gdi_text_with_canvas(emu, surface, dc, bytes, x, y, metrics, clip) {
        mark_gdi_screen_surface_dirty(emu, surface);
        return;
    }
    let mut cursor_x = x;
    let mut glyphs = 0i32;
    for glyph in gdi_glyphs(bytes, metrics.char_width) {
        if glyphs != 0 {
            cursor_x = cursor_x.saturating_add(metrics.extra);
        }
        if glyph.visible {
            draw_gdi_glyph_bitmap(
                emu,
                surface,
                dc.text_color,
                glyph.byte,
                cursor_x,
                y,
                glyph.width,
                metrics.height,
                clip,
            );
        }
        cursor_x = cursor_x.saturating_add(glyph.width);
        glyphs += 1;
    }
    mark_gdi_screen_surface_dirty(emu, surface);
}

fn update_text_current_pos(emu: &mut Emulator, hdc: u32, dc: GdiDc, layout: &GdiTextLayout) {
    if (dc.text_align & TA_UPDATECP) == 0 {
        return;
    }
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        dc.current_x = dc.current_x.saturating_add(layout.width);
    }
}

fn draw_gdi_glyph_bitmap(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    colorref: u32,
    byte: u8,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    clip: Option<(i32, i32, i32, i32)>,
) {
    let Some(rows) = seabios_glyph_8x8(byte) else {
        draw_gdi_glyph_rect(emu, surface, colorref, x, y, width, height, clip);
        return;
    };
    let glyph_clip = glyph_bounds_clip(x, y, width, height, clip);
    let Some(glyph_clip) = glyph_clip else {
        return;
    };
    let scale_x = (width / 8).max(1);
    let scale_y = (height / 8).max(1);
    for (row_index, row_bits) in rows.iter().enumerate() {
        for col in 0..8 {
            if (row_bits & (1 << (7 - col))) == 0 {
                continue;
            }
            draw_gdi_glyph_rect(
                emu,
                surface,
                colorref,
                x + col * scale_x,
                y + row_index as i32 * scale_y,
                scale_x,
                scale_y,
                Some(glyph_clip),
            );
        }
    }
}

fn gdi_pixel_bytes(colorref: u32, bpp: u32) -> [u8; 4] {
    let r = (colorref & 0xff) as u8;
    let g = ((colorref >> 8) & 0xff) as u8;
    let b = ((colorref >> 16) & 0xff) as u8;
    match bpp {
        0..=8 => [r.max(1), 0, 0, 0],
        9..=16 => {
            let mut value =
                (((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)).max(1);
            if value == 0 {
                value = 1;
            }
            let bytes = value.to_le_bytes();
            [bytes[0], bytes[1], 0, 0]
        }
        17..=24 => [b, g, r.max(1), 0],
        _ => [b, g, r.max(1), 0],
    }
}

const GDI_STOCK_BITMAP: u32 = 0x5001_0000;
const GDI_STOCK_OBJECT_BASE: u32 = 0x5002_0000;

const STOCK_WHITE_BRUSH: u32 = 0;
const STOCK_LTGRAY_BRUSH: u32 = 1;
const STOCK_GRAY_BRUSH: u32 = 2;
const STOCK_DKGRAY_BRUSH: u32 = 3;
const STOCK_BLACK_BRUSH: u32 = 4;
const STOCK_NULL_BRUSH: u32 = 5;
const STOCK_WHITE_PEN: u32 = 6;
const STOCK_BLACK_PEN: u32 = 7;
const STOCK_NULL_PEN: u32 = 8;
const STOCK_OEM_FIXED_FONT: u32 = 10;
const STOCK_ANSI_FIXED_FONT: u32 = 11;
const STOCK_ANSI_VAR_FONT: u32 = 12;
const STOCK_SYSTEM_FONT: u32 = 13;
const STOCK_DEVICE_DEFAULT_FONT: u32 = 14;
const STOCK_DEFAULT_PALETTE: u32 = 15;
const STOCK_SYSTEM_FIXED_FONT: u32 = 16;
const STOCK_DEFAULT_GUI_FONT: u32 = 17;
const STOCK_DC_BRUSH: u32 = 18;
const STOCK_DC_PEN: u32 = 19;

fn stock_object_handle(index: u32) -> u32 {
    GDI_STOCK_OBJECT_BASE | (index & 0xffff)
}

fn stock_object_index(handle: u32) -> Option<u32> {
    ((handle & 0xffff_0000) == GDI_STOCK_OBJECT_BASE).then_some(handle & 0xffff)
}

fn is_known_stock_object(index: u32) -> bool {
    matches!(
        index,
        STOCK_WHITE_BRUSH
            | STOCK_LTGRAY_BRUSH
            | STOCK_GRAY_BRUSH
            | STOCK_DKGRAY_BRUSH
            | STOCK_BLACK_BRUSH
            | STOCK_NULL_BRUSH
            | STOCK_WHITE_PEN
            | STOCK_BLACK_PEN
            | STOCK_NULL_PEN
            | STOCK_OEM_FIXED_FONT
            | STOCK_ANSI_FIXED_FONT
            | STOCK_ANSI_VAR_FONT
            | STOCK_SYSTEM_FONT
            | STOCK_DEVICE_DEFAULT_FONT
            | STOCK_DEFAULT_PALETTE
            | STOCK_SYSTEM_FIXED_FONT
            | STOCK_DEFAULT_GUI_FONT
            | STOCK_DC_BRUSH
            | STOCK_DC_PEN
    )
}

fn stock_brush_colorref(handle: u32) -> Option<Option<u32>> {
    match stock_object_index(handle)? {
        STOCK_WHITE_BRUSH => Some(Some(0x00ff_ffff)),
        STOCK_LTGRAY_BRUSH => Some(Some(0x00c0_c0c0)),
        STOCK_GRAY_BRUSH => Some(Some(0x0080_8080)),
        STOCK_DKGRAY_BRUSH => Some(Some(0x0040_4040)),
        STOCK_BLACK_BRUSH => Some(Some(0x0000_0000)),
        STOCK_NULL_BRUSH => Some(None),
        STOCK_DC_BRUSH => Some(Some(0x00ff_ffff)),
        _ => None,
    }
}

fn stock_pen_colorref(handle: u32) -> Option<Option<u32>> {
    match stock_object_index(handle)? {
        STOCK_WHITE_PEN => Some(Some(0x00ff_ffff)),
        STOCK_BLACK_PEN => Some(Some(0x0000_0000)),
        STOCK_NULL_PEN => Some(None),
        STOCK_DC_PEN => Some(Some(0x0000_0000)),
        _ => None,
    }
}

fn is_stock_font(handle: u32) -> bool {
    matches!(
        stock_object_index(handle),
        Some(
            STOCK_OEM_FIXED_FONT
                | STOCK_ANSI_FIXED_FONT
                | STOCK_ANSI_VAR_FONT
                | STOCK_SYSTEM_FONT
                | STOCK_DEVICE_DEFAULT_FONT
                | STOCK_SYSTEM_FIXED_FONT
                | STOCK_DEFAULT_GUI_FONT
        )
    )
}

fn is_sys_color_brush(handle: u32) -> bool {
    (handle & 0xffff_ff00) == 0x5000_0000
}

fn copy_create_bitmap_bits(
    emu: &mut Emulator,
    surface: u32,
    bits: u32,
    width: u32,
    height: u32,
    bpp: u32,
) {
    let dst = read_surface_info(emu, surface).hle();
    let src_stride = width
        .saturating_mul(bpp)
        .saturating_add(15)
        .saturating_div(16)
        .saturating_mul(2);
    for y in 0..height {
        let row = emu
            .memory
            .read_bytes(bits + y * src_stride, src_stride as usize)
            .unwrap_or_default();
        for x in 0..width {
            if let Some(color) = create_bitmap_pixel_colorref(&row, x, bpp) {
                write_surface_pixel_colorref(emu, dst, x as i32, y as i32, color);
            }
        }
    }
}

fn create_bitmap_pixel_colorref(row: &[u8], x: u32, bpp: u32) -> Option<u32> {
    match bpp {
        1 => {
            let byte = *row.get((x / 8) as usize)?;
            let bit = 7 - (x & 7);
            Some(if (byte & (1 << bit)) != 0 { 0x00ff_ffff } else { 0 })
        }
        4 => {
            let byte = *row.get((x / 2) as usize)?;
            let nibble = if (x & 1) == 0 { byte >> 4 } else { byte & 0x0f };
            let v = nibble as u32 * 17;
            Some(v | (v << 8) | (v << 16))
        }
        8 => {
            let v = *row.get(x as usize)? as u32;
            Some(v | (v << 8) | (v << 16))
        }
        16 => {
            let off = x as usize * 2;
            let value = u16::from_le_bytes([*row.get(off)?, *row.get(off + 1)?]) as u32;
            let r = ((value >> 11) & 0x1f) << 3;
            let g = ((value >> 5) & 0x3f) << 2;
            let b = (value & 0x1f) << 3;
            Some(r | (g << 8) | (b << 16))
        }
        24 => {
            let off = x as usize * 3;
            let b = *row.get(off)? as u32;
            let g = *row.get(off + 1)? as u32;
            let r = *row.get(off + 2)? as u32;
            Some(r | (g << 8) | (b << 16))
        }
        32 => {
            let off = x as usize * 4;
            let b = *row.get(off)? as u32;
            let g = *row.get(off + 1)? as u32;
            let r = *row.get(off + 2)? as u32;
            Some(r | (g << 8) | (b << 16))
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct DibBlitInfo {
    width: u32,
    height: u32,
    top_down: bool,
    bpp: u32,
    palette_addr: u32,
    palette_entries: u32,
    palette_stride: u32,
}

fn ordered_span(start: i32, len: i32) -> (i32, i32) {
    if len >= 0 {
        (start, start.saturating_add(len))
    } else {
        (start.saturating_add(len), start)
    }
}

fn dib_top_left_src_rect(x: i32, y: i32, width: i32, height: i32) -> RectI {
    let (left, right) = ordered_span(x, width);
    let (top, bottom) = ordered_span(y, height);
    RectI {
        left,
        top,
        right,
        bottom,
    }
}

fn dib_lower_left_src_rect(dib: DibBlitInfo, x: i32, y: i32, width: i32, height: i32) -> RectI {
    let (left, right) = ordered_span(x, width);
    let (lower, upper) = ordered_span(y, height);
    let dib_height = dib.height as i32;
    RectI {
        left,
        top: dib_height.saturating_sub(upper),
        right,
        bottom: dib_height.saturating_sub(lower),
    }
}

fn dib_stretch_src_rect(
    dib: DibBlitInfo,
    src_x: i32,
    src_y: i32,
    src_w: i32,
    src_h: i32,
    dst_w: i32,
    dst_h: i32,
) -> RectI {
    let non_stretch_from_origin = src_x == 0 && src_y == 0 && src_w == dst_w && src_h == dst_h;
    if dib.top_down && non_stretch_from_origin {
        dib_top_left_src_rect(src_x, src_y, src_w, src_h)
    } else {
        dib_lower_left_src_rect(dib, src_x, src_y, src_w, src_h)
    }
}

fn dib_header_dimensions(emu: &Emulator, header: u32) -> Option<(u32, u32)> {
    if header == 0 {
        return None;
    }
    let header_size = emu.memory.read_u32(header).ok()?;
    if header_size == 12 {
        let width = emu.memory.read_u16(header + 4).ok()? as u32;
        let height = emu.memory.read_u16(header + 6).ok()? as u32;
        return (width != 0 && height != 0).then_some((width, height));
    }
    if header_size < 40 {
        return None;
    }
    let width = emu.memory.read_u32(header + 4).ok()? as i32;
    let height = emu.memory.read_u32(header + 8).ok()? as i32;
    (width > 0 && height != 0).then_some((width as u32, height.unsigned_abs()))
}

fn read_dib_info(emu: &Emulator, info: u32) -> Option<DibBlitInfo> {
    const BI_RGB: u32 = 0;

    let header_size = emu.memory.read_u32(info).ok()?;
    if header_size == 12 {
        let width = emu.memory.read_u16(info + 4).ok()? as u32;
        let height = emu.memory.read_u16(info + 6).ok()? as u32;
        let planes = emu.memory.read_u16(info + 8).ok()?;
        let bpp = emu.memory.read_u16(info + 10).ok()? as u32;
        if width == 0 || height == 0 || planes != 1 {
            return None;
        }
        return Some(DibBlitInfo {
            width,
            height,
            top_down: false,
            bpp,
            palette_addr: info + 12,
            palette_entries: dib_palette_entries(bpp, 0)?,
            palette_stride: 3,
        });
    }
    if header_size < 40 {
        return None;
    }

    let width = emu.memory.read_u32(info + 4).ok()? as i32;
    let raw_height = emu.memory.read_u32(info + 8).ok()? as i32;
    let planes = emu.memory.read_u16(info + 12).ok()?;
    let bpp = emu.memory.read_u16(info + 14).ok()? as u32;
    let compression = emu.memory.read_u32(info + 16).ok()?;
    let colors_used = emu.memory.read_u32(info + 32).ok()?;
    if width <= 0 || raw_height == 0 || planes != 1 || compression != BI_RGB {
        return None;
    }
    Some(DibBlitInfo {
        width: width as u32,
        height: raw_height.unsigned_abs(),
        top_down: raw_height < 0,
        bpp,
        palette_addr: info.checked_add(header_size)?,
        palette_entries: dib_palette_entries(bpp, colors_used)?,
        palette_stride: 4,
    })
}

fn read_dib_palette_colorrefs(emu: &Emulator, dib: DibBlitInfo) -> Option<Vec<u32>> {
    let mut palette = Vec::with_capacity(dib.palette_entries as usize);
    for index in 0..dib.palette_entries {
        let entry = dib
            .palette_addr
            .checked_add(index.checked_mul(dib.palette_stride)?)?;
        let b = emu.memory.read_u8(entry).ok()? as u32;
        let g = emu.memory.read_u8(entry + 1).ok()? as u32;
        let r = emu.memory.read_u8(entry + 2).ok()? as u32;
        palette.push(r | (g << 8) | (b << 16));
    }
    Some(palette)
}

fn read_dib_palette_colorrefs_for_usage(
    emu: &Emulator,
    dib: DibBlitInfo,
    color_use: u32,
    logical_palette: u32,
) -> Option<Vec<u32>> {
    const DIB_PAL_COLORS: u32 = 1;
    if color_use != DIB_PAL_COLORS {
        return read_dib_palette_colorrefs(emu, dib);
    }
    let logical = emu.hle.gdi_palettes.get(&logical_palette);
    let mut palette = Vec::with_capacity(dib.palette_entries as usize);
    for index in 0..dib.palette_entries {
        let entry = dib.palette_addr.checked_add(index.checked_mul(2)?)?;
        let palette_index = emu.memory.read_u16(entry).ok()? as usize;
        let color = logical
            .and_then(|palette| palette.entries.get(palette_index))
            .map(|entry| entry[0] as u32 | ((entry[1] as u32) << 8) | ((entry[2] as u32) << 16))
            .unwrap_or_else(|| {
                let v = palette_index as u32 & 0xff;
                v | (v << 8) | (v << 16)
            });
        palette.push(color);
    }
    Some(palette)
}

fn dib_pixel_colorref(row: &[u8], x: u32, bpp: u32, palette: &[u32]) -> Option<u32> {
    match bpp {
        1 => {
            let byte = *row.get((x / 8) as usize)?;
            let index = ((byte >> (7 - (x & 7))) & 1) as usize;
            palette.get(index).copied().or(Some(if index != 0 {
                0x00ff_ffff
            } else {
                0
            }))
        }
        4 => {
            let byte = *row.get((x / 2) as usize)?;
            let index = (if (x & 1) == 0 { byte >> 4 } else { byte & 0x0f }) as usize;
            palette.get(index).copied().or_else(|| {
                let v = index as u32 * 17;
                Some(v | (v << 8) | (v << 16))
            })
        }
        8 => {
            let index = *row.get(x as usize)? as usize;
            palette.get(index).copied().or_else(|| {
                let v = index as u32;
                Some(v | (v << 8) | (v << 16))
            })
        }
        16 => {
            let off = x.checked_mul(2)? as usize;
            let raw = u16::from_le_bytes([*row.get(off)?, *row.get(off + 1)?]) as u32;
            let r = ((raw >> 10) & 0x1f) << 3;
            let g = ((raw >> 5) & 0x1f) << 3;
            let b = (raw & 0x1f) << 3;
            Some(r | (g << 8) | (b << 16))
        }
        24 => {
            let off = x.checked_mul(3)? as usize;
            let b = *row.get(off)? as u32;
            let g = *row.get(off + 1)? as u32;
            let r = *row.get(off + 2)? as u32;
            Some(r | (g << 8) | (b << 16))
        }
        32 => {
            let off = x.checked_mul(4)? as usize;
            let b = *row.get(off)? as u32;
            let g = *row.get(off + 1)? as u32;
            let r = *row.get(off + 2)? as u32;
            Some(r | (g << 8) | (b << 16))
        }
        _ => None,
    }
}

fn blit_dib_to_surface(
    emu: &mut Emulator,
    dst: SurfaceInfo,
    dst_rect: RectI,
    src_rect: RectI,
    bits: u32,
    info: u32,
    color_use: u32,
    logical_palette: u32,
    start_scan: u32,
    scan_lines: u32,
    clip_dc: Option<GdiDc>,
) -> Option<u32> {
    let dib = read_dib_info(emu, info)?;
    let stride = dib_row_stride(dib.width, dib.bpp)?;
    let palette = read_dib_palette_colorrefs_for_usage(emu, dib, color_use, logical_palette)?;
    let dst_w = dst_rect.width();
    let dst_h = dst_rect.height();
    let src_w = src_rect.width();
    let src_h = src_rect.height();
    if dst_w <= 0 || dst_h <= 0 || src_w <= 0 || src_h <= 0 || scan_lines == 0 {
        return Some(0);
    }
    trace_gdi!(
        "gdi DIB blit dst_rect=({},{}..{},{} {}x{}) src_rect=({},{}..{},{} {}x{}) dib={}x{} top_down={} bpp={} stride={} start={} lines={}",
        dst_rect.left,
        dst_rect.top,
        dst_rect.right,
        dst_rect.bottom,
        dst_w,
        dst_h,
        src_rect.left,
        src_rect.top,
        src_rect.right,
        src_rect.bottom,
        src_w,
        src_h,
        dib.width,
        dib.height,
        dib.top_down,
        dib.bpp,
        stride,
        start_scan,
        scan_lines,
    );

    let left = dst_rect.left.max(0).min(dst.width as i32);
    let top = dst_rect.top.max(0).min(dst.height as i32);
    let right = dst_rect.right.max(0).min(dst.width as i32);
    let bottom = dst_rect.bottom.max(0).min(dst.height as i32);
    if right <= left || bottom <= top {
        return Some(0);
    }

    let tracked_scans = scan_lines.min(dib.height).min(8192) as usize;
    let mut scan_written = vec![false; tracked_scans];
    for y in top..bottom {
        let dst_dy = y.saturating_sub(dst_rect.top);
        let src_top_y = src_rect.top.saturating_add(dst_dy.saturating_mul(src_h) / dst_h);
        if src_top_y < 0 || src_top_y >= dib.height as i32 {
            continue;
        }
        let scan = if dib.top_down {
            src_top_y
        } else {
            dib.height as i32 - 1 - src_top_y
        };
        if scan < start_scan as i32 || scan >= start_scan.saturating_add(scan_lines) as i32 {
            continue;
        }
        let scan_offset = (scan as u32).checked_sub(start_scan)?;
        let row_addr = bits.checked_add(scan_offset.checked_mul(stride)?)?;
        let row = emu.memory.read_bytes(row_addr, stride as usize).ok()?;
        let mut wrote_row = false;
        for x in left..right {
            let dst_dx = x.saturating_sub(dst_rect.left);
            let src_x = src_rect.left.saturating_add(dst_dx.saturating_mul(src_w) / dst_w);
            if src_x < 0 || src_x >= dib.width as i32 {
                continue;
            }
            if clip_dc.is_some_and(|dc| !gdi_dc_draw_point_visible(emu, dc, x, y)) {
                continue;
            }
            let color = dib_pixel_colorref(&row, src_x as u32, dib.bpp, &palette)?;
            write_surface_pixel_colorref(emu, dst, x, y, color);
            wrote_row = true;
        }
        let written_index = scan_offset as usize;
        if wrote_row && written_index < scan_written.len() {
            scan_written[written_index] = true;
        }
    }
    let copied = scan_written.iter().filter(|written| **written).count() as u32;
    Some(if copied != 0 {
        copied
    } else if tracked_scans == 0 {
        scan_lines.min(dib.height.saturating_sub(start_scan))
    } else {
        0
    })
}

fn read_dib_palette(emu: &Emulator, addr: u32, entries: u32, stride: u32) -> Option<Vec<u16>> {
    let mut palette = Vec::with_capacity(entries as usize);
    for index in 0..entries {
        let entry = addr.checked_add(index.checked_mul(stride)?)?;
        let b = emu.memory.read_u8(entry).ok()?;
        let g = emu.memory.read_u8(entry + 1).ok()?;
        let r = emu.memory.read_u8(entry + 2).ok()?;
        palette.push(rgb_to_565(r, g, b));
    }
    Some(palette)
}

fn read_dib_rle_byte(emu: &Emulator, addr: &mut u32, end_addr: u32) -> Option<u8> {
    if *addr >= end_addr {
        return None;
    }
    let value = emu.memory.read_u8(*addr).ok()?;
    *addr = addr.checked_add(1)?;
    Some(value)
}

fn dib_row_stride(width: u32, bpp: u32) -> Option<u32> {
    width.checked_mul(bpp)?.checked_add(31).map(|bits| (bits / 32) * 4)
}

fn dib_pixel_565(row: &[u8], x: u32, bpp: u32, palette: &[u16]) -> Option<u16> {
    match bpp {
        1 => {
            let byte = *row.get((x / 8) as usize)?;
            let index = ((byte >> (7 - (x & 7))) & 1) as usize;
            palette.get(index).copied()
        }
        4 => {
            let byte = *row.get((x / 2) as usize)?;
            let index = (if (x & 1) == 0 { byte >> 4 } else { byte & 0x0f }) as usize;
            palette.get(index).copied()
        }
        8 => {
            let index = *row.get(x as usize)? as usize;
            palette.get(index).copied()
        }
        16 => {
            let off = x.checked_mul(2)? as usize;
            let raw = u16::from_le_bytes([*row.get(off)?, *row.get(off + 1)?]);
            let r = (((raw >> 10) & 0x1f) << 3) as u8;
            let g = (((raw >> 5) & 0x1f) << 3) as u8;
            let b = ((raw & 0x1f) << 3) as u8;
            Some(rgb_to_565(r, g, b))
        }
        24 => {
            let off = x.checked_mul(3)? as usize;
            let b = *row.get(off)?;
            let g = *row.get(off + 1)?;
            let r = *row.get(off + 2)?;
            Some(rgb_to_565(r, g, b))
        }
        32 => {
            let off = x.checked_mul(4)? as usize;
            let b = *row.get(off)?;
            let g = *row.get(off + 1)?;
            let r = *row.get(off + 2)?;
            Some(rgb_to_565(r, g, b))
        }
        _ => None,
    }
}

fn ensure_gdi_screen_surface(emu: &mut Emulator) -> u32 {
    if emu.hle.gdi_screen_surface != 0 {
        return emu.hle.gdi_screen_surface;
    }
    let surf = create_surface_with_format(
        emu,
        emu.backend.width().max(1),
        emu.backend.height().max(1),
        emu.hle.ddraw_bpp.max(8),
        0x200,
    )
    .hle();
    emu.hle.gdi_screen_surface = surf;
    emu.hle.mark_hle_windows_dirty();
    surf
}

fn draw_framebuffer_glyph_bitmap(
    emu: &mut Emulator,
    color: [u8; 4],
    byte: u8,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    clip: Option<(i32, i32, i32, i32)>,
) {
    let Some(rows) = seabios_glyph_8x8(byte) else {
        draw_framebuffer_glyph_rect(emu, color, x, y, width, height, clip);
        return;
    };
    let Some(glyph_clip) = glyph_bounds_clip(x, y, width, height, clip) else {
        return;
    };
    let scale_x = (width / 8).max(1);
    let scale_y = (height / 8).max(1);
    for (row_index, row_bits) in rows.iter().enumerate() {
        for col in 0..8 {
            if (row_bits & (1 << (7 - col))) == 0 {
                continue;
            }
            draw_framebuffer_glyph_rect(
                emu,
                color,
                x + col * scale_x,
                y + row_index as i32 * scale_y,
                scale_x,
                scale_y,
                Some(glyph_clip),
            );
        }
    }
}

fn create_gdi_screen_dc(emu: &mut Emulator, hwnd: u32) -> u32 {
    if emu
        .hle
        .window(hwnd)
        .is_some_and(|window| !window.visible)
    {
        return emu.hle.create_surface_dc(0);
    }
    if window_fully_obscured_by_higher_top_level(emu, hwnd) {
        return emu.hle.create_surface_dc(0);
    }
    if emu
        .hle
        .window(hwnd)
        .map(|window| window.ddraw_owned)
        .unwrap_or(false)
    {
        return emu.hle.create_surface_dc(0);
    }
    let surf = ensure_gdi_screen_surface(emu);
    let hdc = emu.hle.create_surface_dc(surf);
    let (origin_x, origin_y, _, _) = window_client_area(emu, hwnd);
    if let Some(dc) = emu.hle.gdi_dcs.get_mut(&hdc) {
        dc.hwnd = hwnd;
        dc.origin_x = origin_x;
        dc.origin_y = origin_y;
    }
    hdc
}

fn present_and_drop_gdi_dc(emu: &mut Emulator, hdc: u32) {
    let dc = emu.hle.gdi_dcs.remove(&hdc);
    emu.hle.gdi_dc_saves.remove(&hdc);
    if let Some(dc) = dc {
        if dc.surface != 0 {
            let surface = read_surface_info(emu, dc.surface).hle();
            finish_primary_gdi_update(emu, surface).hle();
        }
    }
}
