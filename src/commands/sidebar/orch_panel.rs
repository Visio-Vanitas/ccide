//! Interactive `ORCHESTRATOR` Middle-Panel Renderer (`src/commands/sidebar/orch_panel.rs`).
//!
//! Renders the preset selector and 5-role (`Scout`, `Coder`, `Review`, `Plan`, `Workflow`)
//! model/effort matrix with zero-emoji minimalist telemetry typography.

use crate::orchestrator::{short_effort_badge, OrchestratorConfig, ALL_ROLES};
use crate::session::{classify_model_family, compact_model_for_width, short_model_label};
use crate::tui::context::bar::model_family_color;
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::layout::{align_cell, Align, RowLayout};
use crate::tui::palette::Palette;
use crate::tui::{BOLD, RESET};

pub const TAG_ORCH_PRESET: u8 = 15;
pub const TAG_ORCH_ROLE: u8 = 16;

fn effort_color<'a>(effort: &str, pal: &'a Palette) -> &'a str {
    match short_effort_badge(effort) {
        "low" => &pal.mint,
        "med" => &pal.cyan,
        "high" => &pal.amber,
        "xhig" | "max" => &pal.coral,
        _ => &pal.dim,
    }
}

pub fn build_orchestrator_panel(
    cols: usize,
    mid_h: usize,
    cfg: &OrchestratorConfig,
    orch_cursor: Option<usize>,
    panel_focused: bool,
    pal: &Palette,
) -> PanelSpec {
    let mid_interior = mid_h.saturating_sub(2);
    let box_inner_w = cols.saturating_sub(2).max(12);
    let usable = box_inner_w.saturating_sub(2);
    let row_fmt = RowLayout::new(box_inner_w);
    let (white, dim, cyan, amber, lavender) =
        (&pal.text, &pal.dim, &pal.cyan, &pal.amber, &pal.lavender);

    let mut rows: Vec<BoxRow> = Vec::with_capacity(mid_interior.max(6));
    let prof = cfg.active_profile();

    // Row 0: Preset selector (`● Preset` | `< active_preset >`)
    let preset_cur = orch_cursor == Some(0);
    let preset_bg = if preset_cur {
        pal.cursor_bg.as_str()
    } else {
        ""
    };
    let preset_left = format!("{lavender}●{RESET} {BOLD}{white}Preset{RESET}");
    let p_col = if preset_cur { amber } else { cyan };
    let preset_right = format!("{dim}< {p_col}{}{dim} >{RESET}", cfg.active_preset);
    rows.push(BoxRow {
        text: row_fmt.split_with_bg(&preset_left, &preset_right, 1, preset_bg),
        bg: preset_bg.to_string(),
        tag: Some((TAG_ORCH_PRESET, 0)),
    });

    // Rows 1..=5: 5 Role slots
    for (idx, &role) in ALL_ROLES.iter().enumerate() {
        if rows.len() >= mid_interior {
            break;
        }
        let slot = prof.slot(role);
        let primary = slot.primary_model();
        let fam = classify_model_family(primary);
        let fam_col = model_family_color(fam, 0, pal);
        let role_lbl = align_cell(role.label(), 8, Align::Left);

        // Left fixed: `● ` (2) + `role_lbl` (8) + ` ` (1) = 11; right `[eff]` = 7; min_gap = 1 -> 19
        let budget = usable.saturating_sub(19).max(4);
        let short_m = short_model_label(primary);
        let model_lbl = if slot.models.len() > 1 {
            let extra = format!("+{}", slot.models.len() - 1);
            let m_fit =
                compact_model_for_width(&short_m, budget.saturating_sub(extra.len()).max(3));
            format!("{m_fit}{dim}{extra}{RESET}")
        } else {
            compact_model_for_width(&short_m, budget)
        };

        let is_cur = orch_cursor == Some(idx + 1);
        let row_bg = if is_cur { pal.cursor_bg.as_str() } else { "" };
        let r_col = if is_cur {
            amber.as_str()
        } else {
            white.as_str()
        };
        let left = format!("{fam_col}●{RESET} {r_col}{role_lbl}{RESET} {model_lbl}");

        let eff_badge = format!("[{}]", short_effort_badge(&slot.effort));
        let eff_col = effort_color(&slot.effort, pal);
        let right = format!(
            "{eff_col}{}{RESET}",
            align_cell(&eff_badge, 7, Align::Right)
        );

        rows.push(BoxRow {
            text: row_fmt.split_with_bg(&left, &right, 1, row_bg),
            bg: row_bg.to_string(),
            tag: Some((TAG_ORCH_ROLE, idx)),
        });
    }

    if mid_interior >= 7 && rows.len() < mid_interior {
        while rows.len() + 1 < mid_interior {
            rows.push(BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            });
        }
        let hint = format!("{dim}[click] model · [effort] · [o] sub{RESET}");
        rows.push(BoxRow {
            text: row_fmt.split(&hint, "", 0),
            bg: String::new(),
            tag: None,
        });
    }

    PanelSpec {
        title: "ORCHESTRATOR".to_string(),
        badge: format!("{} · [SUB]", cfg.active_preset),
        accent: lavender.clone(),
        rows,
        fixed_height: Some(mid_h),
        weight: 0.88,
        raw_interior: false,
        focused: panel_focused,
    }
}

