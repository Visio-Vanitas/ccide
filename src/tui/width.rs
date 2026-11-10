//! East-Asian (CJK) display column width measurement and ANSI-aware truncation.

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

/// Truncates `s` so its visible column width never exceeds `max_cols`, appending `…` when truncated.
pub fn trunc_to_width(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    let limit = max_cols.saturating_sub(1);
    let mut out = String::new();
    let mut cur = 0usize;
    for ch in s.chars() {
        let cw = char_display_width(ch);
        if cur + cw > limit {
            break;
        }
        out.push(ch);
        cur += cw;
    }
    out.push('…');
    out
}
