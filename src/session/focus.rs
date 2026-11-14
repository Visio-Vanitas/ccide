//! Tool-Target Line Extraction & Yazi Preview Focus/Highlight IPC (`src/session/focus.rs`).

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::{extract_json_object, extract_json_str, extract_json_u64};
use crate::platform::{
    clean_canonicalize, find_executable, ipc_dir, ipc_path, normalize_slashes, yazi_config_dir,
};
use crate::session::normalize_path_under_cwd;

const MAX_SNIPPET_SCAN_BYTES: u64 = 4 * 1024 * 1024;

/// Matches `url_hash(s)` in `assets/yazi-hscroll.lua` bit-for-bit across Unix and Windows (`/` normalized).
pub fn yazi_url_hash(s: &str) -> String {
    let norm = normalize_slashes(s);
    let mut h1: u64 = 5381;
    let mut h2: u64 = 52711;
    for &b in norm.as_bytes() {
        h1 = ((h1 * 33) + u64::from(b)) % 2_147_483_647;
        h2 = ((h2 * 131) + u64::from(b)) % 2_147_483_647;
    }
    format!("{:08x}{:08x}", h1, h2)
}

fn find_snippet_line_in_file(file_path: &str, snippet: &str) -> Option<(usize, usize)> {
    let trimmed = snippet.trim_matches('\r');
    if trimmed.is_empty() {
        return None;
    }
    let meta = fs::metadata(file_path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SNIPPET_SCAN_BYTES {
        return None;
    }
    let content = fs::read_to_string(file_path).ok()?;
    let span = trimmed.lines().count().max(1);
    if let Some(pos) = content.find(trimmed) {
        let line = 1 + content.as_bytes()[..pos]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        return Some((line, span));
    }
    let anchor = trimmed.lines().map(|l| l.trim()).find(|l| l.len() >= 4)?;
    for (idx, file_ln) in content.lines().enumerate() {
        if file_ln.contains(anchor) {
            return Some((idx + 1, span));
        }
    }
    None
}

/// Resolves `(1-based start_line, span_lines)` for a Claude tool operation on `resolved_file`.
pub fn extract_tool_line_target(
    raw_json: &str,
    tool_name: &str,
    resolved_file: &str,
) -> (usize, usize) {
    if let Some(ns) = extract_json_u64(raw_json, "newStart") {
        if ns >= 1 {
            let nl = extract_json_u64(raw_json, "newLines").unwrap_or(1).max(1) as usize;
            return (ns as usize, nl.min(120));
        }
    }

    match tool_name {
        "Edit" | "Update" | "FileEdit" | "StrReplace" | "MultiEdit" => {
            let cand = extract_json_str(raw_json, "new_string")
                .or_else(|| extract_json_str(raw_json, "new_str"))
                .or_else(|| extract_json_str(raw_json, "old_string"))
                .unwrap_or_default();
            if let Some((ln, sp)) = find_snippet_line_in_file(resolved_file, &cand) {
                return (ln, sp.min(120));
            }
        }
        "NotebookEdit" => {
            let cand = extract_json_str(raw_json, "new_source")
                .or_else(|| extract_json_str(raw_json, "cell_id"))
                .unwrap_or_default();
            if let Some((ln, sp)) = find_snippet_line_in_file(resolved_file, &cand) {
                return (ln, sp.min(80));
            }
        }
        "Read" => {
            if let Some(f_obj) = extract_json_object(raw_json, "file") {
                if let Some(sl) = extract_json_u64(f_obj, "startLine") {
                    if sl >= 1 {
                        let nl = extract_json_u64(f_obj, "numLines").unwrap_or(1).clamp(1, 6);
                        return (sl as usize, nl as usize);
                    }
                }
            }
            if let Some(off) = extract_json_u64(raw_json, "offset") {
                if off >= 1 {
                    let lim = extract_json_u64(raw_json, "limit").unwrap_or(1).clamp(1, 6);
                    return (off as usize, lim as usize);
                }
            }
        }
        "LSP" => {
            if let Some(ln) = extract_json_u64(raw_json, "line") {
                if ln >= 1 {
                    return (ln as usize, 1);
                }
            }
        }
        "Grep" => {
            if let Some(pat) = extract_json_str(raw_json, "pattern") {
                if let Some((ln, _)) = find_snippet_line_in_file(resolved_file, &pat) {
                    return (ln, 1);
                }
            }
        }
        "Write" => {
            return (1, 1);
        }
        _ => {}
    }

    (0, 0)
}

fn write_focus_marker(path_key: &str, line: usize, span: usize, pending: bool, now_s: u64) {
    if path_key.is_empty() {
        return;
    }
    let fpath = ipc_path(&format!("yf_{}.txt", yazi_url_hash(path_key)));
    if line == 0 {
        let _ = fs::remove_file(&fpath);
        return;
    }
    let tmp = ipc_path(&format!(
        "yf_{}.txt.tmp.{}",
        yazi_url_hash(path_key),
        std::process::id()
    ));
    let body = format!(
        "{}\n{}\n{}\n{}\n",
        line,
        span.max(1),
        u8::from(pending),
        now_s
    );
    if fs::write(&tmp, &body).is_ok() && fs::rename(&tmp, &fpath).is_err() {
        let _ = fs::write(&fpath, &body);
        let _ = fs::remove_file(&tmp);
    }
}

/// Reveals `file_path` in the tab's Yazi pane AND scrolls/highlights `(line, span)` in preview.
pub fn emit_yazi_reveal_at(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    file_path: &str,
    line: usize,
    span: usize,
) {
    if tab_yazi_id.is_empty() || file_path.is_empty() || !Path::new(file_path).is_file() {
        return;
    }
    let mapped = normalize_path_under_cwd(file_path, target_cwd);
    let _ = fs::create_dir_all(ipc_dir());
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    write_focus_marker(&mapped, line, span, true, now_s);
    if let Ok(canon) = clean_canonicalize(file_path) {
        let canon_s = canon.to_string_lossy();
        if canon_s != mapped {
            write_focus_marker(&canon_s, line, span, false, now_s);
        }
    }

    let Some(ya_bin) = find_executable("ya", home) else {
        return;
    };
    let hscroll_plugin = yazi_config_dir(home).join("plugins/hscroll.yazi/main.lua");
    let mut cmd = Command::new(ya_bin);
    if hscroll_plugin.exists() {
        let escaped = mapped.replace('\'', "'\\''");
        let plugin_arg = if line > 0 {
            format!("reveal '{}' {} {}", escaped, line, span.max(1))
        } else {
            format!("reveal '{}'", escaped)
        };
        cmd.args(["emit-to", tab_yazi_id, "plugin", "hscroll", &plugin_arg]);
    } else {
        cmd.args(["emit-to", tab_yazi_id, "reveal", &mapped]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut c) = cmd.spawn() {
        let _ = std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

/// Clears Yazi's file-list selection bar (`blur`) when keyboard/mouse focus moves to another column.
pub fn emit_yazi_blur(home: &str, tab_yazi_id: &str) {
    if tab_yazi_id.is_empty()
        || !yazi_config_dir(home)
            .join("plugins/hscroll.yazi/main.lua")
            .exists()
    {
        return;
    }
    if let Some(ya_bin) = find_executable("ya", home) {
        let mut cmd = Command::new(ya_bin);
        cmd.args(["emit-to", tab_yazi_id, "plugin", "hscroll", "blur"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(mut c) = cmd.spawn() {
            let _ = std::thread::spawn(move || {
                let _ = c.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yazi_hash_and_edit_line_locator_are_exact() {
        assert_eq!(
            yazi_url_hash(r"C:\proj\src\a.rs"),
            yazi_url_hash("C:/proj/src/a.rs")
        );
        let tmp = std::env::temp_dir().join(format!("ccide_focus_test_{}.rs", std::process::id()));
        fs::write(&tmp, "fn a() {}\nfn b() {\n    let x = 42;\n}\nfn c() {}\n").unwrap();
        let path_s = tmp.to_string_lossy().into_owned();

        let edit_json = r#"{"tool_name":"Edit","new_string":"fn b() {\n    let x = 42;\n}"}"#;
        assert_eq!(extract_tool_line_target(edit_json, "Edit", &path_s), (2, 3));

        let patch_json = r#"{"tool_name":"Edit","structuredPatch":[{"newStart":19,"newLines":5}]}"#;
        assert_eq!(
            extract_tool_line_target(patch_json, "Edit", &path_s),
            (19, 5)
        );

        let read_resp =
            r#"{"tool_name":"Read","tool_response":{"file":{"startLine":4,"numLines":3}}}"#;
        assert_eq!(extract_tool_line_target(read_resp, "Read", &path_s), (4, 3));

        let _ = fs::remove_file(&tmp);
    }
}
