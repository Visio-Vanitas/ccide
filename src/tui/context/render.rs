//! Clean Context Window & Prompt Cache Breakdown Panel Renderer (`CONTEXT`).

use crate::commands::statusline::CtxTier;
use crate::tui::context::bar::{fmt_tok_compact, render_prop_bar, render_segmented_spectrum_bar};
use crate::tui::context::{ContextSnapshot, TAG_CTX_TOGGLE};
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::palette::Palette;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::{BOLD, RESET};

pub fn build_context_panel(
    snap: &ContextSnapshot,
    cols: usize,
    ctx_h: usize,
    include_toggle_footer: bool,
    pal: &Palette,
) -> PanelSpec {
    let box_inner_w = cols.saturating_sub(2).max(14);
    let content_w = cols.saturating_sub(4).max(12);
    let interior_h = ctx_h.saturating_sub(2).max(4);
    let bar_full_w = box_inner_w.saturating_sub(2).max(8);

    let white = &pal.text;
    let dim = &pal.dim;
    let cyan = &pal.cyan;
    let green = &pal.green;
    let amber = &pal.amber;
    let coral = &pal.coral;
    let blue = &pal.blue;
    let mint = &pal.mint;
    let lavender = &pal.lavender;

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
    let head_left = format!(
        " {tier_col}●{RESET} {BOLD}{white}{used_s}{dim}/{lim_s}{RESET} {BOLD}{tier_col}{}%{RESET}",
        snap.budget.used_pct
    );
    let head_right_plain = format!("free {free_s} ");
    let lw = display_width(&head_left);
    let rw = display_width(&head_right_plain);
    let head_line = if lw + rw < box_inner_w {
        format!(
            "{head_left}{}{dim}free {white}{free_s}{RESET} ",
            " ".repeat(box_inner_w - lw - rw)
        )
    } else {
        head_left
    };
    rows.push(BoxRow {
        text: head_line,
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

    let inline_bar_w = box_inner_w.saturating_sub(23).max(4);
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
        let tok_s = fmt_tok_compact(tok);
        let pct_u = share_pct as u64;
        if box_inner_w >= 28 {
            let mini = render_prop_bar(share_pct, inline_bar_w, col, &pal.divider);
            rows.push(BoxRow {
                text: format!(
                    " {col}●{RESET} {white}{:<7}{RESET} {mini} {white}{:>5}{RESET} {dim}{:>3}%{RESET}",
                    label, tok_s, pct_u
                ),
                bg: String::new(),
                tag: None,
            });
        } else {
            rows.push(BoxRow {
                text: trunc_to_width(
                    &format!(
                        " {col}●{RESET} {white}{:<7}{RESET} {:>5} {:>3}%",
                        label, tok_s, pct_u
                    ),
                    content_w,
                ),
                bg: String::new(),
                tag: None,
            });
        }
    }

    if rows.len() + reserve_bot + 1 < interior_h && box_inner_w >= 28 {
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
        let mini = render_prop_bar(free_pct, inline_bar_w, free_col, &pal.divider);
        rows.push(BoxRow {
            text: format!(
                " {free_col}○{RESET} {dim}{:<7}{RESET} {mini} {free_col}{:>5}{RESET} {dim}{:>3}%{RESET}",
                "FREE",
                free_s,
                free_pct as u64
            ),
            bg: String::new(),
            tag: None,
        });
    }

    if rows.len() + reserve_bot < interior_h {
        let hit_pct = snap.session_hit_pct;
        let hit_col: &str = if snap.turns == 0 {
            dim
        } else if hit_pct >= 70 {
            green
        } else if hit_pct >= 35 {
            amber
        } else {
            coral
        };
        let r_s = fmt_tok_compact(snap.cum_cache_read_tokens);
        let is_cold = snap.turns >= 2 && snap.last_hit_pct == 0;
        let now_plain = if is_cold {
            "cold 0%".to_string()
        } else {
            format!("now {}%", snap.last_hit_pct)
        };
        let now_styled = if is_cold {
            format!("{coral}cold 0%{RESET}")
        } else {
            format!("{dim}now {cyan}{}%{RESET}", snap.last_hit_pct)
        };
        let cache_plain = format!(" ● CACHE {:>3}% · {} · R {}", hit_pct, now_plain, r_s);
        let cache_line = if display_width(&cache_plain) <= box_inner_w {
            format!(
                " {hit_col}●{RESET} {BOLD}{white}CACHE{RESET} {hit_col}{:>3}%{RESET} {dim}·{RESET} {now_styled} {dim}· R{RESET} {cyan}{r_s}{RESET}",
                hit_pct
            )
        } else {
            format!(
                " {hit_col}●{RESET} {white}CACHE{RESET} {hit_col}{hit_pct}%{RESET} {now_styled}"
            )
        };
        rows.push(BoxRow {
            text: cache_line,
            bg: String::new(),
            tag: None,
        });
    }

    if rows.len() + reserve_bot < interior_h {
        let win_s = fmt_tok_compact(snap.model_win_tokens);
        let meta_plain = format!("   win {} · cmp {} · {}t", win_s, lim_s, snap.turns);
        rows.push(BoxRow {
            text: format!("{dim}{}{RESET}", trunc_to_width(&meta_plain, box_inner_w)),
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
    }
}
