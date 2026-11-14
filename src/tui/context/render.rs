//! Clean Context Window & Prompt Cache Breakdown Panel Renderer (`CONTEXT`).
//!
//! Powered by `crate::tui::layout::{Align, ColSpec, ColumnWidths, RowLayout}`.

use crate::commands::statusline::CtxTier;
use crate::tui::context::bar::{fmt_tok_compact, render_segmented_spectrum_bar};
use crate::tui::context::{ContextSnapshot, TAG_CTX_TOGGLE};
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::layout::{Align, ColSpec, ColumnWidths, RowLayout};
use crate::tui::palette::Palette;
use crate::tui::{BOLD, RESET};

pub fn build_context_panel(
    snap: &ContextSnapshot,
    cols: usize,
    ctx_h: usize,
    include_toggle_footer: bool,
    panel_focused: bool,
    pal: &Palette,
) -> PanelSpec {
    let box_inner_w = cols.saturating_sub(2).max(14);
    let interior_h = ctx_h.saturating_sub(2).max(4);
    let bar_full_w = box_inner_w.saturating_sub(2).max(8);
    let row_fmt = RowLayout::new(box_inner_w);

    let (white, dim, cyan, green, amber, coral, blue, mint, lavender) = (
        &pal.text,
        &pal.dim,
        &pal.cyan,
        &pal.green,
        &pal.amber,
        &pal.coral,
        &pal.blue,
        &pal.mint,
        &pal.lavender,
    );

    let tier_col: &str = match snap.budget.tier {
        CtxTier::Crit => coral,
        CtxTier::Warn => amber,
        CtxTier::Ok => mint,
    };

    let free_tokens = snap
        .budget
        .limit_tokens
        .saturating_sub(snap.total_in_tokens);
    let reserve_bot = usize::from(include_toggle_footer);
    let mut rows: Vec<BoxRow> = Vec::with_capacity(interior_h);

    let used_s = fmt_tok_compact(snap.total_in_tokens);
    let lim_s = fmt_tok_compact(snap.budget.limit_tokens);
    let free_s = fmt_tok_compact(free_tokens);
    rows.push(BoxRow {
        text: row_fmt.split(
            &format!(
                "{tier_col}●{RESET} {BOLD}{white}{used_s}{dim}/{lim_s}{RESET} {BOLD}{tier_col}{}%{RESET}",
                snap.budget.used_pct
            ),
            &format!("{dim}free {white}{free_s}{RESET}"),
            1,
        ),
        bg: String::new(),
        tag: None,
    });

    if rows.len() + reserve_bot < interior_h {
        rows.push(BoxRow {
            text: format!(" {}", render_segmented_spectrum_bar(snap, bar_full_w, pal)),
            bg: String::new(),
            tag: None,
        });
    }

    let used_denom = snap.total_in_tokens.max(1) as f64;
    let cats: [(&str, u64, &str); 5] = [
        ("SYS+MD", snap.sys_md_tokens, lavender),
        ("TOOLS", snap.tools_tokens, blue),
        ("TOOL IO", snap.tool_io_tokens, cyan),
        ("THINK", snap.thinking_tokens, amber),
        ("CHAT", snap.chat_tokens, mint),
    ];

    let mut tok_samples: Vec<[String; 1]> =
        cats.iter().map(|&(_, t, _)| [fmt_tok_compact(t)]).collect();
    tok_samples.push([free_s.clone()]);
    let tok_w = ColumnWidths::measure(&tok_samples, [5]).widths[0];

    for &(label, tok, col) in &cats {
        if rows.len() + reserve_bot >= interior_h {
            break;
        }
        let share_pct = if snap.total_in_tokens > 0 {
            ((tok as f64 / used_denom) * 100.0)
                .round()
                .clamp(0.0, 100.0)
        } else {
            0.0
        };
        let dot = format!("{col}●{RESET}");
        let tok_s = fmt_tok_compact(tok);
        let pct_s = format!("{}%", share_pct as u64);
        rows.push(BoxRow {
            text: row_fmt.columns(
                &[
                    ColSpec::Styled(&dot),
                    ColSpec::Fixed {
                        text: label,
                        width: 7,
                        align: Align::Left,
                        style: white,
                    },
                    ColSpec::FillBar {
                        pct: share_pct,
                        fill_col: col,
                        track_col: &pal.divider,
                        min_w: 4,
                    },
                    ColSpec::Fixed {
                        text: &tok_s,
                        width: tok_w,
                        align: Align::Right,
                        style: white,
                    },
                    ColSpec::Fixed {
                        text: &pct_s,
                        width: 4,
                        align: Align::Right,
                        style: dim,
                    },
                ],
                1,
            ),
            bg: String::new(),
            tag: None,
        });
    }

    if rows.len() + reserve_bot + 1 < interior_h {
        let free_pct = if snap.budget.limit_tokens > 0 {
            ((free_tokens as f64 / snap.budget.limit_tokens as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0)
        } else {
            0.0
        };
        let free_col: &str = if free_pct <= 20.0 {
            coral
        } else if free_pct <= 50.0 {
            amber
        } else {
            green
        };
        let dot = format!("{free_col}○{RESET}");
        let pct_s = format!("{}%", free_pct as u64);
        rows.push(BoxRow {
            text: row_fmt.columns(
                &[
                    ColSpec::Styled(&dot),
                    ColSpec::Fixed {
                        text: "FREE",
                        width: 7,
                        align: Align::Left,
                        style: dim,
                    },
                    ColSpec::FillBar {
                        pct: free_pct,
                        fill_col: free_col,
                        track_col: &pal.divider,
                        min_w: 4,
                    },
                    ColSpec::Fixed {
                        text: &free_s,
                        width: tok_w,
                        align: Align::Right,
                        style: free_col,
                    },
                    ColSpec::Fixed {
                        text: &pct_s,
                        width: 4,
                        align: Align::Right,
                        style: dim,
                    },
                ],
                1,
            ),
            bg: String::new(),
            tag: None,
        });
    }

    if rows.len() + reserve_bot < interior_h {
        let hit_pct = snap.session_hit_pct;
        let hit_col: &str = if snap.turns == 0 {
            dim
        } else if hit_pct >= 70.0 {
            green
        } else if hit_pct >= 35.0 {
            amber
        } else {
            coral
        };
        let r_s = fmt_tok_compact(snap.cum_cache_read_tokens);
        let now_styled = if snap.turns >= 2 && snap.last_hit_pct < 0.05 {
            format!("{coral}cold 0.0%{RESET}")
        } else {
            format!("{dim}now {cyan}{:.1}%{RESET}", snap.last_hit_pct)
        };
        rows.push(BoxRow {
            text: row_fmt.split(
                &format!(
                    "{hit_col}●{RESET} {BOLD}{white}CACHE{RESET} {hit_col}{:.1}%{RESET} {dim}·{RESET} {now_styled} {dim}· R{RESET} {cyan}{r_s}{RESET}",
                    hit_pct
                ),
                "",
                0,
            ),
            bg: String::new(),
            tag: None,
        });
    }

    if rows.len() + reserve_bot < interior_h {
        let win_s = fmt_tok_compact(snap.model_win_tokens);
        let mut meta_plain = format!("win {} · cmp {} · {}t", win_s, lim_s, snap.turns);
        match (snap.rl_5h_pct, snap.rl_7d_pct) {
            (Some(h5), Some(d7)) => meta_plain.push_str(&format!(" · 5h:{h5}% 7d:{d7}%")),
            (Some(h5), None) => meta_plain.push_str(&format!(" · 5h:{h5}%")),
            (None, Some(d7)) => meta_plain.push_str(&format!(" · 7d:{d7}%")),
            (None, None) if snap.lines_added > 0 || snap.lines_removed > 0 => {
                meta_plain.push_str(&format!(" · +{}/-{}", snap.lines_added, snap.lines_removed));
            }
            _ => {}
        }
        rows.push(BoxRow {
            text: RowLayout::new(box_inner_w).padding(3, 1).split(
                &format!("{dim}{meta_plain}{RESET}"),
                "",
                0,
            ),
            bg: String::new(),
            tag: None,
        });
    }

    if include_toggle_footer {
        while rows.len() + 1 < interior_h {
            rows.push(BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            });
        }
        while rows.len() >= interior_h {
            rows.pop();
        }
        if rows.len() < interior_h {
            rows.push(BoxRow {
                text: row_fmt.split(
                    &format!("{dim}[{amber}t{dim}/{amber}Tab{dim}] toggle{RESET}"),
                    &format!("{dim}[{amber}HW{dim}]{RESET}"),
                    1,
                ),
                bg: String::new(),
                tag: Some((TAG_CTX_TOGGLE, 0)),
            });
        }
    }

    PanelSpec {
        title: if snap.is_subagent {
            "CTX · SUB".to_string()
        } else {
            "CONTEXT".to_string()
        },
        badge: format!("{}% · [HW]", snap.budget.used_pct),
        accent: tier_col.to_string(),
        rows,
        fixed_height: Some(ctx_h),
        weight: 0.56,
        raw_interior: false,
        focused: panel_focused,
    }
}
