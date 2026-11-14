//! Cross-Platform Signal/Named-Event Wakeup, Pending-Switch Guard & Yazi Reveal IPC.

use std::fs;
use std::time::{Duration, SystemTime};

use crate::commands::workspace::focus_center_after_view_switch;
use crate::ffi::wake_pid;
use crate::platform::{cache_dir, ipc_dir, ipc_path};
use crate::session::transcript::{
    get_main_session_file_ops, get_session_subagents, JsonlTracker, SubagentItem,
};
use crate::session::{emit_yazi_reveal_at, is_valid_model_name, load_config_models, ConfigModels};
use std::collections::HashMap;
use std::time::Instant;

/// Returns `true` if a view-switch request (`req_pid_<pid>.json` / `req_<sid>.json`) was
/// written within the last `2.5s` and has not yet been unlinked by Claude Code.
pub fn has_pending_switch_request(claude_pid: u32, session_id: &str) -> bool {
    let cand = if claude_pid > 1 {
        ipc_path(&format!("req_pid_{}.json", claude_pid))
    } else if !session_id.is_empty() {
        ipc_path(&format!("req_{}.json", session_id))
    } else {
        return false;
    };
    if let Ok(meta) = fs::metadata(&cand) {
        if let Ok(mt) = meta.modified() {
            return SystemTime::now()
                .duration_since(mt)
                .unwrap_or(Duration::ZERO)
                < Duration::from_millis(1500);
        }
    }
    false
}

pub fn spawn_center_focus_restore(
    home: &str,
    tab_yazi_id: &str,
    session_id: &str,
    view_id: &str,
    footer_rank: usize,
) {
    let h = home.to_string();
    let y = tab_yazi_id.to_string();
    let s = session_id.to_string();
    let v = view_id.to_string();
    let _ = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(18));
        focus_center_after_view_switch(&h, &y, &s, &v, footer_rank);
    });
}

pub fn read_pushed_claude_state(
    claude_pid: u32,
    session_id: &str,
) -> Option<(String, Option<(SystemTime, String)>)> {
    let mut cands = Vec::with_capacity(2);
    if claude_pid > 1 {
        cands.push(ipc_path(&format!("cur_pid_{}.txt", claude_pid)));
    }
    if !session_id.is_empty() {
        cands.push(ipc_path(&format!("cur_{}.txt", session_id)));
    }
    for path in cands {
        if let (Ok(meta), Ok(txt)) = (fs::metadata(&path), fs::read_to_string(&path)) {
            let mt = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let mut lines = txt.lines();
            let view = lines.next().unwrap_or("main").trim();
            let norm_view = if view.is_empty() { "main" } else { view }.to_string();
            let raw_model = lines.next().unwrap_or("").trim();
            let valid_model = if is_valid_model_name(raw_model) {
                Some((mt, raw_model.to_string()))
            } else {
                None
            };
            return Some((norm_view, valid_model));
        }
    }
    None
}

pub fn trigger_yazi_reveal(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    file_path: &str,
    line: usize,
    span: usize,
) {
    emit_yazi_reveal_at(home, tab_yazi_id, target_cwd, file_path, line, span);
}

pub fn write_active_view_state(home: &str, tab_yazi_id: &str, view_id: &str) {
    if tab_yazi_id.is_empty() {
        return;
    }
    let p = cache_dir(home).join(format!("active_view_{}.txt", tab_yazi_id));
    let _ = fs::write(p, if view_id.is_empty() { "main" } else { view_id });
}

pub fn register_sidebar_tab(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
    my_pid_str: &str,
) {
    if tab_yazi_id.is_empty() {
        return;
    }
    let _ = fs::write(
        ipc_path(&format!("sb_yazi_{}.pid", tab_yazi_id)),
        my_pid_str,
    );
    let eff_shell = if shell_pid_arg > 0 {
        shell_pid_arg
    } else {
        fs::read_to_string(cache_dir(home).join(format!("tab_shell_{}.txt", tab_yazi_id)))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
    };
    let safe_cwd = target_cwd.replace('\\', "\\\\").replace('"', "\\\"");
    let reg_json = format!(
        "{{\"pid\":{},\"yazi_id\":\"{}\",\"cwd\":\"{}\",\"shell_pid\":{}}}\n",
        my_pid_str, tab_yazi_id, safe_cwd, eff_shell
    );
    let _ = fs::write(
        ipc_path(&format!("sidebar_tab_{}.json", tab_yazi_id)),
        reg_json,
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
) {
    write_active_view_state(home, tab_yazi_id, view_id);
    if !session_id.is_empty() || claude_pid > 1 {
        let _ = fs::create_dir_all(ipc_dir());
        let safe_desc = desc
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\n', '\r', '\t'], " ");
        let safe_type = raw_type
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\n', '\r', '\t'], " ");
        let payload = format!(
            "{{\"id\":\"{}\",\"type\":\"{}\",\"desc\":\"{}\",\"done\":{}}}",
            view_id,
            if safe_type.is_empty() {
                "general-purpose"
            } else {
                &safe_type
            },
            if safe_desc.is_empty() {
                "subagent"
            } else {
                &safe_desc
            },
            if done { "true" } else { "false" }
        );
        if claude_pid > 1 {
            let pid_req = ipc_path(&format!("req_pid_{}.json", claude_pid));
            let pid_tmp = ipc_path(&format!("req_pid_{}.json.tmp", claude_pid));
            if fs::write(&pid_tmp, &payload).is_ok() && fs::rename(&pid_tmp, &pid_req).is_err() {
                let _ = fs::write(&pid_req, &payload);
                let _ = fs::remove_file(&pid_tmp);
            }
            wake_pid(claude_pid);
        } else if !session_id.is_empty() {
            let req_path = ipc_path(&format!("req_{}.json", session_id));
            let _ = fs::write(&req_path, &payload);
        }
    }

    spawn_center_focus_restore(home, tab_yazi_id, session_id, view_id, footer_rank);
}

pub fn update_isolated_session_binding(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
    my_pid_str: &str,
    c_sid: &mut String,
    c_pid: &mut u32,
) -> bool {
    let (sid, cpid) =
        crate::session::resolve_isolated_session_id(home, tab_yazi_id, target_cwd, shell_pid_arg);
    if sid.is_empty() {
        return false;
    }
    let changed = sid != *c_sid;
    if changed {
        let _ = fs::write(ipc_path(&format!("sb_{sid}.pid")), my_pid_str);
    }
    if cpid > 1 && cpid != *c_pid {
        let _ = fs::write(ipc_path(&format!("sb_pid_{cpid}.pid")), my_pid_str);
    }
    *c_sid = sid;
    *c_pid = cpid;
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
