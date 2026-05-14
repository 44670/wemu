// HLE helper write_zero_ret_ok(this, out)
// Write zero to the second argument and return success.
fn hle_write_zero_ret_ok(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HLE helper write_60_ret_ok(this, out)
// Write refresh rate 60 to the second argument and return success.
fn hle_write_60_ret_ok(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 60).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HLE helper write_zero2_ret_ok(this, out_a, out_b)
// Write zero to two output arguments and return success.
fn hle_write_zero2_ret_ok(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out1 = arg(emu, 1);
    let out2 = arg(emu, 2);
    if out1 != 0 {
        emu.memory.write_u32(out1, 0).hle();
    }
    if out2 != 0 {
        emu.memory.write_u32(out2, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}
