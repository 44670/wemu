#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn wemu_console_log(ptr: *const u8, len: usize);
    fn wemu_canvas_text(
        surface_buffer: u32,
        surface_width: u32,
        surface_height: u32,
        pitch: u32,
        bpp: u32,
        text_ptr: *const u8,
        text_len: usize,
        x: i32,
        y: i32,
        font_height: u32,
        extra: i32,
        colorref: u32,
        clip_left: i32,
        clip_top: i32,
        clip_right: i32,
        clip_bottom: i32,
    ) -> i32;
}

fn win_protect_to_perm(protect: u32) -> PagePerm {
    match protect & 0xff {
        0x01 => PagePerm::READ,
        0x02 => PagePerm::READ,
        0x04 => PagePerm::READ | PagePerm::WRITE,
        0x10 => PagePerm::EXEC,
        0x20 => PagePerm::READ | PagePerm::EXEC,
        0x40 => PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        0x80 => PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        _ => PagePerm::READ | PagePerm::WRITE,
    }
}

fn write_find_data_a(emu: &mut Emulator, out: u32, entry: &FindEntry) {
    if out == 0 {
        return;
    }
    emu.memory.memset(out, 0, 320).hle();
    emu.memory.write_u32(out, entry.attrs).hle();
    emu.memory.write_u32(out + 28, (entry.size >> 32) as u32).hle();
    emu.memory.write_u32(out + 32, entry.size as u32).hle();
    emu.memory.write_cstr(out + 44, &entry.name, 260).hle();
}

fn write_find_data_w(emu: &mut Emulator, out: u32, entry: &FindEntry) {
    if out == 0 {
        return;
    }
    emu.memory.memset(out, 0, 592).hle();
    emu.memory.write_u32(out, entry.attrs).hle();
    emu.memory.write_u32(out + 28, (entry.size >> 32) as u32).hle();
    emu.memory.write_u32(out + 32, entry.size as u32).hle();
    emu.memory.write_utf16z(out + 44, &entry.name, 260).hle();
}



// DWORD GetLastError(void)
// Return the current HLE last-error value.
fn hle_get_last_error(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.last_error);
    HleResult::Retn(0)
}

// void SetLastError(DWORD error)
// Store the HLE last-error value.
fn hle_set_last_error(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.last_error = arg(emu, 0);
    ret(emu, 0);
    HleResult::Retn(4)
}

// UINT SetErrorMode(UINT mode)
// Store process error-mode flags and return the previous mode.
fn hle_set_error_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let old = emu.hle.error_mode;
    emu.hle.error_mode = arg(emu, 0);
    ret(emu, old);
    HleResult::Retn(4)
}

// UINT GetErrorMode(void)
// Return the current process error-mode flags.
fn hle_get_error_mode(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.error_mode);
    HleResult::Retn(0)
}


fn ctype1_flags(unit: u16) -> u16 {
    const C1_UPPER: u16 = 0x0001;
    const C1_LOWER: u16 = 0x0002;
    const C1_DIGIT: u16 = 0x0004;
    const C1_SPACE: u16 = 0x0008;
    const C1_PUNCT: u16 = 0x0010;
    const C1_BLANK: u16 = 0x0040;
    const C1_XDIGIT: u16 = 0x0080;
    const C1_ALPHA: u16 = 0x0100;

    let ch = char::from_u32(unit as u32).unwrap_or('\0');
    let mut flags = 0;
    if ch.is_ascii_uppercase() {
        flags |= C1_UPPER | C1_ALPHA;
    }
    if ch.is_ascii_lowercase() {
        flags |= C1_LOWER | C1_ALPHA;
    }
    if ch.is_ascii_digit() {
        flags |= C1_DIGIT;
    }
    if ch.is_ascii_hexdigit() {
        flags |= C1_XDIGIT;
    }
    if ch.is_ascii_whitespace() {
        flags |= C1_SPACE;
    }
    if ch == ' ' || ch == '\t' {
        flags |= C1_BLANK;
    }
    if ch.is_ascii_punctuation() {
        flags |= C1_PUNCT;
    }
    flags
}

fn read_ansi_counted(emu: &Emulator, ptr: u32, count: u32) -> String {
    if count == u32::MAX {
        emu.memory.cstr_lossy(ptr, 4096).hle()
    } else {
        String::from_utf8_lossy(&emu.memory.read_bytes(ptr, count.min(4096) as usize).hle())
            .into_owned()
    }
}

fn read_wide_counted(emu: &Emulator, ptr: u32, count: u32) -> String {
    if count == u32::MAX {
        emu.memory.utf16z_lossy(ptr, 4096).hle()
    } else {
        let mut units = Vec::new();
        for i in 0..count.min(4096) {
            units.push(emu.memory.read_u16(ptr + i * 2).hle());
        }
        String::from_utf16_lossy(&units)
    }
}

fn compare_cstr_result(a: &str, b: &str) -> u32 {
    match a.cmp(b) {
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Equal => 2,
        std::cmp::Ordering::Greater => 3,
    }
}




fn dispatch_target(emu: &Emulator, hwnd: u32, msg: u32, lparam: u32) -> (u32, u32) {
    const WM_TIMER: u32 = 0x0113;
    if msg == WM_TIMER && lparam != 0 {
        return (lparam, emu.guest_time_ms as u32);
    }
    if hwnd == 0 {
        return (0, lparam);
    }
    let proc = emu
        .hle
        .window(hwnd)
        .and_then(|window| (window.proc != 0).then_some(window.proc))
        .unwrap_or(emu.hle.window_proc);
    (proc, lparam)
}



fn edit_backspace(emu: &mut Emulator, hwnd: u32) {
    let mut changed = false;
    if let Some(window) = emu.hle.window_mut(hwnd) {
        changed = window.text.pop().is_some();
    }
    if changed {
        edit_text_changed(emu, hwnd);
    }
}

fn edit_insert_char(emu: &mut Emulator, hwnd: u32, ch: u32) {
    const ES_UPPERCASE: u32 = 0x0008;
    if ch < 0x20 || ch == 0x7f {
        return;
    }
    let Some(mut value) = char::from_u32(ch) else {
        return;
    };
    let mut changed = false;
    if let Some(window) = emu.hle.window_mut(hwnd) {
        if (window.style & ES_UPPERCASE) != 0 {
            value = value.to_ascii_uppercase();
        }
        window.text.push(value);
        changed = true;
    }
    if changed {
        edit_text_changed(emu, hwnd);
    }
}

fn edit_text_changed(emu: &mut Emulator, hwnd: u32) {
    const WM_COMMAND: u32 = 0x0111;
    const EN_CHANGE: u32 = 0x0300;
    let Some((parent, id)) = emu.hle.window(hwnd).map(|window| (window.parent, window.id)) else {
        return;
    };
    render_hle_windows(emu);
    if parent != 0 {
        let message = Message {
            hwnd: parent,
            msg: WM_COMMAND,
            wparam: id | (EN_CHANGE << 16),
            lparam: hwnd,
        };
        emu.hle.app_messages.push(message);
        emu.hle.note_queued_message("edit-change", message);
    }
}

const ACCEL_FVIRTKEY: u16 = 0x0001;
const ACCEL_FSHIFT: u16 = 0x0004;
const ACCEL_FCONTROL: u16 = 0x0008;
const ACCEL_FALT: u16 = 0x0010;










#[derive(Clone, Copy)]
struct GdiLineMetrics {
    height: i32,
    char_width: i32,
    extra: i32,
}

struct GdiTextLayout {
    width: i32,
    height: i32,
    metrics: GdiLineMetrics,
    lines: Vec<GdiTextLine>,
}

struct GdiTextLine {
    start: usize,
    end: usize,
    width: i32,
}

#[derive(Clone, Copy)]
struct GdiGlyph {
    width: i32,
    visible: bool,
    byte: u8,
}

#[derive(Clone, Copy)]
struct GdiTextMetrics {
    char_width: i32,
    extra: i32,
}











fn glyph_bounds_clip(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    clip: Option<(i32, i32, i32, i32)>,
) -> Option<(i32, i32, i32, i32)> {
    let mut bounds = (
        x,
        y,
        x.saturating_add(width.max(1)),
        y.saturating_add(height.max(1)),
    );
    if let Some((left, top, right, bottom)) = clip {
        bounds.0 = bounds.0.max(left);
        bounds.1 = bounds.1.max(top);
        bounds.2 = bounds.2.min(right);
        bounds.3 = bounds.3.min(bottom);
    }
    (bounds.2 > bounds.0 && bounds.3 > bounds.1).then_some(bounds)
}

fn seabios_glyph_8x8(byte: u8) -> Option<[u8; 8]> {
    if !(0x20..=0x7e).contains(&byte) {
        return None;
    }
    let index = (byte - 0x20) as usize * 8;
    Some(SEABIOS_FONT8_PRINTABLE[index..index + 8].try_into().unwrap())
}

