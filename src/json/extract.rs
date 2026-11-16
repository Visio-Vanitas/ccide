//! Scalar and balanced-object JSON field extractors (`src/json/extract.rs`).

/// Returns `true` if the byte at `quote_idx` in `src` is preceded by an odd number of `\` bytes.
pub(crate) fn is_escaped_quote(src: &str, quote_idx: usize) -> bool {
    let bytes = src.as_bytes();
    let mut backslashes = 0usize;
    let mut idx = quote_idx;
    while idx > 0 && bytes[idx - 1] == b'\\' {
        backslashes += 1;
        idx -= 1;
    }
    backslashes % 2 == 1
}

/// Extracts the string value of `"<key>": "<value>"` from a JSON snippet, handling escapes.
pub fn extract_json_str(src: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let mut search_from = 0;
    while let Some(rel_pos) = src[search_from..].find(&needle) {
        let match_start = search_from + rel_pos;
        let pos = match_start + needle.len();
        search_from = pos;
        if is_escaped_quote(src, match_start) {
            continue;
        }
        let rest = src[pos..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let val_part = rest[1..].trim_start();
        if !val_part.starts_with('"') {
            continue;
        }
        let mut out = String::new();
        let mut chars = val_part[1..].chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(esc) = chars.next() {
                    match esc {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'u' => {
                            let hex: String = chars.by_ref().take(4).collect();
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if (0xD800..=0xDBFF).contains(&code) {
                                    let mut look = chars.clone();
                                    if look.next() == Some('\\') && look.next() == Some('u') {
                                        let lo_hex: String = look.by_ref().take(4).collect();
                                        if let Ok(lo) = u32::from_str_radix(&lo_hex, 16) {
                                            if (0xDC00..=0xDFFF).contains(&lo) {
                                                let cp = 0x10000
                                                    + (((code - 0xD800) << 10) | (lo - 0xDC00));
                                                if let Some(ch) = char::from_u32(cp) {
                                                    out.push(ch);
                                                }
                                                chars = look;
                                                continue;
                                            }
                                        }
                                    }
                                } else if let Some(ch) = char::from_u32(code) {
                                    out.push(ch);
                                }
                            }
                        }
                        other => out.push(other),
                    }
                }
            } else if c == '"' {
                return Some(out);
            } else {
                out.push(c);
            }
        }
    }
    None
}

/// Extracts an unsigned integer value of `"<key>": <number>` from a JSON snippet.
pub fn extract_json_u64(src: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{}\"", key);
    let mut search_from = 0;
    while let Some(rel_pos) = src[search_from..].find(&needle) {
        let match_start = search_from + rel_pos;
        let pos = match_start + needle.len();
        search_from = pos;
        if is_escaped_quote(src, match_start) {
            continue;
        }
        let rest = src[pos..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let val_part = rest[1..].trim_start();
        let digits: String = val_part
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            if let Ok(v) = digits.parse::<u64>() {
                return Some(v);
            }
        }
    }
    None
}

/// Extracts a finite floating-point value of `"<key>": <number>` from a JSON snippet.
pub fn extract_json_f64(src: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{}\"", key);
    let mut search_from = 0;
    while let Some(rel_pos) = src[search_from..].find(&needle) {
        let match_start = search_from + rel_pos;
        let pos = match_start + needle.len();
        search_from = pos;
        if is_escaped_quote(src, match_start) {
            continue;
        }
        let rest = src[pos..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let val_part = rest[1..].trim_start();
        if !matches!(val_part.chars().next(), Some(c) if c.is_ascii_digit() || c == '-') {
            continue;
        }
        let num_str: String = val_part
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            .collect();
        if let Ok(v) = num_str.parse::<f64>() {
            if v.is_finite() {
                return Some(v);
            }
        }
    }
    None
}

/// Extracts a boolean literal value of `"<key>": true | false` from a JSON snippet.
pub fn extract_json_bool(src: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{}\"", key);
    let mut search_from = 0;
    while let Some(rel_pos) = src[search_from..].find(&needle) {
        let match_start = search_from + rel_pos;
        let pos = match_start + needle.len();
        search_from = pos;
        if is_escaped_quote(src, match_start) {
            continue;
        }
        let rest = src[pos..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let val_part = rest[1..].trim_start();
        if let Some(after) = val_part.strip_prefix("true") {
            if after
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(true)
            {
                return Some(true);
            }
        } else if let Some(after) = val_part.strip_prefix("false") {
            if after
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(true)
            {
                return Some(false);
            }
        }
    }
    None
}

/// Extracts the balanced `{ ... }` object slice for `"<key>": { ... }`, ignoring braces inside strings.
pub fn extract_json_object<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{}\"", key);
    let mut search_from = 0;
    while let Some(rel_pos) = src[search_from..].find(&needle) {
        let match_start = search_from + rel_pos;
        let pos = match_start + needle.len();
        search_from = pos;
        if is_escaped_quote(src, match_start) {
            continue;
        }
        let rest = src[pos..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let val_trimmed = rest[1..].trim_start();
        if !val_trimmed.starts_with('{') {
            continue;
        }
        let obj_start = src.len() - val_trimmed.len();
        let end = super::scan::find_balanced_span(src.as_bytes(), obj_start, b'{', b'}')?;
        return Some(&src[obj_start..=end]);
    }
    None
}
