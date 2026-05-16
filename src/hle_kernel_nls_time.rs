// DWORD GetTickCount(void)
// Return the guest millisecond clock.
fn hle_get_tick_count(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.refresh_guest_time();
    let now = emu.guest_time_ms as u32;
    ret(emu, now);
    HleResult::Retn(0)
}

// DWORD GetVersion(void)
// Report a Windows XP-like version value.
fn hle_get_version(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0a28_0105);
    HleResult::Retn(0)
}

// DWORD GetProcessVersion(DWORD process_id)
// Report a Windows XP-style process version for compatibility checks.
fn hle_get_process_version(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0005_0001);
    HleResult::Retn(4)
}

// BOOL GetVersionExW(LPOSVERSIONINFOW info)
// Fill a Windows XP-style version structure for startup feature checks.
fn hle_get_version_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        let size = emu.memory.read_u32(out).unwrap_or(0);
        emu.memory.write_u32(out + 4, 5).hle();
        emu.memory.write_u32(out + 8, 1).hle();
        emu.memory.write_u32(out + 12, 2600).hle();
        emu.memory.write_u32(out + 16, 2).hle();
        emu.memory
            .write_utf16z(out + 20, "Service Pack 3", 128)
            .hle();
        if size >= 284 {
            emu.memory.write_u16(out + 276, 3).hle();
            emu.memory.write_u16(out + 278, 0).hle();
            emu.memory.write_u16(out + 280, 0x0100).hle();
            emu.memory.write_u8(out + 282, 1).hle();
            emu.memory.write_u8(out + 283, 0).hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL GetVersionExA(LPOSVERSIONINFOA info)
// Fill a Windows XP-style ANSI version structure for startup feature checks.
fn hle_get_version_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        let size = emu.memory.read_u32(out).unwrap_or(0);
        emu.memory.write_u32(out + 4, 5).hle();
        emu.memory.write_u32(out + 8, 1).hle();
        emu.memory.write_u32(out + 12, 2600).hle();
        emu.memory.write_u32(out + 16, 2).hle();
        emu.memory.write_cstr(out + 20, "Service Pack 3", 128).hle();
        if size >= 156 {
            emu.memory.write_u16(out + 148, 3).hle();
            emu.memory.write_u16(out + 150, 0).hle();
            emu.memory.write_u16(out + 152, 0x0100).hle();
            emu.memory.write_u8(out + 154, 1).hle();
            emu.memory.write_u8(out + 155, 0).hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// int GetLocaleInfoW(LCID locale, LCTYPE type, LPWSTR out, int cch)
// Return stable US-style separators and locale strings used by UI startup code.
fn hle_get_locale_info_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let lctype = arg(emu, 1) & 0xffff;
    let out = arg(emu, 2);
    let cch = arg(emu, 3) as usize;
    let value = locale_info_value(lctype);
    let needed = value.encode_utf16().count() + 1;
    if out != 0 && cch != 0 {
        emu.memory.write_utf16z(out, value, cch).hle();
        if cch < needed {
            emu.hle.last_error = 122;
            ret(emu, 0);
            return HleResult::Retn(16);
        }
    }
    ret(emu, needed as u32);
    HleResult::Retn(16)
}

// int GetLocaleInfoA(LCID locale, LCTYPE type, LPSTR out, int cch)
// Return stable US-style separators and locale strings used by UI startup code.
fn hle_get_locale_info_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let lctype = arg(emu, 1) & 0xffff;
    let out = arg(emu, 2);
    let cch = arg(emu, 3) as usize;
    let value = locale_info_value(lctype);
    let needed = value.len() + 1;
    if out != 0 && cch != 0 {
        emu.memory.write_cstr(out, value, cch).hle();
        if cch < needed {
            emu.hle.last_error = 122;
            ret(emu, 0);
            return HleResult::Retn(16);
        }
    }
    ret(emu, needed as u32);
    HleResult::Retn(16)
}

// LCID GetThreadLocale(void)
// Return a stable US English thread locale.
fn hle_get_thread_locale(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0409);
    HleResult::Retn(0)
}

