// Win32 "A" APIs use the process ANSI code page. wemu does not model code pages
// yet, so this module provides one explicit lossy single-byte ANSI boundary.

const CP1252_CONTROL_TO_UNICODE: [char; 32] = [
    '\u{20ac}', '\u{0081}', '\u{201a}', '\u{0192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02c6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008d}', '\u{017d}', '\u{008f}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02dc}', '\u{2122}', '\u{0161}', '\u{203a}', '\u{0153}', '\u{009d}', '\u{017e}', '\u{0178}',
];

pub fn ansi_bytes_to_string_lossy(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| ansi_byte_to_char(byte)).collect()
}

pub fn string_to_ansi_bytes_lossy(s: &str) -> Vec<u8> {
    s.chars().map(char_to_ansi_byte_lossy).collect()
}

pub fn ansi_bytes_to_utf16_lossy(bytes: &[u8]) -> Vec<u16> {
    ansi_bytes_to_string_lossy(bytes).encode_utf16().collect()
}

pub fn utf16_units_to_ansi_bytes_lossy(units: &[u16]) -> Vec<u8> {
    string_to_ansi_bytes_lossy(&String::from_utf16_lossy(units))
}

pub fn ansi_byte_to_char(byte: u8) -> char {
    match byte {
        0x80..=0x9f => CP1252_CONTROL_TO_UNICODE[(byte - 0x80) as usize],
        _ => byte as char,
    }
}

pub fn char_to_ansi_byte_lossy(ch: char) -> u8 {
    let value = ch as u32;
    if value <= 0x7f || (0xa0..=0xff).contains(&value) {
        return value as u8;
    }
    CP1252_CONTROL_TO_UNICODE
        .iter()
        .position(|&mapped| mapped == ch)
        .map(|index| 0x80 + index as u8)
        .unwrap_or(b'?')
}

#[cfg(test)]
mod tests {
    use super::{
        ansi_bytes_to_string_lossy, ansi_bytes_to_utf16_lossy, string_to_ansi_bytes_lossy,
        utf16_units_to_ansi_bytes_lossy,
    };

    #[test]
    fn ansi_ascii_round_trips() {
        let bytes = b"ABCxyz012";
        let s = ansi_bytes_to_string_lossy(bytes);
        assert_eq!(s, "ABCxyz012");
        assert_eq!(string_to_ansi_bytes_lossy(&s), bytes);
    }

    #[test]
    fn ansi_preserves_latin1_bytes() {
        let s = ansi_bytes_to_string_lossy(&[0xe9, 0xff]);
        assert_eq!(s, "\u{00e9}\u{00ff}");
        assert_eq!(string_to_ansi_bytes_lossy(&s), vec![0xe9, 0xff]);
    }

    #[test]
    fn ansi_maps_common_cp1252_controls() {
        let s = ansi_bytes_to_string_lossy(&[0x80, 0x93, 0x94]);
        assert_eq!(s, "\u{20ac}\u{201c}\u{201d}");
        assert_eq!(string_to_ansi_bytes_lossy(&s), vec![0x80, 0x93, 0x94]);
    }

    #[test]
    fn ansi_replaces_unrepresentable_chars() {
        assert_eq!(string_to_ansi_bytes_lossy("A\u{2603}B"), b"A?B");
    }

    #[test]
    fn ansi_utf16_helpers_use_same_mapping() {
        let units = ansi_bytes_to_utf16_lossy(&[0x41, 0x80]);
        assert_eq!(String::from_utf16_lossy(&units), "A\u{20ac}");
        assert_eq!(utf16_units_to_ansi_bytes_lossy(&units), vec![0x41, 0x80]);
    }
}
