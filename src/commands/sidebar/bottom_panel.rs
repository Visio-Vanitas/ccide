//! Bottom Region (`Hardware Telemetry ⇄ Split CONTEXT + COST`) Panel Assembler (`src/commands/sidebar/bottom_panel.rs`).

use crate::commands::sidebar::hit_mask::FocusZone;
use crate::commands::sidebar::mouse_actions::SidebarSelectState;
use crate::telemetry::chart::build_telemetry_panel_rows;
use crate::telemetry::TelemetrySampler;
use crate::tui::context::{build_context_panel, build_cost_panel, ContextSnapshot};
use crate::tui::grid::PanelSpec;
use crate::tui::palette::Palette;

#[allow(clippy::too_many_arguments)]
pub fn append_bottom_panels(
    panels: &mut Vec<PanelSpec>,
    sel: &mut SidebarSelectState,
    ctx_snap: &ContextSnapshot,
    telemetry: &TelemetrySampler,
    cols: usize,
    inner_w: usize,
    bot_h: usize,
    cost_max_scroll: &mut usize,
    pal: &Palette,
) -> (bool, usize) {
    let split_ctx_cost = sel.show_context && bot_h >= 18;
    // Fixed 11-row compact `CONTEXT` (header + spectrum + 5 components + CACHE + meta = 9 interior rows)
    let ctx_box_h = if split_ctx_cost {
        11.min(bot_h.saturating_sub(8))
    } else {
        bot_h
    };
    let ctx_foc = sel.focus_zone == Some(FocusZone::Context);
    let cost_foc = sel.focus_zone == Some(FocusZone::Cost);
    if sel.show_context {
        if split_ctx_cost {
            panels.push(build_context_panel(
                ctx_snap, cols, ctx_box_h, false, ctx_foc, pal,
            ));
            let (cost_p, max_sc) = build_cost_panel(
                ctx_snap,
                cols,
                bot_h - ctx_box_h,
                sel.cost_scroll,
                sel.cost_lines,
                cost_foc,
                pal,
            );
            *cost_max_scroll = max_sc;
            sel.cost_scroll = sel.cost_scroll.min(*cost_max_scroll);
            panels.push(cost_p);
        } else {
            *cost_max_scroll = 0;
            panels.push(build_context_panel(
                ctx_snap,
                cols,
                bot_h,
                true,
                ctx_foc || cost_foc,
                pal,
            ));
        }
    } else {
        panels.push(PanelSpec {
            title: telemetry.cpu_brand.to_uppercase(),
            badge: format!(
                "{}C·{:.0}G [CTX {}%]",
                telemetry.cpu_cores, telemetry.total_mem_gb, ctx_snap.budget.used_pct
            ),
            accent: pal.mint.clone(),
            rows: build_telemetry_panel_rows(telemetry, inner_w, bot_h, pal),
            fixed_height: Some(bot_h),
            weight: 1.12,
            raw_interior: false,
            focused: sel.focus_zone == Some(FocusZone::Telemetry),
        });
    }
    (split_ctx_cost, ctx_box_h)
}