// BOOL SetThreadLocale(LCID locale)
// Accept the requested thread locale while keeping the emulator locale stable.
fn hle_set_thread_locale(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// LANGID GetSystemDefaultLangID(void)
// Return stable US English for legacy locale startup checks.
fn hle_get_system_default_lang_id(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0409);
    HleResult::Retn(0)
}

// LCID GetUserDefaultLCID(void)
// Return stable US English for legacy locale startup checks.
fn hle_get_user_default_lcid(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0409);
    HleResult::Retn(0)
}

// BOOL IsValidLocale(LCID locale, DWORD flags)
// Treat concrete locale identifiers as valid so CRT locale setup can proceed.
fn hle_is_valid_locale(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, (arg(emu, 0) != 0) as u32);
    HleResult::Retn(8)
}

// BOOL EnumCalendarInfoA(CALINFO_ENUMPROCA proc, LCID locale, CALID calendar, CALTYPE type)
// Succeed with an empty enumeration; callers can fall back to GetLocaleInfoA.
fn hle_enum_calendar_info_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(16)
}

// int LCMapStringA(LCID locale, DWORD flags, LPCSTR src, int src_len, LPSTR dst, int dst_len)
// Copy ANSI text with simple upper/lowercase transforms for locale startup code.
fn hle_lc_map_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const LCMAP_LOWERCASE: u32 = 0x0000_0100;
    const LCMAP_UPPERCASE: u32 = 0x0000_0200;

    let flags = arg(emu, 1);
    let src = arg(emu, 2);
    let src_len = arg(emu, 3) as i32;
    let dst = arg(emu, 4);
    let dst_len = arg(emu, 5) as usize;
    let mut bytes = if src_len < 0 {
        let mut bytes = emu.memory.cstr_lossy(src, 4096).hle().into_bytes();
        bytes.push(0);
        bytes
    } else {
        emu.memory.read_bytes(src, src_len.max(0) as usize).hle()
    };
    if (flags & LCMAP_LOWERCASE) != 0 {
        bytes.make_ascii_lowercase();
    } else if (flags & LCMAP_UPPERCASE) != 0 {
        bytes.make_ascii_uppercase();
    }
    let needed = bytes.len();
    if dst != 0 && dst_len != 0 {
        let count = needed.min(dst_len);
        emu.memory.write_bytes(dst, &bytes[..count]).hle();
        if dst_len < needed {
            emu.hle.last_error = 122;
            ret(emu, 0);
            return HleResult::Retn(24);
        }
    }
    ret(emu, needed as u32);
    HleResult::Retn(24)
}

// int LCMapStringW(LCID locale, DWORD flags, LPCWSTR src, int src_len, LPWSTR dst, int dst_len)
// Copy UTF-16 text with simple ASCII upper/lowercase transforms for locale startup code.
fn hle_lc_map_string_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 1);
    let src = arg(emu, 2);
    let src_len = arg(emu, 3) as i32;
    let dst = arg(emu, 4);
    let dst_len = arg(emu, 5) as usize;
    let result = lc_map_string_w_impl(emu, flags, src, src_len, dst, dst_len);
    ret(emu, result);
    HleResult::Retn(24)
}

// int LCMapStringEx(LPCWSTR locale, DWORD flags, LPCWSTR src, int src_len, LPWSTR dst, int dst_len, void *version, void *reserved, LPARAM sort)
// Map UTF-16 text like LCMapStringW while ignoring locale/version handles.
fn hle_lc_map_string_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 1);
    let src = arg(emu, 2);
    let src_len = arg(emu, 3) as i32;
    let dst = arg(emu, 4);
    let dst_len = arg(emu, 5) as usize;
    let result = lc_map_string_w_impl(emu, flags, src, src_len, dst, dst_len);
    ret(emu, result);
    HleResult::Retn(36)
}

