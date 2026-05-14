use std::io::{self, Write};

use crate::cpu::Reg;
use crate::{Emulator, Error};

pub fn interactive(emu: &mut Emulator, crash: &Error) {
    eprintln!("{crash}");
    print_regs(emu);
    eprintln!("debugger commands: regs, code [addr] [len], x [addr] [len], dd [addr] [count], stack [count], help, quit");

    loop {
        eprint!("wemu-dbg> ");
        let _ = io::stderr().flush();

        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                eprintln!("stdin error: {err}");
                break;
            }
        }

        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else {
            continue;
        };
        match cmd {
            "r" | "regs" => print_regs(emu),
            "c" | "code" => {
                let addr = parse_value(emu, parts.next()).unwrap_or(emu.cpu.eip);
                let len = parse_value(emu, parts.next()).unwrap_or(32).min(256) as usize;
                dump_bytes(emu, addr, len);
            }
            "x" | "mem" => {
                let addr = parse_value(emu, parts.next()).unwrap_or(emu.cpu.eip);
                let len = parse_value(emu, parts.next()).unwrap_or(64).min(1024) as usize;
                dump_bytes(emu, addr, len);
            }
            "dd" | "dwords" => {
                let addr = parse_value(emu, parts.next()).unwrap_or_else(|| emu.cpu.reg(Reg::Esp));
                let count = parse_value(emu, parts.next()).unwrap_or(16).min(128);
                dump_dwords(emu, addr, count);
            }
            "s" | "stack" => {
                let count = parse_value(emu, parts.next()).unwrap_or(16).min(128);
                dump_dwords(emu, emu.cpu.reg(Reg::Esp), count);
            }
            "h" | "help" | "?" => print_help(),
            "q" | "quit" | "exit" => break,
            other => eprintln!("unknown command {other:?}; try help"),
        }
    }
}

pub fn panic_to_error(panic: Box<dyn std::any::Any + Send>) -> Error {
    if let Some(s) = panic.downcast_ref::<String>() {
        Error::Hle(format!("panic: {s}"))
    } else if let Some(s) = panic.downcast_ref::<&'static str>() {
        Error::Hle(format!("panic: {s}"))
    } else {
        Error::Hle("panic: non-string payload".to_string())
    }
}

fn print_regs(emu: &Emulator) {
    eprintln!(
        "insns={} eip={:08x} eflags={:08x} eax={:08x} ecx={:08x} edx={:08x} ebx={:08x}",
        emu.insns,
        emu.cpu.eip,
        emu.cpu.eflags,
        emu.cpu.reg(Reg::Eax),
        emu.cpu.reg(Reg::Ecx),
        emu.cpu.reg(Reg::Edx),
        emu.cpu.reg(Reg::Ebx),
    );
    eprintln!(
        "esp={:08x} ebp={:08x} esi={:08x} edi={:08x}",
        emu.cpu.reg(Reg::Esp),
        emu.cpu.reg(Reg::Ebp),
        emu.cpu.reg(Reg::Esi),
        emu.cpu.reg(Reg::Edi),
    );
}

fn dump_bytes(emu: &Emulator, addr: u32, len: usize) {
    let mut offset = 0usize;
    while offset < len {
        let row_addr = addr.wrapping_add(offset as u32);
        let row_len = (len - offset).min(16);
        match emu.memory.read_bytes(row_addr, row_len) {
            Ok(bytes) => {
                eprint!("{row_addr:08x}:");
                for byte in &bytes {
                    eprint!(" {byte:02x}");
                }
                eprintln!();
            }
            Err(err) => {
                eprintln!("{row_addr:08x}: {err}");
                return;
            }
        }
        offset += row_len;
    }
}

fn dump_dwords(emu: &Emulator, addr: u32, count: u32) {
    for row in 0..((count + 3) / 4) {
        let row_addr = addr.wrapping_add(row * 16);
        eprint!("{row_addr:08x}:");
        for col in 0..4 {
            let index = row * 4 + col;
            if index >= count {
                break;
            }
            let value_addr = addr.wrapping_add(index * 4);
            match emu.memory.read_u32(value_addr) {
                Ok(value) => eprint!(" {value:08x}"),
                Err(err) => {
                    eprint!(" {err}");
                    break;
                }
            }
        }
        eprintln!();
    }
}

fn print_help() {
    eprintln!("regs                 print CPU registers");
    eprintln!("code [addr] [len]    dump bytes at addr, defaults to eip");
    eprintln!("x [addr|reg] [len]   dump memory bytes");
    eprintln!("dd [addr|reg] [count] dump dwords");
    eprintln!("stack [count]        dump dwords from esp");
    eprintln!("quit                 leave debugger and exit");
}

fn parse_value(emu: &Emulator, s: Option<&str>) -> Option<u32> {
    let s = s?;
    match s.to_ascii_lowercase().as_str() {
        "eip" => return Some(emu.cpu.eip),
        "eflags" => return Some(emu.cpu.eflags),
        "eax" => return Some(emu.cpu.reg(Reg::Eax)),
        "ecx" => return Some(emu.cpu.reg(Reg::Ecx)),
        "edx" => return Some(emu.cpu.reg(Reg::Edx)),
        "ebx" => return Some(emu.cpu.reg(Reg::Ebx)),
        "esp" => return Some(emu.cpu.reg(Reg::Esp)),
        "ebp" => return Some(emu.cpu.reg(Reg::Ebp)),
        "esi" => return Some(emu.cpu.reg(Reg::Esi)),
        "edi" => return Some(emu.cpu.reg(Reg::Edi)),
        _ => {}
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else if s
        .chars()
        .any(|ch| ch.is_ascii_hexdigit() && ch.is_ascii_alphabetic())
    {
        u32::from_str_radix(s, 16).ok()
    } else {
        s.parse::<u32>().ok()
    }
}