pub fn step_orch_key(
    orch_cursor: &mut Option<usize>,
    orch_cfg: &mut OrchestratorConfig,
    home: &str,
    effort_only: bool,
    fwd: bool,
) {
    let r = orch_cursor.unwrap_or(0);
    *orch_cursor = Some(r);
    if r == 0 {
        orch_cfg.cycle_preset(fwd);
    } else if let Some(&role) = ALL_ROLES.get(r - 1) {
        if effort_only {
            orch_cfg.cycle_role_effort(role, fwd);
        } else {
            orch_cfg.cycle_role_model(role, fwd);
        }
    }
    orch_cfg.save(home);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sidebar::hit_mask::{
        HitZone, MaskedAction, PointerCapture, SidebarGeomMask,
    };
    use crate::tui::palette::Palette;
    use crate::tui::width::display_width;
    use std::collections::HashMap;

    #[test]
    fn renders_within_width_without_emojis_and_dispatches_clicks() {
        let pal = Palette::default_palette();
        let cfg = OrchestratorConfig::default_with_settings("");
        for cols in [30usize, 38, 46] {
            let spec = build_orchestrator_panel(cols, 12, &cfg, Some(2), true, &pal);
            assert_eq!(spec.title, "ORCHESTRATOR");
            assert!(spec.badge.ends_with("· [SUB]"));
            for r in &spec.rows {
                assert!(
                    display_width(&r.text) <= cols - 2,
                    "overflow at cols={cols}"
                );
                assert!(!r.text.chars().any(|c| c as u32 >= 0x1F300));
            }
        }

        let mask = SidebarGeomMask {
            cols: 40,
            top_h: 10,
            panel3_top_row: 22,
            cost_top_row: 32,
            show_context_panel: false,
            split_ctx_cost: false,
        };
        let mut tags = HashMap::new();
        tags.insert(12, (TAG_ORCH_PRESET, 0));
        tags.insert(14, (TAG_ORCH_ROLE, 1));
        let mut cap = PointerCapture::default();

        assert_eq!(
            cap.on_mouse_down(0, 35, 11, mask.classify(35, 11, &tags, &[])),
            Some(MaskedAction::ToggleOrchestrator)
        );
        assert_eq!(
            cap.on_mouse_down(0, 10, 12, mask.classify(10, 12, &tags, &[])),
            Some(MaskedAction::OrchCyclePreset(true))
        );
        assert_eq!(
            cap.on_mouse_down(0, 10, 14, mask.classify(10, 14, &tags, &[])),
            Some(MaskedAction::OrchCycleModel(1, true))
        );
        let z_eff = mask.classify(35, 14, &tags, &[]);
        assert_eq!(z_eff, HitZone::OrchRoleEffort(1));
        assert_eq!(
            cap.on_mouse_down(0, 35, 14, z_eff),
            Some(MaskedAction::OrchCycleEffort(1, true))
        );
    }

    #[test]
    fn focus_out_clears_sidebar_highlight_and_cost_scrolls_smoothly() {
        use crate::commands::sidebar::dispatch::{
            handle_input_batch, DispatchEnv, SidebarSelectState,
        };
        use crate::commands::sidebar::hit_mask::FocusZone;

        let env = DispatchEnv {
            home: "",
            yazi_id: "",
            cwd: "",
            sid: "",
            cpid: 0,
            main_file: "",
            main_line: 0,
            main_span: 0,
            viewport_h: 8,
            cost_max_scroll: 6,
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
        let (mut subs, mut active) = (Vec::new(), "main".to_string());
        let mut sel = SidebarSelectState {
            show_context: true,
            ..SidebarSelectState::default()
        };
        let mut orch = OrchestratorConfig::default_with_settings("");
        handle_input_batch(
            "\x1b[<0;12;44M\x1b[B\x1b[<65;12;44M",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.focus_zone, Some(FocusZone::Cost));
        assert_eq!(sel.cost_scroll, 2);
        handle_input_batch(
            "\x1b[O",
            &env,
            &tags,
            &mut subs,
            &mut active,
            &mut sel,
            &mut orch,
        );
        assert_eq!(sel.focus_zone, None);
    }
}
