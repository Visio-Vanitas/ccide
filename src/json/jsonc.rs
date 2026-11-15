//! JSONC comment stripper and JSON string escaper (`src/json/jsonc.rs`).

/// Strips `//` line comments, `/* ... */` block comments, and leading `#` line comments
/// outside JSON string literals.
pub fn strip_jsonc(src: &str) -> String {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0usize;
    let mut in_str = false;
    let mut line_start_ws = true;

    while i < len {
        let b = bytes[i];
        if in_str {
            if b == b'\\' && i + 1 < len {
                if let Some(bs) = src[i..].chars().next() {
                    out.push(bs);
                    i += bs.len_utf8();
                }
                if let Some(ch) = src[i..].chars().next() {
                    out.push(ch);
                    i += ch.len_utf8();
                }
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            if let Some(ch) = src[i..].chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
            continue;
        }
        if b == b'"' {
            in_str = true;
            line_start_ws = false;
            out.push('"');
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }
        if line_start_ws && b == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'\n' {
            line_start_ws = true;
            out.push('\n');
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() {
            line_start_ws = false;
        }
        if let Some(ch) = src[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    out
}

/// Escapes `s` for safe embedding inside a double-quoted JSON string literal.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
