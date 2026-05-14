// HRESULT DirectDrawCreate(GUID *guid, LPDIRECTDRAW *out, IUnknown *outer)
// Create a fake DirectDraw COM object and vtables.
fn hle_direct_draw_create(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.ensure_ddraw_tables(&mut emu.memory).hle();
    let out = arg(emu, 1);
    let obj = create_ddraw_object(emu).hle();
    if out != 0 {
        emu.memory.write_u32(out, obj).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectDraw::SetCooperativeLevel(this, HWND hwnd, DWORD flags)
// Mark the associated top-level window as DirectDraw-owned.
fn hle_ddraw_set_cooperative_level(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hwnd = arg(emu, 1);
    let marked = emu.hle.mark_window_ddraw_owned(hwnd);
    trace_ddraw!("ddraw SetCooperativeLevel hwnd={hwnd:08x} marked={marked}");
    if marked {
        emu.hle.mark_hle_windows_dirty();
        remove_pending_paint_message(emu, if hwnd == 0 { 0x0002_0001 } else { hwnd });
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectDraw::EnumDisplayModes(this, DWORD flags, DDSURFACEDESC *filter, void *ctx, LPDDENUMMODESCALLBACK cb)
// Deliver compatible legacy fullscreen modes through the guest callback.
fn hle_ddraw_enum_display_modes(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let _flags = arg(emu, 1);
    let filter = arg(emu, 2);
    let context = arg(emu, 3);
    let callback = arg(emu, 4);
    if callback == 0 {
        ret(emu, 0);
        return HleResult::Retn(20);
    }

    let hle_esp = emu.cpu.reg(Reg::Esp);
    let original_ret = emu.memory.read_u32(hle_esp).hle();
    emu.hle.ddraw_enum_modes = Some(DDrawEnumModesState {
        callback,
        context,
        original_ret,
        callback_esp: hle_esp.wrapping_add(8),
        final_esp: hle_esp.wrapping_add(24),
        modes: display_modes_for_filter(emu, filter),
        next_mode: 0,
    });
    if !dispatch_next_display_mode(emu, entry.addr) {
        finish_display_mode_enum(emu);
    }
    HleResult::Retn(20)
}

// HRESULT __ddraw_enum_modes_continue(void)
// Continue or finish a DirectDraw mode-enumeration callback chain.
fn hle_ddraw_enum_modes_continue(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    let keep_going = emu.cpu.reg(Reg::Eax) == 1;
    if keep_going && dispatch_next_display_mode(emu, entry.addr) {
        return HleResult::Retn(0);
    }
    finish_display_mode_enum(emu);
    HleResult::Retn(0)
}

// HRESULT IDirectDraw::CreateSurface(this, DDSURFACEDESC *desc, IDirectDrawSurface **out, IUnknown *outer)
// Allocate a fake surface object and guest-visible pixel buffer.
fn hle_ddraw_create_surface(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let desc = arg(emu, 1);
    let out = arg(emu, 2);
    let surf = create_surface(emu, desc).hle();
    trace_ddraw!("ddraw IDirectDraw::CreateSurface desc={desc:08x} out={out:08x} surf={surf:08x}");
    if out != 0 {
        emu.memory.write_u32(out, surf).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectDraw::CreatePalette(this, DWORD flags, PALETTEENTRY *entries, IDirectDrawPalette **out, IUnknown *outer)
// Copy palette entries into a fake DirectDraw palette object.
fn hle_ddraw_create_palette(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 1);
    let entries = arg(emu, 2);
    let out = arg(emu, 3);
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 20, PagePerm::READ | PagePerm::WRITE).hle();
    let copy = emu
        .hle
        .alloc_private(&mut emu.memory, 256 * 4, PagePerm::READ | PagePerm::WRITE).hle();
    if entries != 0 {
        let data = emu.memory.read_bytes(entries, 256 * 4).hle();
        emu.memory.write_bytes(copy, &data).hle();
    }
    emu.memory.write_u32(obj, emu.hle.ddraw_palette_vtable).hle();
    emu.memory.write_u32(obj + 4, 1).hle();
    emu.memory.write_u32(obj + 8, flags).hle();
    emu.memory.write_u32(obj + 12, 256).hle();
    emu.memory.write_u32(obj + 16, copy).hle();
    if out != 0 {
        emu.memory.write_u32(out, obj).hle();
    }
    trace_ddraw!(
        "ddraw IDirectDraw::CreatePalette flags={flags:08x} entries={entries:08x} out={out:08x} pal={obj:08x} {}",
        palette_sample(emu, copy)
    );
    ret(emu, 0);
    HleResult::Retn(20)
}

// HRESULT IDirectDraw::CreateClipper(this, DWORD flags, IDirectDrawClipper **out, IUnknown *outer)
// Allocate a fake DirectDraw clipper object.
fn hle_ddraw_create_clipper(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 2);
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 12, PagePerm::READ | PagePerm::WRITE).hle();
    emu.memory.write_u32(obj, emu.hle.ddraw_clipper_vtable).hle();
    emu.memory.write_u32(obj + 4, 1).hle();
    emu.memory.write_u32(obj + 8, 0).hle();
    if out != 0 {
        emu.memory.write_u32(out, obj).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectDraw::GetDisplayMode(this, DDSURFACEDESC *desc)
// Fill the current fake display mode descriptor.
fn hle_ddraw_get_display_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let desc = arg(emu, 1);
    if desc != 0 {
        let surf = create_surface(emu, 0).hle();
        fill_surface_desc(emu, surf, desc).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDraw::SetDisplayMode(this, DWORD w, DWORD h, DWORD bpp)
// Resize the live framebuffer and update fake display pixel depth.
fn hle_ddraw_set_display_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let width = arg(emu, 1).max(1);
    let height = arg(emu, 2).max(1);
    let bpp = arg(emu, 3).max(8);
    trace_ddraw!("ddraw SetDisplayMode {width}x{height}x{bpp}");
    emu.hle.ddraw_width = width;
    emu.hle.ddraw_height = height;
    emu.hle.ddraw_bpp = bpp;
    emu.backend.resize(width, height).hle();
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectDraw::RestoreDisplayMode(this)
// Restore the startup display size and backing framebuffer.
fn hle_ddraw_restore_display_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.ddraw_width = 640;
    emu.hle.ddraw_height = 480;
    emu.hle.ddraw_bpp = 16;
    emu.backend.resize(640, 480).hle();
    ret(emu, 0);
    HleResult::Retn(4)
}

// HRESULT IDirectDrawSurface::GetSurfaceDesc(this, DDSURFACEDESC *desc)
// Fill a DirectDraw surface descriptor from fake surface metadata.
fn hle_surface_get_desc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let desc = arg(emu, 1);
    if desc != 0 {
        fill_surface_desc(emu, surf, desc).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::GetDC(IDirectDrawSurface *this, HDC *out)
// Return a fake HDC tied to this surface so GDI text HLE can raster into it.
fn hle_surface_get_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let out = arg(emu, 1);
    let hdc = emu.hle.create_surface_dc(surf);
    if out != 0 {
        emu.memory.write_u32(out, hdc).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::ReleaseDC(IDirectDrawSurface *this, HDC hdc)
// Release the fake HDC; pixels already live in the DirectDraw surface buffer.
fn hle_surface_release_dc(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 1);
    emu.hle.gdi_dcs.remove(&hdc);
    emu.hle.gdi_dc_saves.remove(&hdc);
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::Lock(this, RECT *rect, DDSURFACEDESC *desc, DWORD flags, HANDLE event)
// Return the surface descriptor with the guest-visible pixel pointer.
fn hle_surface_lock(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let rect = arg(emu, 1);
    let desc = arg(emu, 2);
    let lock_rect = if rect != 0 {
        Some(read_rect(&emu.memory, rect).hle())
    } else {
        None
    };
    if desc != 0 {
        fill_surface_lock_desc(emu, surf, desc, lock_rect).hle();
    }
    if hle_trace_enabled(HLE_TRACE_DDRAW) {
        let surface = read_surface_info(emu, surf).hle();
        let buffer = surface_lock_ptr(surface, lock_rect);
        trace_ddraw!(
            "ddraw DDS::Lock surf={surf:08x} rect={rect:08x} desc={desc:08x} {}x{}x{} pitch={} buffer={buffer:08x}",
            surface.width, surface.height, surface.bpp, surface.pitch
        );
    }
    ret(emu, 0);
    HleResult::Retn(20)
}

// HRESULT IDirectDrawSurface::Unlock(this, void *ptr)
// Present primary surfaces after guest drawing is complete.
fn hle_surface_unlock(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let surface = read_surface_info(emu, surf).hle();
    trace_ddraw!(
        "ddraw DDS::Unlock surf={surf:08x} primary={} {}x{}x{}",
        (surface.caps & DDSCAPS_PRIMARYSURFACE) != 0,
        surface.width,
        surface.height,
        surface.bpp
    );
    present_surface_if_primary(emu, surface).hle();
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::Flip(this, IDirectDrawSurface *target, DWORD flags)
// Copy the attached back buffer into the primary surface.
fn hle_surface_flip(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let surface = read_surface_info(emu, surf).hle();
    let attached = emu.memory.read_u32(surf + 28).unwrap_or(0);
    trace_ddraw!(
        "ddraw DDS::Flip surf={surf:08x} attached={attached:08x} primary={}",
        (surface.caps & DDSCAPS_PRIMARYSURFACE) != 0
    );
    if attached != 0 {
        let src = read_surface_info(emu, attached).hle();
        copy_surface_rect(emu, surface, src, 0, 0, src.full_rect(), None).hle();
    }
    present_surface_if_primary(emu, surface).hle();
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectDrawSurface::Blt(this, RECT *dst, IDirectDrawSurface *src, RECT *src_rect, DWORD flags, DDBLTFX *fx)
// Fill or copy clipped surface rectangles with optional source color key.
fn hle_surface_blt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst_surf = arg(emu, 0);
    let dst_rect_ptr = arg(emu, 1);
    let src_surf = arg(emu, 2);
    let src_rect_ptr = arg(emu, 3);
    let flags = arg(emu, 4);
    let fx = arg(emu, 5);
    let dst = read_surface_info(emu, dst_surf).hle();
    let dst_rect = if dst_rect_ptr != 0 {
        read_rect(&emu.memory, dst_rect_ptr).hle()
    } else {
        dst.full_rect()
    };
    trace_ddraw!(
        "ddraw DDS::Blt dst={dst_surf:08x} primary={} src={src_surf:08x} flags={flags:08x} dst_rect={},{},{},{}",
        (dst.caps & DDSCAPS_PRIMARYSURFACE) != 0,
        dst_rect.left,
        dst_rect.top,
        dst_rect.right,
        dst_rect.bottom
    );

    if (flags & DDBLT_COLORFILL) != 0 || src_surf == 0 {
        let color = read_blt_fill_color(emu, fx);
        fill_surface_rect(emu, dst, dst_rect, color).hle();
        present_surface_if_primary(emu, dst).hle();
        ret(emu, 0);
        return HleResult::Retn(24);
    }

    let src = read_surface_info(emu, src_surf).hle();
    let mut src_rect = if src_rect_ptr != 0 {
        read_rect(&emu.memory, src_rect_ptr).hle()
    } else {
        src.full_rect()
    };
    if dst_rect_ptr != 0 {
        src_rect.right = src_rect.left + src_rect.width().min(dst_rect.width());
        src_rect.bottom = src_rect.top + src_rect.height().min(dst_rect.height());
    }
    let color_key = blt_src_color_key(emu, src, flags, fx);
    copy_surface_rect(
        emu,
        dst,
        src,
        dst_rect.left,
        dst_rect.top,
        src_rect,
        color_key,
    ).hle();
    present_surface_if_primary(emu, dst).hle();
    ret(emu, 0);
    HleResult::Retn(24)
}

// HRESULT IDirectDrawSurface::BltFast(this, DWORD x, DWORD y, IDirectDrawSurface *src, RECT *src_rect, DWORD flags)
// Copy a clipped source rectangle to a destination point.
fn hle_surface_blt_fast(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst_surf = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let src_surf = arg(emu, 3);
    let src_rect_ptr = arg(emu, 4);
    let flags = arg(emu, 5);
    if src_surf != 0 {
        let dst = read_surface_info(emu, dst_surf).hle();
        let src = read_surface_info(emu, src_surf).hle();
        let src_rect = if src_rect_ptr != 0 {
            read_rect(&emu.memory, src_rect_ptr).hle()
        } else {
            src.full_rect()
        };
        let color_key = if (flags & DDBLTFAST_SRCCOLORKEY) != 0 {
            if src.has_color_key {
                Some((src.color_key_low, src.color_key_high))
            } else {
                Some((0, 0))
            }
        } else {
            None
        };
        trace_ddraw!(
            "ddraw DDS::BltFast dst={dst_surf:08x} primary={} src={src_surf:08x} flags={flags:08x} dst={},{} src_rect={},{},{},{}",
            (dst.caps & DDSCAPS_PRIMARYSURFACE) != 0,
            x,
            y,
            src_rect.left,
            src_rect.top,
            src_rect.right,
            src_rect.bottom
        );
        copy_surface_rect(emu, dst, src, x, y, src_rect, color_key).hle();
        present_surface_if_primary(emu, dst).hle();
    }
    ret(emu, 0);
    HleResult::Retn(24)
}

// HRESULT IDirectDrawSurface::AddAttachedSurface(this, IDirectDrawSurface *attached)
// Remember one attached surface pointer.
fn hle_surface_add_attached(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let attached = arg(emu, 1);
    emu.memory.write_u32(surf + 28, attached).hle();
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::GetAttachedSurface(this, DDSCAPS *caps, IDirectDrawSurface **out)
// Return or lazily create the attached back buffer surface.
fn hle_surface_get_attached(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let caps_ptr = arg(emu, 1);
    let out = arg(emu, 2);
    if out != 0 {
        let attached = emu.memory.read_u32(surf + 28).hle();
        if attached == 0 {
            let requested_caps = if caps_ptr != 0 {
                emu.memory.read_u32(caps_ptr).unwrap_or(0)
            } else {
                0
            };
            let new_surf = create_attached_surface(emu, surf, requested_caps).hle();
            emu.memory.write_u32(surf + 28, new_surf).hle();
            emu.memory.write_u32(out, new_surf).hle();
            trace_ddraw!(
                "ddraw DDS::GetAttachedSurface surf={surf:08x} requested_caps={requested_caps:08x} created={new_surf:08x}"
            );
        } else {
            emu.memory.write_u32(out, attached).hle();
            trace_ddraw!("ddraw DDS::GetAttachedSurface surf={surf:08x} attached={attached:08x}");
        }
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectDrawSurface::GetCaps(this, DDSCAPS *out)
// Write stored surface capability flags.
fn hle_surface_get_caps(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, emu.memory.read_u32(surf + 24).hle()).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::GetColorKey(this, DWORD flags, DDCOLORKEY *out)
// Return the stored source color key range.
fn hle_surface_get_color_key(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory
            .write_u32(out, emu.memory.read_u32(surf + 40).unwrap_or(0)).hle();
        emu.memory
            .write_u32(out + 4, emu.memory.read_u32(surf + 44).unwrap_or(0)).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectDrawSurface::GetPalette(this, IDirectDrawPalette **out)
// Return the surface palette pointer.
fn hle_surface_get_palette(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let out = arg(emu, 1);
    let pal = emu.memory.read_u32(surf + 32).hle();
    if out != 0 {
        emu.memory.write_u32(out, pal).hle();
    }
    trace_ddraw!("ddraw DDS::GetPalette surf={surf:08x} out={out:08x} pal={pal:08x}");
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::SetPalette(this, IDirectDrawPalette *palette)
// Store the surface palette pointer.
fn hle_surface_set_palette(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let pal = arg(emu, 1);
    emu.memory.write_u32(surf + 32, pal).hle();
    trace_ddraw!("ddraw DDS::SetPalette surf={surf:08x} pal={pal:08x}");
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawSurface::SetColorKey(this, DWORD flags, DDCOLORKEY *key)
// Store the source color key range.
fn hle_surface_set_color_key(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let key = arg(emu, 2);
    if key != 0 {
        emu.memory.write_u32(surf + 40, emu.memory.read_u32(key).hle()).hle();
        emu.memory
            .write_u32(surf + 44, emu.memory.read_u32(key + 4).hle()).hle();
        emu.memory.write_u32(surf + 48, 1).hle();
    } else {
        emu.memory.write_u32(surf + 48, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectDrawSurface::GetPixelFormat(this, DDPIXELFORMAT *out)
// Fill a DirectDraw pixel format from fake surface depth.
fn hle_surface_get_pixel_format(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let surf = arg(emu, 0);
    let out = arg(emu, 1);
    if out != 0 {
        let bpp = emu.memory.read_u32(surf + 16).hle();
        write_pixel_format(&mut emu.memory, out, bpp).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectDrawPalette::GetEntries(this, DWORD flags, DWORD base, DWORD count, PALETTEENTRY *out)
// Copy fake palette entries to guest memory.
fn hle_palette_get_entries(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let pal = arg(emu, 0);
    let start = arg(emu, 2);
    let count = arg(emu, 3);
    let out = arg(emu, 4);
    let entries = emu.memory.read_u32(pal + 16).hle();
    if out != 0 && entries != 0 {
        let data = emu
            .memory
            .read_bytes(entries + start * 4, (count * 4) as usize).hle();
        emu.memory.write_bytes(out, &data).hle();
    }
    ret(emu, 0);
    HleResult::Retn(20)
}

// HRESULT IDirectDrawPalette::SetEntries(this, DWORD flags, DWORD base, DWORD count, PALETTEENTRY *in)
// Copy guest palette entries into fake palette memory.
fn hle_palette_set_entries(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let pal = arg(emu, 0);
    let start = arg(emu, 2);
    let count = arg(emu, 3);
    let src = arg(emu, 4);
    let entries = emu.memory.read_u32(pal + 16).hle();
    if src != 0 && entries != 0 {
        let data = emu.memory.read_bytes(src, (count * 4) as usize).hle();
        emu.memory.write_bytes(entries + start * 4, &data).hle();
        trace_ddraw!(
            "ddraw DDP::SetEntries pal={pal:08x} start={start} count={count} src={src:08x} {}",
            palette_sample(emu, entries)
        );
    }
    ret(emu, 0);
    HleResult::Retn(20)
}

// HRESULT IDirectDrawClipper::SetHWnd(this, DWORD flags, HWND hwnd)
// Store and mark the window associated with the fake clipper.
fn hle_clipper_set_hwnd(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let clip = arg(emu, 0);
    let hwnd = arg(emu, 2);
    emu.memory.write_u32(clip + 8, hwnd).hle();
    if emu.hle.mark_window_ddraw_owned(hwnd) {
        emu.hle.mark_hle_windows_dirty();
        remove_pending_paint_message(emu, if hwnd == 0 { 0x0002_0001 } else { hwnd });
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HBITMAP CreateDIBSection(HDC hdc, BITMAPINFO *info, UINT usage, void **bits, HANDLE section, DWORD offset)
// Allocate a tracked bitmap and expose its DirectDraw-backed pixel buffer.
fn hle_create_dib_section(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let info = arg(emu, 1);
    let bits_out = arg(emu, 3);
    if info == 0 {
        ret(emu, 0);
        return HleResult::Retn(24);
    }
    let width = (emu.memory.read_u32(info + 4).hle() as i32).max(1) as u32;
    let raw_height = emu.memory.read_u32(info + 8).hle() as i32;
    let height = raw_height.unsigned_abs().max(1);
    let planes = emu.memory.read_u16(info + 12).unwrap_or(1).max(1) as u32;
    let bit_count = emu.memory.read_u16(info + 14).unwrap_or(32).max(1) as u32;
    let bpp = planes.saturating_mul(bit_count).clamp(8, 32);
    let surface = create_gdi_surface_with_format(emu, width, height, bpp).hle();
    let buffer = read_surface_info(emu, surface).map(|surface| surface.buffer).unwrap_or(0);
    if bits_out != 0 {
        emu.memory.write_u32(bits_out, buffer).hle();
    }
    let handle = emu.hle.create_gdi_bitmap(surface);
    ret(emu, handle);
    HleResult::Retn(24)
}

// UINT SetPaletteEntries(HPALETTE palette, UINT start, UINT count, const PALETTEENTRY *entries)
// Update tracked palette entries and report the copied count.
fn hle_set_palette_entries(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let palette = arg(emu, 0);
    let start = arg(emu, 1) as usize;
    let count = arg(emu, 2).min(1024) as usize;
    let entries = arg(emu, 3);
    let Some(palette) = emu.hle.gdi_palettes.get_mut(&palette) else {
        ret(emu, 0);
        return HleResult::Retn(16);
    };
    if palette.entries.len() < start.saturating_add(count) {
        palette.entries.resize(start.saturating_add(count), [0, 0, 0, 0]);
    }
    for index in 0..count {
        let src = entries.wrapping_add((index * 4) as u32);
        palette.entries[start + index] = [
            emu.memory.read_u8(src).hle(),
            emu.memory.read_u8(src + 1).hle(),
            emu.memory.read_u8(src + 2).hle(),
            emu.memory.read_u8(src + 3).hle(),
        ];
    }
    ret(emu, count as u32);
    HleResult::Retn(16)
}

// UINT GetPaletteEntries(HPALETTE palette, UINT start, UINT count, PALETTEENTRY *entries)
// Copy tracked palette entries to guest memory and report the copied count.
fn hle_get_palette_entries(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let palette = arg(emu, 0);
    let start = arg(emu, 1) as usize;
    let count = arg(emu, 2).min(1024) as usize;
    let out = arg(emu, 3);
    let Some(palette) = emu.hle.gdi_palettes.get(&palette) else {
        ret(emu, 0);
        return HleResult::Retn(16);
    };
    let available = palette.entries.len().saturating_sub(start).min(count);
    if out != 0 {
        for index in 0..available {
            let dst = out.wrapping_add((index * 4) as u32);
            emu.memory
                .write_bytes(dst, &palette.entries[start + index])
                .hle();
        }
    }
    ret(emu, available as u32);
    HleResult::Retn(16)
}

// UINT GetSystemPaletteEntries(HDC hdc, UINT start, UINT count, PALETTEENTRY *entries)
// Return a grayscale system palette for 8-bit-era palette probes.
fn hle_get_system_palette_entries(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let start = arg(emu, 1);
    let count = arg(emu, 2).min(256);
    let out = arg(emu, 3);
    if out != 0 {
        for index in 0..count {
            let value = start.wrapping_add(index) as u8;
            let dst = out.wrapping_add(index * 4);
            emu.memory
                .write_bytes(dst, &[value, value, value, 0])
                .hle();
        }
    }
    ret(emu, count);
    HleResult::Retn(16)
}

// UINT GetNearestPaletteIndex(HPALETTE palette, COLORREF color)
// Return the first entry for coarse palette matching.
fn hle_get_nearest_palette_index(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// BOOL TextOutA(HDC hdc, int x, int y, LPCSTR text, int count)
// Draw clipped bitmap glyphs at the requested position on a DirectDraw-backed DC.
fn hle_text_out_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let hdc = arg(emu, 0);
    let x = arg(emu, 1) as i32;
    let y = arg(emu, 2) as i32;
    let text = arg(emu, 3);
    let count = arg(emu, 4);
    let bytes = gdi_text_bytes(emu, text, count);
    let dc = gdi_dc_or_default(emu, hdc);
    let layout = gdi_text_layout(emu, dc, &bytes, false);
    draw_gdi_text(emu, dc, &bytes, x, y, layout.metrics, None);
    update_text_current_pos(emu, hdc, dc, &layout);
    ret(emu, 1);
    HleResult::Retn(20)
}

fn surface_bytes_per_pixel(bpp: u32) -> u32 {
    match bpp {
        0..=8 => 1,
        9..=16 => 2,
        17..=24 => 3,
        _ => 4,
    }
}

fn display_width(emu: &Emulator) -> u32 {
    emu.hle.ddraw_width.max(1)
}

fn display_height(emu: &Emulator) -> u32 {
    emu.hle.ddraw_height.max(1)
}

fn display_bpp(emu: &Emulator) -> u32 {
    emu.hle.ddraw_bpp.max(8)
}

fn desc_bpp(emu: &Emulator, desc: u32, flags: u32) -> u32 {
    if desc != 0 && (flags & DDSD_PIXELFORMAT) != 0 {
        let ddpf_flags = emu.memory.read_u32(desc + 76).unwrap_or(0);
        let bit_count = emu.memory.read_u32(desc + 84).unwrap_or(0);
        if (ddpf_flags & DDPF_PALETTEINDEXED8) != 0 {
            return 8;
        }
        if (ddpf_flags & DDPF_RGB) != 0 && bit_count != 0 {
            return bit_count;
        }
    }
    display_bpp(emu)
}

fn create_ddraw_object(emu: &mut Emulator) -> Result<u32> {
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 20, PagePerm::READ | PagePerm::WRITE)?;
    emu.memory.write_u32(obj, emu.hle.ddraw_vtable)?;
    emu.memory.write_u32(obj + 4, 1)?;
    emu.memory.write_u32(obj + 8, display_width(emu))?;
    emu.memory.write_u32(obj + 12, display_height(emu))?;
    emu.memory.write_u32(obj + 16, display_bpp(emu))?;
    Ok(obj)
}

fn create_surface(emu: &mut Emulator, desc: u32) -> Result<u32> {
    let flags = if desc != 0 {
        emu.memory.read_u32(desc + 4)?
    } else {
        0
    };
    let height = if desc != 0 && (flags & DDSD_HEIGHT) != 0 {
        emu.memory.read_u32(desc + 8)?
    } else {
        display_height(emu)
    };
    let width = if desc != 0 && (flags & DDSD_WIDTH) != 0 {
        emu.memory.read_u32(desc + 12)?
    } else {
        display_width(emu)
    };
    let caps = if desc != 0 {
        emu.memory.read_u32(desc + 104).unwrap_or(0x200)
    } else {
        0x200
    };
    let bpp = desc_bpp(emu, desc, flags);
    trace_ddraw!(
        "ddraw CreateSurface desc={desc:08x} flags={flags:08x} caps={caps:08x} {width}x{height}x{bpp}"
    );
    create_surface_with_format(emu, width, height, bpp, caps)
}

fn create_attached_surface(emu: &mut Emulator, parent: u32, requested_caps: u32) -> Result<u32> {
    let parent = read_surface_info(emu, parent)?;
    let caps = requested_caps & !DDSCAPS_PRIMARYSURFACE;
    let caps = if caps == 0 { DDSCAPS_BACKBUFFER } else { caps };
    create_surface_with_format(emu, parent.width, parent.height, parent.bpp, caps)
}

fn create_surface_with_format(
    emu: &mut Emulator,
    width: u32,
    height: u32,
    bpp: u32,
    caps: u32,
) -> Result<u32> {
    create_surface_with_format_and_guard(emu, width, height, bpp, caps, SURFACE_GUARD_SIZE)
}

fn create_surface_with_format_and_guard(
    emu: &mut Emulator,
    width: u32,
    height: u32,
    bpp: u32,
    caps: u32,
    guard_after: u32,
) -> Result<u32> {
    create_surface_with_format_and_guards(emu, width, height, bpp, caps, PAGE_SIZE, guard_after)
}

fn create_surface_with_format_and_guards(
    emu: &mut Emulator,
    width: u32,
    height: u32,
    bpp: u32,
    caps: u32,
    guard_before: u32,
    guard_after: u32,
) -> Result<u32> {
    emu.hle.ensure_ddraw_tables(&mut emu.memory)?;
    let pitch = width * surface_bytes_per_pixel(bpp);
    let buf_size = pitch.max(1) * height.max(1);
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 52, PagePerm::READ | PagePerm::WRITE)?;
    // Games can legally keep DirectDraw/DIBSection pixel pointers and draw
    // through them, so normal surfaces keep guard pages. Device-dependent
    // HBITMAPs are addressed through GDI handles; keeping those compact avoids
    // exhausting guest heap on bitmap-heavy card games.
    let buffer = if guard_before == 0 && guard_after == 0 {
        emu.hle
            .alloc_compact(&mut emu.memory, buf_size, PagePerm::READ | PagePerm::WRITE)?
    } else {
        emu.hle.alloc_with_guards(
            &mut emu.memory,
            buf_size,
            PagePerm::READ | PagePerm::WRITE,
            guard_before,
            guard_after,
        )?
    };
    emu.memory.write_u32(obj, emu.hle.ddraw_surface_vtable)?;
    emu.memory.write_u32(obj + 4, 1)?;
    emu.memory.write_u32(obj + 8, width)?;
    emu.memory.write_u32(obj + 12, height)?;
    emu.memory.write_u32(obj + 16, bpp)?;
    emu.memory.write_u32(obj + 20, pitch)?;
    emu.memory.write_u32(obj + 24, caps)?;
    emu.memory.write_u32(obj + 28, 0)?;
    emu.memory.write_u32(obj + 32, 0)?;
    emu.memory.write_u32(obj + 36, buffer)?;
    emu.memory.write_u32(obj + 40, 0)?;
    emu.memory.write_u32(obj + 44, 0)?;
    emu.memory.write_u32(obj + 48, 0)?;
    trace_ddraw!(
        "ddraw CreateSurface -> surf={obj:08x} buffer={buffer:08x} size={buf_size:x} guard_before={guard_before:x} guard_after={guard_after:x}"
    );
    Ok(obj)
}

fn fill_surface_desc(emu: &mut Emulator, surf: u32, desc: u32) -> Result<()> {
    let width = emu.memory.read_u32(surf + 8)?;
    let height = emu.memory.read_u32(surf + 12)?;
    let bpp = emu.memory.read_u32(surf + 16)?;
    let pitch = emu.memory.read_u32(surf + 20)?;
    let caps = emu.memory.read_u32(surf + 24)?;
    let buffer = emu.memory.read_u32(surf + 36)?;
    for i in 0..108u32 {
        emu.memory.write_u8(desc + i, 0)?;
    }
    emu.memory.write_u32(desc, 108)?;
    emu.memory
        .write_u32(desc + 4, DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PITCH | DDSD_LPSURFACE | DDSD_PIXELFORMAT)?;
    emu.memory.write_u32(desc + 8, height)?;
    emu.memory.write_u32(desc + 12, width)?;
    emu.memory.write_u32(desc + 16, pitch)?;
    emu.memory.write_u32(desc + 36, buffer)?;
    write_pixel_format(&mut emu.memory, desc + 72, bpp)?;
    emu.memory.write_u32(desc + 104, caps)?;
    Ok(())
}

fn fill_surface_lock_desc(
    emu: &mut Emulator,
    surf: u32,
    desc: u32,
    lock_rect: Option<RectI>,
) -> Result<()> {
    fill_surface_desc(emu, surf, desc)?;
    let surface = read_surface_info(emu, surf)?;
    emu.memory
        .write_u32(desc + 36, surface_lock_ptr(surface, lock_rect))?;
    Ok(())
}

fn surface_lock_ptr(surface: SurfaceInfo, lock_rect: Option<RectI>) -> u32 {
    let Some(rect) = lock_rect else {
        return surface.buffer;
    };
    // DirectDraw partial locks return a pointer to the locked rectangle.
    // lPitch still remains the full-surface stride for row-to-row writes.
    let left = rect.left.max(0).min(surface.width as i32) as u32;
    let top = rect.top.max(0).min(surface.height as i32) as u32;
    surface
        .buffer
        .wrapping_add(top.wrapping_mul(surface.pitch))
        .wrapping_add(left.wrapping_mul(surface.bytes_per_pixel()))
}

fn fill_display_mode_desc(
    emu: &mut Emulator,
    desc: u32,
    width: u32,
    height: u32,
    bpp: u32,
) -> Result<()> {
    for i in 0..108u32 {
        emu.memory.write_u8(desc + i, 0)?;
    }
    emu.memory.write_u32(desc, 108)?;
    emu.memory
        .write_u32(desc + 4, DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PITCH | DDSD_PIXELFORMAT)?;
    emu.memory.write_u32(desc + 8, height)?;
    emu.memory.write_u32(desc + 12, width)?;
    emu.memory
        .write_u32(desc + 16, width * surface_bytes_per_pixel(bpp))?;
    write_pixel_format(&mut emu.memory, desc + 72, bpp)?;
    emu.memory.write_u32(desc + 104, 0)?;
    Ok(())
}

fn write_pixel_format(mem: &mut Memory, out: u32, bpp: u32) -> Result<()> {
    let (flags, red, green, blue) = match bpp {
        8 => (DDPF_PALETTEINDEXED8, 0, 0, 0),
        15 => (DDPF_RGB, 0x7c00, 0x03e0, 0x001f),
        16 => (DDPF_RGB, 0xf800, 0x07e0, 0x001f),
        24 | 32 => (DDPF_RGB, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff),
        _ => (DDPF_RGB, 0xf800, 0x07e0, 0x001f),
    };
    mem.write_u32(out, 32)?;
    mem.write_u32(out + 4, flags)?;
    mem.write_u32(out + 8, 0)?;
    mem.write_u32(out + 12, bpp)?;
    mem.write_u32(out + 16, red)?;
    mem.write_u32(out + 20, green)?;
    mem.write_u32(out + 24, blue)?;
    mem.write_u32(out + 28, 0)?;
    Ok(())
}

fn read_rect(mem: &Memory, ptr: u32) -> Result<RectI> {
    Ok(RectI {
        left: mem.read_u32(ptr)? as i32,
        top: mem.read_u32(ptr + 4)? as i32,
        right: mem.read_u32(ptr + 8)? as i32,
        bottom: mem.read_u32(ptr + 12)? as i32,
    })
}

fn read_surface_info(emu: &Emulator, surf: u32) -> Result<SurfaceInfo> {
    Ok(SurfaceInfo {
        obj: surf,
        width: emu.memory.read_u32(surf + 8)?,
        height: emu.memory.read_u32(surf + 12)?,
        bpp: emu.memory.read_u32(surf + 16)?,
        pitch: emu.memory.read_u32(surf + 20)?,
        caps: emu.memory.read_u32(surf + 24)?,
        palette: emu.memory.read_u32(surf + 32).unwrap_or(0),
        buffer: emu.memory.read_u32(surf + 36)?,
        color_key_low: emu.memory.read_u32(surf + 40).unwrap_or(0),
        color_key_high: emu.memory.read_u32(surf + 44).unwrap_or(0),
        has_color_key: emu.memory.read_u32(surf + 48).unwrap_or(0) != 0,
    })
}

fn present_surface_if_primary(emu: &mut Emulator, surface: SurfaceInfo) -> Result<()> {
    if (surface.caps & DDSCAPS_PRIMARYSURFACE) == 0 {
        return Ok(());
    }
    trace_ddraw!(
        "ddraw present primary surf={:08x} {}x{}x{} pitch={}",
        surface.obj,
        surface.width,
        surface.height,
        surface.bpp,
        surface.pitch
    );
    let is_gdi_screen_surface =
        emu.hle.gdi_screen_surface != 0 && surface.obj == emu.hle.gdi_screen_surface;
    let len = surface.pitch.checked_mul(surface.height).ok_or_else(|| {
        Error::Hle(format!(
            "surface present overflow surf={:08x} pitch={} height={}",
            surface.obj, surface.pitch, surface.height
        ))
    })?;
    if surface.bpp == 8 {
        present_indexed8_surface(emu, surface, is_gdi_screen_surface)?;
    } else {
        if is_gdi_screen_surface {
            redraw_hle_windows_on_surface_if_dirty(emu, surface, surface.obj);
        }
        let bytes = emu.memory.read_bytes(surface.buffer, len as usize)?;
        emu.backend.present_bgra(
            &bytes,
            surface.width,
            surface.height,
            surface.pitch,
            surface.bpp,
        )?;
        if is_gdi_screen_surface && draw_menu_overlays_on_framebuffer(emu) {
            emu.backend.present()?;
        }
    }
    if is_gdi_screen_surface {
        emu.hle.clear_gdi_present_pending();
    }
    emu.note_present();
    Ok(())
}

fn mark_gdi_screen_surface_dirty(emu: &mut Emulator, surface: SurfaceInfo) -> bool {
    if emu.hle.gdi_screen_surface != 0
        && surface.obj == emu.hle.gdi_screen_surface
        && (surface.caps & DDSCAPS_PRIMARYSURFACE) != 0
    {
        emu.hle.mark_gdi_present_pending();
        true
    } else {
        false
    }
}

fn finish_primary_gdi_update(emu: &mut Emulator, surface: SurfaceInfo) -> Result<()> {
    if mark_gdi_screen_surface_dirty(emu, surface) {
        Ok(())
    } else {
        present_surface_if_primary(emu, surface)
    }
}

pub fn flush_gdi_present_if_pending(emu: &mut Emulator) -> Result<bool> {
    // Pure GDI games often repaint a logical update through many tiny BitBlt
    // calls. Presenting each one leaks half-painted states; flush when USER
    // yields/waits, with run_one_frame timeout as the busy-loop fallback.
    if !emu.hle.take_gdi_present_pending() {
        return Ok(false);
    }
    let surf = emu.hle.gdi_screen_surface;
    if surf == 0 {
        return Ok(false);
    }
    let surface = read_surface_info(emu, surf)?;
    present_surface_if_primary(emu, surface)?;
    Ok(true)
}

fn read_palette_rgba(emu: &Emulator, palette_obj: u32) -> [[u8; 4]; 256] {
    let mut palette = [[0, 0, 0, 255]; 256];
    for (i, entry) in palette.iter_mut().enumerate() {
        let v = i as u8;
        *entry = [v, v, v, 255];
    }

    if palette_obj == 0 {
        return palette;
    }
    let entries = emu.memory.read_u32(palette_obj + 16).unwrap_or(0);
    if entries == 0 {
        return palette;
    }
    if let Ok(bytes) = emu.memory.read_bytes(entries, 256 * 4) {
        for (i, entry) in palette.iter_mut().enumerate() {
            let off = i * 4;
            *entry = [bytes[off], bytes[off + 1], bytes[off + 2], 255];
        }
    }
    palette
}

fn palette_sample(emu: &Emulator, entries: u32) -> String {
    if entries == 0 {
        return "entries=00000000".to_string();
    }
    let mut out = format!("entries={entries:08x}");
    for index in [0u32, 1, 2, 15, 64, 128, 255] {
        if let Ok(bytes) = emu.memory.read_bytes(entries + index * 4, 4) {
            out.push_str(&format!(
                " p{index}=({},{},{},{})",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ));
        }
    }
    out
}

fn present_indexed8_surface(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    allow_hle_overlay: bool,
) -> Result<()> {
    let palette = read_palette_rgba(emu, surface.palette);
    trace_ddraw!(
        "ddraw present indexed8 surf={:08x} pal={:08x} {}",
        surface.obj,
        surface.palette,
        if surface.palette == 0 {
            "fallback_palette".to_string()
        } else {
            let entries = emu.memory.read_u32(surface.palette + 16).unwrap_or(0);
            palette_sample(emu, entries)
        }
    );
    let copy_w = emu.backend.width().min(surface.width) as usize;
    let copy_h = emu.backend.height().min(surface.height) as usize;
    for y in 0..copy_h {
        let src = surface.buffer + y as u32 * surface.pitch;
        let row = emu.memory.read_bytes(src, copy_w)?;
        let stride = emu.backend.width() as usize * 4;
        let dst = &mut emu.backend.framebuffer_mut()[y * stride..y * stride + copy_w * 4];
        for (x, index) in row.iter().enumerate() {
            dst[x * 4..x * 4 + 4].copy_from_slice(&palette[*index as usize]);
        }
    }
    if allow_hle_overlay {
        draw_menu_overlays_on_framebuffer(emu);
    }
    emu.backend.present()?;
    Ok(())
}

fn pixel_in_color_key(row: &[u8], off: usize, bytes_per_pixel: usize, low: u32, high: u32) -> bool {
    let value = match bytes_per_pixel {
        1 => row[off] as u32,
        2 => u16::from_le_bytes([row[off], row[off + 1]]) as u32,
        3 => row[off] as u32 | ((row[off + 1] as u32) << 8) | ((row[off + 2] as u32) << 16),
        _ => u32::from_le_bytes([row[off], row[off + 1], row[off + 2], row[off + 3]]),
    };
    value >= low && value <= high
}

fn copy_surface_rect(
    emu: &mut Emulator,
    dst: SurfaceInfo,
    src: SurfaceInfo,
    mut dst_x: i32,
    mut dst_y: i32,
    mut src_rect: RectI,
    color_key: Option<(u32, u32)>,
) -> Result<()> {
    if dst.bytes_per_pixel() != src.bytes_per_pixel() {
        return Err(Error::Hle(format!(
            "surface blit bpp mismatch dst={} src={}",
            dst.bpp, src.bpp
        )));
    }

    let mut width = src_rect.width();
    let mut height = src_rect.height();
    if width <= 0 || height <= 0 {
        return Ok(());
    }

    if dst_x < 0 {
        let shift = -dst_x;
        src_rect.left += shift;
        width -= shift;
        dst_x = 0;
    }
    if dst_y < 0 {
        let shift = -dst_y;
        src_rect.top += shift;
        height -= shift;
        dst_y = 0;
    }
    if src_rect.left < 0 {
        let shift = -src_rect.left;
        dst_x += shift;
        width -= shift;
        src_rect.left = 0;
    }
    if src_rect.top < 0 {
        let shift = -src_rect.top;
        dst_y += shift;
        height -= shift;
        src_rect.top = 0;
    }

    width = width
        .min(dst.width as i32 - dst_x)
        .min(src.width as i32 - src_rect.left);
    height = height
        .min(dst.height as i32 - dst_y)
        .min(src.height as i32 - src_rect.top);
    if width <= 0 || height <= 0 {
        return Ok(());
    }

    let bpp = dst.bytes_per_pixel() as usize;
    let row_bytes = width as usize * bpp;
    let copy_bottom_up = dst.buffer == src.buffer && dst_y > src_rect.top;

    for row_idx in 0..height {
        let row = if copy_bottom_up {
            height - 1 - row_idx
        } else {
            row_idx
        };
        let src_addr = src
            .buffer
            .wrapping_add((src_rect.top + row) as u32 * src.pitch)
            .wrapping_add(src_rect.left as u32 * bpp as u32);
        let dst_addr = dst
            .buffer
            .wrapping_add((dst_y + row) as u32 * dst.pitch)
            .wrapping_add(dst_x as u32 * bpp as u32);
        let src_row = emu.memory.read_bytes(src_addr, row_bytes)?;
        if let Some((low, high)) = color_key {
            for px in 0..width as usize {
                let off = px * bpp;
                if !pixel_in_color_key(&src_row, off, bpp, low, high) {
                    emu.memory
                        .write_bytes(dst_addr.wrapping_add(off as u32), &src_row[off..off + bpp])?;
                }
            }
        } else {
            emu.memory.write_bytes(dst_addr, &src_row)?;
        }
    }
    Ok(())
}

fn fill_surface_rect(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    mut rect: RectI,
    color: u32,
) -> Result<()> {
    rect.left = rect.left.max(0).min(surface.width as i32);
    rect.top = rect.top.max(0).min(surface.height as i32);
    rect.right = rect.right.max(0).min(surface.width as i32);
    rect.bottom = rect.bottom.max(0).min(surface.height as i32);
    let width = rect.width();
    let height = rect.height();
    if width <= 0 || height <= 0 {
        return Ok(());
    }

    let bpp = surface.bytes_per_pixel() as usize;
    let color_bytes = color.to_le_bytes();
    let mut row = vec![0; width as usize * bpp];
    for px in row.chunks_mut(bpp) {
        px.copy_from_slice(&color_bytes[..bpp]);
    }
    for y in rect.top..rect.bottom {
        let addr = surface
            .buffer
            .wrapping_add(y as u32 * surface.pitch)
            .wrapping_add(rect.left as u32 * bpp as u32);
        emu.memory.write_bytes(addr, &row)?;
    }
    Ok(())
}

fn read_blt_fill_color(emu: &Emulator, fx: u32) -> u32 {
    if fx == 0 {
        return 0;
    }
    emu.memory
        .read_u32(fx + DDBLTFX_FILL_COLOR_OFFSET)
        .or_else(|_| emu.memory.read_u32(fx + 4))
        .unwrap_or(0)
}

fn blt_src_color_key(emu: &Emulator, src: SurfaceInfo, flags: u32, fx: u32) -> Option<(u32, u32)> {
    if (flags & DDBLT_KEYSRCOVERRIDE) != 0 && fx != 0 {
        let low = emu
            .memory
            .read_u32(fx + DDBLTFX_SRC_COLOR_KEY_OFFSET)
            .unwrap_or(src.color_key_low);
        let high = emu
            .memory
            .read_u32(fx + DDBLTFX_SRC_COLOR_KEY_OFFSET + 4)
            .unwrap_or(low);
        return Some((low, high));
    }
    if (flags & DDBLT_KEYSRC) != 0 {
        return if src.has_color_key {
            Some((src.color_key_low, src.color_key_high))
        } else {
            Some((0, 0))
        };
    }
    None
}

fn display_modes_for_filter(emu: &Emulator, filter: u32) -> Vec<(u32, u32, u32)> {
    let filter_flags = if filter != 0 {
        emu.memory.read_u32(filter + 4).unwrap_or(0)
    } else {
        0
    };
    let wanted_width = (filter != 0 && (filter_flags & DDSD_WIDTH) != 0)
        .then(|| emu.memory.read_u32(filter + 12).unwrap_or(0).max(1));
    let wanted_height = (filter != 0 && (filter_flags & DDSD_HEIGHT) != 0)
        .then(|| emu.memory.read_u32(filter + 8).unwrap_or(0).max(1));
    let wanted_bpp =
        (filter != 0 && (filter_flags & DDSD_PIXELFORMAT) != 0).then(|| desc_bpp(emu, filter, filter_flags));

    let mut modes = Vec::new();
    for (width, height, bpp) in [
        (640, 480, 8),
        (800, 600, 8),
        (1024, 768, 8),
        (1280, 1024, 8),
        (320, 200, 8),
        (320, 240, 8),
        (640, 480, 16),
        (800, 600, 16),
        (1024, 768, 16),
        (640, 480, 32),
        (800, 600, 32),
        (1024, 768, 32),
    ] {
        if wanted_width.is_some_and(|wanted| wanted != width)
            || wanted_height.is_some_and(|wanted| wanted != height)
            || wanted_bpp.is_some_and(|wanted| wanted != bpp)
        {
            continue;
        }
        modes.push((width, height, bpp));
    }
    if modes.is_empty() {
        modes.push((
            wanted_width.unwrap_or(640),
            wanted_height.unwrap_or(480),
            wanted_bpp.unwrap_or(8),
        ));
    }
    modes
}

fn dispatch_next_display_mode(emu: &mut Emulator, from_entry: u32) -> bool {
    let Some(state) = emu.hle.ddraw_enum_modes.as_mut() else {
        return false;
    };
    if state.next_mode >= state.modes.len() {
        return false;
    }
    let callback = state.callback;
    let context = state.context;
    let original_ret = state.original_ret;
    let callback_esp = state.callback_esp;
    let (width, height, bpp) = state.modes[state.next_mode];
    state.next_mode += 1;

    let desc = emu
        .hle
        .alloc_private(&mut emu.memory, 108, PagePerm::READ | PagePerm::WRITE)
        .hle();
    fill_display_mode_desc(emu, desc, width, height, bpp).hle();
    trace_ddraw!("ddraw EnumDisplayModes callback={callback:08x} mode={width}x{height}x{bpp}");

    let continue_thunk = emu.hle.ddraw_enum_continue_thunk;
    emu.memory.write_u32(callback_esp, continue_thunk).hle();
    emu.memory.write_u32(callback_esp + 4, desc).hle();
    emu.memory.write_u32(callback_esp + 8, context).hle();
    emu.memory.write_u32(callback_esp + 12, original_ret).hle();
    emu.cpu
        .debug_replace_top_call(
            from_entry,
            callback,
            continue_thunk,
            callback_esp + 4,
            callback_esp,
        )
        .hle();
    emu.cpu.set_reg(Reg::Esp, callback_esp);
    emu.cpu.eip = callback;
    true
}

fn finish_display_mode_enum(emu: &mut Emulator) {
    let Some(state) = emu.hle.ddraw_enum_modes.take() else {
        ret(emu, 0);
        return;
    };
    ret(emu, 0);
    emu.cpu.set_reg(Reg::Esp, state.final_esp);
    emu.cpu.eip = state.original_ret;
}

fn create_gdi_surface_with_format(
    emu: &mut Emulator,
    width: u32,
    height: u32,
    bpp: u32,
) -> Result<u32> {
    create_surface_with_format_and_guard(emu, width, height, bpp, 0, PAGE_SIZE)
}

fn create_gdi_bitmap_surface_with_format(
    emu: &mut Emulator,
    width: u32,
    height: u32,
    bpp: u32,
) -> Result<u32> {
    create_surface_with_format_and_guards(emu, width, height, bpp, 0, 0, 0)
}

fn free_surface_allocations(emu: &mut Emulator, surf: u32) -> Result<()> {
    if surf == 0 {
        return Ok(());
    }
    let buffer = emu.memory.read_u32(surf + 36).unwrap_or(0);
    if buffer != 0 {
        let _ = emu.hle.free_alloc(&mut emu.memory, buffer)?;
    }
    let _ = emu.hle.free_alloc(&mut emu.memory, surf)?;
    Ok(())
}

fn tile_surface_rect(emu: &mut Emulator, dst: SurfaceInfo, src: SurfaceInfo, mut rect: RectI) {
    rect.left = rect.left.max(0).min(dst.width as i32);
    rect.top = rect.top.max(0).min(dst.height as i32);
    rect.right = rect.right.max(0).min(dst.width as i32);
    rect.bottom = rect.bottom.max(0).min(dst.height as i32);
    if rect.right <= rect.left || rect.bottom <= rect.top || src.width == 0 || src.height == 0 {
        return;
    }
    for y in rect.top..rect.bottom {
        for x in rect.left..rect.right {
            let sx = (x - rect.left) as u32 % src.width;
            let sy = (y - rect.top) as u32 % src.height;
            if let Some(color) = read_surface_pixel_colorref(emu, src, sx as i32, sy as i32) {
                write_surface_pixel_colorref(emu, dst, x, y, color);
            }
        }
    }
}

fn read_surface_pixel_colorref(
    emu: &Emulator,
    surface: SurfaceInfo,
    x: i32,
    y: i32,
) -> Option<u32> {
    if x < 0 || y < 0 || x >= surface.width as i32 || y >= surface.height as i32 {
        return None;
    }
    let bpp = surface.bytes_per_pixel();
    let addr = surface
        .buffer
        .checked_add(y as u32 * surface.pitch)?
        .checked_add(x as u32 * bpp)?;
    match surface.bpp {
        0..=8 => Some(emu.memory.read_u8(addr).ok()? as u32 * 0x0001_0101),
        9..=16 => {
            let value = emu.memory.read_u16(addr).ok()? as u32;
            let r = ((value >> 11) & 0x1f) << 3;
            let g = ((value >> 5) & 0x3f) << 2;
            let b = (value & 0x1f) << 3;
            Some(r | (g << 8) | (b << 16))
        }
        17..=24 => {
            let b = emu.memory.read_u8(addr).ok()? as u32;
            let g = emu.memory.read_u8(addr + 1).ok()? as u32;
            let r = emu.memory.read_u8(addr + 2).ok()? as u32;
            Some(r | (g << 8) | (b << 16))
        }
        _ => {
            let b = emu.memory.read_u8(addr).ok()? as u32;
            let g = emu.memory.read_u8(addr + 1).ok()? as u32;
            let r = emu.memory.read_u8(addr + 2).ok()? as u32;
            Some(r | (g << 8) | (b << 16))
        }
    }
}

fn write_surface_pixel_colorref(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    x: i32,
    y: i32,
    colorref: u32,
) {
    if x < 0 || y < 0 || x >= surface.width as i32 || y >= surface.height as i32 {
        return;
    }
    let bpp = surface.bytes_per_pixel();
    let addr = surface
        .buffer
        .wrapping_add(y as u32 * surface.pitch)
        .wrapping_add(x as u32 * bpp);
    let bytes = gdi_pixel_bytes(colorref, surface.bpp);
    emu.memory
        .write_bytes(addr, &bytes[..bpp as usize])
        .hle();
}

fn create_surface_from_dib(emu: &mut Emulator, addr: u32, size: u32) -> Option<u32> {
    const BI_RGB: u32 = 0;
    const BI_RLE8: u32 = 1;
    const BI_RLE4: u32 = 2;

    let header_size = emu.memory.read_u32(addr).ok()?;
    if header_size == 12 {
        return create_surface_from_core_dib(emu, addr, size);
    }
    if header_size < 40 || header_size > size {
        return None;
    }

    let width = emu.memory.read_u32(addr + 4).ok()? as i32;
    let raw_height = emu.memory.read_u32(addr + 8).ok()? as i32;
    let planes = emu.memory.read_u16(addr + 12).ok()?;
    let bpp = emu.memory.read_u16(addr + 14).ok()? as u32;
    let compression = emu.memory.read_u32(addr + 16).ok()?;
    let image_size = emu.memory.read_u32(addr + 20).ok()?;
    let colors_used = emu.memory.read_u32(addr + 32).ok()?;
    if width <= 0 || raw_height == 0 || planes != 1 {
        return None;
    }

    let height = raw_height.unsigned_abs();
    let width = width as u32;
    let top_down = raw_height < 0;
    let palette_entries = dib_palette_entries(bpp, colors_used)?;
    let palette_addr = addr.checked_add(header_size)?;
    let bits_addr = palette_addr.checked_add(palette_entries.checked_mul(4)?)?;
    if bits_addr > addr.checked_add(size)? {
        return None;
    }
    let palette = read_dib_palette(emu, palette_addr, palette_entries, 4)?;
    let end_addr = addr.checked_add(size)?;
    let image_end = if image_size != 0 {
        bits_addr.checked_add(image_size).unwrap_or(end_addr).min(end_addr)
    } else {
        end_addr
    };
    match compression {
        BI_RGB => {
            create_surface_from_dib_bits(emu, bits_addr, end_addr, width, height, top_down, bpp, &palette)
        }
        BI_RLE8 if bpp == 8 => {
            create_surface_from_rle8_dib(emu, bits_addr, image_end, width, height, top_down, &palette)
        }
        BI_RLE4 if bpp == 4 => {
            create_surface_from_rle4_dib(emu, bits_addr, image_end, width, height, top_down, &palette)
        }
        _ => None,
    }
}

fn create_surface_from_core_dib(emu: &mut Emulator, addr: u32, size: u32) -> Option<u32> {
    let width = emu.memory.read_u16(addr + 4).ok()? as u32;
    let height = emu.memory.read_u16(addr + 6).ok()? as u32;
    let planes = emu.memory.read_u16(addr + 8).ok()?;
    let bpp = emu.memory.read_u16(addr + 10).ok()? as u32;
    if width == 0 || height == 0 || planes != 1 {
        return None;
    }
    let palette_entries = dib_palette_entries(bpp, 0)?;
    let palette_addr = addr + 12;
    let bits_addr = palette_addr.checked_add(palette_entries.checked_mul(3)?)?;
    if bits_addr > addr.checked_add(size)? {
        return None;
    }
    let palette = read_dib_palette(emu, palette_addr, palette_entries, 3)?;
    create_surface_from_dib_bits(emu, bits_addr, addr + size, width, height, false, bpp, &palette)
}

fn dib_palette_entries(bpp: u32, colors_used: u32) -> Option<u32> {
    if colors_used != 0 {
        return Some(colors_used);
    }
    match bpp {
        1 | 4 | 8 => Some(1u32 << bpp),
        16 | 24 | 32 => Some(0),
        _ => None,
    }
}

fn create_surface_from_dib_bits(
    emu: &mut Emulator,
    bits_addr: u32,
    end_addr: u32,
    width: u32,
    height: u32,
    top_down: bool,
    bpp: u32,
    palette: &[u16],
) -> Option<u32> {
    let stride = dib_row_stride(width, bpp)?;
    let pixel_bytes = stride.checked_mul(height)?;
    if bits_addr.checked_add(pixel_bytes)? > end_addr {
        return None;
    }

    let surface = create_gdi_bitmap_surface_with_format(emu, width, height, 16).ok()?;
    let dst = read_surface_info(emu, surface).ok()?;
    for y in 0..height {
        let src_y = if top_down { y } else { height - 1 - y };
        let row_addr = bits_addr.checked_add(src_y.checked_mul(stride)?)?;
        let row = emu.memory.read_bytes(row_addr, stride as usize).ok()?;
        for x in 0..width {
            let color = dib_pixel_565(&row, x, bpp, palette)?;
            let dst_addr = dst
                .buffer
                .checked_add(y.checked_mul(dst.pitch)?)?
                .checked_add(x.checked_mul(2)?)?;
            emu.memory.write_u16(dst_addr, color).ok()?;
        }
    }
    Some(surface)
}

fn create_surface_from_rle8_dib(
    emu: &mut Emulator,
    bits_addr: u32,
    end_addr: u32,
    width: u32,
    height: u32,
    top_down: bool,
    palette: &[u16],
) -> Option<u32> {
    if palette.is_empty() {
        return None;
    }

    let surface = create_gdi_bitmap_surface_with_format(emu, width, height, 16).ok()?;
    let dst = read_surface_info(emu, surface).ok()?;
    emu.memory
        .memset(dst.buffer, 0, dst.pitch.checked_mul(dst.height)?)
        .ok()?;

    let mut addr = bits_addr;
    let mut x = 0u32;
    let mut y = 0u32;
    while addr < end_addr && y < height {
        let count = read_dib_rle_byte(emu, &mut addr, end_addr)?;
        let value = read_dib_rle_byte(emu, &mut addr, end_addr)?;
        if count != 0 {
            for _ in 0..count {
                if x < width && y < height {
                    let color = palette.get(value as usize).copied()?;
                    write_dib_surface_pixel_565(emu, dst, x, y, top_down, color)?;
                }
                x = x.saturating_add(1);
            }
            continue;
        }

        match value {
            0 => {
                x = 0;
                y = y.saturating_add(1);
            }
            1 => break,
            2 => {
                let dx = read_dib_rle_byte(emu, &mut addr, end_addr)? as u32;
                let dy = read_dib_rle_byte(emu, &mut addr, end_addr)? as u32;
                x = x.saturating_add(dx);
                y = y.saturating_add(dy);
            }
            absolute => {
                for _ in 0..absolute {
                    let index = read_dib_rle_byte(emu, &mut addr, end_addr)?;
                    if x < width && y < height {
                        let color = palette.get(index as usize).copied()?;
                        write_dib_surface_pixel_565(emu, dst, x, y, top_down, color)?;
                    }
                    x = x.saturating_add(1);
                }
                if (absolute & 1) != 0 {
                    let _pad = read_dib_rle_byte(emu, &mut addr, end_addr)?;
                }
            }
        }
    }

    Some(surface)
}

fn create_surface_from_rle4_dib(
    emu: &mut Emulator,
    bits_addr: u32,
    end_addr: u32,
    width: u32,
    height: u32,
    top_down: bool,
    palette: &[u16],
) -> Option<u32> {
    if palette.is_empty() {
        return None;
    }

    let surface = create_gdi_bitmap_surface_with_format(emu, width, height, 16).ok()?;
    let dst = read_surface_info(emu, surface).ok()?;
    emu.memory
        .memset(dst.buffer, 0, dst.pitch.checked_mul(dst.height)?)
        .ok()?;

    let mut addr = bits_addr;
    let mut x = 0u32;
    let mut y = 0u32;
    while addr < end_addr && y < height {
        let count = read_dib_rle_byte(emu, &mut addr, end_addr)?;
        let value = read_dib_rle_byte(emu, &mut addr, end_addr)?;
        if count != 0 {
            for i in 0..count {
                let index = if (i & 1) == 0 {
                    value >> 4
                } else {
                    value & 0x0f
                };
                if x < width && y < height {
                    let color = palette.get(index as usize).copied()?;
                    write_dib_surface_pixel_565(emu, dst, x, y, top_down, color)?;
                }
                x = x.saturating_add(1);
            }
            continue;
        }

        match value {
            0 => {
                x = 0;
                y = y.saturating_add(1);
            }
            1 => break,
            2 => {
                let dx = read_dib_rle_byte(emu, &mut addr, end_addr)? as u32;
                let dy = read_dib_rle_byte(emu, &mut addr, end_addr)? as u32;
                x = x.saturating_add(dx);
                y = y.saturating_add(dy);
            }
            absolute => {
                let byte_count = (absolute as u32).saturating_add(1) / 2;
                for byte_index in 0..byte_count {
                    let packed = read_dib_rle_byte(emu, &mut addr, end_addr)?;
                    for nibble in 0..2 {
                        let pixel_index = byte_index * 2 + nibble;
                        if pixel_index >= absolute as u32 {
                            break;
                        }
                        let index = if nibble == 0 {
                            packed >> 4
                        } else {
                            packed & 0x0f
                        };
                        if x < width && y < height {
                            let color = palette.get(index as usize).copied()?;
                            write_dib_surface_pixel_565(emu, dst, x, y, top_down, color)?;
                        }
                        x = x.saturating_add(1);
                    }
                }
                if (byte_count & 1) != 0 {
                    let _pad = read_dib_rle_byte(emu, &mut addr, end_addr)?;
                }
            }
        }
    }

    Some(surface)
}

fn write_dib_surface_pixel_565(
    emu: &mut Emulator,
    surface: SurfaceInfo,
    x: u32,
    y: u32,
    top_down: bool,
    color: u16,
) -> Option<()> {
    if x >= surface.width || y >= surface.height {
        return Some(());
    }
    let dst_y = if top_down {
        y
    } else {
        surface.height.checked_sub(1)?.checked_sub(y)?
    };
    let dst_addr = surface
        .buffer
        .checked_add(dst_y.checked_mul(surface.pitch)?)?
        .checked_add(x.checked_mul(2)?)?;
    emu.memory.write_u16(dst_addr, color).ok()
}
