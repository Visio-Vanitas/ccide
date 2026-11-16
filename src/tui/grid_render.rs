//! Closed 4-Sided Box Border, Row & Multi-Panel Diff Renderer (`src/tui/grid_render.rs`).

use std::borrow::Cow;
use std::collections::HashMap;

use crate::tui::grid::{allocate_panel_heights, BoxRow, PanelSpec, TermGeom};
use crate::tui::palette::Palette;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::{BOLD, RESET};

pub fn format_top_border(
    cols: usize,
    title: &str,
    badge: &str,
    accent: &str,
    focused: bool,
    pal: &Palette,
) -> String {
    let border = if focused && !accent.is_empty() {
        accent
    } else {
        &pal.border
    };
    let badge_col = if focused { &pal.text } else { &pal.dim };
    let inner = cols.saturating_sub(2);
    if inner < 4 {
        return format!("{border}╭{}╮{RESET}", "─".repeat(inner));
    }
    let raw_t = title.trim();
    let raw_t_w = display_width(raw_t);
    let max_badge_inner = inner.saturating_sub(raw_t_w + 2 + 5);
    let b_clean = if badge.trim().is_empty() || max_badge_inner < 4 {
        String::new()
    } else {
        let fit = crate::session::compact_model_for_width(badge.trim(), max_badge_inner);
        format!(" {} ", fit)
    };
    let b_w = display_width(&b_clean);
    let t_fit = if inner < raw_t_w + 4 + b_w {
        trunc_to_width(raw_t, inner.saturating_sub(b_w + 4).max(1))
    } else {
        raw_t.to_string()
    };
    let t_clean = format!(" {} ", t_fit);
    let t_w = display_width(&t_clean);
    let dashes = inner.saturating_sub(1 + t_w + b_w + if b_w > 0 { 1 } else { 0 });
    if b_w > 0 {
        format!(
            "{border}╭─{RESET}{BOLD}{accent}{t_clean}{RESET}{border}{}{RESET}{badge_col}{b_clean}{RESET}{border}─╮{RESET}",
            "─".repeat(dashes)
        )
    } else {
        format!(
            "{border}╭─{RESET}{BOLD}{accent}{t_clean}{RESET}{border}{}╮{RESET}",
            "─".repeat(dashes)
        )
    }
}

pub fn format_bottom_border(cols: usize, border: &str) -> String {
    format!("{border}╰{}╯{RESET}", "─".repeat(cols.saturating_sub(2)))
}

pub fn format_boxed_row(cols: usize, row: &BoxRow, border: &str) -> String {
    let inner_w = cols.saturating_sub(2);
    let raw_w = display_width(&row.text);
    let (safe_txt, vis_w): (Cow<'_, str>, usize) = if raw_w > inner_w {
        let t = trunc_to_width(&row.text, inner_w);
        let w = display_width(&t);
        (Cow::Owned(t), w)
    } else {
        (Cow::Borrowed(row.text.as_str()), raw_w)
    };
    let pad = inner_w.saturating_sub(vis_w);
    if !row.bg.is_empty() {
        // Re-anchor `row.bg` after every `RESET` AND across trailing `pad` spaces so
        // highlighted rows always paint a seamless full-width rectangle from `│` to `│`.
        let bg_kept = safe_txt.replace(RESET, &format!("{RESET}{}", row.bg));
        let tail_bg = if bg_kept.ends_with(&row.bg) {
            ""
        } else {
            row.bg.as_str()
        };
        format!(
            "{border}│{RESET}{}{bg_kept}{tail_bg}{}{RESET}{border}│{RESET}",
            row.bg,
            " ".repeat(pad)
        )
    } else {
        format!(
            "{border}│{RESET}{}{}{RESET}{border}│{RESET}",
            safe_txt,
            " ".repeat(pad)
        )
    }
}

/// Renders all stacked `PanelSpec`s into `out_buf` using row-level diffing against `prev_rows`
/// so ONLY rows whose content actually changed are emitted to `stdout`.
pub fn render_multi_panel_grid(
    geom: TermGeom,
    panels: &[PanelSpec],
    pal: &Palette,
    out_buf: &mut String,
    row_tag_map: &mut HashMap<usize, (u8, usize)>,
    prev_rows: &mut Vec<String>,
) {
    out_buf.clear();
    row_tag_map.clear();
    if prev_rows.len() != geom.rows + 1 {
        prev_rows.clear();
        prev_rows.resize(geom.rows + 1, String::new());
    }

    let heights = allocate_panel_heights(geom.rows, panels);
    let mut cur_row = 1usize;

    let mut emit_row = |r: usize, line_str: String| {
        if r <= geom.rows && prev_rows[r] != line_str {
            out_buf.push_str(&format!("\x1b[{};1H{}", r, line_str));
            prev_rows[r] = line_str;
        }
    };

    for (sp, &h_box) in panels.iter().zip(heights.iter()) {
        if cur_row > geom.rows || h_box < 2 {
            break;
        }
        let top_r = cur_row;
        let bot_r = (cur_row + h_box - 1).min(geom.rows);
        let interior_cnt = bot_r.saturating_sub(top_r + 1);
        let box_border = if sp.focused && !sp.accent.is_empty() {
            &sp.accent
        } else {
            &pal.border
        };

        emit_row(
            top_r,
            format_top_border(geom.cols, &sp.title, &sp.badge, &sp.accent, sp.focused, pal),
        );

        if sp.raw_interior {
            for (idx, r) in ((top_r + 1)..bot_r).enumerate() {
                let extra = sp.rows.get(idx).map(|br| br.text.as_str()).unwrap_or("");
                emit_row(
                    r,
                    format!(
                        "{box_border}│{RESET}\x1b[{};{}H{box_border}│{RESET}{extra}",
                        r, geom.cols
                    ),
                );
            }
        } else {
            let empty_row = BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            };
            for i in 0..interior_cnt {
                let r = top_r + 1 + i;
                let brow = sp.rows.get(i).unwrap_or(&empty_row);
                emit_row(r, format_boxed_row(geom.cols, brow, box_border));
                if let Some(t) = brow.tag {
                    row_tag_map.insert(r, t);
                }
            }
        }

        if bot_r > top_r {
            emit_row(bot_r, format_bottom_border(geom.cols, box_border));
        }
        cur_row = bot_r + 1;
    }
}
