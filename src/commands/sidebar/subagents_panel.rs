//! Middle Panel (`SUBAGENTS`) Renderer — Pinned `[main]` + Scrollable Subagent List.
//!
//! Renders full-bleed 2-row selection rectangles (`active_view_bg` / `cursor_bg` / `active_cursor_bg`)
//! with a 2-row left vertical rail (`▎` active / `┃` cursor) and inline status/action badges.

use std::path::Path;

use crate::session::transcript::SubagentItem;
use crate::session::ConfigModels;
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::layout::RowLayout;
use crate::tui::palette::Palette;
use crate::tui::width::display_width;
use crate::tui::{BOLD, RESET, SPINNERS};

pub struct SubagentPanelOutput {
    pub panel: PanelSpec,
    pub running_cnt: usize,
    pub viewport_h: usize,
}

fn resolve_row_bg(active: bool, cursor: bool, pal: &Palette) -> &str {
    match (active, cursor) {
        (true, true) => &pal.active_cursor_bg,
        (true, false) => &pal.active_view_bg,
        (false, true) => &pal.cursor_bg,
        (false, false) => "",
    }
}

fn format_rails(active: bool, cursor: bool, pal: &Palette) -> (String, String) {
    if cursor {
        (
            format!("{}┃❯{RESET}", pal.amber),
            format!("{}┃{RESET}  ", pal.amber),
        )
    } else if active {
        (
            format!("{}▎▸{RESET}", pal.cyan),
            format!("{}▎{RESET}  ", pal.cyan),
        )
    } else {
        ("  ".to_string(), "   ".to_string())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_subagents_panel(
    cols: usize,
    mid_h: usize,
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
    tick: usize,
    pal: &Palette,
) -> SubagentPanelOutput {
    let mid_interior = mid_h.saturating_sub(2);
    let box_inner_w = cols.saturating_sub(2).max(12);
    let row_fmt = RowLayout::new(box_inner_w).padding(0, 1);
    let (white, dim, cyan, green, amber) = (&pal.text, &pal.dim, &pal.cyan, &pal.green, &pal.amber);

    let is_main_active = active_view_id == "main";
    let running_cnt = subagents.iter().filter(|a| a.status == "running").count();
    let done_cnt = subagents.len().saturating_sub(running_cnt);
    let mut sub_rows = Vec::with_capacity(mid_interior.max(4));

    let is_main_cursor = cursor_idx == Some(0);
    let main_bg = resolve_row_bg(is_main_active, is_main_cursor, pal);
    let (main_r1, main_r2) = format_rails(is_main_active, is_main_cursor, pal);
    let main_dot = if is_main_active {
        format!("{green}●{RESET}")
    } else {
        format!("{dim}○{RESET}")
    };
    let tool_lbl = if main_tool.is_empty() {
        "chat"
    } else {
        main_tool
    };
    let mut main_left = format!(
        "{main_r1}{main_dot} {BOLD}{white}[main]{RESET} {}{tool_lbl}{RESET}",
        if is_main_active { amber } else { dim }
    );
    if !main_file.is_empty() {
        if let Some(bname) = Path::new(main_file).file_name().and_then(|n| n.to_str()) {
            if !bname.is_empty() {
                main_left.push_str(&format!(" {cyan}{bname}{RESET}"));
            }
        }
    }
    let main_r1_right = if is_main_cursor && !is_main_active {
        format!("{amber}⏎{RESET}")
    } else if is_main_active {
        format!("{green}●{RESET}")
    } else {
        String::new()
    };
    sub_rows.push(BoxRow {
        text: row_fmt.split_with_bg(&main_left, &main_r1_right, 1, main_bg),
        bg: main_bg.to_string(),
        tag: Some((1, 0)),
    });

    let cwd_base = Path::new(target_cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("~");
    let main_sub_left = format!(
        "{main_r2}{dim}{} · {cwd_base}{RESET}",
        cached_cfg.main_model_short
    );
    let main_sub_right = if is_main_cursor && !is_main_active {
        format!("{amber}⏎ switch{RESET}")
    } else if is_main_active {
        format!("{cyan}active{RESET}")
    } else {
        format!("{dim}click/↑↓{RESET}")
    };
    sub_rows.push(BoxRow {
        text: row_fmt.split_with_bg(&main_sub_left, &main_sub_right, 1, main_bg),
        bg: main_bg.to_string(),
        tag: Some((1, 0)),
    });

    let mut all_agent_rows = Vec::with_capacity(subagents.len() * 2);
    for (i_ag, ag) in subagents.iter().enumerate() {
        let is_viewed = ag.full_id == active_view_id;
        let is_cur = cursor_idx == Some(i_ag + 1);
        let row_bg = resolve_row_bg(is_viewed, is_cur, pal);
        let (r1_rail, r2_rail) = format_rails(is_viewed, is_cur, pal);
        let is_done = ag.status != "running";
        let fallback_tag = if !is_done {
            "run"
        } else if ag.status == "stopped" {
            "stop"
        } else {
            "done"
        };
        let eff_tag = if ag.tool.is_empty() {
            fallback_tag
        } else {
            ag.tool.as_str()
        };
        let (icon, tag_col) = if !is_done {
            (
                format!("{cyan}{}{RESET}", SPINNERS[tick % SPINNERS.len()]),
                amber.as_str(),
            )
        } else {
            let ic = if ag.status == "stopped" {
                format!("{amber}■{RESET}")
            } else if is_viewed {
                format!("{green}●{RESET}")
            } else {
                format!("{green}✓{RESET}")
            };
            let tc = if is_viewed || is_cur {
                amber.as_str()
            } else {
                dim.as_str()
            };
            (ic, tc)
        };

        let mut left_hdr = format!(
            "{r1_rail}{icon} {BOLD}{white}[{}]{RESET} {tag_col}{eff_tag}{RESET}",
            ag.short_type
        );
        if !ag.file.is_empty() {
            if let Some(bname) = Path::new(&ag.file).file_name().and_then(|n| n.to_str()) {
                left_hdr.push_str(&format!(" {cyan}{bname}{RESET}"));
            }
        }
        let right_close = if is_done {
            let x_col = if is_cur { amber } else { dim };
            format!("{x_col}×{RESET}")
        } else if is_viewed {
            format!("{cyan}●{RESET}")
        } else {
            String::new()
        };

        all_agent_rows.push(BoxRow {
            text: row_fmt.split_with_bg(&left_hdr, &right_close, 1, row_bg),
            bg: row_bg.to_string(),
            tag: Some((2, i_ag)),
        });

        let sub_mod_lbl = ag
            .model_short
            .as_deref()
            .unwrap_or(&cached_cfg.sub_model_short);
        let desc_line = match &ag.effort {
            Some(eff) => format!("{sub_mod_lbl}·{eff} · {}", ag.desc),
            None => format!("{sub_mod_lbl} · {}", ag.desc),
        };
        let r2_right = if is_cur && !is_viewed {
            format!("{amber}⏎ view{RESET}")
        } else if is_viewed {
            format!("{cyan}viewing{RESET}")
        } else {
            String::new()
        };
        all_agent_rows.push(BoxRow {
            text: row_fmt.split_with_bg(
                &format!("{r2_rail}{dim}{desc_line}{RESET}"),
                &r2_right,
                1,
                row_bg,
            ),
            bg: row_bg.to_string(),
            tag: Some((5, i_ag)),
        });
    }

    let show_clear_bar = done_cnt > 0 && mid_interior >= 5;
    let viewport_h = mid_interior.saturating_sub(2 + usize::from(show_clear_bar));
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
        format!("main+{}", subagents.len())
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
