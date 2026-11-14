//! Lightweight, zero-dependency JSON field extractors for hot-path `.jsonl` transcripts,
//! `settings.json`, and `sessions/*.json`.

/// Returns `true` if the byte at `quote_idx` in `src` is preceded by an odd number of `\` bytes.
fn is_escaped_quote(src: &str, quote_idx: usize) -> bool {
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
        let bytes = src.as_bytes();
        let mut depth = 0usize;
        let mut in_str = false;
        let mut i = obj_start;
        while i < bytes.len() {
            let b = bytes[i];
            if in_str {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == b'"' {
                    in_str = false;
                }
            } else {
                match b {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&src[obj_start..=i]);
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_strings_escapes_and_surrogates() {
        let src = r#"{"type":"assistant","text":"hello"}"#;
        assert_eq!(extract_json_str(src, "text").as_deref(), Some("hello"));
        assert_eq!(extract_json_str(src, "type").as_deref(), Some("assistant"));
        assert_eq!(extract_json_str(r#"{"a":"b"}"#, "zzz"), None);
        assert_eq!(extract_json_str(r#"{"a":1}"#, "a"), None);
        assert_eq!(extract_json_str("", "a"), None);
        assert_eq!(
            extract_json_str(r#"{"a": "b", "a" : "c"}"#, "a").as_deref(),
            Some("b")
        );
        assert_eq!(
            extract_json_str("{\"model\"  :\n  \"claude-opus\" }", "model").as_deref(),
            Some("claude-opus")
        );
        assert_eq!(
            extract_json_str(r#"{"s":"line1\nline2\ttab\rret\"quote\\slash\/end"}"#, "s")
                .as_deref(),
            Some("line1\nline2\ttab\rret\"quote\\slash/end")
        );
        let q = r#"{"s":"say \"hi\" now","after":"x"}"#;
        assert_eq!(extract_json_str(q, "s").as_deref(), Some("say \"hi\" now"));
        assert_eq!(extract_json_str(q, "after").as_deref(), Some("x"));
        assert_eq!(
            extract_json_str(r#"{"s":"中文 é"}"#, "s").as_deref(),
            Some("中文 é")
        );
        assert_eq!(
            extract_json_str(r#"{"s":"a🚀b"}"#, "s").as_deref(),
            Some("a🚀b")
        );
        assert_eq!(
            extract_json_str(r#"{"s":"a\uD83Db"}"#, "s").as_deref(),
            Some("ab")
        );
        assert_eq!(
            extract_json_str(r#"{"text_extra":"nope","text":"yes"}"#, "text").as_deref(),
            Some("yes")
        );
    }

    #[test]
    fn extracts_u64_edge_cases() {
        assert_eq!(extract_json_u64(r#"{"pid":4242}"#, "pid"), Some(4242));
        assert_eq!(extract_json_u64("{\"n\":\n 9001}", "n"), Some(9001));
        assert_eq!(extract_json_u64(r#"{"n":0}"#, "n"), Some(0));
        assert_eq!(extract_json_u64(r#"{"n":1}"#, "missing"), None);
        assert_eq!(extract_json_u64(r#"{"n":"12"}"#, "n"), None);
        assert_eq!(extract_json_u64(r#"{"n":-3}"#, "n"), None);
        assert_eq!(extract_json_u64("", "n"), None);
        assert_eq!(
            extract_json_u64(r#"{"n": null, "n" : 5150}"#, "n"),
            Some(5150)
        );
        assert_eq!(extract_json_u64(r#"{"n": 12, "m": 13}"#, "n"), Some(12));
        assert_eq!(
            extract_json_u64(r#"{"n":18446744073709551615}"#, "n"),
            Some(u64::MAX)
        );
        assert_eq!(extract_json_u64(r#"{"n":18446744073709551616}"#, "n"), None);
        assert_eq!(
            extract_json_u64(r#"{"msg":"\"n\":999","n":42}"#, "n"),
            Some(42)
        );
    }

    #[test]
    fn extracts_f64_bool_and_nested_objects() {
        let src = r#"{"ok":true,"disabled":false,"pct":87.5,"context_window":{"used_percentage":null,"current_usage":{"input_tokens":1200,"note":"brace } in \"str\""}},"rate_limits":{"five_hour":{"used_percentage":64.2}}}"#;
        assert_eq!(extract_json_bool(src, "ok"), Some(true));
        assert_eq!(extract_json_bool(src, "disabled"), Some(false));
        assert_eq!(extract_json_bool(src, "pct"), None);
        assert_eq!(extract_json_f64(src, "pct"), Some(87.5));
        let ctx = extract_json_object(src, "context_window").expect("context_window object");
        assert_eq!(extract_json_f64(ctx, "used_percentage"), None);
        let usage = extract_json_object(ctx, "current_usage").expect("current_usage object");
        assert_eq!(extract_json_u64(usage, "input_tokens"), Some(1200));
        let rl = extract_json_object(src, "rate_limits").expect("rate_limits object");
        assert_eq!(extract_json_f64(rl, "used_percentage"), Some(64.2));
    }
}
