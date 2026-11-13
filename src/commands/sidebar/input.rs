//! Atomic SGR Mouse & Keyboard Escape-Sequence Parser for `ccide-core sidebar`.
//!
//! Design invariants:
//! 1. Consumes complete CSI (`\x1b[...<0x40..=0x7E>`), SS3 (`\x1bO<ch>`), OSC (`\x1b]...\x07`),
//!    APC (`\x1b_...\x1b\\`), and DCS (`\x1bP...\x1b\\`) sequences atomically so truncated or
//!    unrecognized control packets NEVER emit phantom `Escape` or stray ASCII command bytes.
//! 2. Emits both `MouseDown` and `MouseUp` (discarding `(btn & 32) != 0` drag motion packets)
//!    so `hit_mask.rs` can enforce standard press-and-release hit-box capture.

use crate::session::transcript::SubagentItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    MouseDown { btn: u8, col: usize, row: usize },
    MouseUp { btn: u8, col: usize, row: usize },
    WheelUp { row: usize },
    WheelDown { row: usize },
    CursorUp,
    CursorDown,
    PageUp,
    PageDown,
    Home,
    End,
    ActivateSelected,
    DismissSelected,
    ClearAllDone,
    ToggleBottomPanel,
    FocusIn,
    FocusOut,
    Escape,
}

fn skip_st_terminated(bytes: &[u8], mut i: usize) -> usize {
    let len = bytes.len();
    while i < len {
        if bytes[i] == 0x07 {
            return i + 1;
        }
        if bytes[i] == 0x1b && i + 1 < len && bytes[i + 1] == b'\\' {
            return i + 2;
        }
        i += 1;
    }
    len
}

pub fn parse_input_events(input: &str) -> Vec<InputEvent> {
    let mut events = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        if bytes[i] == 0x1b {
            if i + 1 == len {
                events.push(InputEvent::Escape);
                break;
            }
            let next_b = bytes[i + 1];
            if next_b == 0x1b {
                events.push(InputEvent::Escape);
                i += 1;
                continue;
            }
            if matches!(next_b, b']' | b'_' | b'P' | b'^' | b'X') {
                i = skip_st_terminated(bytes, i + 2);
                continue;
            }
            if next_b == b'[' {
                // SGR extended mouse: `\x1b[<btn;col;rowM` (press/motion) or `...m` (release)
                if i + 2 < len && bytes[i + 2] == b'<' {
                    let rest = &input[i + 3..];
                    if let Some(end_idx) = rest.find(['M', 'm']) {
                        let is_press = rest.as_bytes()[end_idx] == b'M';
                        let mut parts = rest[..end_idx].split(';');
                        if let (Some(b_s), Some(c_s), Some(r_s)) =
                            (parts.next(), parts.next(), parts.next())
                        {
                            if let (Ok(btn), Ok(col), Ok(row)) = (
                                b_s.parse::<u32>(),
                                c_s.parse::<usize>(),
                                r_s.parse::<usize>(),
                            ) {
                                if (btn & 64) != 0 {
                                    if is_press {
                                        if (btn & 1) == 0 {
                                            events.push(InputEvent::WheelUp { row });
                                        } else {
                                            events.push(InputEvent::WheelDown { row });
                                        }
                                    }
                                } else if (btn & 32) == 0 {
                                    let low_btn = (btn & 3) as u8;
                                    if is_press {
                                        events.push(InputEvent::MouseDown {
                                            btn: low_btn,
                                            col,
                                            row,
                                        });
                                    } else {
                                        events.push(InputEvent::MouseUp {
                                            btn: low_btn,
                                            col,
                                            row,
                                        });
                                    }
                                }
                            }
                        }
                        i += 3 + end_idx + 1;
                        continue;
                    }
                    // Truncated SGR tail at buffer boundary: discard cleanly, never emit Escape!
                    break;
                }

                // Standard CSI sequence: scan to final byte in `0x40..=0x7E`
                let mut j = i + 2;
                while j < len && !(0x40..=0x7e).contains(&bytes[j]) {
                    j += 1;
                }
                if j >= len {
                    break;
                }
                let final_b = bytes[j];
                let param = &input[i + 2..j];
                match (param, final_b) {
                    ("5", b'~') => events.push(InputEvent::PageUp),
                    ("6", b'~') => events.push(InputEvent::PageDown),
                    ("3", b'~') => events.push(InputEvent::DismissSelected),
                    ("1", b'~') | ("7", b'~') | ("", b'H') => events.push(InputEvent::Home),
                    ("4", b'~') | ("8", b'~') | ("", b'F') => events.push(InputEvent::End),
                    (_, b'A') => events.push(InputEvent::CursorUp),
                    (_, b'B') => events.push(InputEvent::CursorDown),
                    ("", b'I') => events.push(InputEvent::FocusIn),
                    ("", b'O') => events.push(InputEvent::FocusOut),
                    // Horizontal arrows (`C`/`D`) are intentionally ignored.
                    _ => {}
                }
                i = j + 1;
                continue;
            }
            if next_b == b'O' {
                if i + 2 < len {
                    match bytes[i + 2] {
                        b'A' => events.push(InputEvent::CursorUp),
                        b'B' => events.push(InputEvent::CursorDown),
                        b'H' => events.push(InputEvent::Home),
                        b'F' => events.push(InputEvent::End),
                        _ => {}
                    }
                }
                i = (i + 3).min(len);
                continue;
            }
            // Alt-modified key (`\x1b<ch>`): skip the 2-byte pair without firing Escape.
            i = (i + 2).min(len);
            continue;
        }

        match bytes[i] {
            b'k' | b'K' => events.push(InputEvent::CursorUp),
            b'j' | b'J' => events.push(InputEvent::CursorDown),
            b'\r' | b'\n' | b' ' => events.push(InputEvent::ActivateSelected),
            b'x' | b'X' | b'd' | b'D' | 0x7f | 0x08 => events.push(InputEvent::DismissSelected),
            b'c' | b'C' => events.push(InputEvent::ClearAllDone),
            b't' | b'T' | b'\t' => events.push(InputEvent::ToggleBottomPanel),
            b'g' => events.push(InputEvent::Home),
            b'G' => events.push(InputEvent::End),
            _ => {}
        }
        i += 1;
    }

    events
}

