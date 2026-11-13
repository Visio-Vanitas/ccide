//! Bidirectional Signal (`SIGUSR2`), Yazi File+Line Reveal & Tab Registration IPC for `sidebar`.

use std::fs;
use std::time::SystemTime;

use crate::commands::workspace::focus_center_after_view_switch;
use crate::ffi::{kill, SIGUSR2};
use crate::session::{emit_yazi_reveal_at, is_valid_model_name};

pub fn read_pushed_claude_state(
    claude_pid: u32,
    session_id: &str,
) -> Option<(String, Option<(SystemTime, String)>)> {
    let mut cands = Vec::with_capacity(2);
    if claude_pid > 1 {
        cands.push(format!("/tmp/.cc-ide/cur_pid_{}.txt", claude_pid));
    }
    if !session_id.is_empty() {
        cands.push(format!("/tmp/.cc-ide/cur_{}.txt", session_id));
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
    let p = format!("{}/.cache/cc-ide/active_view_{}.txt", home, tab_yazi_id);
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
        format!("/tmp/.cc-ide/sb_yazi_{}.pid", tab_yazi_id),
        my_pid_str,
    );
    let eff_shell = if shell_pid_arg > 0 {
        shell_pid_arg
    } else {
        fs::read_to_string(format!(
            "{}/.cache/cc-ide/tab_shell_{}.txt",
            home, tab_yazi_id
        ))
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
        format!("/tmp/.cc-ide/sidebar_tab_{}.json", tab_yazi_id),
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
        let _ = fs::create_dir_all("/tmp/.cc-ide");
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
            let pid_req = format!("/tmp/.cc-ide/req_pid_{}.json", claude_pid);
            let pid_tmp = format!("{}.tmp", pid_req);
            if fs::write(&pid_tmp, &payload).is_ok() {
                let _ = fs::rename(&pid_tmp, &pid_req);
            }
        }
        if !session_id.is_empty() {
            let req_path = format!("/tmp/.cc-ide/req_{}.json", session_id);
            let tmp_path = format!("{}.tmp", req_path);
            if fs::write(&tmp_path, &payload).is_ok() {
                let _ = fs::rename(&tmp_path, &req_path);
            }
        }
        if claude_pid > 1 {
            unsafe {
                let _ = kill(claude_pid as i32, SIGUSR2);
            }
        }
    }

    focus_center_after_view_switch(home, tab_yazi_id, session_id, view_id, footer_rank);
}