fn lc_map_string_w_impl(
    emu: &mut Emulator,
    flags: u32,
    src: u32,
    src_len: i32,
    dst: u32,
    dst_len: usize,
) -> u32 {
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const LCMAP_LOWERCASE: u32 = 0x0000_0100;
    const LCMAP_UPPERCASE: u32 = 0x0000_0200;
    const LCMAP_SORTKEY: u32 = 0x0000_0400;

    let mut units = if src_len < 0 {
        let mut units = Vec::new();
        for i in 0..4096 {
            let unit = emu.memory.read_u16(src + i * 2).hle();
            units.push(unit);
            if unit == 0 {
                break;
            }
        }
        units
    } else {
        (0..src_len.max(0) as u32)
            .map(|i| emu.memory.read_u16(src + i * 2).hle())
            .collect::<Vec<_>>()
    };
    for unit in &mut units {
        if *unit <= 0x7f {
            let b = *unit as u8;
            *unit = if (flags & LCMAP_LOWERCASE) != 0 {
                b.to_ascii_lowercase() as u16
            } else if (flags & LCMAP_UPPERCASE) != 0 {
                b.to_ascii_uppercase() as u16
            } else {
                *unit
            };
        }
    }
    if (flags & LCMAP_SORTKEY) != 0 {
        let mut bytes = units
            .iter()
            .take_while(|unit| **unit != 0)
            .map(|unit| if *unit <= 0xff { *unit as u8 } else { b'?' })
            .collect::<Vec<_>>();
        bytes.push(0);
        let needed = bytes.len();
        if dst != 0 && dst_len != 0 {
            let count = needed.min(dst_len);
            emu.memory.write_bytes(dst, &bytes[..count]).hle();
            if dst_len < needed {
                emu.hle.last_error = ERROR_INSUFFICIENT_BUFFER;
                return 0;
            }
        }
        return needed as u32;
    }
    let needed = units.len();
    if dst != 0 && dst_len != 0 {
        let count = needed.min(dst_len);
        for (i, unit) in units.iter().take(count).enumerate() {
            emu.memory.write_u16(dst + i as u32 * 2, *unit).hle();
        }
        if dst_len < needed {
            emu.hle.last_error = ERROR_INSUFFICIENT_BUFFER;
            return 0;
        }
    }
    needed as u32
}

// BOOL GetStringTypeA(LCID locale, DWORD type, LPCSTR src, int count, LPWORD out)
// Fill coarse CTYPE1 character classes for ANSI text.
fn hle_get_string_type_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let info_type = arg(emu, 1);
    let src = arg(emu, 2);
    let count_arg = arg(emu, 3) as i32;
    let out = arg(emu, 4);
    let bytes = if count_arg < 0 {
        emu.memory.cstr_lossy(src, 4096).hle().into_bytes()
    } else {
        emu.memory.read_bytes(src, count_arg.max(0) as usize).hle()
    };
    if out != 0 {
        for (i, byte) in bytes.iter().enumerate() {
            let flags = if info_type == 1 {
                ctype1_flags(*byte as u16)
            } else {
                0
            };
            emu.memory.write_u16(out + i as u32 * 2, flags).hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(20)
}

// BOOL GetStringTypeW(DWORD type, LPCWSTR src, int count, LPWORD out)
// Fill coarse CTYPE1 character classes for UTF-16 text.
fn hle_get_string_type_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let info_type = arg(emu, 0);
    let src = arg(emu, 1);
    let count_arg = arg(emu, 2) as i32;
    let out = arg(emu, 3);
    let count = if count_arg < 0 {
        wide_len(&emu.memory, src, 4096)
    } else {
        count_arg.max(0) as usize
    };
    if out != 0 {
        for i in 0..count {
            let unit = emu.memory.read_u16(src + i as u32 * 2).hle();
            let flags = if info_type == 1 {
                ctype1_flags(unit)
            } else {
                0
            };
            emu.memory.write_u16(out + i as u32 * 2, flags).hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(16)
}

// int CompareStringA(LCID locale, DWORD flags, LPCSTR a, int a_len, LPCSTR b, int b_len)
// Compare ANSI strings and return Win32 CSTR ordering values.
fn hle_compare_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 1);
    let mut a = read_ansi_counted(emu, arg(emu, 2), arg(emu, 3));
    let mut b = read_ansi_counted(emu, arg(emu, 4), arg(emu, 5));
    if (flags & 0x0000_0001) != 0 {
        a.make_ascii_lowercase();
        b.make_ascii_lowercase();
    }
    ret(emu, compare_cstr_result(&a, &b));
    HleResult::Retn(24)
}

