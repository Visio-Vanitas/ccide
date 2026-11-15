//! Strict Process-Isolated Session & Tab Resolution (`src/session/mod.rs`).

pub mod binding;
pub mod focus;
pub mod model;
pub mod model_compact;
pub mod scan;
pub mod subagent_state;
pub mod subagents;
pub mod transcript;
pub mod transcript_main;

use std::fs;
use std::path::{Path, PathBuf};

use crate::ffi::{get_ppid_of, is_pid_alive};
use crate::json::extract_json_u64;
use crate::platform::{
    cache_dir, clean_canonicalize, find_executable, ipc_path, normalize_slashes,
};

pub use crate::model_id::is_valid_model_name;
#[allow(unused_imports)]
pub use binding::{choose_binding, commit_binding, BindingChoice, SessionBinding};
pub use focus::{emit_yazi_blur, emit_yazi_reveal_at, extract_tool_line_target};
pub use model::{
    classify_model_family, compact_model_for_width, load_config_models, map_alias_with_settings,
    short_model_label, ConfigModels,
};
pub use scan::{resolve_live_yazi_tab, resolve_yazi_id_from_ppid};

fn ancestor_distance(mut pid: u32, target_ancestor: u32) -> Option<usize> {
    if target_ancestor == 0 || pid == 0 {
        return None;
    }
    for hops in 0..=32 {
        if pid == target_ancestor {
            return Some(hops);
        }
        #[cfg(unix)]
        if pid <= 1 {
            return None;
        }
        #[cfg(windows)]
        if pid == 4 {
            return None;
        }
        if hops == 32 {
            return None;
        }
        match get_ppid_of(pid) {
            Some(p) if p != pid && p > 0 => pid = p,
            _ => return None,
        }
    }
    None
}

/// Strips terminal job suffixes (` (claude)`, ` (node)`) and leading spinner/status glyphs
/// (`✳ `, `◐ `, `● `) from a Center pane title while preserving ASCII punctuation (`[v0.9]`).
pub fn clean_claude_pane_title(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.ends_with(')') {
        if let Some(idx) = s.rfind(" (") {
            let job = s[idx + 2..s.len() - 1].trim().to_ascii_lowercase();
            if job.starts_with("claude") || job.starts_with("node") {
                s = s[..idx].trim_end();
            }
        }
    }
    let cleaned = s
        .trim_start_matches(|c: char| c.is_whitespace() || (!c.is_ascii() && !c.is_alphanumeric()))
        .trim();
    if cleaned.is_empty()
        || cleaned.eq_ignore_ascii_case("claude")
        || cleaned.eq_ignore_ascii_case("claude code")
    {
        return None;
    }
    Some(cleaned.to_string())
}

