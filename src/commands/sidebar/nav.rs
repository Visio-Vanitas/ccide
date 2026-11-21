//! Subagent & Orchestrator Cursor Navigation (`Tab` / `Shift+Tab` / Arrow Helpers).

use crate::commands::sidebar::hit_mask::FocusZone;
use crate::commands::sidebar::input::{default_cursor_for_active, sync_scroll_to_cursor};
use crate::commands::sidebar::mouse_actions::{DispatchEnv, SidebarSelectState};
use crate::orchestrator::ALL_ROLES;
use crate::session::transcript::SubagentItem;

pub fn toggle_bottom_panel(sel: &mut SidebarSelectState) {
    sel.show_context = !sel.show_context;
    sel.sub_cursor = None;
    sel.focus_zone = Some(if sel.show_context {
        FocusZone::Context
    } else {
        FocusZone::Telemetry
    });
}

pub fn set_sub_cursor(
    next: usize,
    env: &DispatchEnv<'_>,
    sel: &mut SidebarSelectState,
    cur_max_scroll: usize,
) {
    sel.focus_zone = Some(FocusZone::Subagents);
    sel.sub_cursor = Some(next);
    if next == 0 {
        sel.sub_scroll = 0;
    } else {
        sync_scroll_to_cursor(next, &mut sel.sub_scroll, env.viewport_h, cur_max_scroll);
    }
}

pub fn handle_tab(
    forward: bool,
    env: &DispatchEnv<'_>,
    subagents: &[SubagentItem],
    active_view_id: &str,
    sel: &mut SidebarSelectState,
    cur_max_scroll: usize,
) {
    if sel.focus_zone != Some(FocusZone::Subagents) {
        toggle_bottom_panel(sel);
        return;
    }
    if sel.show_orchestrator {
        let count = ALL_ROLES.len() + 1;
        sel.orch_cursor = Some(if forward {
            sel.orch_cursor.map(|c| (c + 1) % count).unwrap_or(0)
        } else {
            sel.orch_cursor
                .map(|c| if c == 0 { count - 1 } else { c - 1 })
                .unwrap_or(count - 1)
        });
        return;
    }
    let max_idx = subagents.len();
    let cur = sel
        .sub_cursor
        .unwrap_or_else(|| default_cursor_for_active(active_view_id, subagents));
    let next = if max_idx == 0 {
        0
    } else if forward {
        if cur >= max_idx {
            0
        } else {
            cur + 1
        }
    } else if cur == 0 {
        max_idx
    } else {
        cur - 1
    };
    set_sub_cursor(next, env, sel, cur_max_scroll);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sidebar::dispatch::handle_input_batch;
    use crate::commands::sidebar::hit_mask::SidebarGeomMask;
    use crate::orchestrator::OrchestratorConfig;
    use std::cell::Cell;
    use std::collections::HashMap;

    fn mk_sub(id: &str) -> SubagentItem {
        SubagentItem {
            full_id: id.to_string(),
            status: "running",
            ..SubagentItem::default()
        }
    }

    #[test]
    fn tab_and_backtab_cycle_subagents_and_orchestrator_when_focused_else_toggle_bottom() {
        let env = DispatchEnv {
            home: "",
            yazi_id: "",
            cwd: "",
            sid: "",
            cpid: 0,
            agent: crate::session::AgentKind::Claude,
            main_file: "",
            main_line: 0,
            main_span: 0,
            viewport_h: 2,
            cost_max_scroll: 6,
            switch_signal_ok: Cell::new(None),
            hit_mask: SidebarGeomMask {
                cols: 40,
                top_h: 11,
                panel3_top_row: 30,
                cost_top_row: 41,
                show_context_panel: true,
                split_ctx_cost: true,
            },
        };
        let tags = HashMap::new();
        let mut subs = vec![mk_sub("sub-1"), mk_sub("sub-2")];
        let mut active = "main".to_string();
        let mut sel = SidebarSelectState::default();
        let mut orch = OrchestratorConfig::default_with_settings("");

        // 1. Click SubagentFrame -> FocusZone::Subagents; Tab cycles 1 -> 2 (scroll > 0) -> 0 (scroll == 0)
        handle_input_batch(
            "\x1b[<0;12;15M",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.focus_zone, Some(FocusZone::Subagents));
        assert!(!sel.show_context);

        handle_input_batch(
            "\t",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.sub_cursor, Some(1));
        assert!(!sel.show_context);

        handle_input_batch(
            "\t",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.sub_cursor, Some(2));
        assert!(sel.sub_scroll > 0);
        assert!(!sel.show_context);

        handle_input_batch(
            "\t",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.sub_cursor, Some(0));
        assert_eq!(sel.sub_scroll, 0);
        assert!(!sel.show_context);

        // 2. Shift+Tab (`\x1b[Z`) wraps 0 -> 2 (scroll > 0) -> 1
        handle_input_batch(
            "\x1b[Z",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.sub_cursor, Some(2));
        assert!(sel.sub_scroll > 0);

        handle_input_batch(
            "\x1b[Z",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.sub_cursor, Some(1));

        // 3. `t` toggles bottom panel even from FocusZone::Subagents; clicking Cost + `\t` toggles show_context
        handle_input_batch(
            "t",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert!(sel.show_context);
        assert_eq!(sel.focus_zone, Some(FocusZone::Context));

        handle_input_batch(
            "\x1b[<0;12;44M\t",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert!(!sel.show_context);
        assert_eq!(sel.focus_zone, Some(FocusZone::Telemetry));

        // 4. Orchestrator mode in FocusZone::Subagents: Tab / Shift+Tab cycles orch_cursor without toggling bottom
        sel.focus_zone = Some(FocusZone::Subagents);
        sel.show_orchestrator = true;
        sel.orch_cursor = None;
        handle_input_batch(
            "\t",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.orch_cursor, Some(0));
        assert!(!sel.show_context);

        handle_input_batch(
            "\t",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.orch_cursor, Some(1));

        handle_input_batch(
            "\x1b[Z\x1b[Z",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.orch_cursor, Some(ALL_ROLES.len()));
        assert!(!sel.show_context);
    }
}