// int CompareStringW(LCID locale, DWORD flags, LPCWSTR a, int a_len, LPCWSTR b, int b_len)
// Compare UTF-16 strings and return Win32 CSTR ordering values.
fn hle_compare_string_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 1);
    let mut a = read_wide_counted(emu, arg(emu, 2), arg(emu, 3));
    let mut b = read_wide_counted(emu, arg(emu, 4), arg(emu, 5));
    if (flags & 0x0000_0001) != 0 {
        a = a.to_lowercase();
        b = b.to_lowercase();
    }
    ret(emu, compare_cstr_result(&a, &b));
    HleResult::Retn(24)
}

// UINT GetACP(void)
// Return code page 936 for Rich4 Chinese text paths.
fn hle_get_acp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 950);
    HleResult::Retn(0)
}

// UINT GetOEMCP(void)
// Return code page 936 for OEM text paths.
fn hle_get_oemcp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 950);
    HleResult::Retn(0)
}

// UINT GetConsoleCP(void)
// Return the active ANSI/OEM code page used by the emulator.
fn hle_get_console_cp(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 950);
    HleResult::Retn(0)
}

// BOOL IsValidCodePage(UINT cp)
// Accept common Windows code pages and the emulator's Chinese default.
fn hle_is_valid_code_page(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let cp = arg(emu, 0);
    let ok = matches!(cp, 0 | 437 | 936 | 950 | 1200 | 1252 | 65001);
    ret(emu, ok as u32);
    HleResult::Retn(4)
}

// BOOL GetCPInfo(UINT cp, LPCPINFO info)
// Fill minimal DBCS code-page information.
fn hle_get_cp_info(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    if out != 0 {
        emu.memory.write_u32(out, 1).hle();
        emu.memory.write_u8(out + 4, 0).hle();
        emu.memory.write_u8(out + 5, 0).hle();
        for i in 0..12 {
            emu.memory.write_u8(out + 6 + i, 0).hle();
        }
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// int MultiByteToWideChar(UINT cp, DWORD flags, LPCSTR in, int in_len, LPWSTR out, int out_len)
// Convert guest bytes to UTF-16 using lossy host decoding.
fn hle_multi_byte_to_wide_char(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = arg(emu, 2);
    let cb = arg(emu, 3) as i32;
    let dst = arg(emu, 4);
    let cap = arg(emu, 5) as usize;
    let bytes = if cb < 0 {
        let s = emu.memory.cstr_lossy(src, 4096).hle();
        let mut b = s.into_bytes();
        b.push(0);
        b
    } else {
        emu.memory.read_bytes(src, cb as usize).hle()
    };
    let count = bytes.len().min(cap);
    if dst != 0 && cap != 0 {
        for (i, b) in bytes.iter().take(count).enumerate() {
            emu.memory.write_u16(dst + (i as u32 * 2), *b as u16).hle();
        }
    }
    ret(emu, bytes.len() as u32);
    HleResult::Retn(24)
}

// int WideCharToMultiByte(UINT cp, DWORD flags, LPCWSTR in, int in_len, LPSTR out, int out_len, LPCSTR def, BOOL *used)
// Convert UTF-16 guest text to lossy single-byte output.
fn hle_wide_char_to_multi_byte(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = arg(emu, 2);
    let cch = arg(emu, 3) as i32;
    let dst = arg(emu, 4);
    let cap = arg(emu, 5) as usize;
    let s = if cch < 0 {
        emu.memory.utf16z_lossy(src, 4096).hle()
    } else {
        let mut units = Vec::new();
        for i in 0..cch as usize {
            units.push(emu.memory.read_u16(src + (i as u32 * 2)).hle());
        }
        String::from_utf16_lossy(&units)
    };
    let mut bytes = s.into_bytes();
    if cch < 0 {
        bytes.push(0);
    }
    if dst != 0 && cap != 0 {
        let n = bytes.len().min(cap);
        emu.memory.write_bytes(dst, &bytes[..n]).hle();
    }
    ret(emu, bytes.len() as u32);
    HleResult::Retn(32)
}

// MMRESULT joyGetPos(UINT id, JOYINFO *info)
// Report no joystick driver present.
fn hle_joy_get_pos(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 6);
    HleResult::Retn(8)
}

// MMRESULT joyGetPosEx(UINT id, JOYINFOEX *info)
// Report no connected joystick and leave the caller's capability buffer untouched.
fn hle_joy_get_pos_ex(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const JOYERR_UNPLUGGED: u32 = 167;
    ret(emu, JOYERR_UNPLUGGED);
    HleResult::Retn(8)
}

// MMRESULT joyGetDevCapsA(UINT id, JOYCAPSA *caps, UINT size)
// Report no joystick driver present.
fn hle_joy_get_dev_caps_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 6);
    HleResult::Retn(12)
}

