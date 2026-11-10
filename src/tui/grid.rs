//! Multi-Panel Grid Layout & Closed 4-Sided Box Renderer.

use std::collections::HashMap;

use crate::ffi::{TIOCGWINSZ, Winsize, ioctl};
use crate::tui::width::display_width;
use crate::tui::{BOLD, DIM, GRAY, RESET};

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct TermGeom {
    pub rows: usize,
    pub cols: usize,
    pub cell_w_px: f64,
    pub cell_h_px: f64,
}

/// Queries live terminal rows, columns, and pixel geometry via `ioctl(1, TIOCGWINSZ)`.
pub fn get_terminal_geometry() -> TermGeom {
    unsafe {
        let mut ws = Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if ioctl(1, TIOCGWINSZ, &mut ws as *mut Winsize) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            let cols = ws.ws_col as usize;
            let rows = ws.ws_row as usize;
            let cw = if ws.ws_xpixel > 0 {
                (ws.ws_xpixel as f64) / (cols as f64)
            } else {
                14.0
            };
            let ch = if ws.ws_ypixel > 0 {
                (ws.ws_ypixel as f64) / (rows as f64)
            } else {
                32.0
            };
            return TermGeom {
                rows,
                cols,
                cell_w_px: cw,
                cell_h_px: ch,
            };
        }
    }
    TermGeom {
        rows: 65,
        cols: 43,
        cell_w_px: 14.0,
        cell_h_px: 32.0,
    }
}

#[derive(Clone)]
pub struct BoxRow {
    pub text: String,
    pub bg: &'static str,
    pub tag: Option<(u8, usize)>, // 1 = main, 2 = subagent
}

pub struct PanelSpec {
    pub title: String,
    pub badge: String,
    pub accent: &'static str,
    pub rows: Vec<BoxRow>,
    pub fixed_height: Option<usize>,
    pub weight: f64,
    pub raw_interior: bool,
}

pub fn format_top_border(cols: usize, title: &str, badge: &str, accent: &str) -> String {
    let t_clean = format!(" {} ", title.trim());
    let b_clean = if badge.trim().is_empty() {
        String::new()
    } else {
        format!(" {} ", badge.trim())
    };
    let t_w = display_width(&t_clean);
    let b_w = display_width(&b_clean);
    let inner = cols.saturating_sub(2);
    let dashes = inner.saturating_sub(1 + t_w + b_w + if b_w > 0 { 1 } else { 0 });
    if b_w > 0 {
        format!(
            "{GRAY}╭─{RESET}{BOLD}{accent}{t_clean}{RESET}{GRAY}{}{RESET}{DIM}{b_clean}{RESET}{GRAY}─╮{RESET}",
            "─".repeat(dashes)
        )
    } else {
        format!(
            "{GRAY}╭─{RESET}{BOLD}{accent}{t_clean}{RESET}{GRAY}{}╮{RESET}",
            "─".repeat(dashes)
        )
    }
}

pub fn format_bottom_border(cols: usize) -> String {
    format!("{GRAY}╰{}╯{RESET}", "─".repeat(cols.saturating_sub(2)))
}

pub fn format_boxed_row(cols: usize, row: &BoxRow) -> String {
    let inner_w = cols.saturating_sub(2);
    let vis_w = display_width(&row.text);
    let pad = inner_w.saturating_sub(vis_w);
    if !row.bg.is_empty() {
        format!(
            "{GRAY}│{RESET}{}{}{}{RESET}{GRAY}│{RESET}",
            row.bg,
            row.text,
            " ".repeat(pad)
        )
    } else {
        format!(
            "{GRAY}│{RESET}{}{}{RESET}{GRAY}│{RESET}",
            row.text,
            " ".repeat(pad)
        )
    }
}

/// Deterministically distributes `total_rows` across `specs` so `heights.iter().sum() == total_rows`.
pub fn allocate_panel_heights(total_rows: usize, specs: &[PanelSpec]) -> Vec<usize> {
    let n = specs.len();
    if n == 0 {
        return vec![];
    }
    let mut heights = vec![0usize; n];
    let mut fixed_sum = 0usize;
    let mut flex_indices = Vec::new();
    let mut total_weight = 0.0f64;

    for (i, sp) in specs.iter().enumerate() {
        if let Some(fh) = sp.fixed_height {
            let h = fh.max(3);
            heights[i] = h;
            fixed_sum += h;
        } else {
            flex_indices.push(i);
            total_weight += sp.weight.max(0.1);
        }
    }

    let rem = total_rows.saturating_sub(fixed_sum);
    if !flex_indices.is_empty() {
        let mut assigned = 0usize;
        for (pos, &idx) in flex_indices.iter().enumerate() {
            if pos == flex_indices.len() - 1 {
                heights[idx] = rem.saturating_sub(assigned).max(4);
            } else {
                let share =
                    ((rem as f64) * (specs[idx].weight.max(0.1) / total_weight)).round() as usize;
                let h = share.max(4);
                heights[idx] = h;
                assigned += h;
            }
        }
    }

    let sum_h: usize = heights.iter().sum();
    if sum_h != total_rows
        && let Some(&last_idx) = flex_indices.last().or(Some(&(n - 1)))
    {
        if total_rows >= sum_h {
            heights[last_idx] += total_rows - sum_h;
        } else {
            heights[last_idx] = heights[last_idx].saturating_sub(sum_h - total_rows).max(3);
        }
    }
    heights
}

/// Renders all stacked `PanelSpec`s into `out_buf` using row-level diffing against `prev_rows`
/// so ONLY rows whose content actually changed are emitted to `stdout`.
pub fn render_multi_panel_grid(
    geom: TermGeom,
    panels: &[PanelSpec],
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

        emit_row(
            top_r,
            format_top_border(geom.cols, &sp.title, &sp.badge, sp.accent),
        );

        if sp.raw_interior {
            for r in (top_r + 1)..bot_r {
                emit_row(
                    r,
                    format!("{GRAY}│{RESET}\x1b[{};{}H{GRAY}│{RESET}", r, geom.cols),
                );
            }
        } else {
            let empty_row = BoxRow {
                text: String::new(),
                bg: "",
                tag: None,
            };
            for i in 0..interior_cnt {
                let r = top_r + 1 + i;
                let brow = sp.rows.get(i).unwrap_or(&empty_row);
                emit_row(r, format_boxed_row(geom.cols, brow));
                if let Some(t) = brow.tag {
                    row_tag_map.insert(r, t);
                }
            }
        }

        if bot_r > top_r {
            emit_row(bot_r, format_bottom_border(geom.cols));
        }
        cur_row = bot_r + 1;
    }
}
