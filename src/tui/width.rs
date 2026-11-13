//! East-Asian (CJK) display column width measurement, UTF-8 safe boundary slicing, and ANSI-aware truncation.

/// LTS-compatible equivalent of `str::floor_char_boundary` (compatible with Rust 1.70 LTS).
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        s.len()
    } else {
        let mut i = index;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// Returns the terminal column width (`0`, `1`, or `2`) of a single Unicode scalar value.
pub fn char_display_width(ch: char) -> usize {
    let u = ch as u32;
    if u < 0x20 || (0x7F..0xA0).contains(&u) {
        return 0;
    }
    if (0x0300..=0x036F).contains(&u)
        || (0x200B..=0x200F).contains(&u)
        || (0xFE00..=0xFE0F).contains(&u)
    {
        return 0;
    }
    if matches!(
        u,
        0x231A..=0x231B
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
    ) {
        return 2;
    }
    if (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0x303E).contains(&u)
        || (0x3040..=0x33BF).contains(&u)
        || (0x3400..=0x4DBF).contains(&u)
        || (0x4E00..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE10..=0xFE6F).contains(&u)
        || (0xFF01..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x1F300..=0x1FAFF).contains(&u)
        || (0x20000..=0x3FFFF).contains(&u)
    {
        return 2;
    }
    1
}

/// Computes the visible terminal column width of `s`, ignoring CSI (`\x1b[...m`) and OSC (`\x1b]...\x07`) sequences.
pub fn display_width(s: &str) -> usize {
    let mut w = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next();
                    for esc_c in chars.by_ref() {
                        if ('@'..='~').contains(&esc_c) {
                            break;
                        }
                    }
                } else if next == ']' {
                    chars.next();
                    for esc_c in chars.by_ref() {
                        if esc_c == '\x07' {
                            break;
                        }
                    }
                }
            }
            continue;
        }
        w += char_display_width(c);
    }
    w
}

/// Truncates `s` so its visible column width never exceeds `max_cols`, preserving ANSI CSI/OSC
/// escape sequences and appending `…` (plus `\x1b[0m` when ANSI sequences were present).
pub fn trunc_to_width(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    let limit = max_cols.saturating_sub(1);
    let mut out = String::with_capacity(s.len());
    let mut cur = 0usize;
    let mut saw_ansi = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            saw_ansi = true;
            out.push(ch);
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    out.push(chars.next().unwrap_or('['));
                    for esc_c in chars.by_ref() {
                        out.push(esc_c);
                        if ('@'..='~').contains(&esc_c) {
                            break;
                        }
                    }
                } else if next == ']' {
                    out.push(chars.next().unwrap_or(']'));
                    for esc_c in chars.by_ref() {
                        out.push(esc_c);
                        if esc_c == '\x07' {
                            break;
                        }
                    }
                }
            }
            continue;
        }
        let cw = char_display_width(ch);
        if cur + cw > limit {
            break;
        }
        out.push(ch);
        cur += cw;
    }
    out.push('…');
    if saw_ansi {
        out.push_str("\x1b[0m");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_boundary_clamps_past_end() {
        assert_eq!(floor_char_boundary("abc", 99), 3);
        assert_eq!(floor_char_boundary("", 0), 0);
        assert_eq!(floor_char_boundary("abc", 3), 3);
    }

    #[test]
    fn floor_boundary_snaps_inside_multibyte_cjk() {
        // '级' is U+7EA7, encoded as 3 bytes (E7 BA A7).
        let s = "a级b";
        assert_eq!(s.len(), 5);
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 1); // on 'a'
        for idx in 2..4 {
            assert_eq!(floor_char_boundary(s, idx), 1, "inside '级' at {idx}");
        }
        assert_eq!(floor_char_boundary(s, 4), 4); // on 'b'
    }

    #[test]
    fn floor_boundary_snaps_inside_emoji() {
        // '🚀' is U+1F680, encoded as 4 bytes (F0 9F 9A 80).
        let s = "x🚀";
        assert_eq!(s.len(), 5);
        for idx in 2..4 {
            assert_eq!(floor_char_boundary(s, idx), 1, "inside emoji at {idx}");
        }
        assert_eq!(floor_char_boundary(s, 5), 5);
        // Result must always be a valid char boundary.
        for idx in 0..=s.len() {
            assert!(s.is_char_boundary(floor_char_boundary(s, idx)));
        }
    }

    #[test]
    fn char_width_categories() {
        assert_eq!(char_display_width('a'), 1);
        assert_eq!(char_display_width('级'), 2);
        assert_eq!(char_display_width('🚀'), 2);
        assert_eq!(char_display_width('\u{0301}'), 0); // combining accent
        assert_eq!(char_display_width('\u{200B}'), 0); // zero-width space
        assert_eq!(char_display_width('\x1b'), 0); // control
    }

    #[test]
    fn display_width_plain_and_cjk() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("初级"), 4);
        assert_eq!(display_width("a级"), 3);
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn display_width_ignores_csi_sequences() {
        assert_eq!(display_width("\x1b[31mred\x1b[0m"), 3);
        assert_eq!(display_width("\x1b[1;38;2;68;74;94m级\x1b[0m"), 2);
        assert_eq!(display_width("\x1b[2J"), 0);
    }

    #[test]
    fn display_width_ignores_osc_sequences() {
        assert_eq!(display_width("\x1b]0;title\x07ok"), 2);
        assert_eq!(display_width("\x1b]8;;http://x\x07link\x1b]8;;\x07"), 4);
        // Unterminated OSC swallows the remainder rather than counting it.
        assert_eq!(display_width("\x1b]unterminated"), 0);
    }

    #[test]
    fn trunc_to_width_zero_returns_empty() {
        assert_eq!(trunc_to_width("abc", 0), "");
        assert_eq!(trunc_to_width("级级", 0), "");
    }

    #[test]
    fn trunc_to_width_keeps_string_that_fits() {
        assert_eq!(trunc_to_width("abc", 3), "abc");
        assert_eq!(trunc_to_width("级级", 4), "级级");
        assert_eq!(trunc_to_width("", 5), "");
    }

    #[test]
    fn trunc_to_width_appends_ellipsis_and_respects_budget() {
        let out = trunc_to_width("abcdef", 4);
        assert_eq!(out, "abc…");
        assert_eq!(display_width(&out), 4);

        // CJK: budget 3 -> 1 column reserved for '…', only one wide glyph fits.
        let out = trunc_to_width("级级级", 3);
        assert_eq!(out, "级…");
        assert!(display_width(&out) <= 3);

        // Budget of 2 fits no wide glyph before the ellipsis.
        let out = trunc_to_width("级级", 2);
        assert_eq!(out, "…");
        assert_eq!(display_width(&out), 1);
    }

    #[test]
    fn trunc_to_width_output_never_exceeds_budget() {
        let samples = ["hello world", "初级终端渲染", "a🚀b🚀c", "mixed 级 text 🚀"];
        for s in samples {
            for max in 1..=display_width(s) + 2 {
                let out = trunc_to_width(s, max);
                assert!(
                    display_width(&out) <= max,
                    "trunc_to_width({s:?}, {max}) -> {out:?} (width {})",
                    display_width(&out)
                );
            }
        }
    }
}
