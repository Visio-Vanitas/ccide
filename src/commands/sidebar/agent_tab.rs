//! Multi-Agent (`agy` · `codex`) Tab Watcher, Session Binding & Transcript Refresh (`src/commands/sidebar/agent_tab.rs`).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::sidebar::ipc::{
    refresh_session_and_subagents, trigger_yazi_reveal, update_isolated_session_binding,
};
use crate::ffi::ProcToken;
use crate::json::{extract_json_str, extract_json_u64, json_escape};
use crate::platform::{atomic_write, cache_dir, ipc_path};
use crate::session::transcript::{JsonlTracker, MainSessionOps, SubagentItem};
use crate::session::{
    agent_session_to_binding, agy, codex, load_config_models_for, normalize_path_under_cwd,
    read_tab_agent, resolve_agent_session, AgentKind, AgentSession, ConfigModels, SessionBinding,
};

pub struct TabAgentWatcher {
    pub kind: AgentKind,
    path: PathBuf,
    last_mtime: Option<SystemTime>,
}

impl TabAgentWatcher {
    pub fn new(home: &str, tab_yazi_id: &str) -> Self {
        let path = cache_dir(home).join(format!("tab_agent_{tab_yazi_id}.txt"));
        let last_mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self {
            kind: read_tab_agent(home, tab_yazi_id),
            path,
            last_mtime,
        }
    }

    pub fn reload_if_changed(&mut self, home: &str, tab_yazi_id: &str) -> bool {
        let mt = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if mt == self.last_mtime {
            return false;
        }
        self.last_mtime = mt;
        let next = read_tab_agent(home, tab_yazi_id);
        std::mem::replace(&mut self.kind, next) != next
    }
}

fn read_hook_last_file(home: &str, yid: &str) -> Option<(String, String, usize, usize)> {
    let txt = (!yid.is_empty())
        .then(|| {
            fs::read_to_string(cache_dir(home).join(format!("main_last_file_{yid}.json"))).ok()
        })
        .flatten()?;
    let file = extract_json_str(&txt, "file").filter(|s| !s.is_empty())?;
    Some((
        extract_json_str(&txt, "tool").unwrap_or_default(),
        file,
        extract_json_u64(&txt, "line").unwrap_or(0) as usize,
        extract_json_u64(&txt, "span").unwrap_or(0) as usize,
    ))
}

