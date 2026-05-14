// HRESULT DirectSoundCreate(LPGUID guid, LPDIRECTSOUND *out, IUnknown *outer)
// Create a fake DirectSound COM object.
fn hle_direct_sound_create(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.ensure_dsound_tables(&mut emu.memory).hle();
    let out = arg(emu, 1);
    let obj = create_dsound_object(emu).hle();
    if out != 0 {
        emu.memory.write_u32(out, obj).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectSound::CreateSoundBuffer(this, DSBUFFERDESC *desc, IDirectSoundBuffer **out, IUnknown *outer)
// Create a fake DirectSound buffer and backing guest memory.
fn hle_dsound_create_sound_buffer(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let desc = arg(emu, 1);
    let out = arg(emu, 2);
    let obj = create_dsound_buffer(emu, desc).hle();
    if out != 0 {
        emu.memory.write_u32(out, obj).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectSound::GetCaps(this, DSCAPS *caps)
// Fill enough DirectSound capability fields for old games.
fn hle_dsound_get_caps(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let caps = arg(emu, 1);
    if caps != 0 {
        // Enough DSCAPS for installers and old games that check stereo/16-bit support.
        emu.memory.write_u32(caps, 96).hle();
        emu.memory.write_u32(caps + 4, 0x0000_0a0a).hle();
        emu.memory.write_u32(caps + 8, 11025).hle();
        emu.memory.write_u32(caps + 12, 48000).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectSound::DuplicateSoundBuffer(this, IDirectSoundBuffer *src, IDirectSoundBuffer **out)
// Return the source fake buffer and increment its reference count.
fn hle_dsound_duplicate_sound_buffer(
    emu: &mut Emulator,
    _: &HleEntry,
) -> HleResult {
    let src = arg(emu, 1);
    let out = arg(emu, 2);
    if out != 0 {
        emu.memory.write_u32(out, src).hle();
    }
    if src != 0 {
        let refs = emu.memory.read_u32(src + 4).unwrap_or(0).saturating_add(1);
        emu.memory.write_u32(src + 4, refs).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectSound::GetSpeakerConfig(this, DWORD *out)
// Report stereo speaker configuration.
fn hle_dsound_get_speaker_config(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 4).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectSoundBuffer::GetCaps(this, DSBCAPS *out)
// Fill fake buffer size and capability flags.
fn hle_dsound_buffer_get_caps(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 20).hle();
        emu.memory.write_u32(out + 4, 0x0000_c080).hle();
        emu.memory
            .write_u32(out + 8, emu.memory.read_u32(this + 8).unwrap_or(0)).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectSoundBuffer::GetCurrentPosition(this, DWORD *play, DWORD *write)
// Return stored fake play/write cursor positions.
fn hle_dsound_buffer_get_current_position(
    emu: &mut Emulator,
    _: &HleEntry,
) -> HleResult {
    let this = arg(emu, 0);
    let play_out = arg(emu, 1);
    let write_out = arg(emu, 2);
    if play_out != 0 {
        emu.memory
            .write_u32(play_out, emu.memory.read_u32(this + 16).unwrap_or(0)).hle();
    }
    if write_out != 0 {
        emu.memory
            .write_u32(write_out, emu.memory.read_u32(this + 20).unwrap_or(0)).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectSoundBuffer::GetFormat(this, WAVEFORMATEX *out, DWORD cap, DWORD *written)
// Return a simple stereo 16-bit PCM wave format.
fn hle_dsound_buffer_get_format(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    let cap = arg(emu, 2);
    let written_out = arg(emu, 3);
    let needed = 18;
    if written_out != 0 {
        emu.memory.write_u32(written_out, needed).hle();
    }
    if out != 0 && cap >= 16 {
        emu.memory.write_u16(out, 1).hle();
        emu.memory.write_u16(out + 2, 2).hle();
        emu.memory.write_u32(out + 4, 22050).hle();
        emu.memory.write_u32(out + 8, 22050 * 4).hle();
        emu.memory.write_u16(out + 12, 4).hle();
        emu.memory.write_u16(out + 14, 16).hle();
        if cap >= 18 {
            emu.memory.write_u16(out + 16, 0).hle();
        }
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectSoundBuffer::GetStatus(this, DWORD *status)
// Return the fake buffer playback status flags.
fn hle_dsound_buffer_get_status(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory
            .write_u32(out, emu.memory.read_u32(this + 24).unwrap_or(0)).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectSoundBuffer::Lock(this, DWORD offset, DWORD bytes, void **p1, DWORD *b1, void **p2, DWORD *b2, DWORD flags)
// Expose contiguous fake sound buffer memory for guest writes.
fn hle_dsound_buffer_lock(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let mut offset = arg(emu, 1);
    let mut bytes = arg(emu, 2);
    let data_out1 = arg(emu, 3);
    let bytes_out1 = arg(emu, 4);
    let data_out2 = arg(emu, 5);
    let bytes_out2 = arg(emu, 6);
    let flags = arg(emu, 7);
    let buffer_bytes = emu.memory.read_u32(this + 8).hle().max(1);
    let data = emu.memory.read_u32(this + 12).hle();

    if (flags & 0x2) != 0 || bytes == 0 {
        offset = 0;
        bytes = buffer_bytes;
    }
    if offset >= buffer_bytes {
        ret(emu, 0x8878_001e);
        return HleResult::Retn(32);
    }
    bytes = bytes.min(buffer_bytes - offset);

    if data_out1 != 0 {
        emu.memory.write_u32(data_out1, data + offset).hle();
    }
    if bytes_out1 != 0 {
        emu.memory.write_u32(bytes_out1, bytes).hle();
    }
    if data_out2 != 0 {
        emu.memory.write_u32(data_out2, 0).hle();
    }
    if bytes_out2 != 0 {
        emu.memory.write_u32(bytes_out2, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(32)
}

// HRESULT IDirectSoundBuffer::Play(this, DWORD reserved1, DWORD priority, DWORD flags)
// Mark the fake sound buffer as playing or looping.
fn hle_dsound_buffer_play(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let flags = arg(emu, 3);
    let mut status = 1;
    if (flags & 1) != 0 {
        status |= 4;
    }
    emu.memory.write_u32(this + 24, status).hle();
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectSoundBuffer::SetCurrentPosition(this, DWORD pos)
// Store the fake play cursor modulo buffer size.
fn hle_dsound_buffer_set_current_position(
    emu: &mut Emulator,
    _: &HleEntry,
) -> HleResult {
    let this = arg(emu, 0);
    let pos = arg(emu, 1);
    let size = emu.memory.read_u32(this + 8).unwrap_or(1).max(1);
    emu.memory.write_u32(this + 16, pos % size).hle();
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectSoundBuffer::Stop(this)
// Clear fake playback status.
fn hle_dsound_buffer_stop(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    emu.memory.write_u32(this + 24, 0).hle();
    ret(emu, 0);
    HleResult::Retn(4)
}

fn create_dsound_object(emu: &mut Emulator) -> Result<u32> {
    emu.hle.ensure_dsound_tables(&mut emu.memory)?;
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 8, PagePerm::READ | PagePerm::WRITE)?;
    emu.memory.write_u32(obj, emu.hle.dsound_vtable)?;
    emu.memory.write_u32(obj + 4, 1)?;
    Ok(obj)
}

fn create_dsound_buffer(emu: &mut Emulator, desc: u32) -> Result<u32> {
    emu.hle.ensure_dsound_tables(&mut emu.memory)?;
    let mut buffer_bytes = 4096;
    if desc != 0 {
        buffer_bytes = emu.memory.read_u32(desc + 8).unwrap_or(0).max(4096);
    }
    let data = emu.hle.alloc(
        &mut emu.memory,
        buffer_bytes,
        PagePerm::READ | PagePerm::WRITE,
    )?;
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 28, PagePerm::READ | PagePerm::WRITE)?;
    emu.memory.write_u32(obj, emu.hle.dsound_buffer_vtable)?;
    emu.memory.write_u32(obj + 4, 1)?;
    emu.memory.write_u32(obj + 8, buffer_bytes)?;
    emu.memory.write_u32(obj + 12, data)?;
    emu.memory.write_u32(obj + 16, 0)?;
    emu.memory.write_u32(obj + 20, 0)?;
    emu.memory.write_u32(obj + 24, 0)?;
    Ok(obj)
}
