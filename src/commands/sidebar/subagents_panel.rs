//! Middle Panel (`SUBAGENTS`) Renderer — Pinned `[<badge>]` + Scrollable Subagent List.
//!
//! Renders full-bleed 2-row selection rectangles (`active_view_bg` / `cursor_bg` / `active_cursor_bg`)
//! with a 2-row left vertical rail (`▎` active / `┃` cursor) and inline status/action badges.

use crate::commands::sidebar::hit_mask::FocusZone;
use crate::commands::sidebar::mouse_actions::SidebarSelectState;
use crate::commands::sidebar::orch_panel::build_orchestrator_panel;
use crate::commands::sidebar::subagents_rows::{build_agent_item_rows, push_main_rows};
use crate::orchestrator::OrchestratorConfig;
use crate::session::transcript::SubagentItem;
use crate::session::{classify_model_family_for, AgentKind, ConfigModels};
use crate::tui::grid::{BoxRow, PanelSpec, TermGeom};
use crate::tui::layout::RowLayout;
use crate::tui::logo::{build_top_logo_panel, SpriteStore};
use crate::tui::palette::Palette;
use crate::tui::width::display_width;
use crate::tui::RESET;

pub struct SubagentPanelOutput {
    pub panel: PanelSpec,
    pub running_cnt: usize,
    pub viewport_h: usize,
}