pub fn sync_scroll_to_cursor(
    cursor_idx: usize,
    sub_scroll_offset: &mut usize,
    viewport_h: usize,
    max_scroll: usize,
) {
    if cursor_idx == 0 {
        return;
    }
    let i_ag = cursor_idx - 1;
    let item_top = i_ag * 2;
    let item_bot = item_top + 2;
    if item_top < *sub_scroll_offset {
        *sub_scroll_offset = item_top.min(max_scroll);
    } else if viewport_h >= 2 && item_bot > *sub_scroll_offset + viewport_h {
        *sub_scroll_offset = item_bot.saturating_sub(viewport_h).min(max_scroll);
    }
}

pub fn default_cursor_for_active(active_view_id: &str, subagents: &[SubagentItem]) -> usize {
    if active_view_id == "main" {
        return 0;
    }
    subagents
        .iter()
        .position(|a| a.full_id == active_view_id)
        .map(|idx| idx + 1)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_press_release_wheel_and_ignores_drag_and_truncated_tails() {
        let stream = "\x1b[<64;10;18M\x1b[<65;10;18M\x1b[<0;37;14M\x1b[<32;20;9M\x1b[<0;37;14m\x1b[<2;8;16M\x1b[<32;12;";
        let parsed = parse_input_events(stream);
        assert_eq!(
            parsed,
            vec![
                InputEvent::WheelUp { row: 18 },
                InputEvent::WheelDown { row: 18 },
                InputEvent::MouseDown {
                    btn: 0,
                    col: 37,
                    row: 14
                },
                InputEvent::MouseUp {
                    btn: 0,
                    col: 37,
                    row: 14
                },
                InputEvent::MouseDown {
                    btn: 2,
                    col: 8,
                    row: 16
                },
            ]
        );
    }

    #[test]
    fn parses_arrows_and_never_emits_escape_on_focus_or_apc_replies() {
        let stream = "\x1b[I\x1b_Gi=71;OK\x1b\\\x1b[B\x1bOB\x1b[D\x1b[C\r\x1b[3~c\x1b[O\x1b";
        let parsed = parse_input_events(stream);
        assert_eq!(
            parsed,
            vec![
                InputEvent::FocusIn,
                InputEvent::CursorDown,
                InputEvent::CursorDown,
                InputEvent::ActivateSelected,
                InputEvent::DismissSelected,
                InputEvent::ClearAllDone,
                InputEvent::FocusOut,
                InputEvent::Escape,
            ]
        );
    }
}
