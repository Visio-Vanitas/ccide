//! Multi-Agent (`claude` · `agy` · `codex`) Tab Watcher & Refresh Dispatcher (`src/commands/sidebar/agent_tab.rs`).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use crate::commands::sidebar::agent_refresh::{refresh_non_claude_tab, TabRefresh};
use crate::commands::sidebar::ipc::{
    refresh_session_and_subagents, update_isolated_session_binding,
};
use crate::ffi::ProcToken;
use crate::platform::cache_dir;
use crate::session::transcript::JsonlTracker;
use crate::session::{read_tab_agent, AgentKind, AgentSession, SessionBinding};

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
    last_click: &Instant,
    status_notice: &mut Option<String>,
    main_tracker: &mut HashMap<String, JsonlTracker>,
    sub_trackers: &mut HashMap<String, JsonlTracker>,
    seen_main_seq: &mut Option<usize>,
    seen_sub_seq: &mut HashMap<String, usize>,
) -> TabRefresh {
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
        let (tool, file, line, span, cfg, subs) = refresh_session_and_subagents(
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
        if ch {
            main_tracker.clear();
            sub_trackers.clear();
            *seen_main_seq = None;
            seen_sub_seq.clear();
        }
        TabRefresh {
            rebound: ch,
            tool,
            file,
            line,
            span,
            cfg,
            subs,
            swarm: None,
        }
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
            active_view_id,
            seen_sub_seq,
        )
    }
}
