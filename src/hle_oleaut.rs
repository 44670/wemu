const VARIANT_SIZE: usize = 16;
const VT_EMPTY: u16 = 0;

// BSTR SysAllocStringLen(const OLECHAR *src, UINT len)
// Allocate a length-prefixed UTF-16 BSTR and copy len characters when provided.
fn hle_sys_alloc_string_len(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = arg(emu, 0);
    let len = arg(emu, 1);
    let bstr = alloc_bstr_len(emu, src, len).hle();
    ret(emu, bstr);
    HleResult::Retn(8)
}

// void SysFreeString(BSTR bstr)
// Free BSTRs allocated by this HLE when possible.
fn hle_sys_free_string(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let bstr = arg(emu, 0);
    if bstr >= 4 {
        emu.hle.free_alloc(&mut emu.memory, bstr - 4).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// BOOL SysReAllocStringLen(BSTR *out, const OLECHAR *src, UINT len)
// Replace a BSTR pointer with a newly allocated length-prefixed UTF-16 string.
fn hle_sys_re_alloc_string_len(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let src = arg(emu, 1);
    let len = arg(emu, 2);
    if out == 0 {
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    let old = emu.memory.read_u32(out).unwrap_or(0);
    if old >= 4 {
        emu.hle.free_alloc(&mut emu.memory, old - 4).hle();
    }
    let bstr = alloc_bstr_len(emu, src, len).hle();
    emu.memory.write_u32(out, bstr).hle();
    ret(emu, 1);
    HleResult::Retn(12)
}

// UINT SysStringLen(BSTR bstr)
// Return the UTF-16 character count stored in the BSTR byte-length prefix.
fn hle_sys_string_len(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let bstr = arg(emu, 0);
    let len = if bstr >= 4 {
        emu.memory.read_u32(bstr - 4).unwrap_or(0) / 2
    } else {
        0
    };
    ret(emu, len);
    HleResult::Retn(4)
}

// HRESULT VariantChangeTypeEx(VARIANTARG *dst, VARIANTARG *src, LCID lcid, USHORT flags, VARTYPE vt)
// Copy the source VARIANT and update its type tag for simple automation startup paths.
fn hle_variant_change_type_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let vt = arg(emu, 4) as u16;
    if dst != 0 {
        if src != 0 {
            let bytes = emu.memory.read_bytes(src, VARIANT_SIZE).hle();
            emu.memory.write_bytes(dst, &bytes).hle();
        } else {
            emu.memory.write_bytes(dst, &[0; VARIANT_SIZE]).hle();
        }
        emu.memory.write_u16(dst, vt).hle();
    }
    ret(emu, 0);
    HleResult::Retn(20)
}

// HRESULT VariantClear(VARIANTARG *variant)
// Reset a VARIANT to VT_EMPTY without following contained pointers.
fn hle_variant_clear(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let variant = arg(emu, 0);
    if variant != 0 {
        emu.memory.write_bytes(variant, &[0; VARIANT_SIZE]).hle();
        emu.memory.write_u16(variant, VT_EMPTY).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// HRESULT VariantCopyInd(VARIANT *dst, const VARIANTARG *src)
// Copy the flat 32-bit VARIANT payload; by-ref dereference is deferred until needed.
fn hle_variant_copy_ind(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    if dst != 0 {
        if src != 0 {
            let bytes = emu.memory.read_bytes(src, VARIANT_SIZE).hle();
            emu.memory.write_bytes(dst, &bytes).hle();
        } else {
            emu.memory.write_bytes(dst, &[0; VARIANT_SIZE]).hle();
        }
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

fn alloc_bstr_len(emu: &mut Emulator, src: u32, len: u32) -> Result<u32> {
    let bytes = len.saturating_mul(2);
    let base = emu.hle.alloc(
        &mut emu.memory,
        bytes.saturating_add(6),
        PagePerm::READ | PagePerm::WRITE,
    )?;
    emu.memory.write_u32(base, bytes)?;
    let data = base + 4;
    for i in 0..len {
        let unit = if src != 0 {
            emu.memory.read_u16(src + i * 2).unwrap_or(0)
        } else {
            0
        };
        emu.memory.write_u16(data + i * 2, unit)?;
    }
    emu.memory.write_u16(data + bytes, 0)?;
    Ok(data)
}
