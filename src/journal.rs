use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::{Error, Result};

const KEY_EVENT_REPLAY_DELAY_MS: u64 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalEvent {
    Move { x: u32, y: u32 },
    ButtonDown { x: u32, y: u32 },
    ButtonUp { x: u32, y: u32 },
    Click { x: u32, y: u32 },
    KeyDown { vk: u32 },
    KeyUp { vk: u32 },
    Text { text: String },
}

#[derive(Clone, Debug, Default)]
pub struct Journal {
    events: Vec<JournalItem>,
    next: usize,
    wait_until_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum JournalItem {
    Move { x: u32, y: u32 },
    ButtonDown { x: u32, y: u32 },
    ButtonUp { x: u32, y: u32 },
    Click { x: u32, y: u32 },
    KeyDown { vk: u32 },
    KeyUp { vk: u32 },
    Text { text: String },
    Wait { ms: u64 },
}

#[derive(Debug)]
pub struct JournalRecorder {
    file: File,
    last_ms: Option<u64>,
}

impl Journal {
    pub fn from_path(path: &Path) -> Result<Self> {
        let script = fs::read_to_string(path)?;
        Self::parse(&script)
    }

    pub fn parse(script: &str) -> Result<Self> {
        let mut events = Vec::new();
        for (idx, raw) in script.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(text) = line.strip_prefix("text,") {
                events.push(JournalItem::Text {
                    text: text.to_string(),
                });
                continue;
            }
            let parts = line
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            match parts.as_slice() {
                ["move", x, y] => events.push(JournalItem::Move {
                    x: parse_u32(idx, x)?,
                    y: parse_u32(idx, y)?,
                }),
                ["down", x, y] => events.push(JournalItem::ButtonDown {
                    x: parse_u32(idx, x)?,
                    y: parse_u32(idx, y)?,
                }),
                ["up", x, y] => events.push(JournalItem::ButtonUp {
                    x: parse_u32(idx, x)?,
                    y: parse_u32(idx, y)?,
                }),
                ["click", x, y] => events.push(JournalItem::Click {
                    x: parse_u32(idx, x)?,
                    y: parse_u32(idx, y)?,
                }),
                ["key", key] => {
                    let vk = parse_key(idx, key)?;
                    events.push(JournalItem::KeyDown { vk });
                    events.push(JournalItem::KeyUp { vk });
                }
                ["keydown", key] => events.push(JournalItem::KeyDown {
                    vk: parse_key(idx, key)?,
                }),
                ["keyup", key] => events.push(JournalItem::KeyUp {
                    vk: parse_key(idx, key)?,
                }),
                ["wait", seconds] => events.push(JournalItem::Wait {
                    ms: parse_wait_ms(idx, seconds)?,
                }),
                _ => {
                    return Err(Error::Cli(format!(
                        "invalid journal line {}: expected move,x,y, down,x,y, up,x,y, click,x,y, key,key, keydown,key, keyup,key, text,value, or wait,seconds",
                        idx + 1
                    )));
                }
            }
        }
        Ok(Self {
            events,
            next: 0,
            wait_until_ms: None,
        })
    }

    pub fn next_event(&mut self, now_ms: u64) -> Option<JournalEvent> {
        loop {
            if let Some(wait_until_ms) = self.wait_until_ms {
                if now_ms < wait_until_ms {
                    return None;
                }
                self.wait_until_ms = None;
            }
            let event = self.events.get(self.next)?.clone();
            self.next += 1;
            match event {
                JournalItem::Move { x, y } => return Some(JournalEvent::Move { x, y }),
                JournalItem::ButtonDown { x, y } => {
                    return Some(JournalEvent::ButtonDown { x, y });
                }
                JournalItem::ButtonUp { x, y } => return Some(JournalEvent::ButtonUp { x, y }),
                JournalItem::Click { x, y } => return Some(JournalEvent::Click { x, y }),
                JournalItem::KeyDown { vk } => {
                    self.wait_until_ms = Some(now_ms.saturating_add(KEY_EVENT_REPLAY_DELAY_MS));
                    return Some(JournalEvent::KeyDown { vk });
                }
                JournalItem::KeyUp { vk } => {
                    self.wait_until_ms = Some(now_ms.saturating_add(KEY_EVENT_REPLAY_DELAY_MS));
                    return Some(JournalEvent::KeyUp { vk });
                }
                JournalItem::Text { text } => return Some(JournalEvent::Text { text }),
                JournalItem::Wait { ms } => {
                    self.wait_until_ms = Some(now_ms.saturating_add(ms));
                }
            }
        }
    }

    pub fn next_wakeup_ms(&self) -> Option<u64> {
        self.wait_until_ms
    }
}

