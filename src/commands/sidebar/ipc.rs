//! Cross-Platform Signal/Named-Event Wakeup, Pending-Switch Guard & Yazi Reveal IPC.

use std::collections::HashMap;
use std::fs;
use std::time::{Duration, Instant};

pub use crate::commands::sidebar::ipc_guard::{
    claude_signalable, has_pending_switch_request, read_pushed_claude_state,
};
use crate::commands::workspace::focus_center_after_view_switch;
use crate::ffi::{wake_token, ProcToken};
use crate::json::json_escape;
use crate::platform::{atomic_write, cache_dir, ipc_dir, ipc_path};
use crate::session::transcript::{
    get_main_session_file_ops, get_session_subagents, JsonlTracker, SubagentItem,
};
use crate::session::{emit_yazi_reveal_at, load_config_models, ConfigModels, SessionBinding};

pub fn spawn_center_focus_restore(home: &str, yid: &str, sid: &str, view: &str, rank: usize) {
    let (h, y, s, v) = (
        home.to_string(),
        yid.to_string(),
        sid.to_string(),
        view.to_string(),
    );
    let _ = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(18));
        focus_center_after_view_switch(&h, &y, &s, &v, rank);
    });
}

pub fn trigger_yazi_reveal(home: &str, yid: &str, cwd: &str, fp: &str, ln: usize, sp: usize) {
    emit_yazi_reveal_at(home, yid, cwd, fp, ln, sp);
}

pub fn write_active_view_state(home: &str, yid: &str, view: &str) {
    if !yid.is_empty() {
        let val = if view.is_empty() { "main" } else { view };
        let _ = atomic_write(
            &cache_dir(home).join(format!("active_view_{yid}.txt")),
            val.as_bytes(),
        );
    }
}

pub fn register_sidebar_tab(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
    my_tok_str: &str,
) {
    if tab_yazi_id.is_empty() {
        return;
    }
    let _ = atomic_write(
        &ipc_path(&format!("sb_yazi_{}.pid", tab_yazi_id)),
        my_tok_str.as_bytes(),
    );
    let eff_shell = if shell_pid_arg > 0 {
        shell_pid_arg
    } else {
        fs::read_to_string(cache_dir(home).join(format!("tab_shell_{}.txt", tab_yazi_id)))
            .ok()
            .and_then(|s| ProcToken::parse(&s).map(|t| t.pid))
            .unwrap_or(0)
    };
    let safe_cwd = json_escape(target_cwd);
    let safe_tok = json_escape(my_tok_str);
    let reg_json = format!(
        "{{\"pid\":\"{}\",\"yazi_id\":\"{}\",\"cwd\":\"{}\",\"shell_pid\":{}}}\n",
        safe_tok, tab_yazi_id, safe_cwd, eff_shell
    );
    let _ = atomic_write(
        &ipc_path(&format!("sidebar_tab_{}.json", tab_yazi_id)),
        reg_json.as_bytes(),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn trigger_native_view_switch(
    home: &str,
    tab_yazi_id: &str,
    session_id: &str,
    claude_pid: u32,
    view_id: &str,
    raw_type: &str,
    desc: &str,
    done: bool,
    footer_rank: usize,
) -> bool {
    write_active_view_state(home, tab_yazi_id, view_id);
    let mut signaled = true;
    if !session_id.is_empty() || claude_pid > 1 {
        let _ = fs::create_dir_all(ipc_dir());
        let st = json_escape(&raw_type.replace(['\n', '\r', '\t'], " "));
        let sd = json_escape(&desc.replace(['\n', '\r', '\t'], " "));
        let payload = format!(
            "{{\"id\":\"{}\",\"type\":\"{}\",\"desc\":\"{}\",\"done\":{}}}",
            json_escape(view_id),
            if st.is_empty() {
                "general-purpose"
            } else {
                &st
            },
            if sd.is_empty() { "subagent" } else { &sd },
            done
        );
        if claude_pid > 1 {
            if let Some((dir, tok)) = claude_signalable(claude_pid) {
                let _ = atomic_write(
                    &dir.join(format!("req_pid_{claude_pid}.json")),
                    payload.as_bytes(),
                );
                let _ = wake_token(&tok);
            } else {
                let _ = atomic_write(
                    &ipc_path(&format!("req_pid_{claude_pid}.json")),
                    payload.as_bytes(),
                );
                signaled = false;
            }
        } else if !session_id.is_empty() {
            let _ = atomic_write(
                &ipc_path(&format!("req_{session_id}.json")),
                payload.as_bytes(),
            );
        }
    }
    spawn_center_focus_restore(home, tab_yazi_id, session_id, view_id, footer_rank);
    signaled
}

pub fn update_isolated_session_binding(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
    my_tok: ProcToken,
    cur: &mut SessionBinding,
) -> bool {
    let my_tok_str = my_tok.encode();
    let rm_if_mine = |p: std::path::PathBuf| {
        if fs::read_to_string(&p)
            .ok()
            .and_then(|s| ProcToken::parse(&s))
            .map(|t| t == my_tok)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&p);
        }
    };
    let b = crate::session::resolve_isolated_session_binding(
        home,
        tab_yazi_id,
        target_cwd,
        shell_pid_arg,
    );
    if b.session_id.is_empty() {
        if cur.ui_pid > 1 && !crate::ffi::is_pid_alive(cur.ui_pid) {
            rm_if_mine(ipc_path(&format!("sb_pid_{}.pid", cur.ui_pid)));
            cur.ui_pid = 0;
        }
        return false;
    }
    let changed = b != *cur;
    if b.session_id != cur.session_id {
        if !cur.session_id.is_empty() {
            rm_if_mine(ipc_path(&format!("sb_{}.pid", cur.session_id)));
        }
        let _ = atomic_write(
            &ipc_path(&format!("sb_{}.pid", b.session_id)),
            my_tok_str.as_bytes(),
        );
    }
    if b.ui_pid != cur.ui_pid {
        if cur.ui_pid > 1 {
            rm_if_mine(ipc_path(&format!("sb_pid_{}.pid", cur.ui_pid)));
        }
        if b.ui_pid > 1 {
            let _ = atomic_write(
                &ipc_path(&format!("sb_pid_{}.pid", b.ui_pid)),
                my_tok_str.as_bytes(),
            );
        }
    }
    *cur = b;
    changed
}

