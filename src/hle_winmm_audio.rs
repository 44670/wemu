// UINT mixerGetNumDevs(void)
// Report no mixer devices because audio output is not emulated yet.
fn hle_mixer_get_num_devs(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(0)
}

// MMRESULT mixerOpen(LPHMIXER out, UINT id, DWORD_PTR cb, DWORD_PTR inst, DWORD flags)
// Fail mixer open with MMSYSERR_NODRIVER and clear the output handle.
fn hle_mixer_open(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const MMSYSERR_NODRIVER: u32 = 6;
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u32(out, 0).hle();
    }
    ret(emu, MMSYSERR_NODRIVER);
    HleResult::Retn(20)
}

// MMRESULT mixerClose(HMIXER mixer)
// Accept close of a null/fake mixer handle.
fn hle_mixer_close(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// MMRESULT mixer*(HMIXEROBJ mixer, void *details, DWORD flags)
// Report no mixer driver for line/control queries and writes.
fn hle_mixer_no_driver_12(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const MMSYSERR_NODRIVER: u32 = 6;
    ret(emu, MMSYSERR_NODRIVER);
    HleResult::Retn(12)
}

// MMRESULT waveOutOpen(LPHWAVEOUT out, UINT device, WAVEFORMATEX *fmt, DWORD cb, DWORD inst, DWORD flags)
// Open a fake no-sound wave output handle.
fn hle_wave_out_open(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u32(out, 0x5700_0000).hle();
    }
    ret(emu, 0);
    HleResult::Retn(24)
}

