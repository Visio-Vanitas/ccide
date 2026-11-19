//! Subagent & Swarm Member Dismissal Helpers (`src/commands/sidebar/dismiss.rs`).

use crate::commands::sidebar::mouse_actions::{DispatchEnv, SidebarSelectState};
use crate::session::is_subagent_alive;
use crate::session::transcript::{append_dismissed_subagents, SubagentItem};

pub fn dismiss_single_completed(
    d_idx: usize,
    env: &DispatchEnv<'_>,
    subagents: &mut Vec<SubagentItem>,
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    switched_view: &mut bool,
) {
    if let Some(ag) = subagents.get(d_idx) {
        if !is_subagent_alive(ag.status) {
            let dismissed_id = ag.full_id.clone();
            append_dismissed_subagents(env.home, env.sid, std::slice::from_ref(&dismissed_id));
            subagents.remove(d_idx);
            sel.sub_cursor = sel.sub_cursor.map(|c| c.min(subagents.len()));
            if *active_view_id == dismissed_id {
                env.switch_to_main(active_view_id, &mut sel.last_click_instant);
                *switched_view = true;
            }
        }
    }
}

pub fn dismiss_all_completed(
    env: &DispatchEnv<'_>,
    subagents: &mut Vec<SubagentItem>,
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    switched_view: &mut bool,
) {
    let done_ids: Vec<String> = subagents
        .iter()
        .filter(|a| !is_subagent_alive(a.status))
        .map(|a| a.full_id.clone())
        .collect();
    if !done_ids.is_empty() {
        let reset_main = done_ids.iter().any(|id| id == active_view_id);
        append_dismissed_subagents(env.home, env.sid, &done_ids);
        subagents.retain(|a| is_subagent_alive(a.status));
        sel.sub_scroll = 0;
        sel.sub_cursor = sel.sub_cursor.map(|c| c.min(subagents.len()));
        if reset_main {
            env.switch_to_main(active_view_id, &mut sel.last_click_instant);
            *switched_view = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sidebar::hit_mask::SidebarGeomMask;
    use crate::session::AgentKind;
    use std::cell::Cell;

    fn mk_item(id: &str, status: &'static str) -> SubagentItem {
        SubagentItem {
            full_id: id.to_string(),
            status,
            ..SubagentItem::default()
        }
    }

    #[test]
    fn protects_waiting_and_running_while_dismissing_killed_and_resetting_active_view() {
        let tmp = std::env::temp_dir().join(format!("ccide_dismiss_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let home = tmp.to_string_lossy().into_owned();
        let env = DispatchEnv {
            home: &home,
            yazi_id: "",
            cwd: ".",
            sid: "sess_dismiss_test",
            cpid: 0,
            agent: AgentKind::Agy,
            main_file: "",
            main_line: 0,
            main_span: 0,
            viewport_h: 10,
            cost_max_scroll: 0,
            hit_mask: SidebarGeomMask {
                cols: 40,
                top_h: 10,
                panel3_top_row: 30,
                cost_top_row: 40,
                show_context_panel: false,
                split_ctx_cost: false,
            },
            switch_signal_ok: Cell::new(None),
        };
        let mut subs = vec![
            mk_item("w_run", "running"),
            mk_item("w_wait", "waiting"),
            mk_item("w_kill", "killed"),
            mk_item("w_done", "completed"),
        ];
        let mut active = "w_kill".to_string();
        let mut sel = SidebarSelectState::default();
        let mut switched = false;

        dismiss_single_completed(1, &env, &mut subs, &mut active, &mut sel, &mut switched);
        assert_eq!(subs.len(), 4);
        assert_eq!(active, "w_kill");

        dismiss_single_completed(2, &env, &mut subs, &mut active, &mut sel, &mut switched);
        assert_eq!(subs.len(), 3);
        assert_eq!(active, "main");
        assert!(switched);

        let mut switched_all = false;
        dismiss_all_completed(&env, &mut subs, &mut active, &mut sel, &mut switched_all);
        let remaining: Vec<&str> = subs.iter().map(|s| s.full_id.as_str()).collect();
        assert_eq!(remaining, vec!["w_run", "w_wait"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