// SeaBIOS vgafont8 printable ASCII rows. Source notes mark the individual fonts public domain.
const SEABIOS_FONT8_PRINTABLE: [u8; 95 * 8] = [
    // 0x20 ' '
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // 0x21 '!'
    0x30, 0x78, 0x78, 0x30, 0x30, 0x00, 0x30, 0x00,
    // 0x22 '"'
    0x6c, 0x6c, 0x6c, 0x00, 0x00, 0x00, 0x00, 0x00,
    // 0x23 '#'
    0x6c, 0x6c, 0xfe, 0x6c, 0xfe, 0x6c, 0x6c, 0x00,
    // 0x24 '$'
    0x30, 0x7c, 0xc0, 0x78, 0x0c, 0xf8, 0x30, 0x00,
    // 0x25 '%'
    0x00, 0xc6, 0xcc, 0x18, 0x30, 0x66, 0xc6, 0x00,
    // 0x26 '&'
    0x38, 0x6c, 0x38, 0x76, 0xdc, 0xcc, 0x76, 0x00,
    // 0x27 "'"
    0x60, 0x60, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
    // 0x28 '('
    0x18, 0x30, 0x60, 0x60, 0x60, 0x30, 0x18, 0x00,
    // 0x29 ')'
    0x60, 0x30, 0x18, 0x18, 0x18, 0x30, 0x60, 0x00,
    // 0x2a '*'
    0x00, 0x66, 0x3c, 0xff, 0x3c, 0x66, 0x00, 0x00,
    // 0x2b '+'
    0x00, 0x30, 0x30, 0xfc, 0x30, 0x30, 0x00, 0x00,
    // 0x2c ','
    0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x30, 0x60,
    // 0x2d '-'
    0x00, 0x00, 0x00, 0xfc, 0x00, 0x00, 0x00, 0x00,
    // 0x2e '.'
    0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x30, 0x00,
    // 0x2f '/'
    0x06, 0x0c, 0x18, 0x30, 0x60, 0xc0, 0x80, 0x00,
    // 0x30 '0'
    0x7c, 0xc6, 0xce, 0xde, 0xf6, 0xe6, 0x7c, 0x00,
    // 0x31 '1'
    0x30, 0x70, 0x30, 0x30, 0x30, 0x30, 0xfc, 0x00,
    // 0x32 '2'
    0x78, 0xcc, 0x0c, 0x38, 0x60, 0xcc, 0xfc, 0x00,
    // 0x33 '3'
    0x78, 0xcc, 0x0c, 0x38, 0x0c, 0xcc, 0x78, 0x00,
    // 0x34 '4'
    0x1c, 0x3c, 0x6c, 0xcc, 0xfe, 0x0c, 0x1e, 0x00,
    // 0x35 '5'
    0xfc, 0xc0, 0xf8, 0x0c, 0x0c, 0xcc, 0x78, 0x00,
    // 0x36 '6'
    0x38, 0x60, 0xc0, 0xf8, 0xcc, 0xcc, 0x78, 0x00,
    // 0x37 '7'
    0xfc, 0xcc, 0x0c, 0x18, 0x30, 0x30, 0x30, 0x00,
    // 0x38 '8'
    0x78, 0xcc, 0xcc, 0x78, 0xcc, 0xcc, 0x78, 0x00,
    // 0x39 '9'
    0x78, 0xcc, 0xcc, 0x7c, 0x0c, 0x18, 0x70, 0x00,
    // 0x3a ':'
    0x00, 0x30, 0x30, 0x00, 0x00, 0x30, 0x30, 0x00,
    // 0x3b ';'
    0x00, 0x30, 0x30, 0x00, 0x00, 0x30, 0x30, 0x60,
    // 0x3c '<'
    0x18, 0x30, 0x60, 0xc0, 0x60, 0x30, 0x18, 0x00,
    // 0x3d '='
    0x00, 0x00, 0xfc, 0x00, 0x00, 0xfc, 0x00, 0x00,
    // 0x3e '>'
    0x60, 0x30, 0x18, 0x0c, 0x18, 0x30, 0x60, 0x00,
    // 0x3f '?'
    0x78, 0xcc, 0x0c, 0x18, 0x30, 0x00, 0x30, 0x00,
    // 0x40 '@'
    0x7c, 0xc6, 0xde, 0xde, 0xde, 0xc0, 0x78, 0x00,
    // 0x41 'A'
    0x30, 0x78, 0xcc, 0xcc, 0xfc, 0xcc, 0xcc, 0x00,
    // 0x42 'B'
    0xfc, 0x66, 0x66, 0x7c, 0x66, 0x66, 0xfc, 0x00,
    // 0x43 'C'
    0x3c, 0x66, 0xc0, 0xc0, 0xc0, 0x66, 0x3c, 0x00,
    // 0x44 'D'
    0xf8, 0x6c, 0x66, 0x66, 0x66, 0x6c, 0xf8, 0x00,
    // 0x45 'E'
    0xfe, 0x62, 0x68, 0x78, 0x68, 0x62, 0xfe, 0x00,
    // 0x46 'F'
    0xfe, 0x62, 0x68, 0x78, 0x68, 0x60, 0xf0, 0x00,
    // 0x47 'G'
    0x3c, 0x66, 0xc0, 0xc0, 0xce, 0x66, 0x3e, 0x00,
    // 0x48 'H'
    0xcc, 0xcc, 0xcc, 0xfc, 0xcc, 0xcc, 0xcc, 0x00,
    // 0x49 'I'
    0x78, 0x30, 0x30, 0x30, 0x30, 0x30, 0x78, 0x00,
    // 0x4a 'J'
    0x1e, 0x0c, 0x0c, 0x0c, 0xcc, 0xcc, 0x78, 0x00,
    // 0x4b 'K'
    0xe6, 0x66, 0x6c, 0x78, 0x6c, 0x66, 0xe6, 0x00,
    // 0x4c 'L'
    0xf0, 0x60, 0x60, 0x60, 0x62, 0x66, 0xfe, 0x00,
    // 0x4d 'M'
    0xc6, 0xee, 0xfe, 0xfe, 0xd6, 0xc6, 0xc6, 0x00,
    // 0x4e 'N'
    0xc6, 0xe6, 0xf6, 0xde, 0xce, 0xc6, 0xc6, 0x00,
    // 0x4f 'O'
    0x38, 0x6c, 0xc6, 0xc6, 0xc6, 0x6c, 0x38, 0x00,
    // 0x50 'P'
    0xfc, 0x66, 0x66, 0x7c, 0x60, 0x60, 0xf0, 0x00,
    // 0x51 'Q'
    0x78, 0xcc, 0xcc, 0xcc, 0xdc, 0x78, 0x1c, 0x00,
    // 0x52 'R'
    0xfc, 0x66, 0x66, 0x7c, 0x6c, 0x66, 0xe6, 0x00,
    // 0x53 'S'
    0x78, 0xcc, 0xe0, 0x70, 0x1c, 0xcc, 0x78, 0x00,
    // 0x54 'T'
    0xfc, 0xb4, 0x30, 0x30, 0x30, 0x30, 0x78, 0x00,
    // 0x55 'U'
    0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xfc, 0x00,
    // 0x56 'V'
    0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0x78, 0x30, 0x00,
    // 0x57 'W'
    0xc6, 0xc6, 0xc6, 0xd6, 0xfe, 0xee, 0xc6, 0x00,
    // 0x58 'X'
    0xc6, 0xc6, 0x6c, 0x38, 0x38, 0x6c, 0xc6, 0x00,
    // 0x59 'Y'
    0xcc, 0xcc, 0xcc, 0x78, 0x30, 0x30, 0x78, 0x00,
    // 0x5a 'Z'
    0xfe, 0xc6, 0x8c, 0x18, 0x32, 0x66, 0xfe, 0x00,
    // 0x5b '['
    0x78, 0x60, 0x60, 0x60, 0x60, 0x60, 0x78, 0x00,
    // 0x5c '\\'
    0xc0, 0x60, 0x30, 0x18, 0x0c, 0x06, 0x02, 0x00,
    // 0x5d ']'
    0x78, 0x18, 0x18, 0x18, 0x18, 0x18, 0x78, 0x00,
    // 0x5e '^'
    0x10, 0x38, 0x6c, 0xc6, 0x00, 0x00, 0x00, 0x00,
    // 0x5f '_'
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
    // 0x60 '`'
    0x30, 0x30, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00,
    // 0x61 'a'
    0x00, 0x00, 0x78, 0x0c, 0x7c, 0xcc, 0x76, 0x00,
    // 0x62 'b'
    0xe0, 0x60, 0x60, 0x7c, 0x66, 0x66, 0xdc, 0x00,
    // 0x63 'c'
    0x00, 0x00, 0x78, 0xcc, 0xc0, 0xcc, 0x78, 0x00,
    // 0x64 'd'
    0x1c, 0x0c, 0x0c, 0x7c, 0xcc, 0xcc, 0x76, 0x00,
    // 0x65 'e'
    0x00, 0x00, 0x78, 0xcc, 0xfc, 0xc0, 0x78, 0x00,
    // 0x66 'f'
    0x38, 0x6c, 0x60, 0xf0, 0x60, 0x60, 0xf0, 0x00,
    // 0x67 'g'
    0x00, 0x00, 0x76, 0xcc, 0xcc, 0x7c, 0x0c, 0xf8,
    // 0x68 'h'
    0xe0, 0x60, 0x6c, 0x76, 0x66, 0x66, 0xe6, 0x00,
    // 0x69 'i'
    0x30, 0x00, 0x70, 0x30, 0x30, 0x30, 0x78, 0x00,
    // 0x6a 'j'
    0x0c, 0x00, 0x0c, 0x0c, 0x0c, 0xcc, 0xcc, 0x78,
    // 0x6b 'k'
    0xe0, 0x60, 0x66, 0x6c, 0x78, 0x6c, 0xe6, 0x00,
    // 0x6c 'l'
    0x70, 0x30, 0x30, 0x30, 0x30, 0x30, 0x78, 0x00,
    // 0x6d 'm'
    0x00, 0x00, 0xcc, 0xfe, 0xfe, 0xd6, 0xc6, 0x00,
    // 0x6e 'n'
    0x00, 0x00, 0xf8, 0xcc, 0xcc, 0xcc, 0xcc, 0x00,
    // 0x6f 'o'
    0x00, 0x00, 0x78, 0xcc, 0xcc, 0xcc, 0x78, 0x00,
    // 0x70 'p'
    0x00, 0x00, 0xdc, 0x66, 0x66, 0x7c, 0x60, 0xf0,
    // 0x71 'q'
    0x00, 0x00, 0x76, 0xcc, 0xcc, 0x7c, 0x0c, 0x1e,
    // 0x72 'r'
    0x00, 0x00, 0xdc, 0x76, 0x66, 0x60, 0xf0, 0x00,
    // 0x73 's'
    0x00, 0x00, 0x7c, 0xc0, 0x78, 0x0c, 0xf8, 0x00,
    // 0x74 't'
    0x10, 0x30, 0x7c, 0x30, 0x30, 0x34, 0x18, 0x00,
    // 0x75 'u'
    0x00, 0x00, 0xcc, 0xcc, 0xcc, 0xcc, 0x76, 0x00,
    // 0x76 'v'
    0x00, 0x00, 0xcc, 0xcc, 0xcc, 0x78, 0x30, 0x00,
    // 0x77 'w'
    0x00, 0x00, 0xc6, 0xd6, 0xfe, 0xfe, 0x6c, 0x00,
    // 0x78 'x'
    0x00, 0x00, 0xc6, 0x6c, 0x38, 0x6c, 0xc6, 0x00,
    // 0x79 'y'
    0x00, 0x00, 0xcc, 0xcc, 0xcc, 0x7c, 0x0c, 0xf8,
    // 0x7a 'z'
    0x00, 0x00, 0xfc, 0x98, 0x30, 0x64, 0xfc, 0x00,
    // 0x7b '{'
    0x1c, 0x30, 0x30, 0xe0, 0x30, 0x30, 0x1c, 0x00,
    // 0x7c '|'
    0x18, 0x18, 0x18, 0x00, 0x18, 0x18, 0x18, 0x00,
    // 0x7d '}'
    0xe0, 0x30, 0x30, 0x1c, 0x30, 0x30, 0xe0, 0x00,
    // 0x7e '~'
    0x76, 0xdc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];