// MMRESULT joySetCapture(HWND hwnd, UINT id, UINT period, BOOL changed)
// Refuse joystick capture because no joystick backend is present.
fn hle_joy_set_capture(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const JOYERR_UNPLUGGED: u32 = 167;
    ret(emu, JOYERR_UNPLUGGED);
    HleResult::Retn(16)
}

// MMRESULT joyReleaseCapture(UINT id)
// Accept release after a failed/fake joystick capture.
fn hle_joy_release_capture(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// BOOL QueryPerformanceCounter(LARGE_INTEGER *out)
// Report a monotonic deterministic counter in guest milliseconds.
fn hle_query_performance_counter(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    emu.refresh_guest_time();
    if out != 0 {
        emu.memory.write_u32(out, emu.guest_time_ms as u32).hle();
        emu.memory
            .write_u32(out + 4, (emu.guest_time_ms >> 32) as u32)
            .hle();
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// BOOL QueryPerformanceFrequency(LARGE_INTEGER *out)
// Report a millisecond-scale performance counter frequency.
fn hle_query_performance_frequency(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.write_u32(out, 1000).hle();
        emu.memory.write_u32(out + 4, 0).hle();
    }
    ret(emu, 1);
    HleResult::Retn(4)
}

// void GetLocalTime(SYSTEMTIME *out)
// Fill a stable local time structure for date/time UI paths.
fn hle_get_local_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        for (i, value) in [2026u16, 5, 4, 7, 12, 0, 0, 0].iter().enumerate() {
            emu.memory
                .write_u16(out + (i as u32 * 2), *value)
                .hle();
        }
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// void GetSystemTime(SYSTEMTIME *out)
// Fill a stable UTC time structure for date/time UI paths.
fn hle_get_system_time(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        for (i, value) in [2026u16, 5, 4, 7, 4, 0, 0, 0].iter().enumerate() {
            emu.memory
                .write_u16(out + (i as u32 * 2), *value)
                .hle();
        }
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// DWORD GetTimeZoneInformation(TIME_ZONE_INFORMATION *out)
// Report UTC with zero bias and no daylight transition rules.
fn hle_get_time_zone_information(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    if out != 0 {
        emu.memory.memset(out, 0, 172).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// int GetDateFormatW(LCID lcid, DWORD flags, SYSTEMTIME *st, LPCWSTR fmt, LPWSTR out, int cch)
// Return a fixed short date string suitable for Notepad insertion paths.
fn hle_get_date_format_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 4);
    let cch = arg(emu, 5) as usize;
    let value = "May 7, 2026";
    let needed = value.encode_utf16().count() + 1;
    if out != 0 && cch != 0 {
        emu.memory.write_utf16z(out, value, cch).hle();
    }
    ret(emu, needed as u32);
    HleResult::Retn(24)
}

// int GetTimeFormatW(LCID lcid, DWORD flags, SYSTEMTIME *st, LPCWSTR fmt, LPWSTR out, int cch)
// Return a fixed wall-clock text for Notepad time/date insertion paths.
fn hle_get_time_format_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 4);
    let cch = arg(emu, 5) as usize;
    let value = "12:00 PM";
    let needed = value.encode_utf16().count() + 1;
    if out != 0 && cch != 0 {
        emu.memory.write_utf16z(out, value, cch).hle();
    }
    ret(emu, needed as u32);
    HleResult::Retn(24)
}

// LANGID GetUserDefaultLangID(void)
// Return US English to keep startup layout and font selection simple.
fn hle_get_user_default_lang_id(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0x0409);
    HleResult::Retn(0)
}

