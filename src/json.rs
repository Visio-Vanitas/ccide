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
                            if let Ok(code) = u32::from_str_radix(&hex, 16)
                                && let Some(ch) = char::from_u32(code)
                            {
                                out.push(ch);
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
        let digits: String = val_part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty()
            && let Ok(v) = digits.parse::<u64>()
        {
            return Some(v);
        }
    }
    None
}