fn sys_colorref(index: u32) -> u32 {
    match index & 0xff {
        0 => 0x00c0_c0c0,
        1 => 0x0000_8080,
        5 => 0x00ff_ffff,
        6 => 0x0000_0000,
        8 => 0x0000_0000,
        13 => 0x0080_0000,
        14 => 0x00ff_ffff,
        15 => 0x00c0_c0c0,
        16 => 0x0080_8080,
        18 => 0x0000_0000,
        20 => 0x00ff_ffff,
        _ => 0x00c0_c0c0,
    }
}











const RT_BITMAP: u32 = 2;
const RT_MENU: u32 = 4;
const RT_DIALOG: u32 = 5;
const RT_STRING: u32 = 6;
const RT_ACCELERATOR: u32 = 9;
const LANG_ENGLISH_US: u32 = 0x0409;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_DLGFRAME: u32 = 0x0040_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_CAPTION: u32 = WS_BORDER | WS_DLGFRAME;
const DS_SETFONT: u32 = 0x0000_0040;
const DS_SHELLFONT: u32 = 0x0000_0048;
const GWL_WNDPROC: i32 = -4;
const GWL_HWNDPARENT: i32 = -8;
const GWL_ID: i32 = -12;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const GWLP_USERDATA: i32 = -21;
const DIALOG_TITLE_HEIGHT: i32 = 24;
const DIALOG_BORDER: i32 = 4;
const MENU_BAR_HEIGHT: i32 = 20;

fn style_has_hle_frame(style: u32) -> bool {
    (style & (WS_CAPTION | WS_THICKFRAME)) != 0
}


#[derive(Clone)]
struct DialogTemplate {
    style: u32,
    ex_style: u32,
    x: i16,
    y: i16,
    cx: i16,
    cy: i16,
    title: String,
    controls: Vec<DialogControlTemplate>,
}

#[derive(Clone)]
struct DialogControlTemplate {
    style: u32,
    ex_style: u32,
    id: u32,
    class_name: String,
    text: String,
    x: i16,
    y: i16,
    cx: i16,
    cy: i16,
}

struct TemplateReader {
    base: u32,
    size: u32,
    off: u32,
}

impl TemplateReader {
    fn new(base: u32, size: u32) -> Self {
        Self { base, size, off: 0 }
    }

    fn addr(&self) -> Option<u32> {
        (self.off <= self.size).then_some(self.base.wrapping_add(self.off))
    }

    fn remaining(&self) -> u32 {
        self.size.saturating_sub(self.off)
    }

    fn read_u8(&mut self, mem: &Memory) -> Option<u8> {
        if self.remaining() < 1 {
            return None;
        }
        let value = mem.read_u8(self.base.wrapping_add(self.off)).ok()?;
        self.off = self.off.wrapping_add(1);
        Some(value)
    }

    fn read_u16(&mut self, mem: &Memory) -> Option<u16> {
        if self.remaining() < 2 {
            return None;
        }
        let value = mem.read_u16(self.base.wrapping_add(self.off)).ok()?;
        self.off = self.off.wrapping_add(2);
        Some(value)
    }

    fn read_i16(&mut self, mem: &Memory) -> Option<i16> {
        Some(self.read_u16(mem)? as i16)
    }

    fn read_u32(&mut self, mem: &Memory) -> Option<u32> {
        if self.remaining() < 4 {
            return None;
        }
        let value = mem.read_u32(self.base.wrapping_add(self.off)).ok()?;
        self.off = self.off.wrapping_add(4);
        Some(value)
    }

    fn align4(&mut self) {
        self.off = (self.off + 3) & !3;
    }

    fn skip(&mut self, bytes: u32) -> Option<()> {
        if self.remaining() < bytes {
            return None;
        }
        self.off = self.off.wrapping_add(bytes);
        Some(())
    }
}


#[derive(Clone, Debug)]
enum ResourceKey {
    Id(u32),
    Name(String),
}

























fn read_template_name(mem: &Memory, r: &mut TemplateReader) -> Option<String> {
    let first = r.read_u16(mem)?;
    if first == 0 {
        return Some(String::new());
    }
    if first == 0xffff {
        let atom = r.read_u16(mem)?;
        return Some(template_atom_name(atom).to_string());
    }
    let mut units = vec![first];
    while r.remaining() >= 2 {
        let unit = r.read_u16(mem)?;
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Some(String::from_utf16_lossy(&units))
}

fn template_atom_name(atom: u16) -> &'static str {
    match atom {
        0x0080 => "Button",
        0x0081 => "Edit",
        0x0082 => "Static",
        0x0083 => "ListBox",
        0x0084 => "ScrollBar",
        0x0085 => "ComboBox",
        _ => "#",
    }
}

















fn rgb_to_565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

fn write_msg(emu: &mut Emulator, out: u32, message: Message) {
    emu.memory.write_u32(out, message.hwnd).hle();
    emu.memory.write_u32(out + 4, message.msg).hle();
    emu.memory.write_u32(out + 8, message.wparam).hle();
    emu.memory.write_u32(out + 12, message.lparam).hle();
    emu.memory.write_u32(out + 16, emu.guest_time_ms as u32).hle();
    emu.memory.write_u32(out + 20, 0).hle();
    emu.memory.write_u32(out + 24, 0).hle();
}




fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32)
}

fn ansi_len(mem: &Memory, s: u32, cap: usize) -> usize {
    let mut len = 0u32;
    while (len as usize) < cap {
        if mem.read_u8(s.wrapping_add(len)).hle() == 0 {
            break;
        }
        len += 1;
    }
    len as usize
}

fn compare_ansi_strings(emu: &Emulator, a: u32, b: u32, ignore_case: bool) -> i32 {
    for i in 0..(1u32 << 20) {
        let mut av = emu.memory.read_u8(a + i).hle();
        let mut bv = emu.memory.read_u8(b + i).hle();
        if ignore_case {
            av = av.to_ascii_lowercase();
            bv = bv.to_ascii_lowercase();
        }
        if av != bv || av == 0 {
            return av as i32 - bv as i32;
        }
    }
    0
}



















