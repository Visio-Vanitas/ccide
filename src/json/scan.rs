//! Balanced JSON array/object span scanners (`src/json/scan.rs`).

use super::extract::is_escaped_quote;

/// Finds the matching `close_b` index balancing `open_b` starting at `start`, skipping string literals.
pub fn find_balanced_span(bytes: &[u8], start: usize, open_b: u8, close_b: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_str = false;
    let mut i = start;
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
        } else if b == b'"' {
            in_str = true;
        } else if b == open_b {
            depth += 1;
        } else if b == close_b {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Extracts the balanced `[ ... ]` array slice for `"<key>": [ ... ]`, ignoring escaped quotes.
pub fn extract_json_array_slice<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{}\"", key);
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(&needle) {
        let match_start = from + rel;
        let pos = match_start + needle.len();
        from = pos;
        if is_escaped_quote(src, match_start) {
            continue;
        }
        let rest = src[pos..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let val = rest[1..].trim_start();
        if !val.starts_with('[') {
            continue;
        }
        let arr_start = src.len() - val.len();
        let end = find_balanced_span(src.as_bytes(), arr_start, b'[', b']')?;
        return Some(&src[arr_start..=end]);
    }
    None
}

/// Iterates top-level `{ ... }` object slices inside a `[ ... ]` JSON array slice.
pub fn iter_array_objects(arr_slice: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = arr_slice.as_bytes();
    let mut i = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let Some(j) = find_balanced_span(bytes, i, b'{', b'}') else {
                break;
            };
            out.push(&arr_slice[i..=j]);
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// Extracts quoted string elements inside a `[ ... ]` JSON array slice.
pub fn extract_string_list_in_array(arr_slice: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = arr_slice.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'"' {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() {
                out.push(unescape_key(&arr_slice[start..j]));
                i = j + 1;
                continue;
            }
            break;
        }
        i += 1;
    }
    out
}

fn unescape_key(raw: &str) -> String {
    if !raw.contains('\\') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Iterates top-level `(unescaped_key, raw_value_slice)` entries inside a `{ ... }` JSON object slice,
/// stopping cleanly at the outer closing `}`.
pub fn iter_object_members(obj_slice: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let Some(open_idx) = obj_slice.find('{') else {
        return out;
    };
    let bytes = obj_slice.as_bytes();
    let mut i = open_idx + 1;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'}' || bytes[i] != b'"' {
            break;
        }
        let k_start = i + 1;
        let mut k_end = k_start;
        while k_end < bytes.len() {
            if bytes[k_end] == b'\\' {
                k_end += 2;
                continue;
            }
            if bytes[k_end] == b'"' {
                break;
            }
            k_end += 1;
        }
        if k_end >= bytes.len() {
            break;
        }
        let key = unescape_key(&obj_slice[k_start..k_end]);
        let mut j = k_end + 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b':' {
            break;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        let v_start = j;
        match bytes[v_start] {
            b'{' => {
                let Some(v_end) = find_balanced_span(bytes, v_start, b'{', b'}') else {
                    break;
                };
                out.push((key, &obj_slice[v_start..=v_end]));
                i = v_end + 1;
            }
            b'[' => {
                let Some(v_end) = find_balanced_span(bytes, v_start, b'[', b']') else {
                    break;
                };
                out.push((key, &obj_slice[v_start..=v_end]));
                i = v_end + 1;
            }
            b'"' => {
                let mut q = v_start + 1;
                while q < bytes.len() {
                    if bytes[q] == b'\\' {
                        q += 2;
                        continue;
                    }
                    if bytes[q] == b'"' {
                        break;
                    }
                    q += 1;
                }
                if q >= bytes.len() {
                    break;
                }
                out.push((key, &obj_slice[v_start..=q]));
                i = q + 1;
            }
            _ => {
                let mut q = v_start;
                while q < bytes.len() && !matches!(bytes[q], b',' | b'}') {
                    q += 1;
                }
                let raw_v = obj_slice[v_start..q].trim_end();
                if !raw_v.is_empty() {
                    out.push((key, raw_v));
                }
                i = q;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterates_object_members_with_escapes_nesting_and_trailing_text() {
        assert!(iter_object_members("{}").is_empty());
        assert!(iter_object_members("  {  } ").is_empty());
        let src = r#"{"a\"1":{"nested":[1,2],"note":"}"},"b":[true,false],"c":"str","d":42},"trailing":{"ignore":1}"#;
        let members = iter_object_members(src);
        assert_eq!(members.len(), 4);
        assert_eq!(members[0].0, "a\"1");
        assert_eq!(members[0].1, r#"{"nested":[1,2],"note":"}"}"#);
        assert_eq!(members[1], ("b".to_string(), "[true,false]"));
        assert_eq!(members[2], ("c".to_string(), "\"str\""));
        assert_eq!(members[3], ("d".to_string(), "42"));
        assert_eq!(
            extract_string_list_in_array(r#"["file:\/\/\/C:\/repo","C:\\Users\\dev"]"#),
            vec!["file:///C:/repo", r"C:\Users\dev"]
        );
    }
}
