//! Lightweight zero-dependency top-level TOML scalar extractor (`src/json/toml_lite.rs`).
//!
//! Parses top-level `key = value` assignments occurring BEFORE the first `[table]` header
//! (sufficient for `$CODEX_HOME/config.toml` keys like `model`, `model_context_window`,
//! `model_reasoning_effort`).

fn strip_unquoted_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if in_dq {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_dq = false;
            }
        } else if in_sq {
            if b == b'\'' {
                in_sq = false;
            }
        } else {
            match b {
                b'"' => in_dq = true,
                b'\'' => in_sq = true,
                b'#' => return line[..i].trim_end(),
                _ => {}
            }
        }
        i += 1;
    }
    line.trim_end()
}

fn unescape_double_quoted(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                other => out.push(other),
            }
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

/// Extracts a top-level TOML scalar (`"..."`, `'...'`, integer, float, or boolean) for `key`
/// occurring before any `[table]` or `[[array]]` section header.
pub fn extract_toml_scalar(src: &str, key: &str) -> Option<String> {
    for raw_line in src.lines() {
        let line = strip_unquoted_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            break;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let k = lhs.trim().trim_matches('"').trim_matches('\'');
        if k != key {
            continue;
        }
        let val = rhs.trim();
        if val.is_empty() || val.starts_with('[') || val.starts_with('{') {
            return None;
        }
        if val.starts_with('"') {
            return unescape_double_quoted(val);
        }
        if let Some(rest) = val.strip_prefix('\'') {
            let end = rest.find('\'')?;
            return Some(rest[..end].to_string());
        }
        let clean: String = val
            .chars()
            .filter(|&c| c != '_')
            .take_while(|c| !c.is_whitespace())
            .collect();
        return (!clean.is_empty()).then_some(clean);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_top_level_toml_scalars_before_tables() {
        let toml = r#"
# Codex CLI configuration
model = "gpt-5.4" # inline comment with [brackets]
model_context_window = 272_000
model_reasoning_effort = 'high'
fast_mode = true

[projects."/workspace/demo"]
model = "o4-mini"
trust_level = "trusted"
"#;
        assert_eq!(
            extract_toml_scalar(toml, "model").as_deref(),
            Some("gpt-5.4")
        );
        assert_eq!(
            extract_toml_scalar(toml, "model_context_window").as_deref(),
            Some("272000")
        );
        assert_eq!(
            extract_toml_scalar(toml, "model_reasoning_effort").as_deref(),
            Some("high")
        );
        assert_eq!(
            extract_toml_scalar(toml, "fast_mode").as_deref(),
            Some("true")
        );
        assert_eq!(extract_toml_scalar(toml, "trust_level"), None);
    }
}
