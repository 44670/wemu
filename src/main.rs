use std::path::PathBuf;
use std::time::Duration;

use wemu::{Emulator, Error, FrontendKind, JournalInput, RunConfig};

#[cfg(target_arch = "wasm32")]
mod wasm_alloc;
#[cfg(target_arch = "wasm32")]
mod wasm_exports;

fn parse_u32(s: &str) -> Result<u32, Error> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|e| Error::Cli(format!("invalid number {s}: {e}")))
    } else {
        s.parse::<u32>()
            .map_err(|e| Error::Cli(format!("invalid number {s}: {e}")))
    }
}

fn parse_mount(s: &str) -> Result<(char, PathBuf), Error> {
    let (drive, path) = s
        .split_once('=')
        .ok_or_else(|| Error::Cli(format!("mount must look like C:=/path, got {s}")))?;
    let drive = drive.strip_suffix(':').unwrap_or(drive);
    let mut chars = drive.chars();
    let letter = chars
        .next()
        .ok_or_else(|| Error::Cli("empty drive letter".to_string()))?
        .to_ascii_uppercase();
    if chars.next().is_some() || !letter.is_ascii_alphabetic() {
        return Err(Error::Cli(format!("invalid drive letter in mount {s}")));
    }
    Ok((letter, PathBuf::from(path)))
}

fn usage() {
    eprintln!(
        "usage: wemu --mount C:=/dir --cmdline C:\\game.exe [--arg VALUE] [--cwd C:\\path] [--frontend headless|sdl2] [--sdl-ws 127.0.0.1:8765] [--max-insns N] [--breakpoint 0xADDR] [--screenshot out.png] [--replay script.txt|inline:wait,1;key,ENTER] [--record script.txt] [--state-interval SECONDS] [--trace] [--trace-after N] [--debug-on-crash] [--strict-hle-imports]\n       default: cwd is the command-line executable parent, unlimited instructions, screenshot /tmp/wemu.png; debug builds enable strict HLE imports"
    );
}

fn parse_args() -> Result<RunConfig, Error> {
    let mut args = std::env::args().skip(1);
    let mut cfg = RunConfig::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cmdline" => {
                cfg.cmdline = Some(
                    args.next()
                        .ok_or_else(|| Error::Cli(format!("{arg} needs a command line")))?,
                );
            }
            "--arg" => {
                cfg.args.push(
                    args.next()
                        .ok_or_else(|| Error::Cli("--arg needs a value".to_string()))?,
                );
            }
            "--mount" => {
                let mount = args
                    .next()
                    .ok_or_else(|| Error::Cli("--mount needs C:=/path".to_string()))?;
                cfg.mounts.push(parse_mount(&mount)?);
            }
            "--cwd" => {
                let cwd = args
                    .next()
                    .ok_or_else(|| Error::Cli("--cwd needs a Windows path".to_string()))?;
                let bytes = cwd.as_bytes();
                if bytes.len() >= 2 && bytes[1] == b':' {
                    cfg.cwd_drive = bytes[0].to_ascii_uppercase() as char;
                    cfg.cwd_path = cwd[2..].to_string();
                    if cfg.cwd_path.is_empty() {
                        cfg.cwd_path = "\\".to_string();
                    }
                } else {
                    cfg.cwd_path = cwd;
                }
            }
            "--max-insns" => {
                let n = args
                    .next()
                    .ok_or_else(|| Error::Cli("--max-insns needs a number".to_string()))?;
                cfg.max_insns = n
                    .parse::<u64>()
                    .map_err(|e| Error::Cli(format!("invalid --max-insns {n}: {e}")))?;
            }
            "--breakpoint" => {
                let bp = args
                    .next()
                    .ok_or_else(|| Error::Cli("--breakpoint needs an address".to_string()))?;
                cfg.breakpoints.push(parse_u32(&bp)?);
            }
            "--screenshot" => {
                cfg.screenshot =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        Error::Cli("--screenshot needs a path".to_string())
                    })?));
            }
            "--journal" | "--replay" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::Cli(format!("{arg} needs a path or inline:script")))?;
                cfg.journal = Some(JournalInput::from_cli(value));
            }
            "--record" => {
                cfg.record =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        Error::Cli("--record needs a path".to_string())
                    })?));
            }
            "--frontend" => {
                let frontend = args
                    .next()
                    .ok_or_else(|| Error::Cli("--frontend needs headless or sdl2".to_string()))?;
                cfg.frontend = match frontend.as_str() {
                    "headless" => FrontendKind::Headless,
                    "sdl2" => FrontendKind::Sdl2,
                    _ => {
                        return Err(Error::Cli(format!(
                            "invalid --frontend {frontend}: expected headless or sdl2"
                        )));
                    }
                };
            }
            "--sdl2" => cfg.frontend = FrontendKind::Sdl2,
            "--sdl-ws" => {
                cfg.sdl_ws = Some(
                    args.next()
                        .ok_or_else(|| Error::Cli("--sdl-ws needs host:port".to_string()))?,
                );
                cfg.frontend = FrontendKind::Sdl2;
            }
            "--trace" => cfg.trace = true,
            "--debug-on-crash" => cfg.debug_on_crash = true,
            "--strict-hle-imports" => cfg.strict_hle_imports = true,
            "--state-interval" => {
                let n = args
                    .next()
                    .ok_or_else(|| Error::Cli("--state-interval needs seconds".to_string()))?;
                let seconds = n
                    .parse::<u64>()
                    .map_err(|e| Error::Cli(format!("invalid --state-interval {n}: {e}")))?;
                cfg.state_interval = if seconds == 0 {
                    None
                } else {
                    Some(Duration::from_secs(seconds))
                };
            }
            "--trace-after" => {
                let n = args
                    .next()
                    .ok_or_else(|| Error::Cli("--trace-after needs a number".to_string()))?;
                cfg.trace_after = n
                    .parse::<u64>()
                    .map_err(|e| Error::Cli(format!("invalid --trace-after {n}: {e}")))?;
            }
            other => return Err(Error::Cli(format!("unknown argument {other}"))),
        }
    }

    if cfg.cmdline.is_none() {
        usage();
        return Err(Error::Cli("--cmdline is required".to_string()));
    }
    if cfg.mounts.is_empty() {
        usage();
        return Err(Error::Cli("--mount is required".to_string()));
    }
    Ok(cfg)
}

fn main() {
    let cfg = match parse_args() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };

    match Emulator::run_config(&cfg) {
        Ok((stop, emu)) => {
            println!(
                "wemu stopped: {:?}, insns={}, eip={:08x} eax={:08x} ecx={:08x} edx={:08x} ebx={:08x} esp={:08x} ebp={:08x} esi={:08x} edi={:08x}",
                stop,
                emu.insns,
                emu.cpu.eip,
                emu.cpu.reg(wemu::cpu::Reg::Eax),
                emu.cpu.reg(wemu::cpu::Reg::Ecx),
                emu.cpu.reg(wemu::cpu::Reg::Edx),
                emu.cpu.reg(wemu::cpu::Reg::Ebx),
                emu.cpu.reg(wemu::cpu::Reg::Esp),
                emu.cpu.reg(wemu::cpu::Reg::Ebp),
                emu.cpu.reg(wemu::cpu::Reg::Esi),
                emu.cpu.reg(wemu::cpu::Reg::Edi),
            );
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