const WIN_FACE: [u8; 4] = [198, 198, 198, 255];
const WIN_SHADOW: [u8; 4] = [128, 128, 128, 255];
const WIN_HIGHLIGHT: [u8; 4] = [255, 255, 255, 255];
const WIN_TEXT: [u8; 4] = [0, 0, 0, 255];
const WIN_DISABLED_TEXT: [u8; 4] = [128, 128, 128, 255];




fn draw_framebuffer_text_left(
    emu: &mut Emulator,
    text: &str,
    x: i32,
    y: i32,
    rect: RectI,
    color: [u8; 4],
) {
    let bytes = narrow_text_bytes(text);
    if bytes.is_empty() {
        return;
    }
    draw_framebuffer_text_bytes(emu, &bytes, x, y, rect, color);
}

fn draw_framebuffer_text_right(emu: &mut Emulator, text: &str, rect: RectI, color: [u8; 4]) {
    let bytes = narrow_text_bytes(text);
    if bytes.is_empty() {
        return;
    }
    let metrics = hle_dialog_text_metrics();
    let width = (bytes.len() as i32).saturating_mul(metrics.char_width);
    let x = rect.right.saturating_sub(width).saturating_sub(5);
    let y = rect.top + (rect.height().saturating_sub(metrics.height) / 2).max(1);
    draw_framebuffer_text_bytes(emu, &bytes, x, y, rect, color);
}

