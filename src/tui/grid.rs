//! Multi-Panel Grid Layout & Closed 4-Sided Box Renderer.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::ffi::query_terminal_winsize;
use crate::tui::palette::Palette;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::{BOLD, RESET};

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct TermGeom {
    pub rows: usize,
    pub cols: usize,
    pub cell_w_px: f64,
    pub cell_h_px: f64,
}

/// Queries live terminal rows, columns, and pixel geometry (`TIOCGWINSZ` on Unix, `GetConsoleScreenBufferInfo` on Windows).
pub fn get_terminal_geometry() -> TermGeom {
    if let Some((rows, cols, cw, ch)) = query_terminal_winsize() {
        return TermGeom {
            rows,
            cols,
            cell_w_px: cw,
            cell_h_px: ch,
        };
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
    pub bg: String,
    pub tag: Option<(u8, usize)>,
}

pub struct PanelSpec {
    pub title: String,
    pub badge: String,
    pub accent: String,
    pub rows: Vec<BoxRow>,
    pub fixed_height: Option<usize>,
    pub weight: f64,
    pub raw_interior: bool,
    pub focused: bool,
}

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
    let t_clean = format!(" {} ", title.trim());
    let t_w = display_width(&t_clean);
    let inner = cols.saturating_sub(2);
    let max_badge_inner = inner.saturating_sub(t_w + 5);
    let b_clean = if badge.trim().is_empty() || max_badge_inner < 4 {
        String::new()
    } else {
        let fit = crate::session::compact_model_for_width(badge.trim(), max_badge_inner);
        format!(" {} ", fit)
    };
    let b_w = display_width(&b_clean);
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
    if sum_h != total_rows {
        if let Some(&last_idx) = flex_indices.last().or(Some(&(n - 1))) {
            if total_rows >= sum_h {
                heights[last_idx] += total_rows - sum_h;
            } else {
                heights[last_idx] = heights[last_idx].saturating_sub(sum_h - total_rows).max(3);
            }
        }
    }
    heights
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(fixed_height: Option<usize>, weight: f64) -> PanelSpec {
        PanelSpec {
            title: "T".to_string(),
            badge: String::new(),
            accent: String::new(),
            rows: Vec::new(),
            fixed_height,
            weight,
            raw_interior: false,
            focused: false,
        }
    }

    /// Smallest `total_rows` for which the exact-sum invariant is guaranteed.
    ///
    /// Every flex panel claims at least 4 rows and every fixed panel at least
    /// 3, so below this floor the layout clamps to minimum box heights and may
    /// legitimately exceed `total_rows`. Real terminals are always far larger.
    const EXACT_SUM_FLOOR: usize = 32;

    #[test]
    fn zero_panels_yield_no_heights() {
        assert!(allocate_panel_heights(40, &[]).is_empty());
    }

    #[test]
    fn single_flex_panel_takes_all_rows() {
        let heights = allocate_panel_heights(40, &[spec(None, 1.0)]);
        assert_eq!(heights, vec![40]);
        assert_eq!(heights.iter().sum::<usize>(), 40);
    }

    #[test]
    fn exact_row_sum_for_flex_only_split() {
        let specs = [spec(None, 1.0), spec(None, 1.0), spec(None, 1.0)];
        let heights = allocate_panel_heights(60, &specs);
        assert_eq!(heights.iter().sum::<usize>(), 60);
        assert_eq!(heights.len(), 3);
    }

    #[test]
    fn exact_row_sum_with_fixed_and_flex_mix() {
        let specs = [
            spec(Some(3), 1.0),
            spec(None, 2.0),
            spec(Some(10), 1.0),
            spec(None, 3.0),
            spec(None, 1.0),
        ];
        for total in [EXACT_SUM_FLOOR, 33, 47, 65, 80, 120] {
            let heights = allocate_panel_heights(total, &specs);
            assert_eq!(
                heights.iter().sum::<usize>(),
                total,
                "total={total} heights={heights:?}"
            );
            assert_eq!(heights.len(), specs.len());
        }
    }

    #[test]
    fn exact_row_sum_across_many_totals_and_shapes() {
        let shapes: Vec<Vec<PanelSpec>> = vec![
            vec![spec(None, 1.0), spec(None, 1.0)],
            vec![spec(None, 1.0), spec(None, 2.0), spec(None, 5.0)],
            vec![spec(Some(5), 1.0), spec(None, 1.0)],
            vec![spec(Some(7), 1.0), spec(Some(7), 1.0), spec(None, 1.0)],
            vec![
                spec(None, 0.1),
                spec(None, 0.1),
                spec(None, 0.1),
                spec(None, 0.1),
            ],
        ];
        for specs in &shapes {
            for total in EXACT_SUM_FLOOR..=200 {
                let heights = allocate_panel_heights(total, specs);
                assert_eq!(
                    heights.iter().sum::<usize>(),
                    total,
                    "total={total} heights={heights:?}"
                );
            }
        }
    }

    #[test]
    fn heights_are_clamped_below_minimum_rows() {
        // Below the aggregate floor the result may exceed `total_rows`, but every
        // panel still gets at least its minimum box height.
        for total in 1usize..EXACT_SUM_FLOOR {
            let specs = [spec(None, 1.0), spec(None, 1.0)];
            let heights = allocate_panel_heights(total, &specs);
            assert!(heights.iter().all(|&h| h >= 3), "total={total} {heights:?}");
        }
    }

    #[test]
    fn fixed_heights_are_clamped_to_minimum_three() {
        let specs = [spec(Some(0), 1.0), spec(Some(1), 1.0), spec(None, 1.0)];
        let heights = allocate_panel_heights(40, &specs);
        assert!(heights[0] >= 3);
        assert!(heights[1] >= 3);
        assert_eq!(heights.iter().sum::<usize>(), 40);
    }

    #[test]
    fn weights_influence_distribution_when_rows_allow() {
        let specs = [spec(None, 1.0), spec(None, 3.0)];
        let heights = allocate_panel_heights(100, &specs);
        assert_eq!(heights.iter().sum::<usize>(), 100);
        assert!(
            heights[1] > heights[0],
            "expected weighted split, got {heights:?}"
        );
    }

    #[test]
    fn zero_or_negative_weights_are_treated_as_minimum() {
        let specs = [spec(None, 0.0), spec(None, -5.0), spec(None, 1.0)];
        let heights = allocate_panel_heights(60, &specs);
        assert_eq!(heights.iter().sum::<usize>(), 60);
        assert!(heights[0] >= 4);
        assert!(heights[1] >= 4);
    }

    #[test]
    fn tiny_total_above_floor_still_sums_exactly() {
        for total in 8usize..=EXACT_SUM_FLOOR {
            let specs = [spec(None, 1.0), spec(None, 1.0)];
            let heights = allocate_panel_heights(total, &specs);
            assert_eq!(
                heights.iter().sum::<usize>(),
                total,
                "total={total} heights={heights:?}"
            );
        }
    }

    #[test]
    fn fixed_only_specs_sum_to_total_when_fitting() {
        let specs = [spec(Some(10), 1.0), spec(Some(20), 1.0)];
        let heights = allocate_panel_heights(40, &specs);
        assert_eq!(heights.iter().sum::<usize>(), 40);
        assert_eq!(heights[0], 10);
        assert_eq!(heights[1], 30);
    }
}
