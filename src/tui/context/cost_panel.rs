//! Dedicated Per-Model Consumption & `tokscale` Cost Breakdown Panel (`COST`).

use crate::tui::context::bar::{
    fmt_tok_compact, fmt_usd, model_family_color, render_multi_model_bar, render_prop_bar,
};
use crate::tui::context::{ContextSnapshot, TAG_CTX_TOGGLE};
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::palette::Palette;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::{BOLD, RESET};

pub fn build_cost_panel(
    snap: &ContextSnapshot,
    cols: usize,
    cost_h: usize,
    scroll_offset: usize,
    pal: &Palette,
) -> (PanelSpec, usize) {
    let box_inner_w = cols.saturating_sub(2).max(14);
    let content_w = cols.saturating_sub(4).max(12);
    let interior_h = cost_h.saturating_sub(2).max(4);
    let bar_full_w = box_inner_w.saturating_sub(2).max(8);

    let white = &pal.text;
    let dim = &pal.dim;
    let cyan = &pal.cyan;
    let green = &pal.green;
    let amber = &pal.amber;
    let mint = &pal.mint;

    let mut rows: Vec<BoxRow> = Vec::with_capacity(interior_h);

    // 1. Total spend + Prompt Cache savings + Burn Rate summary row
    let cost_s = fmt_usd(snap.cost_usd);
    let save_s = fmt_usd(snap.saved_usd);
    let rate_tail = if let Some(bph) = snap.burn_usd_per_hr {
        format!(" · {}/h", fmt_usd(bph))
    } else {
        format!(" · {}t", snap.session_turns)
    };
    let sum_plain = format!(
        " ● {} · -{} (-{}%){}",
        cost_s, save_s, snap.saved_pct, rate_tail
    );
    let sum_line = if display_width(&sum_plain) <= box_inner_w {
        format!(
            " {mint}●{RESET} {BOLD}{amber}{cost_s}{RESET} {dim}· -{green}{save_s}{RESET} {dim}(-{green}{}%{dim}){rate_tail}{RESET}",
            snap.saved_pct
        )
    } else {
        format!(
            " {mint}●{RESET} {BOLD}{amber}{cost_s}{RESET} {dim}-{green}{}%{dim}{rate_tail}{RESET}",
            snap.saved_pct
        )
    };
    rows.push(BoxRow {
        text: sum_line,
        bg: String::new(),
        tag: None,
    });

    // 2. Multi-model segmented proportional strip
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

    // Reserve 1 bottom row for `[t/Tab] toggle [HW]` when `interior_h >= 5`
    let has_footer = interior_h >= 5;
    let list_avail_h = interior_h.saturating_sub(rows.len() + usize::from(has_footer));

    let mut item_rows: Vec<BoxRow> = Vec::new();
    if snap.model_costs.is_empty() {
        item_rows.push(BoxRow {
            text: trunc_to_width(
                &format!(" {dim}○ Awaiting model usage telemetry...{RESET}"),
                content_w,
            ),
            bg: String::new(),
            tag: None,
        });
    } else {
        let two_line_mode = snap.model_costs.len() * 2 <= list_avail_h && list_avail_h >= 2;
        let mut fam_counts: [(&str, usize); 5] = [
            ("claude", 0),
            ("gemini", 0),
            ("deepseek", 0),
            ("openai", 0),
            ("grok", 0),
        ];

        for m in &snap.model_costs {
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
            let m_cost_s = fmt_usd(m.cost_usd);
            let share_u = m.share_pct;

            if two_line_mode {
                let name_w = 16usize.min(box_inner_w.saturating_sub(19).max(8));
                let name_trunc = trunc_to_width(&m.model_short, name_w);
                let pad_name = name_w.saturating_sub(display_width(&name_trunc));
                let name_padded = format!("{}{}", name_trunc, " ".repeat(pad_name));
                let fixed_cols = 3 + name_w + 1 + 6 + 1 + 4;
                if box_inner_w >= fixed_cols + 4 {
                    let bw = box_inner_w - fixed_cols;
                    let mini = render_prop_bar(share_u as f64, bw, m_col, &pal.divider);
                    item_rows.push(BoxRow {
                        text: format!(
                            " {m_col}●{RESET} {BOLD}{white}{name_padded}{RESET} {mini} {amber}{:>6}{RESET} {dim}{:>3}%{RESET}",
                            m_cost_s, share_u
                        ),
                        bg: String::new(),
                        tag: None,
                    });
                } else {
                    item_rows.push(BoxRow {
                        text: trunc_to_width(
                            &format!(
                                " {m_col}●{RESET} {BOLD}{white}{}{RESET} {amber}{m_cost_s}{RESET} {dim}{share_u}%{RESET}",
                                m.model_short
                            ),
                            box_inner_w,
                        ),
                        bg: String::new(),
                        tag: None,
                    });
                }

                let in_s = fmt_tok_compact(m.input_tokens);
                let cr_s = fmt_tok_compact(m.cache_read_tokens);
                let out_s = fmt_tok_compact(m.output_tokens);
                let detail_plain =
                    format!("   {}t · I {} · R {} · O {}", m.turns, in_s, cr_s, out_s);
                let detail_line = if display_width(&detail_plain) <= box_inner_w {
                    format!(
                        "   {dim}{}t · I{RESET} {white}{in_s}{RESET} {dim}· R{RESET} {cyan}{cr_s}{RESET} {dim}· O{RESET} {mint}{out_s}{RESET}",
                        m.turns
                    )
                } else {
                    trunc_to_width(&detail_plain, box_inner_w)
                };
                item_rows.push(BoxRow {
                    text: detail_line,
                    bg: String::new(),
                    tag: None,
                });
            } else {
                let tot_s = fmt_tok_compact(m.total_tokens());
                let name_w = 11usize.min(box_inner_w.saturating_sub(20).max(6));
                let name_trunc = trunc_to_width(&m.model_short, name_w);
                let pad_name = name_w.saturating_sub(display_width(&name_trunc));
                let name_padded = format!("{}{}", name_trunc, " ".repeat(pad_name));
                let fixed_cols = 3 + name_w + 1 + 6 + 1 + 5 + 1 + 4;
                if box_inner_w >= fixed_cols + 4 {
                    let bw = box_inner_w - fixed_cols;
                    let mini = render_prop_bar(share_u as f64, bw, m_col, &pal.divider);
                    item_rows.push(BoxRow {
                        text: format!(
                            " {m_col}●{RESET} {white}{name_padded}{RESET} {mini} {amber}{:>6}{RESET} {cyan}{:>5}{RESET} {dim}{:>3}%{RESET}",
                            m_cost_s, tot_s, share_u
                        ),
                        bg: String::new(),
                        tag: None,
                    });
                } else {
                    item_rows.push(BoxRow {
                        text: trunc_to_width(
                            &format!(
                                " {m_col}●{RESET} {white}{name_padded}{RESET} {amber}{:>6}{RESET} {cyan}{:>5}{RESET} {dim}{:>3}%{RESET}",
                                m_cost_s, tot_s, share_u
                            ),
                            box_inner_w,
                        ),
                        bg: String::new(),
                        tag: None,
                    });
                }
            }
        }
    }

    let max_scroll = item_rows.len().saturating_sub(list_avail_h);
    let clamped_scroll = scroll_offset.min(max_scroll);
    for r in item_rows
        .into_iter()
        .skip(clamped_scroll)
        .take(list_avail_h)
    {
        rows.push(r);
    }

    if has_footer {
        while rows.len() + 1 < interior_h {
            rows.push(BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            });
        }
        let foot_left = format!(" {dim}[{amber}t{dim}/{amber}Tab{dim}] toggle");
        let foot_right_plain = "[HW] ";
        let flw = display_width(&foot_left);
        let frw = display_width(foot_right_plain);
        let foot_row = if flw + frw < box_inner_w {
            format!(
                "{foot_left}{}{dim}[{amber}HW{dim}]{RESET} ",
                " ".repeat(box_inner_w - flw - frw)
            )
        } else {
            format!(" {dim}[{amber}t{dim}] ⇄ [{amber}HW{dim}]{RESET}")
        };
        rows.push(BoxRow {
            text: foot_row,
            bg: String::new(),
            tag: Some((TAG_CTX_TOGGLE, 0)),
        });
    }

    let n_models = snap.model_costs.len();
    let badge = if max_scroll > 0 {
        format!("↕ {}m · {} [HW]", n_models, cost_s)
    } else if n_models > 0 {
        format!("{}m · {} [HW]", n_models, cost_s)
    } else {
        format!("{} · [HW]", cost_s)
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
        },
        max_scroll,
    )
}
