//! Multi-Agent (`agy` · `codex`) Tab Refresh, Swarm Snapshot & Context Sampling (`src/commands/sidebar/agent_refresh.rs`).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::sidebar::ipc::{trigger_yazi_reveal, write_active_view_state};
use crate::ffi::ProcToken;
use crate::json::{extract_json_str, extract_json_u64, json_escape};
use crate::platform::{atomic_write, cache_dir, ipc_path};
use crate::session::transcript::{JsonlTracker, MainSessionOps, SubagentItem};
use crate::session::{
    agent_session_to_binding, agy, codex, load_agy_swarm, load_config_models_for,
    normalize_path_under_cwd, resolve_agent_session, AgentKind, AgentSession, AgySwarmSnapshot,
    ConfigModels, SessionBinding,
};
use crate::tui::context::{ContextSnapshot, SessionContextAnalyzer};

pub struct TabRefresh {
    pub rebound: bool,
    pub tool: String,
    pub file: String,
    pub line: usize,
    pub span: usize,
    pub cfg: ConfigModels,
    pub subs: Vec<SubagentItem>,
    pub swarm: Option<AgySwarmSnapshot>,
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
            let p = ipc_path(&format!("sb_{}.pid", next.session_id));
            let _ = atomic_write(&p, my_tok_str.as_bytes());
        }
    }
    if next.ui_pid != prev.ui_pid {
        if prev.ui_pid > 1 {
            rm_if_mine(ipc_path(&format!("sb_pid_{}.pid", prev.ui_pid)));
        }
        if next.ui_pid > 1 {
            let p = ipc_path(&format!("sb_pid_{}.pid", next.ui_pid));
            let _ = atomic_write(&p, my_tok_str.as_bytes());
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
    active_view_id: &mut String,
    seen_sub_seq: &mut HashMap<String, usize>,
) -> TabRefresh {
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
        seen_sub_seq.clear();
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
        if *active_view_id == "main" {
            trigger_yazi_reveal(home, tab_yazi_id, target_cwd, &ops.file, ops.line, ops.span);
        }
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
            let p = cache_dir(home).join(format!("main_last_file_{tab_yazi_id}.json"));
            let _ = atomic_write(&p, main_json.as_bytes());
        }
    }
    *seen_main_seq = Some(ops.seq);

    let (subs, swarm) = match (&*cur_sess, kind) {
        (Some(sess), AgentKind::Agy) => {
            let fb_key = format!("__fallback_{}", sess.session_id);
            let mut fb_tr = sub_trackers
                .remove(&fb_key)
                .unwrap_or_else(|| JsonlTracker::from_tail(None));
            let snap = load_agy_swarm(
                home,
                &sess.session_id,
                &sess.transcript_path,
                sub_trackers,
                &mut fb_tr,
            );
            sub_trackers.insert(fb_key, fb_tr);
            let members = snap.members.clone();
            (members, Some(snap))
        }
        (Some(sess), AgentKind::Codex) => (
            codex::codex_subagents(home, &sess.session_id, &sess.transcript_path),
            None,
        ),
        _ => (Vec::new(), None),
    };

    if cur_sess.is_some()
        && *active_view_id != "main"
        && !subs.iter().any(|a| a.full_id == *active_view_id)
    {
        *active_view_id = "main".to_string();
        write_active_view_state(home, tab_yazi_id, "main");
    }

    for ag in &subs {
        let prev = seen_sub_seq.insert(ag.full_id.clone(), ag.file_op_seq);
        let advanced = prev.map(|p| ag.file_op_seq > p).unwrap_or(false);
        if *active_view_id == ag.full_id && !ag.file.is_empty() && advanced {
            let norm_fp = normalize_path_under_cwd(&ag.file, target_cwd);
            trigger_yazi_reveal(home, tab_yazi_id, target_cwd, &norm_fp, ag.line, ag.span);
        }
    }

    let coord_mod = swarm.as_ref().and_then(|s| s.coordinator_model.as_deref());
    let live_mod = cur_sess
        .as_ref()
        .map(|s| s.model_id.as_str())
        .filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case("auto"))
        .or(coord_mod)
        .or_else(|| (!ops.model.is_empty()).then_some(ops.model.as_str()));
    let cfg = load_config_models_for(kind, home, &sb.session_id, tab_yazi_id, live_mod);

    TabRefresh {
        rebound,
        tool: ops.tool,
        file: ops.file,
        line: ops.line,
        span: ops.span,
        cfg,
        subs,
        swarm,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn sample_ctx(
    an: &mut SessionContextAnalyzer,
    kind: AgentKind,
    home: &str,
    cwd: &str,
    sess: Option<&AgentSession>,
    swarm: Option<&AgySwarmSnapshot>,
    view: &str,
    sid: &str,
    data_pid: u32,
    subagents: &[SubagentItem],
    cfg: &ConfigModels,
) -> ContextSnapshot {
    let active_sub_m = (view != "main")
        .then(|| subagents.iter().find(|a| a.full_id == view))
        .flatten()
        .and_then(|a| a.model_raw.as_deref())
        .unwrap_or(&cfg.sub_model_short);
    match (kind, sess) {
        (AgentKind::Agy, Some(asess)) => {
            if let Some(sw) = swarm.filter(|s| s.active) {
                an.sample_agent_swarm(home, cwd, asess, view, &sw.worker_transcripts)
            } else {
                an.sample_agent(home, cwd, asess, "main")
            }
        }
        (AgentKind::Codex, Some(asess)) => an.sample_agent(home, cwd, asess, "main"),
        _ => an.sample(
            home,
            cwd,
            sid,
            data_pid,
            view,
            &cfg.main_model_raw,
            active_sub_m,
        ),
    }
}