// int MulDiv(int n, int mul, int div)
// Compute a rounded 32-bit signed multiply/divide result.
fn hle_mul_div(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let n = arg(emu, 0) as i32 as i64;
    let mul = arg(emu, 1) as i32 as i64;
    let div = arg(emu, 2) as i32 as i64;
    let value = if div == 0 { -1 } else { (n * mul + div / 2) / div };
    ret(emu, value as i32 as u32);
    HleResult::Retn(12)
}

// int lstrlenA(LPCSTR s)
// Return the length of a NUL-terminated ANSI string.
fn hle_lstrlen_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, ansi_len(&emu.memory, arg(emu, 0), 1 << 20) as u32);
    HleResult::Retn(4)
}

// LPSTR CharUpperA(LPSTR str_or_char)
// Uppercase an ANSI string in place, or uppercase a low-word character value.
fn hle_char_upper_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = arg(emu, 0);
    if (value & 0xffff_0000) == 0 {
        ret(emu, (value as u8).to_ascii_uppercase() as u32);
        return HleResult::Retn(4);
    }
    for i in 0..4096u32 {
        let addr = value.wrapping_add(i);
        let byte = emu.memory.read_u8(addr).hle();
        if byte == 0 {
            break;
        }
        emu.memory.write_u8(addr, byte.to_ascii_uppercase()).hle();
    }
    ret(emu, value);
    HleResult::Retn(4)
}

// LPSTR lstrcpyA(LPSTR dst, LPCSTR src)
// Copy a NUL-terminated ANSI string and return dst.
fn hle_lstrcpy_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let len = ansi_len(&emu.memory, src, 1 << 20);
    for i in 0..=len {
        let byte = emu.memory.read_u8(src + i as u32).hle();
        emu.memory.write_u8(dst + i as u32, byte).hle();
    }
    ret(emu, dst);
    HleResult::Retn(8)
}

// BOOL OemToCharA(LPCSTR src, LPSTR dst)
// Copy a NUL-terminated OEM byte string as the current ANSI code page.
fn hle_oem_to_char_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = arg(emu, 0);
    let dst = arg(emu, 1);
    let len = ansi_len(&emu.memory, src, 1 << 20);
    for i in 0..=len {
        let byte = emu.memory.read_u8(src + i as u32).hle();
        emu.memory.write_u8(dst + i as u32, byte).hle();
    }
    ret(emu, 1);
    HleResult::Retn(8)
}

// BOOL OemToCharBuffA(LPCSTR src, LPSTR dst, DWORD len)
// Copy a fixed-length OEM byte buffer as the current ANSI code page.
fn hle_oem_to_char_buff_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let src = arg(emu, 0);
    let dst = arg(emu, 1);
    let len = arg(emu, 2) as usize;
    if len != 0 {
        let bytes = emu.memory.read_bytes(src, len).hle();
        emu.memory.write_bytes(dst, &bytes).hle();
    }
    ret(emu, 1);
    HleResult::Retn(12)
}

// LPSTR lstrcpynA(LPSTR dst, LPCSTR src, int max)
// Copy a bounded ANSI string and NUL-terminate when capacity is nonzero.
fn hle_lstrcpyn_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let max = arg(emu, 2) as usize;
    if max != 0 {
        let len = ansi_len(&emu.memory, src, 1 << 20).min(max.saturating_sub(1));
        for i in 0..len {
            let byte = emu.memory.read_u8(src + i as u32).hle();
            emu.memory.write_u8(dst + i as u32, byte).hle();
        }
        emu.memory.write_u8(dst + len as u32, 0).hle();
    }
    ret(emu, dst);
    HleResult::Retn(12)
}

// LPWSTR lstrcpynW(LPWSTR dst, LPCWSTR src, int max)
// Copy a bounded UTF-16 string and NUL-terminate when capacity is nonzero.
fn hle_lstrcpyn_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let max = arg(emu, 2) as usize;
    lstrcpyn_w_impl(emu, dst, src, max);
    ret(emu, dst);
    HleResult::Retn(12)
}