fn sync_sidebar_ipc_pids(my_tok: ProcToken, prev: &SessionBinding, next: &SessionBinding) {
    let my_tok_str = my_tok.encode();
    let rm_if_mine = |p: PathBuf| {
        if fs::read_to_string(&p)
            .ok()
            .and_then(|s| ProcToken::parse(&s))
            == Some(my_tok)
        {
            let _ = fs::remove_file(&p);
        }
    };
    if next.session_id != prev.session_id {
        if !prev.session_id.is_empty() {
            rm_if_mine(ipc_path(&format!("sb_{}.pid", prev.session_id)));
        }
        if !next.session_id.is_empty() {
            let _ = atomic_write(
                &ipc_path(&format!("sb_{}.pid", next.session_id)),
                my_tok_str.as_bytes(),
            );
        }
    }
    if next.ui_pid != prev.ui_pid {
        if prev.ui_pid > 1 {
            rm_if_mine(ipc_path(&format!("sb_pid_{}.pid", prev.ui_pid)));
        }
        if next.ui_pid > 1 {
            let _ = atomic_write(
                &ipc_path(&format!("sb_pid_{}.pid", next.ui_pid)),
                my_tok_str.as_bytes(),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn refresh_non_claude_tab(
    kind: AgentKind,
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
    my_tok: ProcToken,
    sb: &mut SessionBinding,
    cur_sess: &mut Option<AgentSession>,
    main_tracker: &mut HashMap<String, JsonlTracker>,
    sub_trackers: &mut HashMap<String, JsonlTracker>,
    seen_main_seq: &mut Option<usize>,
    status_notice: &mut Option<String>,
) -> (
    bool,
    String,
    String,
    usize,
    usize,
    ConfigModels,
    Vec<SubagentItem>,
) {
    *status_notice = None;
    let resolved = resolve_agent_session(home, tab_yazi_id, target_cwd, shell_pid_arg, kind);
    let next_sb = resolved
        .as_ref()
        .map(agent_session_to_binding)
        .unwrap_or_default();
    let rebound =
        next_sb.session_id != sb.session_id || next_sb.transcript_path != sb.transcript_path;
    sync_sidebar_ipc_pids(my_tok, sb, &next_sb);
    *sb = next_sb;
    *cur_sess = resolved;

    if rebound {
        main_tracker.clear();
        sub_trackers.clear();
        *seen_main_seq = None;
    }

    let mut ops = MainSessionOps {
        tool: String::new(),
        file: String::new(),
        line: 0,
        span: 0,
        model: String::new(),
        model_mtime: UNIX_EPOCH,
        seq: 0,
    };
    if let Some(ref sess) = *cur_sess {
        let tr = main_tracker
            .entry(sess.transcript_path.to_string_lossy().into_owned())
            .or_insert_with(|| JsonlTracker::from_tail(None));
        let parsed = match kind {
            AgentKind::Agy => agy::agy_main_file_ops(&sess.transcript_path, tr),
            AgentKind::Codex => codex::codex_main_file_ops(&sess.transcript_path, tr),
            AgentKind::Claude => None,
        };
        if let Some(mut m) = parsed {
            if !m.file.is_empty() {
                m.file = normalize_path_under_cwd(&m.file, target_cwd);
            }
            ops = m;
        }
    }
    if ops.file.is_empty() {
        if let Some((ht, hf, hl, hs)) = read_hook_last_file(home, tab_yazi_id) {
            (ops.tool, ops.file, ops.line, ops.span) =
                (ht, normalize_path_under_cwd(&hf, target_cwd), hl, hs);
        }
    }

    let init_reveal = seen_main_seq.is_none() && !ops.file.is_empty();
    let seq_advanced = seen_main_seq.map(|p| ops.seq > p).unwrap_or(false) && !ops.file.is_empty();
    if init_reveal || seq_advanced {
        trigger_yazi_reveal(home, tab_yazi_id, target_cwd, &ops.file, ops.line, ops.span);
        if !tab_yazi_id.is_empty() {
            let now_s = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let main_json = format!(
                "{{\"file\":\"{}\",\"tool\":\"{}\",\"line\":{},\"span\":{},\"ts\":{now_s}}}\n",
                json_escape(&ops.file),
                json_escape(&ops.tool),
                ops.line,
                ops.span
            );
            let _ = atomic_write(
                &cache_dir(home).join(format!("main_last_file_{tab_yazi_id}.json")),
                main_json.as_bytes(),
            );
        }
    }
    *seen_main_seq = Some(ops.seq);

    let live_mod = cur_sess
        .as_ref()
        .map(|s| s.model_id.as_str())
        .filter(|m| !m.is_empty())
        .or_else(|| (!ops.model.is_empty()).then_some(ops.model.as_str()));
    let cfg = load_config_models_for(kind, home, &sb.session_id, tab_yazi_id, live_mod);
    let subs = match (&*cur_sess, kind) {
        (Some(sess), AgentKind::Agy) => {
            let tr = sub_trackers
                .entry(sess.session_id.clone())
                .or_insert_with(|| JsonlTracker::from_tail(None));
            agy::agy_subagents(&sess.transcript_path, tr)
        }
        (Some(sess), AgentKind::Codex) => {
            codex::codex_subagents(home, &sess.session_id, &sess.transcript_path)
        }
        _ => Vec::new(),
    };
    (rebound, ops.tool, ops.file, ops.line, ops.span, cfg, subs)
}

#[allow(clippy::too_many_arguments)]
pub fn refresh_tab_dispatch(
    kind: AgentKind,
    rebind_due: bool,
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
    my_tok: ProcToken,
    sb: &mut SessionBinding,
    cur_sess: &mut Option<AgentSession>,
    active_view_id: &mut String,
    last_click: &std::time::Instant,
    status_notice: &mut Option<String>,
    main_tracker: &mut HashMap<String, JsonlTracker>,
    sub_trackers: &mut HashMap<String, JsonlTracker>,
    seen_main_seq: &mut Option<usize>,
    seen_sub_seq: &mut HashMap<String, usize>,
) -> (
    bool,
    String,
    String,
    usize,
    usize,
    ConfigModels,
    Vec<SubagentItem>,
) {
    if kind == AgentKind::Claude {
        let ch = rebind_due
            && update_isolated_session_binding(
                home,
                tab_yazi_id,
                target_cwd,
                shell_pid_arg,
                my_tok,
                sb,
            );
        let (mt, mf, ml, ms, cfg, subs) = refresh_session_and_subagents(
            home,
            tab_yazi_id,
            target_cwd,
            &sb.session_id,
            sb.ui_pid,
            active_view_id,
            last_click,
            status_notice,
            main_tracker,
            sub_trackers,
            seen_main_seq,
            seen_sub_seq,
        );
        (ch, mt, mf, ml, ms, cfg, subs)
    } else {
        refresh_non_claude_tab(
            kind,
            home,
            tab_yazi_id,
            target_cwd,
            shell_pid_arg,
            my_tok,
            sb,
            cur_sess,
            main_tracker,
            sub_trackers,
            seen_main_seq,
            status_notice,
        )
    }
}
