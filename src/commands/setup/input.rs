//! Raw Terminal Guard & SGR Mouse / Keyboard Parser for `ccide-core setup`.

use std::io::Write;

use crate::ffi::{tcgetattr, tcsetattr, Termios};

pub const TAG_SETUP_ROW: u8 = 20;
pub const TAG_APPLY_BTN: u8 = 21;
pub const TOTAL_SETUP_ROWS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupKey {
    Up,
    Down,
    Left,
    Right,
    ToggleOrActivate,
    Enter,
    InstallNow,
    RefreshDoctor,
    Uninstall,
    PresetIdx(usize),
    Click { col: usize, row: usize },
    Quit,
}

pub fn parse_setup_keys(raw: &str) -> Vec<SetupKey> {
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        if bytes[i] == 0x1b {
            if i + 2 < len && bytes[i + 1] == b'[' && bytes[i + 2] == b'<' {
                let rest = &raw[i + 3..];
                if let Some(end_idx) = rest.find(['M', 'm']) {
                    if rest.as_bytes()[end_idx] == b'M' {
                        let mut parts = rest[..end_idx].split(';');
                        if let (Some(bs), Some(cs), Some(rs)) =
                            (parts.next(), parts.next(), parts.next())
                        {
                            if let (Ok(btn), Ok(col), Ok(row)) =
                                (bs.parse::<u32>(), cs.parse::<usize>(), rs.parse::<usize>())
                            {
                                match btn {
                                    0 => out.push(SetupKey::Click { col, row }),
                                    64 => out.push(SetupKey::Up),
                                    65 => out.push(SetupKey::Down),
                                    _ => {}
                                }
                            }
                        }
                    }
                    i += 3 + end_idx + 1;
                    continue;
                }
            }
            if i + 2 < len && (bytes[i + 1] == b'[' || bytes[i + 1] == b'O') {
                match bytes[i + 2] {
                    b'A' => out.push(SetupKey::Up),
                    b'B' => out.push(SetupKey::Down),
                    b'C' => out.push(SetupKey::Right),
                    b'D' => out.push(SetupKey::Left),
                    _ => {}
                }
                i += 3;
                continue;
            }
            out.push(SetupKey::Quit);
            i += 1;
            continue;
        }
        match bytes[i] {
            b'k' | b'K' => out.push(SetupKey::Up),
            b'j' | b'J' => out.push(SetupKey::Down),
            b'h' | b'H' => out.push(SetupKey::Left),
            b'l' | b'L' => out.push(SetupKey::Right),
            b' ' => out.push(SetupKey::ToggleOrActivate),
            b'\r' | b'\n' => out.push(SetupKey::Enter),
            b'i' | b'I' => out.push(SetupKey::InstallNow),
            b'r' | b'R' => out.push(SetupKey::RefreshDoctor),
            b'u' | b'U' => out.push(SetupKey::Uninstall),
            b'1'..=b'6' => out.push(SetupKey::PresetIdx((bytes[i] - b'1') as usize)),
            b'q' | b'Q' | 0x03 => out.push(SetupKey::Quit),
            _ => {}
        }
        i += 1;
    }
    out
}

pub struct RawTermGuard {
    fd: i32,
    orig: Termios,
    active: bool,
}

impl RawTermGuard {
    pub fn enter(fd: i32) -> Self {
        let mut orig = Termios::zeroed();
        let active = unsafe { tcgetattr(fd, &mut orig) == 0 };
        if active {
            let mut raw = orig;
            raw.make_raw();
            unsafe {
                let _ = tcsetattr(fd, 0, &raw);
            }
        }
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[49m\x1b[2J\x1b[3J");
        let _ = out.flush();
        Self { fd, orig, active }
    }
}

impl Drop for RawTermGuard {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?1000l\x1b[?1006l\x1b[?1049l\x1b[?25h\x1b[0m");
        let _ = out.flush();
        if self.active {
            unsafe {
                let _ = tcsetattr(self.fd, 0, &self.orig);
            }
        }
    }
}
