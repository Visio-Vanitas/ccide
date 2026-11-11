//! Lightweight, zero-dependency JSON field extractors for hot-path `.jsonl` transcripts,
//! `settings.json`, and `sessions/*.json`.

/// Extracts the string value of `"<key>": "<value>"` from a JSON snippet, handling escapes.
pub fn extract_json_str(src: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let mut search_from = 0;
    while let Some(rel_pos) = src[search_from..].find(&needle) {
        let pos = search_from + rel_pos + needle.len();
        search_from = pos;
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
                                if let Some(ch) = char::from_u32(code) {
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
        let pos = search_from + rel_pos + needle.len();
        search_from = pos;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_string() {
        let src = r#"{"type":"assistant","text":"hello"}"#;
        assert_eq!(extract_json_str(src, "text").as_deref(), Some("hello"));
        assert_eq!(extract_json_str(src, "type").as_deref(), Some("assistant"));
    }

    #[test]
    fn returns_none_for_missing_key_or_value() {
        assert_eq!(extract_json_str(r#"{"a":"b"}"#, "zzz"), None);
        assert_eq!(extract_json_str(r#"{"a":1}"#, "a"), None);
        assert_eq!(extract_json_str("", "a"), None);
        assert_eq!(
            extract_json_str(r#"{"a": "b", "a" : "c"}"#, "a").as_deref(),
            Some("b")
        );
    }

    #[test]
    fn tolerates_whitespace_around_colon() {
        let src = "{\"model\"  :\n  \"claude-opus\" }";
        assert_eq!(
            extract_json_str(src, "model").as_deref(),
            Some("claude-opus")
        );
    }

    #[test]
    fn decodes_simple_escapes() {
        let src = r#"{"s":"line1\nline2\ttab\rret\"quote\\slash\/end"}"#;
        assert_eq!(
            extract_json_str(src, "s").as_deref(),
            Some("line1\nline2\ttab\rret\"quote\\slash/end")
        );
    }

    #[test]
    fn escaped_quotes_do_not_terminate_value() {
        let src = r#"{"s":"say \"hi\" now","after":"x"}"#;
        assert_eq!(
            extract_json_str(src, "s").as_deref(),
            Some("say \"hi\" now")
        );
        assert_eq!(extract_json_str(src, "after").as_deref(), Some("x"));
    }

    #[test]
    fn decodes_u_escapes() {
        let src = r#"{"s":"中文 é"}"#;
        assert_eq!(extract_json_str(src, "s").as_deref(), Some("中文 é"));
    }

    #[test]
    fn decodes_surrogate_pair_u_escapes_without_panicking() {
        // Each \uXXXX is decoded independently; lone surrogates yield no char.
        // The surrounding text must still survive intact.
        let src = r#"{"s":"a🚀b"}"#;
        let got = extract_json_str(src, "s").expect("value");
        assert!(got.starts_with('a'), "got {got:?}");
        assert!(got.ends_with('b'), "got {got:?}");
    }

    #[test]
    fn exact_quoted_key_is_required() {
        // `"text_extra"` must not satisfy a lookup for `text`.
        let src = r#"{"text_extra":"nope","text":"yes"}"#;
        assert_eq!(extract_json_str(src, "text").as_deref(), Some("yes"));
    }

    #[test]
    fn extracts_u64_plain_and_with_whitespace() {
        assert_eq!(extract_json_u64(r#"{"pid":4242}"#, "pid"), Some(4242));
        assert_eq!(extract_json_u64("{\"n\":\n 9001}", "n"), Some(9001));
        assert_eq!(extract_json_u64(r#"{"n":0}"#, "n"), Some(0));
    }

    #[test]
    fn extract_u64_missing_or_non_numeric() {
        assert_eq!(extract_json_u64(r#"{"n":1}"#, "missing"), None);
        assert_eq!(extract_json_u64(r#"{"n":"12"}"#, "n"), None);
        assert_eq!(extract_json_u64(r#"{"n":-3}"#, "n"), None);
        assert_eq!(extract_json_u64("", "n"), None);
    }

    #[test]
    fn extract_u64_skips_non_value_occurrence() {
        // The first `"n"` has a non-numeric value, so the scan advances to the
        // next occurrence of the key.
        assert_eq!(
            extract_json_u64(r#"{"n": null, "n" : 5150}"#, "n"),
            Some(5150)
        );
        assert_eq!(extract_json_u64(r#"{"n": 12, "m": 13}"#, "n"), Some(12));
    }

    #[test]
    fn extract_u64_handles_large_values_and_overflow() {
        assert_eq!(
            extract_json_u64(r#"{"n":18446744073709551615}"#, "n"),
            Some(u64::MAX)
        );
        assert_eq!(extract_json_u64(r#"{"n":18446744073709551616}"#, "n"), None);
    }
}
