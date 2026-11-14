//! Input-Batch Event Dispatcher for `ccide-core sidebar` (`src/commands/sidebar/dispatch.rs`).

use std::collections::HashMap;

use crate::commands::sidebar::hit_mask::FocusZone;
use crate::commands::sidebar::input::{
    default_cursor_for_active, parse_input_events, sync_scroll_to_cursor, InputEvent,
};
use crate::commands::sidebar::ipc::spawn_center_focus_restore;
use crate::commands::sidebar::mouse_actions::{
    apply_masked_action, dismiss_all_completed, dismiss_single_completed,
};
pub use crate::commands::sidebar::mouse_actions::{DispatchEnv, SidebarSelectState};
use crate::orchestrator::{OrchestratorConfig, ALL_ROLES};
use crate::session::transcript::SubagentItem;

pub fn handle_input_batch(
    raw_in: &str,
    env: &DispatchEnv<'_>,
    row_tag_map: &HashMap<usize, (u8, usize)>,
    subagents: &mut Vec<SubagentItem>,
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    orch_cfg: &mut OrchestratorConfig,
) {
    let (mut clear_all_done, mut restore_focus, mut switched_view) = (false, false, false);
    let m = &env.hit_mask;

    for ev in parse_input_events(raw_in) {
        let max_idx = subagents.len();
        let cur_max_scroll = (subagents.len() * 2).saturating_sub(env.viewport_h);
        let mut dismiss_idx: Option<usize> = None;

        match ev {
            InputEvent::FocusIn => sel.pane_has_focus = true,
            // Clicking Center (`Claude Code`) or Left (`Yazi`/`Lazygit`) clears Right Sidebar highlights!
            InputEvent::FocusOut => {
                sel.pane_has_focus = false;
                sel.focus_zone = None;
                sel.sub_cursor = None;
                sel.orch_cursor = None;
            }
            InputEvent::ToggleBottomPanel => {
                sel.show_context = !sel.show_context;
                sel.sub_cursor = None;
                sel.focus_zone = Some(if sel.show_context {
                    FocusZone::Context
                } else {
                    FocusZone::Telemetry
                });
            }
            InputEvent::ToggleOrchestrator => {
                sel.show_orchestrator = !sel.show_orchestrator;
                sel.sub_cursor = None;
                sel.orch_cursor = None;
                sel.focus_zone = Some(FocusZone::Subagents);
            }
            InputEvent::Escape => {
                if sel.show_orchestrator {
                    sel.show_orchestrator = false;
                    sel.orch_cursor = None;
                } else if sel.sub_cursor.is_some() || sel.focus_zone.is_some() {
                    sel.sub_cursor = None;
                    sel.focus_zone = None;
                } else if active_view_id != "main" {
                    env.switch_to_main(active_view_id, &mut sel.last_click_instant);
                    switched_view = true;
                } else if sel.show_context {
                    sel.show_context = false;
                }
            }
            InputEvent::CursorUp => match sel.focus_zone {
                Some(FocusZone::Cost) if sel.show_context => {
                    sel.cost_scroll = sel.cost_scroll.saturating_sub(1);
                }
                _ if sel.show_orchestrator => {
                    sel.focus_zone = Some(FocusZone::Subagents);
                    sel.orch_cursor = Some(sel.orch_cursor.unwrap_or(0).saturating_sub(1));
                }
                _ => {
                    sel.focus_zone = Some(FocusZone::Subagents);
                    let next = sel
                        .sub_cursor
                        .unwrap_or_else(|| default_cursor_for_active(active_view_id, subagents))
                        .saturating_sub(1);
                    sel.sub_cursor = Some(next);
                    sync_scroll_to_cursor(
                        next,
                        &mut sel.sub_scroll,
                        env.viewport_h,
                        cur_max_scroll,
                    );
                }
            },
            InputEvent::CursorDown => match sel.focus_zone {
                Some(FocusZone::Cost) if sel.show_context => {
                    sel.cost_scroll = (sel.cost_scroll + 1).min(env.cost_max_scroll);
                }
                _ if sel.show_orchestrator => {
                    sel.focus_zone = Some(FocusZone::Subagents);
                    sel.orch_cursor = Some(
                        sel.orch_cursor
                            .map(|c| (c + 1).min(ALL_ROLES.len()))
                            .unwrap_or(0),
                    );
                }
                _ => {
                    sel.focus_zone = Some(FocusZone::Subagents);
                    let next = (sel
                        .sub_cursor
                        .unwrap_or_else(|| default_cursor_for_active(active_view_id, subagents))
                        + 1)
                    .min(max_idx);
                    sel.sub_cursor = Some(next);
                    sync_scroll_to_cursor(
                        next,
                        &mut sel.sub_scroll,
                        env.viewport_h,
                        cur_max_scroll,
                    );
                }
            },
            InputEvent::PageUp => match sel.focus_zone {
                Some(FocusZone::Cost) if sel.show_context => {
                    sel.cost_scroll = sel.cost_scroll.saturating_sub(3);
                }
                _ => {
                    sel.focus_zone = Some(FocusZone::Subagents);
                    let step = (env.viewport_h / 2).max(1);
                    let next = sel
                        .sub_cursor
                        .unwrap_or_else(|| default_cursor_for_active(active_view_id, subagents))
                        .saturating_sub(step);
                    sel.sub_cursor = Some(next);
                    if next == 0 {
                        sel.sub_scroll = 0;
                    } else {
                        sync_scroll_to_cursor(
                            next,
                            &mut sel.sub_scroll,
                            env.viewport_h,
                            cur_max_scroll,
                        );
                    }
                }
            },
            InputEvent::PageDown => match sel.focus_zone {
                Some(FocusZone::Cost) if sel.show_context => {
                    sel.cost_scroll = (sel.cost_scroll + 3).min(env.cost_max_scroll);
                }
                _ => {
                    sel.focus_zone = Some(FocusZone::Subagents);
                    let step = (env.viewport_h / 2).max(1);
                    let next = (sel
                        .sub_cursor
                        .unwrap_or_else(|| default_cursor_for_active(active_view_id, subagents))
                        + step)
                        .min(max_idx);
                    sel.sub_cursor = Some(next);
                    sync_scroll_to_cursor(
                        next,
                        &mut sel.sub_scroll,
                        env.viewport_h,
                        cur_max_scroll,
                    );
                }
            },
            InputEvent::Home => match sel.focus_zone {
                Some(FocusZone::Cost) if sel.show_context => sel.cost_scroll = 0,
                _ => {
                    (sel.focus_zone, sel.sub_cursor, sel.sub_scroll) =
                        (Some(FocusZone::Subagents), Some(0), 0);
                }
            },
            InputEvent::End => match sel.focus_zone {
                Some(FocusZone::Cost) if sel.show_context => {
                    sel.cost_scroll = env.cost_max_scroll;
                }
                _ => {
                    (sel.focus_zone, sel.sub_cursor, sel.sub_scroll) =
                        (Some(FocusZone::Subagents), Some(max_idx), cur_max_scroll);
                }
            },
            InputEvent::CursorLeft | InputEvent::CursorRight => {
                if sel.show_orchestrator && sel.focus_zone == Some(FocusZone::Subagents) {
                    crate::commands::sidebar::orch_panel::step_orch_key(
                        &mut sel.orch_cursor,
                        orch_cfg,
                        env.home,
                        false,
                        matches!(ev, InputEvent::CursorRight),
                    );
                }
            }
            InputEvent::ActivateSelected => {
                if sel.show_orchestrator && sel.focus_zone == Some(FocusZone::Subagents) {
                    crate::commands::sidebar::orch_panel::step_orch_key(
                        &mut sel.orch_cursor,
                        orch_cfg,
                        env.home,
                        true,
                        true,
                    );
                } else {
                    let target = sel
                        .sub_cursor
                        .unwrap_or_else(|| default_cursor_for_active(active_view_id, subagents));
                    (sel.sub_cursor, sel.focus_zone) = (Some(target), Some(FocusZone::Subagents));
                    if target == 0 {
                        env.switch_to_main(active_view_id, &mut sel.last_click_instant);
                        switched_view = true;
                    } else if let Some(ag) = subagents.get(target - 1) {
                        env.switch_to_agent(ag, active_view_id, &mut sel.last_click_instant);
                        switched_view = true;
                    }
                }
            }
            InputEvent::DismissSelected => {
                let target = sel
                    .sub_cursor
                    .unwrap_or_else(|| default_cursor_for_active(active_view_id, subagents));
                if target >= 1 {
                    dismiss_idx = Some(target - 1);
                }
            }
            InputEvent::ClearAllDone => clear_all_done = true,
            InputEvent::WheelUp { row } => {
                if row > m.top_h && row < m.panel3_top_row {
                    if sel.pane_has_focus {
                        sel.focus_zone = Some(FocusZone::Subagents);
                    }
                    sel.sub_scroll = sel.sub_scroll.saturating_sub(2);
                } else if row >= m.panel3_top_row {
                    if sel.pane_has_focus {
                        sel.focus_zone = Some(m.bottom_zone_at(row));
                    }
                    if m.split_ctx_cost && row >= m.cost_top_row {
                        sel.cost_scroll = sel.cost_scroll.saturating_sub(1);
                    }
                }
            }
            InputEvent::WheelDown { row } => {
                if row > m.top_h && row < m.panel3_top_row {
                    if sel.pane_has_focus {
                        sel.focus_zone = Some(FocusZone::Subagents);
                    }
                    sel.sub_scroll = (sel.sub_scroll + 2).min(cur_max_scroll);
                } else if row >= m.panel3_top_row {
                    if sel.pane_has_focus {
                        sel.focus_zone = Some(m.bottom_zone_at(row));
                    }
                    if m.split_ctx_cost && row >= m.cost_top_row {
                        sel.cost_scroll = (sel.cost_scroll + 1).min(env.cost_max_scroll);
                    }
                }
            }
            InputEvent::MouseDown { btn, col, row } | InputEvent::MouseUp { btn, col, row } => {
                let z = m.classify(col, row, row_tag_map, subagents);
                let act = if matches!(ev, InputEvent::MouseDown { .. }) {
                    sel.pointer.on_mouse_down(btn, col, row, z)
                } else {
                    sel.pointer.on_mouse_up(btn, col, row, z)
                };
                if let Some(a) = act {
                    apply_masked_action(
                        a,
                        env,
                        subagents,
                        active_view_id,
                        sel,
                        orch_cfg,
                        &mut dismiss_idx,
                        &mut clear_all_done,
                        &mut restore_focus,
                        &mut switched_view,
                    );
                }
            }
        }

        if let Some(d_idx) = dismiss_idx {
            dismiss_single_completed(
                d_idx,
                env,
                subagents,
                active_view_id,
                sel,
                &mut switched_view,
            );
        }
    }

    if clear_all_done {
        dismiss_all_completed(env, subagents, active_view_id, sel, &mut switched_view);
    }
    if restore_focus && !switched_view {
        spawn_center_focus_restore(env.home, env.yazi_id, env.sid, active_view_id, 0);
    }
}