fn lstrcpyn_w_impl(emu: &mut Emulator, dst: u32, src: u32, max: usize) {
    if max == 0 {
        return;
    }
    let len = wide_len(&emu.memory, src, 1 << 20).min(max.saturating_sub(1));
    for i in 0..len {
        let unit = emu.memory.read_u16(src + i as u32 * 2).hle();
        emu.memory.write_u16(dst + i as u32 * 2, unit).hle();
    }
    emu.memory.write_u16(dst + len as u32 * 2, 0).hle();
}

// LPSTR lstrcatA(LPSTR dst, LPCSTR src)
// Append a NUL-terminated ANSI string to dst and return dst.
fn hle_lstrcat_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let dst_len = ansi_len(&emu.memory, dst, 1 << 20);
    let src_len = ansi_len(&emu.memory, src, 1 << 20);
    for i in 0..=src_len {
        let byte = emu.memory.read_u8(src + i as u32).hle();
        emu.memory.write_u8(dst + dst_len as u32 + i as u32, byte).hle();
    }
    ret(emu, dst);
    HleResult::Retn(8)
}

// LPWSTR lstrcatW(LPWSTR dst, LPCWSTR src)
// Append a NUL-terminated UTF-16 string to dst and return dst.
fn hle_lstrcat_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let dst_len = wide_len(&emu.memory, dst, 1 << 20);
    let src_len = wide_len(&emu.memory, src, 1 << 20);
    for i in 0..=src_len {
        let unit = emu.memory.read_u16(src + i as u32 * 2).hle();
        emu.memory
            .write_u16(dst + (dst_len as u32 + i as u32) * 2, unit)
            .hle();
    }
    ret(emu, dst);
    HleResult::Retn(8)
}

// int lstrcmpA(LPCSTR a, LPCSTR b)
// Compare two ANSI strings with case-sensitive byte ordering.
fn hle_lstrcmp_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = compare_ansi_strings(emu, arg(emu, 0), arg(emu, 1), false);
    ret(emu, value as u32);
    HleResult::Retn(8)
}

// int lstrcmpiA(LPCSTR a, LPCSTR b)
// Compare two ANSI strings with ASCII case folded.
fn hle_lstrcmpi_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = compare_ansi_strings(emu, arg(emu, 0), arg(emu, 1), true);
    ret(emu, value as u32);
    HleResult::Retn(8)
}

// LPWSTR lstrcpyW(LPWSTR dst, LPCWSTR src)
// Copy a NUL-terminated UTF-16 string and return dst.
fn hle_lstrcpy_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let dst = arg(emu, 0);
    let src = arg(emu, 1);
    let len = wide_len(&emu.memory, src, 1 << 20);
    for i in 0..=len {
        let unit = emu.memory.read_u16(src + i as u32 * 2).hle();
        emu.memory.write_u16(dst + i as u32 * 2, unit).hle();
    }
    ret(emu, dst);
    HleResult::Retn(8)
}

// int lstrlenW(LPCWSTR s)
// Return the length of a NUL-terminated UTF-16 string.
fn hle_lstrlen_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, wide_len(&emu.memory, arg(emu, 0), 1 << 20) as u32);
    HleResult::Retn(4)
}

// BOOL IsTextUnicode(const void *buf, int len, int *flags)
// Heuristically report UTF-16 text when many odd bytes are zero.
fn hle_is_text_unicode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let buf = arg(emu, 0);
    let len = arg(emu, 1).min(512) as usize;
    let bytes = if buf != 0 {
        emu.memory.read_bytes(buf, len).unwrap_or_default()
    } else {
        Vec::new()
    };
    let odd_zero = bytes
        .iter()
        .enumerate()
        .filter(|(i, byte)| i % 2 == 1 && **byte == 0)
        .count();
    ret(emu, (odd_zero > bytes.len() / 8) as u32);
    HleResult::Retn(12)
}

fn locale_info_value(lctype: u32) -> &'static str {
    match lctype {
        0x0000_0002 => "English (United States)",
        0x0000_0003 => "ENU",
        0x0000_0006 => "United States",
        0x0000_000e => ".",
        0x0000_000f => ",",
        0x0000_0059 => "en",
        0x0000_005a => "US",
        0x0000_005c => "en-US",
        _ => "",
    }
}