#[allow(clippy::too_many_arguments)]
pub fn build_subagents_panel(
    cols: usize,
    mid_h: usize,
    agent_kind: AgentKind,
    target_cwd: &str,
    active_view_id: &str,
    cursor_idx: Option<usize>,
    panel_focused: bool,
    sub_scroll_offset: &mut usize,
    main_tool: &str,
    main_file: &str,
    subagents: &[SubagentItem],
    cached_cfg: &ConfigModels,
    active_preset: &str,
    status_notice: Option<&str>,
    tick: usize,
    pal: &Palette,
) -> SubagentPanelOutput {
    let mid_interior = mid_h.saturating_sub(2);
    let box_inner_w = cols.saturating_sub(2).max(12);
    let row_fmt = RowLayout::new(box_inner_w).padding(0, 1);
    let (dim, cyan, amber) = (&pal.dim, &pal.cyan, &pal.amber);

    let is_main_active = active_view_id == "main";
    let running_cnt = subagents.iter().filter(|a| a.status == "running").count();
    let done_cnt = subagents.len().saturating_sub(running_cnt);
    let mut sub_rows = Vec::with_capacity(mid_interior.max(4));

    push_main_rows(
        &mut sub_rows,
        &row_fmt,
        agent_kind.badge(),
        target_cwd,
        is_main_active,
        cursor_idx == Some(0),
        main_tool,
        main_file,
        cached_cfg,
        pal,
    );

    let show_notice = agent_kind.supports_view_switch()
        && status_notice.filter(|s| !s.is_empty()).is_some()
        && mid_interior >= 4;
    if let Some(notice) = status_notice.filter(|_| show_notice) {
        sub_rows.push(BoxRow {
            text: row_fmt.split(&format!(" {amber}! {notice}{RESET}"), "", 0),
            bg: String::new(),
            tag: None,
        });
    }

    let all_agent_rows = build_agent_item_rows(
        &row_fmt,
        active_view_id,
        cursor_idx,
        subagents,
        cached_cfg,
        tick,
        pal,
    );

    let show_clear_bar = done_cnt > 0 && mid_interior >= 5 + usize::from(show_notice);
    let reserved = 2 + usize::from(show_notice) + usize::from(show_clear_bar);
    let viewport_h = mid_interior.saturating_sub(reserved);
    let max_scroll = all_agent_rows.len().saturating_sub(viewport_h);
    *sub_scroll_offset = (*sub_scroll_offset).min(max_scroll);
    sub_rows.extend(
        all_agent_rows
            .into_iter()
            .skip(*sub_scroll_offset)
            .take(viewport_h),
    );

    if show_clear_bar {
        while sub_rows.len() + 1 < mid_interior {
            sub_rows.push(BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            });
        }
        let full_clear =
            format!("{dim}↑↓ ⏎ · [{amber}x{dim}] del · [{amber}c{dim}] clear {done_cnt}{RESET}");
        let short_clear = format!("{dim}[{amber}c{dim}] × clear {done_cnt} done{RESET}");
        let chosen = if display_width(&full_clear) + 2 <= box_inner_w {
            full_clear
        } else {
            short_clear
        };
        sub_rows.push(BoxRow {
            text: RowLayout::new(box_inner_w).split(&chosen, "", 0),
            bg: String::new(),
            tag: Some((3, 0)),
        });
    }

    let base_badge = if max_scroll > 0 {
        let first_item = (*sub_scroll_offset / 2) + 1;
        let last_item = (first_item + (viewport_h / 2).max(1) - 1).min(subagents.len());
        if running_cnt > 0 {
            format!(
                "{running_cnt}run · ↕{first_item}-{last_item}/{}",
                subagents.len()
            )
        } else {
            format!("↕ {first_item}-{last_item}/{}", subagents.len())
        }
    } else if running_cnt == 0 {
        format!("{}+{}", agent_kind.badge(), subagents.len())
    } else {
        format!("{running_cnt} run/{}", subagents.len())
    };
    let sub_badge = if cols >= 32 {
        format!("[{active_preset}] · {base_badge}")
    } else {
        format!("[{active_preset}]")
    };

    SubagentPanelOutput {
        panel: PanelSpec {
            title: "SUBAGENTS".to_string(),
            badge: sub_badge,
            accent: cyan.clone(),
            rows: sub_rows,
            fixed_height: Some(mid_h),
            weight: 0.88,
            raw_interior: false,
            focused: panel_focused,
        },
        running_cnt,
        viewport_h,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_top_and_mid_panels(
    geom: TermGeom,
    logos: &SpriteStore,
    agent_kind: AgentKind,
    target_cwd: &str,
    active_view_id: &str,
    sel: &mut SidebarSelectState,
    main_tool: &str,
    main_file: &str,
    subagents: &[SubagentItem],
    cached_cfg: &ConfigModels,
    orch_cfg: &OrchestratorConfig,
    tick: usize,
    pal: &Palette,
) -> (PanelSpec, usize, PanelSpec, usize, usize, usize, usize) {
    let is_main_active = active_view_id == "main";
    let active_sub = (!is_main_active)
        .then(|| subagents.iter().find(|a| a.full_id == active_view_id))
        .flatten();
    let (top_family, top_badge) = match (is_main_active, active_sub) {
        (true, _) => (
            classify_model_family_for(agent_kind, &cached_cfg.main_model_raw),
            cached_cfg.main_model_short.clone(),
        ),
        (false, Some(ag)) if ag.model_raw.is_some() && ag.model_short.is_some() => (
            classify_model_family_for(agent_kind, ag.model_raw.as_deref().unwrap_or("")),
            ag.model_short.clone().unwrap_or_default(),
        ),
        _ => (cached_cfg.sub_family, cached_cfg.sub_model_short.clone()),
    };
    let (top_panel, top_h) = build_top_logo_panel(geom, logos, top_family, top_badge, pal);
    let rem_after_top = geom.rows.saturating_sub(top_h);
    let bot_h = (((rem_after_top as f64) * 0.58).round() as usize)
        .clamp(12, rem_after_top.saturating_sub(6).max(12));
    let mid_h = rem_after_top.saturating_sub(bot_h).max(4);
    sel.sub_cursor = sel.sub_cursor.map(|c| c.min(subagents.len()));
    let sub_foc = sel.focus_zone == Some(FocusZone::Subagents);
    let (mid_panel, running_cnt, viewport_h) = if sel.show_orchestrator {
        let p = build_orchestrator_panel(geom.cols, mid_h, orch_cfg, sel.orch_cursor, sub_foc, pal);
        let rc = subagents.iter().filter(|a| a.status == "running").count();
        (p, rc, mid_h.saturating_sub(2))
    } else {
        let o = build_subagents_panel(
            geom.cols,
            mid_h,
            agent_kind,
            target_cwd,
            active_view_id,
            sel.sub_cursor,
            sub_foc,
            &mut sel.sub_scroll,
            main_tool,
            main_file,
            subagents,
            cached_cfg,
            &orch_cfg.active_preset,
            sel.status_notice.as_deref(),
            tick,
            pal,
        );
        (o.panel, o.running_cnt, o.viewport_h)
    };
    (
        top_panel,
        top_h,
        mid_panel,
        mid_h,
        bot_h,
        running_cnt,
        viewport_h,
    )
}