// MMRESULT waveOutGetDevCapsA(UINT_PTR device, WAVEOUTCAPSA *caps, UINT size)
// Return a small fake wave-output capability block for silent playback.
fn hle_wave_out_get_dev_caps_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const WAVE_FORMAT_1M08: u32 = 0x0000_0001;
    const WAVE_FORMAT_1S08: u32 = 0x0000_0002;
    const WAVE_FORMAT_1M16: u32 = 0x0000_0004;
    const WAVE_FORMAT_1S16: u32 = 0x0000_0008;
    const WAVECAPS_VOLUME: u32 = 0x0000_0004;
    let caps = arg(emu, 1);
    let size = arg(emu, 2).min(52);
    if caps != 0 && size != 0 {
        emu.memory.memset(caps, 0, size).hle();
        if size >= 2 {
            emu.memory.write_u16(caps, 1).hle();
        }
        if size >= 4 {
            emu.memory.write_u16(caps + 2, 1).hle();
        }
        if size >= 8 {
            emu.memory.write_u32(caps + 4, 0x0001_0000).hle();
        }
        if size > 8 {
            emu.memory
                .write_cstr(caps + 8, "wemu waveout", (size - 8).min(32) as usize)
                .hle();
        }
        if size >= 44 {
            emu.memory
                .write_u32(
                    caps + 40,
                    WAVE_FORMAT_1M08 | WAVE_FORMAT_1S08 | WAVE_FORMAT_1M16 | WAVE_FORMAT_1S16,
                )
                .hle();
        }
        if size >= 46 {
            emu.memory.write_u16(caps + 44, 2).hle();
        }
        if size >= 52 {
            emu.memory.write_u32(caps + 48, WAVECAPS_VOLUME).hle();
        }
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// MMRESULT waveOutGetPosition(HWAVEOUT out, MMTIME *time, UINT size)
// Report silent playback as current guest time in milliseconds.
fn hle_wave_out_get_position(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const TIME_MS: u32 = 0x0001;
    let time = arg(emu, 1);
    let size = arg(emu, 2);
    if time != 0 && size >= 8 {
        emu.memory.write_u32(time, TIME_MS).hle();
        emu.memory.write_u32(time + 4, emu.guest_time_ms as u32).hle();
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// MMRESULT waveOutGetID(HWAVEOUT out, UINT *device_id)
// Return the fake silent wave output device identifier.
fn hle_wave_out_get_id(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let id_out = arg(emu, 1);
    if id_out != 0 {
        emu.memory.write_u32(id_out, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// MMRESULT midiOutOpen(LPHMIDIOUT out, UINT device, DWORD cb, DWORD inst, DWORD flags)
// Open a fake no-sound MIDI output handle.
fn hle_midi_out_open(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u32(out, 0x5700_0100).hle();
    }
    ret(emu, 0);
    HleResult::Retn(20)
}

// MMRESULT midiStreamOpen(HMIDISTRM *out, UINT *device, DWORD count, DWORD cb, DWORD inst, DWORD flags)
// Open a fake no-sound MIDI stream handle.
fn hle_midi_stream_open(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u32(out, 0x5700_0200).hle();
    }
    ret(emu, 0);
    HleResult::Retn(24)
}

// MMRESULT timeGetDevCaps(LPTIMECAPS caps, UINT size)
// Return a coarse but valid multimedia timer capability range.
fn hle_time_get_dev_caps(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let caps = arg(emu, 0);
    let size = arg(emu, 1);
    if caps != 0 && size >= 8 {
        emu.memory.write_u32(caps, 1).hle();
        emu.memory.write_u32(caps + 4, 1000).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// MMRESULT acmMetrics(HACMOBJ obj, UINT metric, void *out)
// Report an installed-but-empty Audio Compression Manager driver set.
fn hle_acm_metrics(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const MMSYSERR_NOERROR: u32 = 0;
    const MMSYSERR_INVALHANDLE: u32 = 5;
    const MMSYSERR_INVALPARAM: u32 = 11;
    const MMSYSERR_NOTSUPPORTED: u32 = 8;

    const ACM_METRIC_COUNT_DRIVERS: u32 = 1;
    const ACM_METRIC_COUNT_CODECS: u32 = 2;
    const ACM_METRIC_COUNT_CONVERTERS: u32 = 3;
    const ACM_METRIC_COUNT_FILTERS: u32 = 4;
    const ACM_METRIC_COUNT_DISABLED: u32 = 5;
    const ACM_METRIC_COUNT_HARDWARE: u32 = 6;
    const ACM_METRIC_COUNT_LOCAL_DRIVERS: u32 = 20;
    const ACM_METRIC_COUNT_LOCAL_CODECS: u32 = 21;
    const ACM_METRIC_COUNT_LOCAL_CONVERTERS: u32 = 22;
    const ACM_METRIC_COUNT_LOCAL_FILTERS: u32 = 23;
    const ACM_METRIC_COUNT_LOCAL_DISABLED: u32 = 24;
    const ACM_METRIC_MAX_SIZE_FORMAT: u32 = 50;
    const ACM_METRIC_MAX_SIZE_FILTER: u32 = 51;

    let obj = arg(emu, 0);
    let metric = arg(emu, 1);
    let out = arg(emu, 2);

    let result = match metric {
        ACM_METRIC_COUNT_DRIVERS
        | ACM_METRIC_COUNT_CODECS
        | ACM_METRIC_COUNT_CONVERTERS
        | ACM_METRIC_COUNT_FILTERS
        | ACM_METRIC_COUNT_DISABLED
        | ACM_METRIC_COUNT_HARDWARE
        | ACM_METRIC_COUNT_LOCAL_DRIVERS
        | ACM_METRIC_COUNT_LOCAL_CODECS
        | ACM_METRIC_COUNT_LOCAL_CONVERTERS
        | ACM_METRIC_COUNT_LOCAL_FILTERS
        | ACM_METRIC_COUNT_LOCAL_DISABLED => {
            if obj != 0 {
                MMSYSERR_INVALHANDLE
            } else if out == 0 {
                MMSYSERR_INVALPARAM
            } else {
                emu.memory.write_u32(out, 0).hle();
                MMSYSERR_NOERROR
            }
        }
        ACM_METRIC_MAX_SIZE_FORMAT => {
            if obj != 0 {
                MMSYSERR_INVALHANDLE
            } else if out == 0 {
                MMSYSERR_INVALPARAM
            } else {
                emu.memory.write_u32(out, 18).hle();
                MMSYSERR_NOERROR
            }
        }
        ACM_METRIC_MAX_SIZE_FILTER => {
            if obj != 0 {
                MMSYSERR_INVALHANDLE
            } else if out == 0 {
                MMSYSERR_INVALPARAM
            } else {
                emu.memory.write_u32(out, 16).hle();
                MMSYSERR_NOERROR
            }
        }
        _ => MMSYSERR_NOTSUPPORTED,
    };

    ret(emu, result);
    HleResult::Retn(12)
}

// HMMIO mmioOpenA(LPSTR name, MMIOINFO *info, DWORD flags)
// Open a guest path or MMIOINFO memory buffer as a fake multimedia I/O handle.
fn hle_mmio_open_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const MMIO_CREATE: u32 = 0x0000_1000;
    const MMIO_WRITE: u32 = 0x0000_0001;
    const MMIO_READWRITE: u32 = 0x0000_0002;
    const MMIOERR_CANNOTOPEN: u32 = 259;

    let name = arg(emu, 0);
    let info = arg(emu, 1);
    let flags = arg(emu, 2);
    let wants_write = (flags & (MMIO_WRITE | MMIO_READWRITE | MMIO_CREATE)) != 0;
    let wants_read = (flags & MMIO_WRITE) == 0 || (flags & MMIO_READWRITE) != 0;

    if name == 0 {
        let buffer = if info != 0 {
            emu.memory.read_u32(info + 20).unwrap_or(0)
        } else {
            0
        };
        let len = if info != 0 {
            emu.memory.read_u32(info + 16).unwrap_or(0)
        } else {
            0
        };
        if buffer != 0 && len != 0 {
            let bytes = emu.memory.read_bytes(buffer, len as usize).hle();
            let h = emu.hle.alloc_handle(Handle::File(FileHandle::memory(
                format!("mmio:{info:08x}"),
                Rc::new(RefCell::new(bytes)),
                wants_write,
            )));
            mmio_write_open_info(emu, info, h, 0);
            ret(emu, h);
            return HleResult::Retn(12);
        }
        mmio_write_error(emu, info, MMIOERR_CANNOTOPEN);
        ret(emu, 0);
        return HleResult::Retn(12);
    }

    let raw_name = emu.memory.cstr_lossy(name, 1024).unwrap_or_default();
    let path = emu.hle.translate_path(&emu.memory, name).hle();
    let creation = if (flags & MMIO_CREATE) != 0 { 2 } else { 3 };
    let access = if (flags & MMIO_READWRITE) != 0 {
        0xc000_0000
    } else if wants_write {
        0x4000_0000
    } else {
        0x8000_0000
    };
    match emu.hle.open_virtual_file(&raw_name, access, creation) {
        VirtualOpen::Opened(h) => {
            mmio_write_open_info(emu, info, h, 0);
            ret(emu, h);
            return HleResult::Retn(12);
        }
        VirtualOpen::Failed(_) => {
            mmio_write_error(emu, info, MMIOERR_CANNOTOPEN);
            ret(emu, 0);
            return HleResult::Retn(12);
        }
        VirtualOpen::Miss => {}
    }
    match open_host_file_candidates(&raw_name, &path, wants_read, wants_write, creation) {
        Ok((file, _)) => {
            let h = emu.hle.alloc_handle(Handle::File(FileHandle::host(
                emu.hle.guest_path_key(&raw_name),
                file,
                wants_write,
            )));
            mmio_write_open_info(emu, info, h, 0);
            ret(emu, h);
        }
        Err(_) => {
            mmio_write_error(emu, info, MMIOERR_CANNOTOPEN);
            ret(emu, 0);
        }
    }
    HleResult::Retn(12)
}

// MMRESULT mmioClose(HMMIO hmmio, UINT flags)
// Close a fake multimedia I/O handle.
fn hle_mmio_close(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let result = if emu.hle.close_handle(h) { 0 } else { 5 };
    ret(emu, result);
    HleResult::Retn(8)
}

// LONG mmioRead(HMMIO hmmio, HPSTR out, LONG len)
// Read bytes from a fake multimedia I/O handle.
fn hle_mmio_read(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let out = arg(emu, 1);
    let len = arg(emu, 2) as i32;
    if out == 0 || len <= 0 {
        ret(emu, 0);
        return HleResult::Retn(12);
    }
    let mut bytes = vec![0; len as usize];
    let Some(read) = mmio_read_handle(emu, h, &mut bytes) else {
        ret(emu, u32::MAX);
        return HleResult::Retn(12);
    };
    emu.memory.write_bytes(out, &bytes[..read]).hle();
    ret(emu, read as u32);
    HleResult::Retn(12)
}

// LONG mmioSeek(HMMIO hmmio, LONG offset, int origin)
// Seek a fake multimedia I/O handle.
fn hle_mmio_seek(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let offset = arg(emu, 1) as i32 as i64;
    let origin = arg(emu, 2);
    match mmio_seek_handle(emu, h, offset, origin) {
        Some(pos) => ret(emu, pos as u32),
        None => ret(emu, u32::MAX),
    }
    HleResult::Retn(12)
}

// MMRESULT mmioGetInfo(HMMIO hmmio, MMIOINFO *info, UINT flags)
// Fill minimal buffered-I/O metadata for a fake multimedia I/O handle.
fn hle_mmio_get_info(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let info = arg(emu, 1);
    let Some(pos) = mmio_current_pos(emu, h) else {
        ret(emu, 5);
        return HleResult::Retn(12);
    };
    mmio_write_info(emu, info, h, pos as u32, 0);
    ret(emu, 0);
    HleResult::Retn(12)
}

// MMRESULT mmioSetInfo(HMMIO hmmio, const MMIOINFO *info, UINT flags)
// Accept MMIOINFO updates and honor simple pchNext/lBufOffset seek state.
fn hle_mmio_set_info(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let info = arg(emu, 1);
    if info != 0 {
        let pch_buffer = emu.memory.read_u32(info + 20).unwrap_or(0);
        let pch_next = emu.memory.read_u32(info + 24).unwrap_or(0);
        let offset = emu.memory.read_u32(info + 36).unwrap_or(0);
        if pch_buffer != 0 && pch_next >= pch_buffer {
            let next = offset.wrapping_add(pch_next.wrapping_sub(pch_buffer));
            let _ = mmio_seek_handle(emu, h, next as i32 as i64, 0);
        }
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// MMRESULT mmioAdvance(HMMIO hmmio, MMIOINFO *info, UINT flags)
// Refresh minimal MMIOINFO metadata without maintaining a separate buffer.
fn hle_mmio_advance(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let info = arg(emu, 1);
    let pos = mmio_current_pos(emu, h).unwrap_or(0) as u32;
    mmio_write_info(emu, info, h, pos, 0);
    ret(emu, 0);
    HleResult::Retn(12)
}

// MMRESULT mmioDescend(HMMIO hmmio, MMCKINFO *ck, const MMCKINFO *parent, UINT flags)
// Parse or scan RIFF-style chunks and position at the chunk payload.
fn hle_mmio_descend(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const MMIO_FINDCHUNK: u32 = 0x0010;
    const MMIO_FINDRIFF: u32 = 0x0020;
    const MMIO_FINDLIST: u32 = 0x0040;
    const MMIOERR_CHUNKNOTFOUND: u32 = 265;
    const MMIOERR_INVALIDFILE: u32 = 272;

    let h = arg(emu, 0);
    let ck = arg(emu, 1);
    let parent = arg(emu, 2);
    let flags = arg(emu, 3);
    if ck == 0 {
        ret(emu, MMIOERR_INVALIDFILE);
        return HleResult::Retn(16);
    }
    let target_id = emu.memory.read_u32(ck).unwrap_or(0);
    let target_type = emu.memory.read_u32(ck + 8).unwrap_or(0);
    let start = mmio_current_pos(emu, h).unwrap_or(0);
    let end = if parent != 0 {
        let parent_data = emu.memory.read_u32(parent + 12).unwrap_or(0) as u64;
        let parent_size = emu.memory.read_u32(parent + 4).unwrap_or(0) as u64;
        parent_data.saturating_add(parent_size)
    } else {
        mmio_len(emu, h).unwrap_or(u64::MAX)
    };

    let mut pos = start;
    while pos.saturating_add(8) <= end {
        let Some(id) = mmio_read_u32_at(emu, h, pos) else {
            break;
        };
        let Some(size) = mmio_read_u32_at(emu, h, pos + 4) else {
            break;
        };
        let is_riff = id == fourcc(b"RIFF");
        let is_list = id == fourcc(b"LIST");
        let typ = if is_riff || is_list {
            mmio_read_u32_at(emu, h, pos + 8).unwrap_or(0)
        } else {
            0
        };
        let match_chunk = if (flags & MMIO_FINDRIFF) != 0 {
            is_riff && (target_type == 0 || target_type == typ)
        } else if (flags & MMIO_FINDLIST) != 0 {
            is_list && (target_type == 0 || target_type == typ)
        } else if (flags & MMIO_FINDCHUNK) != 0 {
            id == target_id
        } else {
            true
        };
        let payload = if is_riff || is_list { pos + 12 } else { pos + 8 };
        let payload_size = if is_riff || is_list {
            size.saturating_sub(4)
        } else {
            size
        };
        if match_chunk {
            mmio_write_ckinfo(emu, ck, id, payload_size, typ, payload as u32);
            let _ = mmio_seek_handle(emu, h, payload as i64, 0);
            ret(emu, 0);
            return HleResult::Retn(16);
        }
        pos = pos.saturating_add(8 + padded_mmio_size(size) as u64);
    }
    ret(emu, MMIOERR_CHUNKNOTFOUND);
    HleResult::Retn(16)
}

// MMRESULT mmioAscend(HMMIO hmmio, MMCKINFO *ck, UINT flags)
// Seek to the padded end of a chunk described by MMCKINFO.
fn hle_mmio_ascend(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let h = arg(emu, 0);
    let ck = arg(emu, 1);
    if ck != 0 {
        let data = emu.memory.read_u32(ck + 12).unwrap_or(0);
        let size = emu.memory.read_u32(ck + 4).unwrap_or(0);
        let _ = mmio_seek_handle(emu, h, data.wrapping_add(padded_mmio_size(size)) as i32 as i64, 0);
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

fn mmio_write_error(emu: &mut Emulator, info: u32, err: u32) {
    if info != 0 {
        emu.memory.write_u32(info + 8, err).hle();
    }
}

fn mmio_write_open_info(emu: &mut Emulator, info: u32, h: u32, err: u32) {
    if info != 0 {
        let pos = mmio_current_pos(emu, h).unwrap_or(0) as u32;
        mmio_write_info(emu, info, h, pos, err);
    }
}

fn mmio_write_info(emu: &mut Emulator, info: u32, h: u32, pos: u32, err: u32) {
    if info == 0 {
        return;
    }
    emu.memory.write_bytes(info, &[0; 72]).hle();
    emu.memory.write_u32(info + 8, err).hle();
    emu.memory.write_u32(info + 36, pos).hle();
    emu.memory.write_u32(info + 40, pos).hle();
    emu.memory.write_u32(info + 68, h).hle();
}

fn mmio_write_ckinfo(emu: &mut Emulator, ck: u32, id: u32, size: u32, typ: u32, data: u32) {
    emu.memory.write_u32(ck, id).hle();
    emu.memory.write_u32(ck + 4, size).hle();
    emu.memory.write_u32(ck + 8, typ).hle();
    emu.memory.write_u32(ck + 12, data).hle();
    emu.memory.write_u32(ck + 16, 0).hle();
}

fn mmio_read_handle(emu: &mut Emulator, h: u32, out: &mut [u8]) -> Option<usize> {
    match emu.hle.handle_mut(h)? {
        Handle::File(file) => file.read_sync(out).ok(),
        _ => None,
    }
}

fn mmio_current_pos(emu: &mut Emulator, h: u32) -> Option<u64> {
    match emu.hle.handle_mut(h)? {
        Handle::File(file) => Some(file.current_pos()),
        _ => None,
    }
}

fn mmio_len(emu: &mut Emulator, h: u32) -> Option<u64> {
    match emu.hle.handle_mut(h)? {
        Handle::File(file) => Some(file.size()),
        _ => None,
    }
}

fn mmio_seek_handle(emu: &mut Emulator, h: u32, offset: i64, origin: u32) -> Option<u64> {
    match emu.hle.handle_mut(h)? {
        Handle::File(file) => file.seek(offset, origin).ok(),
        _ => None,
    }
}

fn mmio_read_u32_at(emu: &mut Emulator, h: u32, pos: u64) -> Option<u32> {
    let mut bytes = [0; 4];
    match emu.hle.handle_mut(h)? {
        Handle::File(file) => {
            file.read_exact_at_sync(pos, &mut bytes).ok()?;
        }
        _ => return None,
    }
    Some(u32::from_le_bytes(bytes))
}

fn padded_mmio_size(size: u32) -> u32 {
    size.wrapping_add(1) & !1
}

fn fourcc(bytes: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*bytes)
}
