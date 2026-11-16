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
            depth -= 1;
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
                out.push(arr_slice[start..j].to_string());
                i = j + 1;
                continue;
            }
            break;
        }
        i += 1;
    }
    out
}