fn draw_framebuffer_text_bytes(
    emu: &mut Emulator,
    bytes: &[u8],
    x: i32,
    y: i32,
    rect: RectI,
    color: [u8; 4],
) {
    let metrics = hle_dialog_text_metrics();
    let clip = Some((rect.left + 2, rect.top + 2, rect.right - 2, rect.bottom - 2));
    let mut cursor_x = x;
    let mut glyphs = 0i32;
    for glyph in gdi_glyphs(bytes, metrics.char_width) {
        if glyphs != 0 {
            cursor_x = cursor_x.saturating_add(metrics.extra);
        }
        if glyph.visible {
            draw_framebuffer_glyph_bitmap(
                emu,
                color,
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
}






fn draw_inset_edge(emu: &mut Emulator, surface: SurfaceInfo, rect: RectI) {
    draw_rect_outline(emu, surface, rect, 0x8410, 1);
    let inner = RectI {
        left: rect.left + 1,
        top: rect.top + 1,
        right: rect.right - 1,
        bottom: rect.bottom - 1,
    };
    draw_rect_outline(emu, surface, inner, 0xffff, 1);
}


fn draw_text_center(emu: &mut Emulator, surf: u32, text: &str, rect: RectI, colorref: u32) {
    let bytes = narrow_text_bytes(text);
    if bytes.is_empty() {
        return;
    }
    let metrics = hle_dialog_text_metrics();
    let width = (bytes.len() as i32).saturating_mul(metrics.char_width);
    let x = rect.left + (rect.width().saturating_sub(width) / 2).max(2);
    let y = rect.top + (rect.height().saturating_sub(metrics.height) / 2).max(1);
    draw_text_bytes(emu, surf, &bytes, x, y, rect, colorref);
}

fn draw_text_right(emu: &mut Emulator, surf: u32, text: &str, rect: RectI, colorref: u32) {
    let bytes = narrow_text_bytes(text);
    if bytes.is_empty() {
        return;
    }
    let metrics = hle_dialog_text_metrics();
    let width = (bytes.len() as i32).saturating_mul(metrics.char_width);
    let x = rect.right.saturating_sub(width).saturating_sub(5);
    let y = rect.top + (rect.height().saturating_sub(metrics.height) / 2).max(1);
    draw_text_bytes(emu, surf, &bytes, x, y, rect, colorref);
}

fn draw_text_left(
    emu: &mut Emulator,
    surf: u32,
    text: &str,
    x: i32,
    y: i32,
    rect: RectI,
    colorref: u32,
) {
    let bytes = narrow_text_bytes(text);
    if !bytes.is_empty() {
        draw_text_bytes(emu, surf, &bytes, x, y, rect, colorref);
    }
}

fn draw_text_bytes(
    emu: &mut Emulator,
    surf: u32,
    bytes: &[u8],
    x: i32,
    y: i32,
    rect: RectI,
    colorref: u32,
) {
    draw_gdi_text(
        emu,
        GdiDc {
            surface: surf,
            hwnd: 0,
            selected_font: 0,
            selected_bitmap: 0,
            selected_brush: 0,
            selected_pen: 0,
            selected_palette: 0,
            rop2: R2_COPYPEN,
            layout: 0,
            map_mode: MM_TEXT,
            text_align: TA_LEFT | TA_TOP,
            text_extra: 0,
            text_color: colorref,
            bk_color: 0x00ff_ffff,
            bk_mode: 1,
            origin_x: 0,
            origin_y: 0,
            brush_origin_x: 0,
            brush_origin_y: 0,
            current_x: 0,
            current_y: 0,
        },
        bytes,
        x,
        y,
        hle_dialog_text_metrics(),
        Some((rect.left + 2, rect.top + 2, rect.right - 2, rect.bottom - 2)),
    );
}


fn narrow_text_bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| if ch.is_ascii() { ch as u8 } else { b'?' })
        .collect()
}











#[derive(Clone, Copy)]
struct CreateWindowArgs {
    ex_style: u32,
    class_ptr: u32,
    name_ptr: u32,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: u32,
    menu: u32,
    inst: u32,
    param: u32,
}




fn wide_text_bytes(emu: &Emulator, text: u32, count: u32) -> Vec<u8> {
    if text == 0 {
        return Vec::new();
    }
    let max = if count == u32::MAX {
        wide_len(&emu.memory, text, 4096)
    } else {
        count.min(4096) as usize
    };
    let mut s = String::new();
    for i in 0..max {
        let unit = emu.memory.read_u16(text + (i as u32 * 2)).hle();
        if unit == 0 {
            break;
        }
        let ch = char::from_u32(unit as u32).unwrap_or(' ');
        s.push(if ch.is_ascii() { ch } else { '?' });
    }
    s.into_bytes()
}

fn wide_len(mem: &Memory, addr: u32, max: usize) -> usize {
    if addr == 0 {
        return 0;
    }
    for i in 0..max {
        if mem.read_u16(addr + (i as u32 * 2)).unwrap_or(0) == 0 {
            return i;
        }
    }
    max
}


#[cfg(test)]
mod tests {
    use super::*;

    const TEST_STACK: u32 = 0x0001_0000;
    const TEST_DATA: u32 = 0x0001_1000;

    fn dummy_entry(callback: HleCallback) -> HleEntry {
        HleEntry {
            addr: 0,
            dll: "test.dll",
            name: "test",
            callback,
        }
    }

    fn write_test_rect(emu: &mut Emulator, addr: u32, rect: (i32, i32, i32, i32)) {
        emu.memory.write_u32(addr, rect.0 as u32).unwrap();
        emu.memory.write_u32(addr + 4, rect.1 as u32).unwrap();
        emu.memory.write_u32(addr + 8, rect.2 as u32).unwrap();
        emu.memory.write_u32(addr + 12, rect.3 as u32).unwrap();
    }

    fn read_test_rect(emu: &Emulator, addr: u32) -> (i32, i32, i32, i32) {
        (
            emu.memory.read_u32(addr).unwrap() as i32,
            emu.memory.read_u32(addr + 4).unwrap() as i32,
            emu.memory.read_u32(addr + 8).unwrap() as i32,
            emu.memory.read_u32(addr + 12).unwrap() as i32,
        )
    }

    fn prepare_hle_stack(emu: &mut Emulator) {
        emu.memory
            .map(TEST_STACK, 0x3000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK);
        emu.memory.write_u32(TEST_STACK, 0x0040_0000).unwrap();
    }

    fn write_arg(emu: &mut Emulator, index: u32, value: u32) {
        emu.memory
            .write_u32(TEST_STACK + 4 + index * 4, value)
            .unwrap();
    }

    fn register_test_window(emu: &mut Emulator, hwnd: u32, parent: u32, proc: u32) {
        emu.hle.register_window(HleWindow {
            hwnd,
            parent,
            id: 0,
            class_name: "TestWindow".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 10,
                top: 20,
                right: 160,
                bottom: 140,
            },
            style: WS_VISIBLE,
            ex_style: 0,
            proc,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Window,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });
    }

    fn test_message(hwnd: u32, msg: u32, wparam: u32) -> Message {
        Message {
            hwnd,
            msg,
            wparam,
            lparam: 0,
        }
    }

    fn setup_peek_message_args(emu: &mut Emulator, remove: u32) {
        write_arg(emu, 0, TEST_DATA);
        write_arg(emu, 1, 0);
        write_arg(emu, 2, 0);
        write_arg(emu, 3, 0);
        write_arg(emu, 4, remove);
    }

    #[test]
    fn peek_message_no_remove_keeps_message() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        setup_peek_message_args(&mut emu, 0);
        emu.hle
            .app_messages
            .push(test_message(0x0002_0001, 0x0400, 0x55));

        assert_eq!(
            hle_peek_message_a(&mut emu, &dummy_entry(hle_peek_message_a)),
            HleResult::Retn(20)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 1);
        assert_eq!(emu.memory.read_u32(TEST_DATA + 4).unwrap(), 0x0400);
        assert_eq!(emu.hle.app_messages.len(), 1);
    }

    #[test]
    fn peek_message_remove_dequeues_normal_message() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        setup_peek_message_args(&mut emu, 1);
        emu.hle
            .app_messages
            .push(test_message(0x0002_0001, 0x0400, 0x55));

        assert_eq!(
            hle_peek_message_a(&mut emu, &dummy_entry(hle_peek_message_a)),
            HleResult::Retn(20)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 1);
        assert_eq!(emu.memory.read_u32(TEST_DATA).unwrap(), 0x0002_0001);
        assert_eq!(emu.memory.read_u32(TEST_DATA + 4).unwrap(), 0x0400);
        assert!(emu.hle.app_messages.is_empty());
    }

    #[test]
    fn peek_message_prefers_input_queue() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        setup_peek_message_args(&mut emu, 1);
        emu.hle
            .app_messages
            .push(test_message(0x0002_0001, 0x0400, 0x11));
        emu.hle
            .input_messages
            .push(test_message(0x0002_0001, 0x0100, 0x41));

        assert_eq!(
            hle_peek_message_a(&mut emu, &dummy_entry(hle_peek_message_a)),
            HleResult::Retn(20)
        );

        assert_eq!(emu.memory.read_u32(TEST_DATA + 4).unwrap(), 0x0100);
        assert!(emu.hle.input_messages.is_empty());
        assert_eq!(emu.hle.app_messages.len(), 1);
    }

    #[test]
    fn peek_message_pm_remove_keeps_paint_message_queued() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        setup_peek_message_args(&mut emu, 1);
        emu.hle
            .app_messages
            .push(test_message(0x0002_0001, 0x000f, 0));

        assert_eq!(
            hle_peek_message_a(&mut emu, &dummy_entry(hle_peek_message_a)),
            HleResult::Retn(20)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 1);
        assert_eq!(emu.memory.read_u32(TEST_DATA + 4).unwrap(), 0x000f);
        assert_eq!(emu.hle.app_messages.len(), 1);
    }

    #[test]
    fn peek_message_does_not_pump_due_user_timer() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        setup_peek_message_args(&mut emu, 0);
        let target = emu.hle.delay_target(10, 0);
        emu.hle.set_timer(0, 1, 0, 0, 0, target);
        emu.insns = 10 * crate::HEADLESS_INSNS_PER_MS;

        assert_eq!(
            hle_peek_message_a(&mut emu, &dummy_entry(hle_peek_message_a)),
            HleResult::Retn(20)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 0);
        assert!(emu.hle.app_messages.is_empty());
        assert_eq!(emu.hle.timers[0].due_count, 0);

        emu.service_guest().unwrap();
        assert_eq!(emu.hle.app_messages.len(), 1);
        assert_eq!(emu.hle.app_messages[0].msg, 0x0113);
        assert_eq!(emu.hle.timers[0].due_count, 1);
    }

    #[test]
    fn get_message_quit_returns_zero_and_removes_message() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        write_arg(&mut emu, 0, TEST_DATA);
        write_arg(&mut emu, 1, 0);
        write_arg(&mut emu, 2, 0);
        write_arg(&mut emu, 3, 0);
        emu.hle.app_messages.push(test_message(0, 0x0012, 7));

        assert_eq!(
            hle_get_message_a(&mut emu, &dummy_entry(hle_get_message_a)),
            HleResult::Retn(16)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 0);
        assert_eq!(emu.memory.read_u32(TEST_DATA + 4).unwrap(), 0x0012);
        assert!(emu.hle.app_messages.is_empty());
    }

    #[test]
    fn message_filter_matches_child_window_and_message_range() {
        let mut emu = Emulator::new();
        register_test_window(&mut emu, 0x0002_0001, 0, 0x0040_1000);
        register_test_window(&mut emu, 0x0002_0005, 0x0002_0001, 0x0040_1000);
        emu.hle
            .app_messages
            .push(test_message(0x0002_0005, 0x0111, 0x77));

        assert!(emu
            .hle
            .has_matching_message(MessageFilter::new(0x0002_0001, 0x0100, 0x0200)));
        assert!(!emu
            .hle
            .has_matching_message(MessageFilter::new(0x0002_0009, 0x0100, 0x0200)));
        assert!(!emu
            .hle
            .has_matching_message(MessageFilter::new(0x0002_0001, 0x0201, 0x0201)));
    }

    #[test]
    fn translate_accelerator_queues_command_for_matching_key() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let table = emu.hle.alloc_accelerator_handle(HleAcceleratorTable {
            items: vec![HleAccelerator {
                flags: ACCEL_FVIRTKEY,
                key: 0x70,
                cmd: 0x1234,
            }],
        });
        emu.memory.write_u32(TEST_DATA, 0x0002_0001).unwrap();
        emu.memory.write_u32(TEST_DATA + 4, 0x0100).unwrap();
        emu.memory.write_u32(TEST_DATA + 8, 0x70).unwrap();
        emu.memory.write_u32(TEST_DATA + 12, 0).unwrap();
        write_arg(&mut emu, 0, 0x0002_0001);
        write_arg(&mut emu, 1, table);
        write_arg(&mut emu, 2, TEST_DATA);

        assert_eq!(
            hle_translate_accelerator_a(&mut emu, &dummy_entry(hle_translate_accelerator_a)),
            HleResult::Retn(12)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 1);
        assert_eq!(emu.hle.app_messages.len(), 1);
        assert_eq!(emu.hle.app_messages[0].hwnd, 0x0002_0001);
        assert_eq!(emu.hle.app_messages[0].msg, 0x0111);
        assert_eq!(emu.hle.app_messages[0].wparam, 0x1234);
    }

    #[test]
    fn virtual_alloc_free_reuses_released_region() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        write_arg(&mut emu, 0, 0);
        write_arg(&mut emu, 1, 0x10000);
        write_arg(&mut emu, 2, 0x1000);
        write_arg(&mut emu, 3, 0x40);

        assert_eq!(
            hle_virtual_alloc(&mut emu, &dummy_entry(hle_virtual_alloc)),
            HleResult::Retn(16)
        );
        let first = emu.cpu.reg(Reg::Eax);
        assert_eq!(first, VIRTUAL_BASE);
        assert!(emu.memory.is_mapped(first, PagePerm::WRITE));

        write_arg(&mut emu, 0, first);
        write_arg(&mut emu, 1, 0);
        write_arg(&mut emu, 2, 0x8000);
        assert_eq!(
            hle_virtual_free(&mut emu, &dummy_entry(hle_virtual_free)),
            HleResult::Retn(12)
        );
        assert!(!emu.memory.is_mapped(first, PagePerm::WRITE));

        write_arg(&mut emu, 0, 0);
        write_arg(&mut emu, 1, 0x8000);
        write_arg(&mut emu, 2, 0x1000);
        write_arg(&mut emu, 3, 0x04);
        hle_virtual_alloc(&mut emu, &dummy_entry(hle_virtual_alloc));

        assert_eq!(emu.cpu.reg(Reg::Eax), first);
        assert!(emu.memory.is_mapped(first, PagePerm::WRITE));
    }

    #[test]
    fn virtual_alloc_recommits_pages_inside_existing_region() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        write_arg(&mut emu, 0, 0);
        write_arg(&mut emu, 1, 0x10000);
        write_arg(&mut emu, 2, 0x1000);
        write_arg(&mut emu, 3, 0x04);
        hle_virtual_alloc(&mut emu, &dummy_entry(hle_virtual_alloc));
        let base = emu.cpu.reg(Reg::Eax);
        let page = base + 0x2000;

        write_arg(&mut emu, 0, page);
        write_arg(&mut emu, 1, 0x1000);
        write_arg(&mut emu, 2, 0x4000);
        hle_virtual_free(&mut emu, &dummy_entry(hle_virtual_free));
        assert!(!emu.memory.is_mapped(page, PagePerm::WRITE));

        write_arg(&mut emu, 0, page);
        write_arg(&mut emu, 1, 0x1000);
        write_arg(&mut emu, 2, 0x1000);
        write_arg(&mut emu, 3, 0x04);
        hle_virtual_alloc(&mut emu, &dummy_entry(hle_virtual_alloc));

        assert_eq!(emu.cpu.reg(Reg::Eax), page);
        assert!(emu.memory.is_mapped(page, PagePerm::WRITE));
    }

    #[test]
    fn virtual_query_reports_arena_region() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        write_arg(&mut emu, 0, 0);
        write_arg(&mut emu, 1, 0x1234);
        write_arg(&mut emu, 2, 0x1000);
        write_arg(&mut emu, 3, 0x40);
        hle_virtual_alloc(&mut emu, &dummy_entry(hle_virtual_alloc));
        let base = emu.cpu.reg(Reg::Eax);
        let mbi = TEST_STACK + 0x800;

        write_arg(&mut emu, 0, base + 0x10);
        write_arg(&mut emu, 1, mbi);
        write_arg(&mut emu, 2, 28);
        assert_eq!(
            hle_virtual_query(&mut emu, &dummy_entry(hle_virtual_query)),
            HleResult::Retn(12)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 28);
        assert_eq!(emu.memory.read_u32(mbi).unwrap(), base);
        assert_eq!(emu.memory.read_u32(mbi + 4).unwrap(), base);
        assert_eq!(emu.memory.read_u32(mbi + 8).unwrap(), 0x40);
        assert_eq!(emu.memory.read_u32(mbi + 12).unwrap(), 0x10000);
        assert_eq!(emu.memory.read_u32(mbi + 16).unwrap(), 0x1000);
        assert_eq!(emu.memory.read_u32(mbi + 20).unwrap(), 0x40);
        assert_eq!(emu.memory.read_u32(mbi + 24).unwrap(), 0x20000);
    }

    #[test]
    fn intersect_rect_writes_destination() {
        let mut emu = Emulator::new();
        emu.memory
            .map(TEST_STACK, 0x3000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK);
        emu.memory.write_u32(TEST_STACK + 4, TEST_DATA).unwrap();
        emu.memory.write_u32(TEST_STACK + 8, TEST_DATA + 0x10).unwrap();
        emu.memory.write_u32(TEST_STACK + 0x0c, TEST_DATA + 0x20).unwrap();
        write_test_rect(&mut emu, TEST_DATA + 0x10, (10, 20, 50, 70));
        write_test_rect(&mut emu, TEST_DATA + 0x20, (30, 10, 80, 60));

        assert_eq!(
            hle_intersect_rect(&mut emu, &dummy_entry(hle_intersect_rect)),
            HleResult::Retn(12)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 1);
        assert_eq!(read_test_rect(&emu, TEST_DATA), (30, 20, 50, 60));
    }

    #[test]
    fn intersect_rect_clears_empty_destination() {
        let mut emu = Emulator::new();
        emu.memory
            .map(TEST_STACK, 0x3000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK);
        emu.memory.write_u32(TEST_STACK + 4, TEST_DATA).unwrap();
        emu.memory.write_u32(TEST_STACK + 8, TEST_DATA + 0x10).unwrap();
        emu.memory.write_u32(TEST_STACK + 0x0c, TEST_DATA + 0x20).unwrap();
        write_test_rect(&mut emu, TEST_DATA, (1, 2, 3, 4));
        write_test_rect(&mut emu, TEST_DATA + 0x10, (0, 0, 10, 10));
        write_test_rect(&mut emu, TEST_DATA + 0x20, (20, 20, 30, 30));

        hle_intersect_rect(&mut emu, &dummy_entry(hle_intersect_rect));

        assert_eq!(emu.cpu.reg(Reg::Eax), 0);
        assert_eq!(read_test_rect(&emu, TEST_DATA), (0, 0, 0, 0));
    }

    #[test]
    fn draw_text_calc_rect_uses_longest_crlf_line() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let text = TEST_DATA;
        let rect = TEST_DATA + 0x100;
        emu.memory
            .write_cstr(text, "AAAAAAAAAA\r\nBB", 64)
            .unwrap();
        write_test_rect(&mut emu, rect, (0, 0, 512, 200));
        write_arg(&mut emu, 0, 0);
        write_arg(&mut emu, 1, text);
        write_arg(&mut emu, 2, u32::MAX);
        write_arg(&mut emu, 3, rect);
        write_arg(&mut emu, 4, 0x0400);

        assert_eq!(
            hle_draw_text_a(&mut emu, &dummy_entry(hle_draw_text_a)),
            HleResult::Retn(20)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 32);
        assert_eq!(read_test_rect(&emu, rect), (0, 0, 80, 32));
    }

    #[test]
    fn draw_text_clips_to_rect() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let surf = create_surface(&mut emu, 0).unwrap();
        let surface = read_surface_info(&emu, surf).unwrap();
        let hdc = emu.hle.create_surface_dc(surf);
        let text = TEST_DATA;
        let rect = TEST_DATA + 0x100;
        emu.memory.write_cstr(text, "A", 2).unwrap();
        write_test_rect(&mut emu, rect, (1, 1, 4, 2));
        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, text);
        write_arg(&mut emu, 2, u32::MAX);
        write_arg(&mut emu, 3, rect);
        write_arg(&mut emu, 4, 0);

        assert_eq!(
            hle_draw_text_a(&mut emu, &dummy_entry(hle_draw_text_a)),
            HleResult::Retn(20)
        );

        let bpp = surface.bytes_per_pixel();
        let pixel = |x: u32, y: u32| surface.buffer + y * surface.pitch + x * bpp;
        assert_eq!(emu.memory.read_u16(pixel(0, 1)).unwrap(), 0);
        assert_ne!(emu.memory.read_u16(pixel(3, 1)).unwrap(), 0);
        assert_eq!(emu.memory.read_u16(pixel(4, 1)).unwrap(), 0);
        assert_eq!(emu.memory.read_u16(pixel(3, 0)).unwrap(), 0);
    }

    #[test]
    fn mci_does_not_complete_background_music_immediately() {
        assert!(mci_should_complete_notify("play vfw window from 0 notify"));
        assert!(!mci_should_complete_notify("play mid from 0 notify"));
        assert!(!mci_should_complete_notify("play cdtrack from 2 notify"));
    }

    #[test]
    fn dispatch_message_routes_timer_proc() {
        let mut emu = Emulator::new();
        emu.hle.window_proc = 0x401000;
        emu.guest_time_ms = 1234;

        assert_eq!(
            dispatch_target(&emu, 0x0002_0001, 0x0113, 0x402000),
            (0x402000, 1234)
        );
        assert_eq!(
            dispatch_target(&emu, 0x0002_0001, 0x0201, 0x55),
            (0x401000, 0x55)
        );
    }

    #[test]
    fn dispatch_message_ignores_null_hwnd_thread_messages() {
        let mut emu = Emulator::new();
        emu.hle.window_proc = 0x0040_1000;

        assert_eq!(dispatch_target(&emu, 0, 0x000f, 0x55), (0, 0x55));
    }

    #[test]
    fn dispatch_message_uses_tracked_window_proc() {
        let mut emu = Emulator::new();
        emu.hle.window_proc = 0x0040_1000;
        register_test_window(&mut emu, 0x0002_0001, 0, 0x0040_2000);

        assert_eq!(
            dispatch_target(&emu, 0x0002_0001, 0x0201, 0x55),
            (0x0040_2000, 0x55)
        );
    }

    #[test]
    fn dispatch_message_updates_tracked_edit_text() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        register_test_window(&mut emu, 0x0002_0001, 0, 0x0040_1000);
        emu.hle.register_window(HleWindow {
            hwnd: 0x0002_0005,
            parent: 0x0002_0001,
            id: 42,
            class_name: "Edit".to_string(),
            text: String::new(),
            rect: WindowRect {
                left: 20,
                top: 30,
                right: 120,
                bottom: 50,
            },
            style: WS_VISIBLE,
            ex_style: 0,
            proc: 0,
            user_data: 0,
            extra: std::collections::HashMap::new(),
            enabled: true,
            visible: true,
            control_kind: HleControlKind::Edit,
            background_brush: 0,
            invalid_rect: None,
            erase_pending: false,
            last_generated_paint_frame: 0,
            ddraw_owned: false,
        });
        emu.memory.write_u32(TEST_DATA, 0x0002_0005).unwrap();
        emu.memory.write_u32(TEST_DATA + 4, 0x0102).unwrap();
        emu.memory.write_u32(TEST_DATA + 8, b'A' as u32).unwrap();
        emu.memory.write_u32(TEST_DATA + 12, 0).unwrap();
        write_arg(&mut emu, 0, TEST_DATA);

        assert_eq!(
            hle_dispatch_message_a(&mut emu, &dummy_entry(hle_dispatch_message_a)),
            HleResult::Retn(4)
        );

        assert_eq!(emu.hle.window(0x0002_0005).unwrap().text, "A");
        assert_eq!(emu.hle.app_messages.len(), 1);
        assert_eq!(emu.hle.app_messages[0].hwnd, 0x0002_0001);
        assert_eq!(emu.hle.app_messages[0].msg, 0x0111);
        assert_eq!(emu.hle.app_messages[0].wparam, 42 | (0x0300 << 16));
        assert_eq!(emu.hle.app_messages[0].lparam, 0x0002_0005);
    }

    #[test]
    fn window_long_extra_slots_round_trip_positive_offsets() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        register_test_window(&mut emu, 0x0002_0001, 0, 0x0040_1000);

        write_arg(&mut emu, 0, 0x0002_0001);
        write_arg(&mut emu, 1, 0);
        write_arg(&mut emu, 2, 0x1234_5678);
        assert_eq!(
            hle_set_window_long_a(&mut emu, &dummy_entry(hle_set_window_long_a)),
            HleResult::Retn(12)
        );
        assert_eq!(emu.cpu.reg(Reg::Eax), 0);

        write_arg(&mut emu, 0, 0x0002_0001);
        write_arg(&mut emu, 1, 0);
        write_arg(&mut emu, 2, 0x8765_4321);
        hle_set_window_long_w(&mut emu, &dummy_entry(hle_set_window_long_w));
        assert_eq!(emu.cpu.reg(Reg::Eax), 0x1234_5678);

        write_arg(&mut emu, 0, 0x0002_0001);
        write_arg(&mut emu, 1, 0);
        assert_eq!(
            hle_get_window_long_w(&mut emu, &dummy_entry(hle_get_window_long_w)),
            HleResult::Retn(8)
        );
        assert_eq!(emu.cpu.reg(Reg::Eax), 0x8765_4321);
    }

    #[test]
    fn repainting_move_window_requeues_size_before_paint() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        register_test_window(&mut emu, 0x0002_0001, 0, 0x0040_1000);
        emu.hle.app_messages.push(Message {
            hwnd: 0x0002_0001,
            msg: 0x000f,
            wparam: 0,
            lparam: 0,
        });

        write_arg(&mut emu, 0, 0x0002_0001);
        write_arg(&mut emu, 1, 30);
        write_arg(&mut emu, 2, 40);
        write_arg(&mut emu, 3, 100);
        write_arg(&mut emu, 4, 80);
        write_arg(&mut emu, 5, 1);
        assert_eq!(
            hle_move_window(&mut emu, &dummy_entry(hle_move_window)),
            HleResult::Retn(24)
        );

        assert_eq!(emu.hle.app_messages.len(), 2);
        assert_eq!(emu.hle.app_messages[0].msg, 0x0005);
        assert_eq!(emu.hle.app_messages[0].lparam, 100 | (80 << 16));
        assert_eq!(emu.hle.app_messages[1].msg, 0x000f);
        assert!(emu.hle.window(0x0002_0001).unwrap().invalid_rect.is_some());
    }

    #[test]
    fn wm_size_reports_client_size_for_framed_window() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        register_test_window(&mut emu, 0x0002_0001, 0, 0x0040_1000);
        emu.hle.window_mut(0x0002_0001).unwrap().style = WS_VISIBLE | WS_CAPTION;

        write_arg(&mut emu, 0, 0x0002_0001);
        write_arg(&mut emu, 1, 30);
        write_arg(&mut emu, 2, 40);
        write_arg(&mut emu, 3, 100);
        write_arg(&mut emu, 4, 80);
        write_arg(&mut emu, 5, 0);
        assert_eq!(
            hle_move_window(&mut emu, &dummy_entry(hle_move_window)),
            HleResult::Retn(24)
        );

        assert_eq!(emu.hle.app_messages.len(), 1);
        assert_eq!(emu.hle.app_messages[0].msg, 0x0005);
        let expected_w = (100 - DIALOG_BORDER * 2) as u32;
        let expected_h = (80 - DIALOG_TITLE_HEIGHT - DIALOG_BORDER) as u32;
        assert_eq!(emu.hle.app_messages[0].lparam, expected_w | (expected_h << 16));
    }

    #[test]
    fn show_window_queues_initial_paint_for_top_level_window() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        register_test_window(&mut emu, 0x0002_0001, 0, 0);

        write_arg(&mut emu, 0, 0x0002_0001);
        write_arg(&mut emu, 1, 1);
        assert_eq!(
            hle_show_window(&mut emu, &dummy_entry(hle_show_window)),
            HleResult::Retn(8)
        );

        assert!(emu.hle.window(0x0002_0001).unwrap().invalid_rect.is_some());
        assert!(emu
            .hle
            .app_messages
            .iter()
            .any(|message| message.msg == 0x000f));
    }

    #[test]
    fn show_window_queues_initial_paint_for_child_window() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        register_test_window(&mut emu, 0x0002_0001, 0, 0);
        register_test_window(&mut emu, 0x0002_0005, 0x0002_0001, 0);
        emu.hle.window_mut(0x0002_0005).unwrap().visible = false;

        write_arg(&mut emu, 0, 0x0002_0005);
        write_arg(&mut emu, 1, 1);
        assert_eq!(
            hle_show_window(&mut emu, &dummy_entry(hle_show_window)),
            HleResult::Retn(8)
        );

        assert!(emu.hle.window(0x0002_0005).unwrap().invalid_rect.is_some());
        assert!(emu
            .hle
            .app_messages
            .iter()
            .any(|message| message.hwnd == 0x0002_0005 && message.msg == 0x000f));
    }

    #[test]
    fn older_top_level_screen_dc_is_clipped_by_newer_window() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        register_test_window(&mut emu, 0x0002_0001, 0, 0);
        register_test_window(&mut emu, 0x0002_0009, 0, 0);
        emu.hle.window_mut(0x0002_0001).unwrap().rect = WindowRect {
            left: 0,
            top: 0,
            right: 200,
            bottom: 200,
        };
        emu.hle.window_mut(0x0002_0009).unwrap().rect = WindowRect {
            left: 50,
            top: 50,
            right: 150,
            bottom: 150,
        };

        write_arg(&mut emu, 0, 0x0002_0001);
        assert_eq!(
            hle_get_dc(&mut emu, &dummy_entry(hle_get_dc)),
            HleResult::Retn(4)
        );

        let hdc = emu.cpu.reg(Reg::Eax);
        let dc = gdi_dc_or_default(&emu, hdc);
        assert_ne!(dc.surface, 0);
        assert!(!gdi_dc_draw_point_visible(&emu, dc, 75, 75));
        assert!(gdi_dc_draw_point_visible(&emu, dc, 25, 25));

        register_test_window(&mut emu, 0x0002_000d, 0, 0);
        emu.hle.window_mut(0x0002_000d).unwrap().rect = WindowRect {
            left: 0,
            top: 0,
            right: 200,
            bottom: 200,
        };
        emu.hle.focus_window = 0x0002_0009;
        write_arg(&mut emu, 0, 0x0002_0009);
        assert_eq!(
            hle_get_dc(&mut emu, &dummy_entry(hle_get_dc)),
            HleResult::Retn(4)
        );

        let active_dc = gdi_dc_or_default(&emu, emu.cpu.reg(Reg::Eax));
        assert!(gdi_dc_draw_point_visible(&emu, active_dc, 75, 75));
    }

    #[test]
    fn get_message_parks_and_wakes_live_task() {
        let mut emu = Emulator::new();
        emu.backend = Box::new(crate::backend::HeadlessBackend::new_live(640, 480));
        prepare_hle_stack(&mut emu);
        let out = TEST_DATA;
        write_arg(&mut emu, 0, out);
        write_arg(&mut emu, 1, 0);
        write_arg(&mut emu, 2, 0);
        write_arg(&mut emu, 3, 0);

        let addr = emu
            .hle
            .register_symbol("user32.dll", "GetMessageA", hle_get_message_a);
        emu.cpu.eip = addr;
        assert_eq!(Hle::dispatch(&mut emu).unwrap(), None);
        assert!(matches!(
            emu.hle_tasks[0].wait,
            HleWaitState::Message {
                out: TEST_DATA,
                filter: MessageFilter { hwnd: 0, min: 0, max: 0 },
            }
        ));
        assert_eq!(emu.cpu.eip, addr);

        emu.hle.post_mouse_move(10, 20);
        emu.service_frontend().unwrap();
        assert!(emu.hle_tasks[0].wait.is_ready());
        assert_eq!(emu.cpu.eip, addr);

        assert_eq!(Hle::dispatch(&mut emu).unwrap(), None);
        assert_eq!(emu.cpu.reg(Reg::Eax), 1);
        assert_eq!(emu.cpu.eip, 0x0040_0000);
        assert_eq!(emu.cpu.reg(Reg::Esp), TEST_STACK + 20);
        assert_eq!(emu.memory.read_u32(out + 4).unwrap(), 0x0200);
        assert_eq!(emu.memory.read_u32(out + 12).unwrap(), mouse_lparam(10, 20));
    }

    #[test]
    fn multimedia_timer_interrupt_injection_preserves_guest_stack() {
        let mut emu = Emulator::new();
        emu.memory
            .map(TEST_STACK, 0x2000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        emu.cpu.set_reg(Reg::Esp, TEST_STACK + 0x800);
        emu.memory
            .write_u32(TEST_STACK + 0x800, 0x0040_1000)
            .unwrap();
        emu.guest_time_ms = 20;
        let target = emu.hle.delay_target(20, 0);
        let id = emu
            .hle
            .set_mm_timer(0x0040_1f2d, 0xfeed, 1, 0, 0, target);

        assert!(dispatch_due_mm_timer_interrupt(&mut emu));

        let callback_esp = TEST_STACK + 0x7e8;
        assert_eq!(emu.cpu.eip, 0x0040_1f2d);
        assert_eq!(emu.cpu.reg(Reg::Esp), callback_esp);
        assert_eq!(
            emu.memory.read_u32(callback_esp).unwrap(),
            emu.hle.async_return_thunk()
        );
        assert_eq!(emu.memory.read_u32(callback_esp + 4).unwrap(), id);
        assert_eq!(emu.memory.read_u32(callback_esp + 8).unwrap(), 0);
        assert_eq!(emu.memory.read_u32(callback_esp + 12).unwrap(), 0xfeed);
    }

    #[test]
    fn compatible_dc_select_bitmap_returns_stock_bitmap() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);

        write_arg(&mut emu, 0, 0);
        assert_eq!(
            hle_create_compatible_dc(&mut emu, &dummy_entry(hle_create_compatible_dc)),
            HleResult::Retn(4)
        );
        let hdc = emu.cpu.reg(Reg::Eax);
        let bitmap = emu.hle.create_gdi_bitmap(0x1234_0000);

        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, bitmap);
        assert_eq!(
            hle_select_object(&mut emu, &dummy_entry(hle_select_object)),
            HleResult::Retn(8)
        );
        assert_eq!(emu.cpu.reg(Reg::Eax), GDI_STOCK_BITMAP);
        assert_eq!(emu.hle.gdi_dcs.get(&hdc).unwrap().surface, 0x1234_0000);

        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, GDI_STOCK_BITMAP);
        hle_select_object(&mut emu, &dummy_entry(hle_select_object));
        assert_eq!(emu.cpu.reg(Reg::Eax), bitmap);
        let dc = emu.hle.gdi_dcs.get(&hdc).unwrap();
        assert_eq!(dc.selected_bitmap, GDI_STOCK_BITMAP);
        assert_eq!(dc.surface, 0);
    }

    #[test]
    fn move_to_line_to_draws_with_selected_pen() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let surface = create_gdi_surface_with_format(&mut emu, 5, 3, 16).unwrap();
        let hdc = emu.hle.create_surface_dc(surface);
        let pen = emu.hle.create_gdi_pen(0, 1, 0x0000_00ff);

        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, pen);
        hle_select_object(&mut emu, &dummy_entry(hle_select_object));

        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, 1);
        write_arg(&mut emu, 2, 1);
        write_arg(&mut emu, 3, 0);
        assert_eq!(
            hle_move_to_ex(&mut emu, &dummy_entry(hle_move_to_ex)),
            HleResult::Retn(16)
        );

        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, 3);
        write_arg(&mut emu, 2, 1);
        assert_eq!(
            hle_line_to(&mut emu, &dummy_entry(hle_line_to)),
            HleResult::Retn(12)
        );

        let dst = read_surface_info(&emu, surface).unwrap();
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 1, 1).unwrap() & 0xff, 0xf8);
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 2, 1).unwrap() & 0xff, 0xf8);
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 3, 1).unwrap() & 0xff, 0xf8);
        let dc = emu.hle.gdi_dcs.get(&hdc).unwrap();
        assert_eq!((dc.current_x, dc.current_y), (3, 1));
    }

    #[test]
    fn crt_sprintf_formats_to_guest_buffer() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let out = TEST_DATA;
        let fmt = TEST_DATA + 0x80;
        let name = TEST_DATA + 0x100;
        emu.memory.write_cstr(fmt, "%s:%d", 16).unwrap();
        emu.memory.write_cstr(name, "ball", 16).unwrap();

        write_arg(&mut emu, 0, out);
        write_arg(&mut emu, 1, fmt);
        write_arg(&mut emu, 2, name);
        write_arg(&mut emu, 3, 42);
        assert_eq!(
            hle_crt_sprintf(&mut emu, &dummy_entry(hle_crt_sprintf)),
            HleResult::Retn(0)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), 7);
        assert_eq!(emu.memory.cstr_lossy(out, 16).unwrap(), "ball:42");
    }

    #[test]
    fn crt_strstr_returns_guest_substring_pointer() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let haystack = TEST_DATA;
        let needle = TEST_DATA + 0x80;
        emu.memory
            .write_cstr(haystack, "space cadet pinball", 32)
            .unwrap();
        emu.memory.write_cstr(needle, "cadet", 16).unwrap();

        write_arg(&mut emu, 0, haystack);
        write_arg(&mut emu, 1, needle);
        assert_eq!(
            hle_crt_strstr(&mut emu, &dummy_entry(hle_crt_strstr)),
            HleResult::Retn(0)
        );

        assert_eq!(emu.cpu.reg(Reg::Eax), haystack + 6);
    }

    #[test]
    fn register_class_captures_background_brush() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let class = TEST_DATA;
        let name = TEST_DATA + 0x80;
        emu.memory.write_utf16z(name, "CardWnd", 16).unwrap();
        emu.memory.write_u32(class + 4, 0x0040_1000).unwrap();
        emu.memory.write_u32(class + 16, 0x0100_0000).unwrap();
        emu.memory.write_u32(class + 28, 0x4000_1234).unwrap();
        emu.memory.write_u32(class + 36, name).unwrap();
        write_arg(&mut emu, 0, class);

        assert_eq!(
            hle_register_class_w(&mut emu, &dummy_entry(hle_register_class_w)),
            HleResult::Retn(4)
        );
        let atom = emu.cpu.reg(Reg::Eax);
        assert_eq!(
            emu.hle.window_background_for_class("cardwnd", 0),
            0x4000_1234
        );
        assert_eq!(
            emu.hle.window_background_for_class("ignored", atom),
            0x4000_1234
        );
    }

    #[test]
    fn set_dibits_to_device_copies_bottom_up_dib() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let surface = create_gdi_surface_with_format(&mut emu, 2, 2, 16).unwrap();
        let hdc = emu.hle.create_surface_dc(surface);
        let info = TEST_DATA;
        let bits = TEST_DATA + 0x40;

        emu.memory.write_u32(info, 40).unwrap();
        emu.memory.write_u32(info + 4, 2).unwrap();
        emu.memory.write_u32(info + 8, 2).unwrap();
        emu.memory.write_u16(info + 12, 1).unwrap();
        emu.memory.write_u16(info + 14, 24).unwrap();
        emu.memory.write_u32(info + 16, 0).unwrap();
        emu.memory.write_u32(info + 20, 16).unwrap();
        emu.memory.write_u32(info + 32, 0).unwrap();
        emu.memory
            .write_bytes(
                bits,
                &[
                    0xff, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, // bottom: blue, green
                    0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, // top: red, white
                ],
            )
            .unwrap();

        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, 0);
        write_arg(&mut emu, 2, 0);
        write_arg(&mut emu, 3, 2);
        write_arg(&mut emu, 4, 2);
        write_arg(&mut emu, 5, 0);
        write_arg(&mut emu, 6, 0);
        write_arg(&mut emu, 7, 0);
        write_arg(&mut emu, 8, 2);
        write_arg(&mut emu, 9, bits);
        write_arg(&mut emu, 10, info);
        write_arg(&mut emu, 11, 0);

        assert_eq!(
            hle_set_dibits_to_device(&mut emu, &dummy_entry(hle_set_dibits_to_device)),
            HleResult::Retn(48)
        );
        assert_eq!(emu.cpu.reg(Reg::Eax), 2);
        let dst = read_surface_info(&emu, surface).unwrap();
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 0, 0).unwrap() & 0x00ff_ffff, 0x0000_00f8);
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 1, 0).unwrap() & 0x00ff_ffff, 0x00f8_fcf8);
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 0, 1).unwrap() & 0x00ff_ffff, 0x00f8_0000);
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 1, 1).unwrap() & 0x00ff_ffff, 0x0000_fc00);
    }

    #[test]
    fn set_dibits_to_device_uses_lower_left_source_y() {
        let mut emu = Emulator::new();
        prepare_hle_stack(&mut emu);
        let surface = create_gdi_surface_with_format(&mut emu, 2, 1, 32).unwrap();
        let hdc = emu.hle.create_surface_dc(surface);
        let info = TEST_DATA;
        let bits = TEST_DATA + 0x40;

        emu.memory.write_u32(info, 40).unwrap();
        emu.memory.write_u32(info + 4, 4).unwrap();
        emu.memory.write_u32(info + 8, 4).unwrap();
        emu.memory.write_u16(info + 12, 1).unwrap();
        emu.memory.write_u16(info + 14, 32).unwrap();
        emu.memory.write_u32(info + 16, 0).unwrap();
        emu.memory.write_u32(info + 20, 64).unwrap();
        emu.memory.write_u32(info + 32, 0).unwrap();

        let mut dib = Vec::new();
        for lower_y in 0..4 {
            for x in 0..4 {
                let r = (lower_y * 0x10 + x + 1) as u8;
                dib.extend_from_slice(&[0, 0, r, 0]);
            }
        }
        emu.memory.write_bytes(bits, &dib).unwrap();

        write_arg(&mut emu, 0, hdc);
        write_arg(&mut emu, 1, 0);
        write_arg(&mut emu, 2, 0);
        write_arg(&mut emu, 3, 2);
        write_arg(&mut emu, 4, 1);
        write_arg(&mut emu, 5, 1);
        write_arg(&mut emu, 6, 0);
        write_arg(&mut emu, 7, 0);
        write_arg(&mut emu, 8, 4);
        write_arg(&mut emu, 9, bits);
        write_arg(&mut emu, 10, info);
        write_arg(&mut emu, 11, 0);

        assert_eq!(
            hle_set_dibits_to_device(&mut emu, &dummy_entry(hle_set_dibits_to_device)),
            HleResult::Retn(48)
        );
        let dst = read_surface_info(&emu, surface).unwrap();
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 0, 0).unwrap() & 0xff, 0x02);
        assert_eq!(read_surface_pixel_colorref(&emu, dst, 1, 0).unwrap() & 0xff, 0x03);
    }
}
