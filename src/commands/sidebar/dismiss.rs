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
