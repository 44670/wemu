use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::{Error, Result};

pub fn write_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let png = encode_rgba_png(width, height, rgba)?;
    let mut file = File::create(path)?;
    file.write_all(&png)?;
    Ok(())
}

pub fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(Error::Memory(format!(
            "rgba length mismatch: got {}, expected {}",
            rgba.len(),
            expected
        )));
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(6);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);
    write_chunk(&mut out, b"IHDR", &ihdr);

    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * stride..(y + 1) * stride]);
    }
    write_chunk(&mut out, b"IDAT", &zlib_store(&raw));
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_data = Vec::with_capacity(kind.len() + data.len());
    crc_data.extend_from_slice(kind);
    crc_data.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_data).to_be_bytes());
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16 + data.len() / 65535 * 5);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut offset = 0;
    while offset < data.len() {
        let remaining = data.len() - offset;
        let len = remaining.min(65535);
        let final_block = offset + len == data.len();
        out.push(if final_block { 0x01 } else { 0x00 });
        let len16 = len as u16;
        out.extend_from_slice(&len16.to_le_bytes());
        out.extend_from_slice(&(!len16).to_le_bytes());
        out.extend_from_slice(&data[offset..offset + len]);
        offset += len;
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for &b in data {
        s1 = (s1 + b as u32) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    (s2 << 16) | s1
}