fn query_tmux_pane_title_bounded(tmux_bin: &Path, center_pane: &str) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new(tmux_bin)
        .args([
            "-L",
            "ccide",
            "display-message",
            "-p",
            "-t",
            center_pane,
            "#{pane_title}",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_millis(150);
    loop {
        if let Ok(Some(st)) = child.try_wait() {
            if !st.success() {
                return None;
            }
            let mut out = String::new();
            if let Some(mut so) = child.stdout.take() {
                let _ = so.read_to_string(&mut out);
            }
            return clean_claude_pane_title(&out);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_tab_pane_title(home: &str, tab_yazi_id: &str, query_tmux: bool) -> Option<String> {
    if tab_yazi_id.is_empty() {
        return None;
    }
    let spec =
        fs::read_to_string(ipc_path(&format!("panes_{tab_yazi_id}.txt"))).unwrap_or_default();
    if query_tmux {
        if let Some(rest) = spec.trim().strip_prefix("tmux:") {
            let center_pane = rest.split(':').next().unwrap_or("");
            if !center_pane.is_empty() {
                if let Some(tmux) = find_executable("tmux", home) {
                    return query_tmux_pane_title_bounded(&tmux, center_pane);
                }
            }
        }
    }
    if !spec.trim().starts_with("tmux:") {
        if let Ok(raw) = fs::read_to_string(ipc_path(&format!("tab_title_{tab_yazi_id}.txt"))) {
            if let Some(t) = clean_claude_pane_title(&raw) {
                return Some(t);
            }
        }
    }
    None
}

fn is_claimed_by_live_sibling(cdir: &Path, session_id: &str, my_yid: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    let Ok(raw) = fs::read_to_string(cdir.join(format!("session_{session_id}.yazi"))) else {
        return false;
    };
    let other_yid = raw.trim();
    if other_yid.is_empty() || other_yid == my_yid {
        return false;
    }
    let Ok(reg) = fs::read_to_string(ipc_path(&format!("sidebar_tab_{other_yid}.json"))) else {
        return false;
    };
    let pid = extract_json_u64(&reg, "pid").unwrap_or(0) as u32;
    is_pid_alive(pid)
}

/// Strictly resolves `SessionBinding { session_id, ui_pid, data_pid }` for this tab's Claude
/// process, preserving the interactive viewer `ui_pid` (`e.pid`) for `SIGUSR2` view-switching
/// while tracking the attached `bg` session's `session_id` and `data_pid` under FleetView.
pub fn resolve_isolated_session_binding(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
) -> SessionBinding {
    let cdir = cache_dir(home);
    let shell_pid = if shell_pid_arg > 0 {
        shell_pid_arg
    } else if !tab_yazi_id.is_empty() {
        fs::read_to_string(cdir.join(format!("tab_shell_{}.txt", tab_yazi_id)))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
    } else {
        0
    };
    let canon_target = clean_canonicalize(target_cwd).unwrap_or_else(|_| PathBuf::from(target_cwd));
    let alive_entries = scan::scan_alive_sessions(home);
    let shell_alive = shell_pid > 0 && is_pid_alive(shell_pid);

    let choice = choose_binding(
        &alive_entries,
        tab_yazi_id,
        &canon_target,
        shell_pid,
        shell_alive,
        |pid| ancestor_distance(pid, shell_pid),
        |qt| read_tab_pane_title(home, tab_yazi_id, qt),
        |sid| is_claimed_by_live_sibling(&cdir, sid, tab_yazi_id),
        |sid| {
            fs::read_to_string(cdir.join(format!("session_{}.yazi", sid)))
                .map(|s| s.trim() == tab_yazi_id)
                .unwrap_or(false)
        },
    );
    if choice.should_commit {
        commit_binding(&cdir, &choice.binding.session_id, tab_yazi_id);
    }
    choice.binding
}

pub fn resolve_isolated_session_id(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
) -> (String, u32) {
    let b = resolve_isolated_session_binding(home, tab_yazi_id, target_cwd, shell_pid_arg);
    (b.session_id, b.ui_pid)
}

/// Normalizes `file_path` so if its canonical realpath lies under `canonicalize(target_cwd)`,
/// it uses `target_cwd`'s spelling (ensuring Yazi's `reveal` matches the directory spelling).
pub fn normalize_path_under_cwd(file_path: &str, target_cwd: &str) -> String {
    if file_path.is_empty() || target_cwd.is_empty() {
        return file_path.to_string();
    }
    if let (Ok(canon_file), Ok(canon_cwd)) = (
        clean_canonicalize(file_path),
        clean_canonicalize(target_cwd),
    ) {
        if let Ok(rel) = canon_file.strip_prefix(&canon_cwd) {
            let sep = if cfg!(windows) && target_cwd.contains('\\') {
                '\\'
            } else {
                '/'
            };
            let base = target_cwd.trim_end_matches(['/', '\\']);
            let rel_norm = normalize_slashes(&rel.to_string_lossy());
            if rel_norm.is_empty() {
                return base.to_string();
            }
            let rel_native = if sep == '\\' {
                rel_norm.replace('/', "\\")
            } else {
                rel_norm
            };
            return format!("{}{}{}", base, sep, rel_native);
        }
    }
    file_path.to_string()
}

#[cfg(test)]
mod tests {
    use super::clean_claude_pane_title;

    #[test]
    fn cleans_iterm_and_spinner_pane_titles_accurately() {
        assert_eq!(
            clean_claude_pane_title("palette-syntax-ghostty-support (claude)"),
            Some("palette-syntax-ghostty-support".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("✳ high-concurrency-optimization (claude)"),
            Some("high-concurrency-optimization".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("✳ agent connection test (claude)"),
            Some("agent connection test".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("◐ 并发优化测试 (claude.exe)"),
            Some("并发优化测试".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("✳ [v0.10] .github fix (claude)"),
            Some("[v0.10] .github fix".to_string())
        );
        assert_eq!(clean_claude_pane_title("Claude Code (claude)"), None);
        assert_eq!(clean_claude_pane_title("claude"), None);
    }
}
