//! SGR Mouse & Keyboard Escape-Sequence Parser for `ccide-core sidebar`.

use crate::session::transcript::SubagentItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    LeftClick { col: usize, row: usize },
    SecondaryClick { row: usize },
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
    Escape,
}

pub fn parse_input_events(input: &str) -> Vec<InputEvent> {
    let mut events = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        if bytes[i] == 0x1b {
            if i + 2 < len && bytes[i + 1] == b'[' && bytes[i + 2] == b'<' {
                let rest = &input[i + 3..];
                if let Some(end_idx) = rest.find(['M', 'm']) {
                    let kind = rest.as_bytes()[end_idx] as char;
                    let body = &rest[..end_idx];
                    let mut parts = body.split(';');
                    if let (Some(b_str), Some(c_str), Some(r_str)) =
                        (parts.next(), parts.next(), parts.next())
                    {
                        if kind == 'M' {
                            if let (Ok(btn), Ok(col), Ok(row)) = (
                                b_str.parse::<u32>(),
                                c_str.parse::<usize>(),
                                r_str.parse::<usize>(),
                            ) {
                                match btn {
                                    0 => events.push(InputEvent::LeftClick { col, row }),
                                    1 | 2 => events.push(InputEvent::SecondaryClick { row }),
                                    64 => events.push(InputEvent::WheelUp { row }),
                                    65 => events.push(InputEvent::WheelDown { row }),
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
                let mode = bytes[i + 1];
                let code = bytes[i + 2];
                if mode == b'[' && code.is_ascii_digit() {
                    let mut j = i + 2;
                    while j < len && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                        j += 1;
                    }
                    if j < len {
                        let final_b = bytes[j];
                        let param = &input[i + 2..j];
                        match (param, final_b) {
                            ("5", b'~') => events.push(InputEvent::PageUp),
                            ("6", b'~') => events.push(InputEvent::PageDown),
                            ("3", b'~') => events.push(InputEvent::DismissSelected),
                            ("1", b'~') | ("7", b'~') => events.push(InputEvent::Home),
                            ("4", b'~') | ("8", b'~') => events.push(InputEvent::End),
                            (_, b'A') => events.push(InputEvent::CursorUp),
                            (_, b'B') => events.push(InputEvent::CursorDown),
                            (_, b'C') => events.push(InputEvent::ActivateSelected),
                            (_, b'D') => events.push(InputEvent::DismissSelected),
                            _ => {}
                        }
                        i = j + 1;
                        continue;
                    }
                } else {
                    match code {
                        b'A' => events.push(InputEvent::CursorUp),
                        b'B' => events.push(InputEvent::CursorDown),
                        b'C' => events.push(InputEvent::ActivateSelected),
                        b'D' => events.push(InputEvent::DismissSelected),
                        b'H' => events.push(InputEvent::Home),
                        b'F' => events.push(InputEvent::End),
                        _ => {}
                    }
                    i += 3;
                    continue;
                }
            }

            events.push(InputEvent::Escape);
            i += 1;
            continue;
        }

        match bytes[i] {
            b'k' | b'K' => events.push(InputEvent::CursorUp),
            b'j' | b'J' => events.push(InputEvent::CursorDown),
            b'\r' | b'\n' | b' ' | b'l' | b'L' => events.push(InputEvent::ActivateSelected),
            b'x' | b'X' | b'd' | b'D' | b'h' | b'H' | 0x7f | 0x08 => {
                events.push(InputEvent::DismissSelected);
            }
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
    fn parses_sgr_wheel_and_click_events() {
        let stream = "\x1b[<64;10;18M\x1b[<65;10;18M\x1b[<0;37;14M\x1b[<0;37;14m\x1b[<2;8;16M";
        let parsed = parse_input_events(stream);
        assert_eq!(
            parsed,
            vec![
                InputEvent::WheelUp { row: 18 },
                InputEvent::WheelDown { row: 18 },
                InputEvent::LeftClick { col: 37, row: 14 },
                InputEvent::SecondaryClick { row: 16 },
            ]
        );
    }

    #[test]
    fn parses_arrow_bursts_and_keyboard_shortcuts() {
        let stream = "\x1b[B\x1bOB\r\x1b[3~c\x1b";
        let parsed = parse_input_events(stream);
        assert_eq!(
            parsed,
            vec![
                InputEvent::CursorDown,
                InputEvent::CursorDown,
                InputEvent::ActivateSelected,
                InputEvent::DismissSelected,
                InputEvent::ClearAllDone,
                InputEvent::Escape,
            ]
        );
    }

    #[test]
    fn sync_scroll_tracks_cursor_into_viewport() {
        let mut off = 0usize;
        sync_scroll_to_cursor(3, &mut off, 6, 14);
        assert_eq!(off, 0);
        sync_scroll_to_cursor(4, &mut off, 6, 14);
        assert_eq!(off, 2);
        sync_scroll_to_cursor(1, &mut off, 6, 14);
        assert_eq!(off, 0);
    }
}
