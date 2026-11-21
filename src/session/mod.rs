//! Strict Process-Isolated Session & Tab Resolution (`src/session/mod.rs`).

pub mod agent;
pub mod agent_bind;
pub mod agy;
pub mod agy_swarm;
pub mod binding;
pub mod codex;
pub mod codex_discover;
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

use crate::ffi::{get_ppid_of, is_pid_alive, ProcToken};
use crate::json::extract_json_str;
use crate::platform::{cache_dir, clean_canonicalize, find_executable, ipc_path};

pub use crate::model_id::is_valid_model_name;
#[allow(unused_imports)]
pub use agent::{agy_home, codex_home, is_safe_id, read_tab_agent, write_tab_agent, AgentKind};
#[allow(unused_imports)]
pub use agent_bind::{agent_session_to_binding, resolve_agent_session, AgentSession};
#[allow(unused_imports)]
pub use agy::{
    agy_cid_for_cwd, agy_main_file_ops, agy_settings_model, agy_subagents, agy_transcript_path,
};
#[allow(unused_imports)]
pub use agy_swarm::{
    agy_brain_root, agy_is_member_of, agy_parent_coordinator, agy_resolve_cid,
    agy_resolve_cid_detail, agy_sys_gen, load_agy_swarm, read_board_tail, swarm_member_status,
    AgySwarmSnapshot,
};
#[allow(unused_imports)]
pub use binding::{choose_binding, commit_binding, BindingChoice, SessionBinding};
#[allow(unused_imports)]
pub use codex::{
    codex_config, codex_main_file_ops, codex_subagents, find_rollout_by_id, find_rollout_for_cwd,
    CodexConfig,
};
pub use focus::{emit_yazi_blur, emit_yazi_reveal_at, extract_tool_line_target};
#[allow(unused_imports)]
pub use model::{
    classify_model_family, classify_model_family_for, compact_model_for_width, load_config_models,
    load_config_models_for, map_alias_with_settings, short_model_label, ConfigModels,
};
pub use scan::{resolve_live_yazi_tab, resolve_yazi_id_from_ppid};
#[allow(unused_imports)]
pub use subagents::is_subagent_alive;
#[allow(unused_imports)]
pub use transcript_main::{normalize_path_under_cwd, MainSessionOps};

fn ancestor_distance(mut pid: u32, target_ancestor: u32) -> Option<usize> {
    if target_ancestor == 0 || pid == 0 {
        return None;
    }
    let root = if cfg!(windows) { 4 } else { 1 };
    for hops in 0..32 {
        if pid == target_ancestor {
            return Some(hops);
        }
        if pid <= root {
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

/// Parses `panes_<yid>.txt` locally without cross-layer imports: returns `(is_tmux, center_pane)`.
fn parse_tmux_spec(raw: &str) -> (bool, Option<&str>) {
    let t = raw.trim();
    if t.starts_with("v2;") {
        let is_tmux = t.split(';').any(|kv| kv == "b=tmux");
        let c = if is_tmux {
            t.split(';')
                .find_map(|kv| kv.strip_prefix("c="))
                .filter(|c| !c.is_empty())
        } else {
            None
        };
        return (is_tmux, c);
    }
    if let Some(rest) = t.strip_prefix("tmux:") {
        return (true, rest.split(':').next().filter(|c| !c.is_empty()));
    }
    (false, None)
}

fn read_tab_pane_title(home: &str, tab_yazi_id: &str, query_tmux: bool) -> Option<String> {
    if tab_yazi_id.is_empty() {
        return None;
    }
    let spec =
        fs::read_to_string(ipc_path(&format!("panes_{tab_yazi_id}.txt"))).unwrap_or_default();
    let (is_tmux, center_opt) = parse_tmux_spec(&spec);
    if is_tmux {
        return center_opt
            .filter(|_| query_tmux)
            .and_then(|cp| query_tmux_pane_title_bounded(&find_executable("tmux", home)?, cp));
    }
    let raw = fs::read_to_string(ipc_path(&format!("tab_title_{tab_yazi_id}.txt"))).ok()?;
    clean_claude_pane_title(&raw)
}

pub(crate) fn is_claimed_by_live_sibling(cdir: &Path, session_id: &str, my_yid: &str) -> bool {
    if !is_safe_id(session_id) {
        return false;
    }
    let Ok(raw) = fs::read_to_string(cdir.join(format!("session_{session_id}.yazi"))) else {
        return false;
    };
    let other_yid = raw.trim();
    if !is_safe_id(other_yid) || other_yid == my_yid {
        return false;
    }
    let Ok(reg) = fs::read_to_string(ipc_path(&format!("sidebar_tab_{other_yid}.json"))) else {
        return false;
    };
    let Some(tok) = extract_json_str(&reg, "pid").and_then(|s| ProcToken::parse(&s)) else {
        return false;
    };
    tok.is_alive_match()
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
    let mut shell_tok = ProcToken {
        pid: shell_pid_arg,
        start_ms: 0,
    };
    if shell_pid_arg > 0 {
        shell_tok = ProcToken::current(shell_pid_arg).unwrap_or(shell_tok);
    } else if !tab_yazi_id.is_empty() {
        let p = cdir.join(format!("tab_shell_{tab_yazi_id}.txt"));
        shell_tok = fs::read_to_string(p)
            .ok()
            .and_then(|s| ProcToken::parse(&s))
            .unwrap_or(shell_tok);
    }
    let shell_pid = shell_tok.pid;
    let canon_target = clean_canonicalize(target_cwd).unwrap_or_else(|_| PathBuf::from(target_cwd));
    let alive_entries = scan::scan_alive_sessions(home);
    let shell_alive = if shell_tok.start_ms > 0 {
        shell_tok.is_alive_match()
    } else {
        shell_pid > 0 && is_pid_alive(shell_pid)
    };

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
            fs::read_to_string(cdir.join(format!("session_{sid}.yazi")))
                .ok()
                .is_some_and(|s| s.trim() == tab_yazi_id)
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

#[cfg(test)]
mod tests {
    use super::{clean_claude_pane_title, parse_tmux_spec};

    #[test]
    fn cleans_titles_and_parses_v1_v2_tmux_specs() {
        let chk = |raw, exp| assert_eq!(clean_claude_pane_title(raw).as_deref(), exp);
        chk("palette-syntax (claude)", Some("palette-syntax"));
        chk("✳ high-concurrency (claude)", Some("high-concurrency"));
        chk(
            "✳ agent connection test (claude)",
            Some("agent connection test"),
        );
        chk("◐ 并发优化测试 (claude.exe)", Some("并发优化测试"));
        chk(
            "✳ [v0.10] .github fix (claude)",
            Some("[v0.10] .github fix"),
        );
        chk("Claude Code (claude)", None);
        chk("claude", None);
        assert_eq!(
            parse_tmux_spec("v2;b=tmux;k=1;c=%12;lp=30"),
            (true, Some("%12"))
        );
        assert_eq!(parse_tmux_spec("tmux:%1:%2,%3"), (true, Some("%1")));
        assert_eq!(parse_tmux_spec("v2;b=wezterm;k=1;c=5;lp=30"), (false, None));
    }
}