impl JournalRecorder {
    pub fn from_path(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let file = File::create(path)?;
        Ok(Self {
            file,
            last_ms: None,
        })
    }

    pub fn start(&mut self, now_ms: u64) {
        self.last_ms.get_or_insert(now_ms);
    }

    pub fn record_event(&mut self, now_ms: u64, event: JournalEvent) -> Result<()> {
        let prev = *self.last_ms.get_or_insert(now_ms);
        let wait_ms = now_ms.saturating_sub(prev);
        self.last_ms = Some(now_ms);
        if wait_ms != 0 {
            writeln!(self.file, "wait,{}", format_wait_seconds(wait_ms))?;
        }
        match event {
            JournalEvent::Move { x, y } => writeln!(self.file, "move,{x},{y}")?,
            JournalEvent::ButtonDown { x, y } => writeln!(self.file, "down,{x},{y}")?,
            JournalEvent::ButtonUp { x, y } => writeln!(self.file, "up,{x},{y}")?,
            JournalEvent::Click { x, y } => writeln!(self.file, "click,{x},{y}")?,
            JournalEvent::KeyDown { vk } => writeln!(self.file, "keydown,{}", journal_key(vk))?,
            JournalEvent::KeyUp { vk } => writeln!(self.file, "keyup,{}", journal_key(vk))?,
            JournalEvent::Text { text } => writeln!(self.file, "text,{}", journal_text(&text))?,
        }
        self.file.flush()?;
        Ok(())
    }
}

fn parse_u32(line_idx: usize, s: &str) -> Result<u32> {
    s.parse::<u32>().map_err(|err| {
        Error::Cli(format!(
            "invalid journal number on line {}: {s}: {err}",
            line_idx + 1
        ))
    })
}

fn parse_wait_ms(line_idx: usize, s: &str) -> Result<u64> {
    let seconds = s.parse::<f64>().map_err(|err| {
        Error::Cli(format!(
            "invalid journal wait seconds on line {}: {s}: {err}",
            line_idx + 1
        ))
    })?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(Error::Cli(format!(
            "invalid journal wait seconds on line {}: {s}",
            line_idx + 1
        )));
    }
    let ms = seconds * 1000.0;
    if ms > u64::MAX as f64 {
        return Err(Error::Cli(format!(
            "journal wait is too large on line {}: {s}",
            line_idx + 1
        )));
    }
    Ok(ms.round() as u64)
}

fn parse_key(line_idx: usize, s: &str) -> Result<u32> {
    parse_key_value(s)
        .ok_or_else(|| Error::Cli(format!("invalid journal key on line {}: {s}", line_idx + 1)))
}

pub(crate) fn parse_key_value(s: &str) -> Option<u32> {
    let upper = s.trim().to_ascii_uppercase();
    match upper.as_str() {
        "SHIFT" => Some(0x10),
        "CTRL" | "CONTROL" => Some(0x11),
        "ALT" => Some(0x12),
        "ENTER" | "RETURN" => Some(0x0d),
        "ESC" | "ESCAPE" => Some(0x1b),
        "BACKSPACE" | "BACK" => Some(0x08),
        "TAB" => Some(0x09),
        "SPACE" => Some(0x20),
        "LEFT" => Some(0x25),
        "UP" => Some(0x26),
        "RIGHT" => Some(0x27),
        "DOWN" => Some(0x28),
        "DELETE" | "DEL" => Some(0x2e),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "SEMICOLON" | ";" => Some(0xba),
        "EQUAL" | "EQUALS" | "=" => Some(0xbb),
        "COMMA" | "," => Some(0xbc),
        "MINUS" | "-" => Some(0xbd),
        "PERIOD" | "." => Some(0xbe),
        "SLASH" | "/" => Some(0xbf),
        "BACKQUOTE" | "GRAVE" | "`" => Some(0xc0),
        "LEFTBRACKET" | "[" => Some(0xdb),
        "BACKSLASH" | "\\" => Some(0xdc),
        "RIGHTBRACKET" | "]" => Some(0xdd),
        "QUOTE" | "APOSTROPHE" | "'" => Some(0xde),
        _ if upper.starts_with('F') && upper.len() >= 2 => upper[1..]
            .parse::<u32>()
            .ok()
            .and_then(|n| (1..=24).contains(&n).then_some(0x6f + n)),
        _ if upper.len() == 1 => {
            let byte = upper.as_bytes()[0];
            byte.is_ascii_alphanumeric().then_some(byte as u32)
        }
        _ => {
            if let Some(hex) = upper.strip_prefix("0X") {
                u32::from_str_radix(hex, 16).ok()
            } else {
                upper.parse::<u32>().ok()
            }
        }
    }
}