#[allow(clippy::too_many_arguments)]
pub fn refresh_session_and_subagents(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    cached_session_id: &str,
    cached_claude_pid: u32,
    active_view_id: &mut String,
    last_click_instant: &Instant,
    status_notice: &mut Option<String>,
    main_tracker: &mut HashMap<String, JsonlTracker>,
    sub_trackers: &mut HashMap<String, JsonlTracker>,
    seen_main_seq: &mut Option<usize>,
    seen_sub_seq: &mut HashMap<String, usize>,
) -> (
    String,
    String,
    usize,
    usize,
    ConfigModels,
    Vec<SubagentItem>,
) {
    let m_ops = get_main_session_file_ops(
        home,
        cached_session_id,
        tab_yazi_id,
        target_cwd,
        main_tracker,
    );
    let init_reveal = seen_main_seq.is_none() && !m_ops.file.is_empty();
    if (init_reveal || seen_main_seq.map(|p| m_ops.seq > p).unwrap_or(false))
        && active_view_id == "main"
    {
        trigger_yazi_reveal(
            home,
            tab_yazi_id,
            target_cwd,
            &m_ops.file,
            m_ops.line,
            m_ops.span,
        );
    }
    *seen_main_seq = Some(m_ops.seq);

    let mut pushed_model_cand = None;
    if let Some((target_cur, p_mod)) =
        read_pushed_claude_state(cached_claude_pid, cached_session_id)
    {
        *status_notice = None;
        pushed_model_cand = p_mod;
        if last_click_instant.elapsed() > Duration::from_millis(650)
            && !has_pending_switch_request(cached_claude_pid, cached_session_id)
            && target_cur != *active_view_id
        {
            *active_view_id = target_cur;
            write_active_view_state(home, tab_yazi_id, active_view_id);
            if active_view_id == "main" {
                trigger_yazi_reveal(
                    home,
                    tab_yazi_id,
                    target_cwd,
                    &m_ops.file,
                    m_ops.line,
                    m_ops.span,
                );
            }
        }
    }

    let t_cand = (!m_ops.model.is_empty()).then_some((m_ops.model_mtime, m_ops.model.as_str()));
    let p_cand = pushed_model_cand.as_ref().map(|(mt, s)| (*mt, s.as_str()));
    let cfg = load_config_models(home, cached_session_id, tab_yazi_id, t_cand, p_cand);
    let subs = get_session_subagents(home, cached_session_id, target_cwd, sub_trackers);

    for ag in &subs {
        let prev = seen_sub_seq.insert(ag.full_id.clone(), ag.file_op_seq);
        if prev.map(|p| ag.file_op_seq > p).unwrap_or(false) && *active_view_id == ag.full_id {
            trigger_yazi_reveal(home, tab_yazi_id, target_cwd, &ag.file, ag.line, ag.span);
        }
    }
    (m_ops.tool, m_ops.file, m_ops.line, m_ops.span, cfg, subs)
}
