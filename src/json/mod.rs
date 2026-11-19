//! Lightweight, zero-dependency JSON field extractors, balanced scanners, and JSONC helpers.

pub mod extract;
pub mod jsonc;
pub mod scan;
pub mod toml_lite;

pub use extract::*;
pub use jsonc::*;
pub use scan::*;
pub use toml_lite::*;

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
    fn extracts_f64_bool_nested_objects_and_arrays() {
        let src = r#"{"ok":true,"disabled":false,"pct":87.5,"note":"\"items\":[9]","items":["a","b"],"context_window":{"used_percentage":null,"current_usage":{"input_tokens":1200,"note":"brace } in \"str\""}},"rate_limits":{"five_hour":{"used_percentage":64.2}}}"#;
        assert_eq!(extract_json_bool(src, "ok"), Some(true));
        assert_eq!(extract_json_bool(src, "disabled"), Some(false));
        assert_eq!(extract_json_bool(src, "pct"), None);
        assert_eq!(extract_json_f64(src, "pct"), Some(87.5));
        let arr = extract_json_array_slice(src, "items").expect("items array");
        assert_eq!(extract_string_list_in_array(arr), vec!["a", "b"]);
        let ctx = extract_json_object(src, "context_window").expect("context_window object");
        assert_eq!(extract_json_f64(ctx, "used_percentage"), None);
        let usage = extract_json_object(ctx, "current_usage").expect("current_usage object");
        assert_eq!(extract_json_u64(usage, "input_tokens"), Some(1200));
        let rl = extract_json_object(src, "rate_limits").expect("rate_limits object");
        assert_eq!(extract_json_f64(rl, "used_percentage"), Some(64.2));
    }
}