fn format_wait_seconds(ms: u64) -> String {
    if ms % 1000 == 0 {
        return (ms / 1000).to_string();
    }
    let mut s = format!("{}.{:03}", ms / 1000, ms % 1000);
    while s.ends_with('0') {
        s.pop();
    }
    s
}

fn journal_key(vk: u32) -> String {
    match vk {
        0x08 => "BACKSPACE".to_string(),
        0x09 => "TAB".to_string(),
        0x0d => "ENTER".to_string(),
        0x10 => "SHIFT".to_string(),
        0x11 => "CTRL".to_string(),
        0x12 => "ALT".to_string(),
        0x1b => "ESC".to_string(),
        0x20 => "SPACE".to_string(),
        0x23 => "END".to_string(),
        0x24 => "HOME".to_string(),
        0x25 => "LEFT".to_string(),
        0x26 => "UP".to_string(),
        0x27 => "RIGHT".to_string(),
        0x28 => "DOWN".to_string(),
        0x2e => "DELETE".to_string(),
        0xba => "SEMICOLON".to_string(),
        0xbb => "EQUALS".to_string(),
        0xbc => "COMMA".to_string(),
        0xbd => "MINUS".to_string(),
        0xbe => "PERIOD".to_string(),
        0xbf => "SLASH".to_string(),
        0xc0 => "BACKQUOTE".to_string(),
        0xdb => "LEFTBRACKET".to_string(),
        0xdc => "BACKSLASH".to_string(),
        0xdd => "RIGHTBRACKET".to_string(),
        0xde => "QUOTE".to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6f),
        0x30..=0x39 | 0x41..=0x5a => (vk as u8 as char).to_string(),
        _ => format!("0x{vk:02x}"),
    }
}

fn journal_text(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\r' | '\n' | '#' => ' ',
            _ => ch,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Journal, JournalEvent, JournalRecorder};

    #[test]
    fn parses_moves_buttons_clicks_comments_and_waits() {
        let mut journal = Journal::parse(
            "
            # enter the menu
            move,10,20
            wait,0.005
            down,30,40
            up,30,40
            click,50,60 # another click
            keydown,A
            wait,0.025
            keyup,0x41
            text,Player One
            ",
        )
        .unwrap();

        assert_eq!(
            journal.next_event(0),
            Some(JournalEvent::Move { x: 10, y: 20 })
        );
        assert_eq!(journal.next_event(0), None);
        assert_eq!(journal.next_event(4), None);
        assert_eq!(
            journal.next_event(5),
            Some(JournalEvent::ButtonDown { x: 30, y: 40 })
        );
        assert_eq!(
            journal.next_event(5),
            Some(JournalEvent::ButtonUp { x: 30, y: 40 })
        );
        assert_eq!(
            journal.next_event(5),
            Some(JournalEvent::Click { x: 50, y: 60 })
        );
        assert_eq!(
            journal.next_event(5),
            Some(JournalEvent::KeyDown { vk: 0x41 })
        );
        assert_eq!(journal.next_event(5), None);
        assert_eq!(journal.next_event(104), None);
        assert_eq!(journal.next_event(105), None);
        assert_eq!(journal.next_event(129), None);
        assert_eq!(
            journal.next_event(130),
            Some(JournalEvent::KeyUp { vk: 0x41 })
        );
        assert_eq!(journal.next_event(130), None);
        assert_eq!(journal.next_event(229), None);
        assert_eq!(
            journal.next_event(230),
            Some(JournalEvent::Text {
                text: "Player One".to_string()
            })
        );
        assert_eq!(journal.next_event(230), None);
    }

    #[test]
    fn recorder_writes_relative_waits_and_buttons() {
        let path =
            std::env::temp_dir().join(format!("wemu-journal-recorder-{}.txt", std::process::id()));
        let mut recorder = JournalRecorder::from_path(&path).unwrap();
        recorder.start(100);
        recorder
            .record_event(110, JournalEvent::Move { x: 1, y: 2 })
            .unwrap();
        recorder
            .record_event(110, JournalEvent::ButtonDown { x: 3, y: 4 })
            .unwrap();
        recorder
            .record_event(130, JournalEvent::ButtonUp { x: 3, y: 4 })
            .unwrap();
        recorder
            .record_event(131, JournalEvent::KeyDown { vk: 0x41 })
            .unwrap();
        recorder
            .record_event(151, JournalEvent::KeyUp { vk: 0x41 })
            .unwrap();
        recorder
            .record_event(
                151,
                JournalEvent::Text {
                    text: "AoE#1".to_string(),
                },
            )
            .unwrap();
        drop(recorder);

        let script = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(
            script,
            "wait,0.01\nmove,1,2\ndown,3,4\nwait,0.02\nup,3,4\nwait,0.001\nkeydown,A\nwait,0.02\nkeyup,A\ntext,AoE 1\n"
        );
    }
}
