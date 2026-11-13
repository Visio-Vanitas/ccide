//! Dedicated Per-Model Consumption, `TPS` & `TTFT` Breakdown Panel (`COST`).
//!
//! Supports 3 configurable display densities (`~/.config/cc-ide/viewer.json` `"cost_lines"` / `ccide setup`):
//! - `3` lines (default, detailed): Row 1 `● model  $usd  pct%`, Row 2 `turns · I · R · O`, Row 3 `TPS · TTFT`.
//! - `2` lines (brief, no TPS/TTFT): Row 1 `● model  $usd  pct%`, Row 2 `turns · I · R · O`.
//! - `1` line (compact USD summary): Row 1 `● model  $usd  pct%`.

use crate::session::compact_model_for_width;
use crate::tui::context::bar::{
    compute_cache_hit_pct, fmt_share_pct, fmt_tok_compact, fmt_tps, fmt_ttft, fmt_usd,
    fmt_usd_precise, model_family_color, render_multi_model_bar,
};
use crate::tui::context::{ContextSnapshot, TAG_CTX_TOGGLE};
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::layout::{Align, ColumnWidths, RowLayout};
use crate::tui::palette::Palette;
use crate::tui::width::display_width;
use crate::tui::{BOLD, RESET};

pub fn build_cost_panel(
    snap: &ContextSnapshot,
    cols: usize,
    cost_h: usize,
    scroll_offset: usize,
    cost_lines: u8,
    panel_focused: bool,
    pal: &Palette,
) -> (PanelSpec, usize) {
    let box_inner_w = cols.saturating_sub(2).max(14);
    let interior_h = cost_h.saturating_sub(2).max(4);
    let bar_full_w = box_inner_w.saturating_sub(2).max(8);
    let density = cost_lines.clamp(1, 3);
    let row_fmt = RowLayout::new(box_inner_w);
    let sub_fmt = RowLayout::new(box_inner_w).padding(3, 1);

    let (white, dim, cyan, green, amber, mint) = (
        &pal.text, &pal.dim, &pal.cyan, &pal.green, &pal.amber, &pal.mint,
    );

    let mut rows: Vec<BoxRow> = Vec::with_capacity(interior_h);

    let cost_hdr_s = fmt_usd_precise(snap.cost_usd);
    let save_s = fmt_usd_precise(snap.saved_usd);
    let rate_tail = if let Some(bph) = snap.burn_usd_per_hr {
        format!("{}/h", fmt_usd_precise(bph))
    } else if snap.lines_added > 0 || snap.lines_removed > 0 {
        format!("+{}/-{}", snap.lines_added, snap.lines_removed)
    } else {
        format!("{}t", snap.session_turns)
    };
    let full_sum = format!(
        "{mint}●{RESET} {BOLD}{amber}{cost_hdr_s}{RESET} {dim}· -{green}{save_s}{RESET} {dim}(-{green}{}%{dim}) · {rate_tail}{RESET}",
        snap.saved_pct
    );
    let compact_sum = format!(
        "{mint}●{RESET} {BOLD}{amber}{}{RESET} {dim}· -{green}{}{RESET} {dim}(-{green}{}%{dim}) · {rate_tail}{RESET}",
        fmt_usd(snap.cost_usd),
        fmt_usd(snap.saved_usd),
        snap.saved_pct
    );
    let sum_left = if display_width(&full_sum) + 2 <= box_inner_w {
        full_sum
    } else {
        compact_sum
    };
    rows.push(BoxRow {
        text: row_fmt.split(&sum_left, "", 0),
        bg: String::new(),
        tag: None,
    });

    if interior_h >= 5 {
        rows.push(BoxRow {
            text: format!(
                " {}",
                render_multi_model_bar(&snap.model_costs, bar_full_w, pal)
            ),
            bg: String::new(),
            tag: None,
        });
    }

    let has_footer = interior_h >= 5;
    let list_avail_h = interior_h.saturating_sub(rows.len() + usize::from(has_footer));

    let mut item_rows: Vec<BoxRow> = Vec::new();
    if snap.model_costs.is_empty() {
        item_rows.push(BoxRow {
            text: row_fmt.split(
                &format!("{dim}○ Awaiting model usage telemetry...{RESET}"),
                "",
                0,
            ),
            bg: String::new(),
            tag: None,
        });
    } else {
        let total_cost: f64 = snap.model_costs.iter().map(|m| m.cost_usd).sum();
        let cells: Vec<[String; 2]> = snap
            .model_costs
            .iter()
            .map(|m| {
                [
                    fmt_usd_precise(m.cost_usd),
                    fmt_share_pct(m.cost_usd, total_cost, m.share_pct),
                ]
            })
            .collect();
        let cw = ColumnWidths::measure(&cells, [5, 3]);
        let mut fam_counts: [(&str, usize); 5] = [
            ("claude", 0),
            ("gemini", 0),
            ("deepseek", 0),
            ("openai", 0),
            ("grok", 0),
        ];

        for (m, [c_s, p_s]) in snap.model_costs.iter().zip(cells.iter()) {
            let rank = fam_counts
                .iter_mut()
                .find(|(f, _)| *f == m.family)
                .map(|(_, cnt)| {
                    let r = *cnt;
                    *cnt += 1;
                    r
                })
                .unwrap_or(0);
            let m_col = model_family_color(m.family, rank, pal);

            let right = format!(
                "{} {}",
                cw.fmt_col(0, amber, c_s, Align::Right),
                cw.fmt_col(1, dim, p_s, Align::Right)
            );
            let right_w = cw.widths[0] + 1 + cw.widths[1];
            let name_budget = box_inner_w.saturating_sub(2 + 2 + 1 + right_w).max(6);
            let fit_name = compact_model_for_width(&m.model_short, name_budget);
            let left = if density >= 2 {
                format!("{m_col}●{RESET} {BOLD}{white}{fit_name}{RESET}")
            } else {
                format!("{m_col}●{RESET} {white}{fit_name}{RESET}")
            };
            item_rows.push(BoxRow {
                text: row_fmt.split(&left, &right, 1),
                bg: String::new(),
                tag: None,
            });

            if density >= 2 {
                let in_s = fmt_tok_compact(m.input_tokens);
                let cr_s = fmt_tok_compact(m.cache_read_tokens);
                let out_s = fmt_tok_compact(m.output_tokens);
                let sub_left = format!(
                    "{dim}{}t · I{RESET} {white}{in_s}{RESET} {dim}· R{RESET} {cyan}{cr_s}{RESET} {dim}· O{RESET} {mint}{out_s}{RESET}",
                    m.turns
                );
                item_rows.push(BoxRow {
                    text: sub_fmt.split(&sub_left, "", 0),
                    bg: String::new(),
                    tag: None,
                });
            }

            if density >= 3 {
                let tps_s = fmt_tps(m.tps);
                let ttft_s = fmt_ttft(m.ttft_ms);
                let hit_pct = compute_cache_hit_pct(
                    m.cache_read_tokens,
                    m.input_tokens.saturating_add(m.cache_read_tokens),
                );
                let tps_col = if m.tps.is_some() { mint } else { dim };
                let hit_col = if hit_pct >= 70.0 {
                    green
                } else if hit_pct >= 35.0 {
                    amber
                } else {
                    dim
                };
                let ttft_col = if m.ttft_ms.is_some() { cyan } else { dim };
                let perf_left = format!(
                    "{dim}TPS{RESET} {tps_col}{tps_s}{RESET} {dim}· HIT{RESET} {hit_col}{hit_pct:.1}%{RESET}"
                );
                let perf_right = format!("{dim}TTFT{RESET} {ttft_col}{ttft_s}{RESET}");
                item_rows.push(BoxRow {
                    text: sub_fmt.split(&perf_left, &perf_right, 1),
                    bg: String::new(),
                    tag: None,
                });
            }
        }
    }

    let max_scroll = item_rows.len().saturating_sub(list_avail_h);
    let clamped_scroll = scroll_offset.min(max_scroll);
    rows.extend(
        item_rows
            .into_iter()
            .skip(clamped_scroll)
            .take(list_avail_h),
    );

    if has_footer {
        while rows.len() + 1 < interior_h {
            rows.push(BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            });
        }
        let foot_left = if max_scroll > 0 {
            format!("{dim}[{amber}t{dim}] toggle · {cyan}↕{dim} scroll{RESET}")
        } else {
            format!("{dim}[{amber}t{dim}/{amber}Tab{dim}] toggle{RESET}")
        };
        rows.push(BoxRow {
            text: row_fmt.split(&foot_left, &format!("{dim}[{amber}HW{dim}]{RESET}"), 1),
            bg: String::new(),
            tag: Some((TAG_CTX_TOGGLE, 0)),
        });
    }

    let n_models = snap.model_costs.len();
    let badge = if max_scroll > 0 {
        format!(
            "↕ {}/{} · {} [HW]",
            clamped_scroll + 1,
            max_scroll + 1,
            cost_hdr_s
        )
    } else if n_models > 0 {
        format!("{}m · {} [HW]", n_models, cost_hdr_s)
    } else {
        format!("{} · [HW]", cost_hdr_s)
    };

    (
        PanelSpec {
            title: "COST".to_string(),
            badge,
            accent: amber.to_string(),
            rows,
            fixed_height: Some(cost_h),
            weight: 0.56,
            raw_interior: false,
            focused: panel_focused,
        },
        max_scroll,
    )
}
