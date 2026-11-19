//! Row Builders for the `SUBAGENTS` Middle Panel (`src/commands/sidebar/subagents_rows.rs`).

use std::path::Path;

use crate::session::transcript::SubagentItem;
use crate::session::ConfigModels;
use crate::tui::grid::BoxRow;
use crate::tui::layout::RowLayout;
use crate::tui::palette::Palette;
use crate::tui::{BOLD, RESET, SPINNERS};

pub fn resolve_row_bg(active: bool, cursor: bool, pal: &Palette) -> &str {
    match (active, cursor) {
        (true, true) => &pal.active_cursor_bg,
        (true, false) => &pal.active_view_bg,
        (false, true) => &pal.cursor_bg,
        (false, false) => "",
    }
}

pub fn format_rails(active: bool, cursor: bool, pal: &Palette) -> (String, String) {
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
pub fn push_main_rows(
    sub_rows: &mut Vec<BoxRow>,
    row_fmt: &RowLayout,
    agent_badge: &str,
    target_cwd: &str,
    is_main_active: bool,
    is_main_cursor: bool,
    main_tool: &str,
    main_file: &str,
    cached_cfg: &ConfigModels,
    pal: &Palette,
) {
    let (white, dim, cyan, green, amber) = (&pal.text, &pal.dim, &pal.cyan, &pal.green, &pal.amber);
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
        "{main_r1}{main_dot} {BOLD}{white}[{agent_badge}]{RESET} {}{tool_lbl}{RESET}",
        if is_main_active { amber } else { dim }
    );
    if let Some(bname) = Path::new(main_file)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
    {
        main_left.push_str(&format!(" {cyan}{bname}{RESET}"));
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
}

#[allow(clippy::too_many_arguments)]
pub fn build_agent_item_rows(
    row_fmt: &RowLayout,
    active_view_id: &str,
    cursor_idx: Option<usize>,
    subagents: &[SubagentItem],
    cached_cfg: &ConfigModels,
    tick: usize,
    pal: &Palette,
) -> Vec<BoxRow> {
    let (white, dim, cyan, green, amber) = (&pal.text, &pal.dim, &pal.cyan, &pal.green, &pal.amber);
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
            (
                ic,
                if is_viewed || is_cur {
                    amber.as_str()
                } else {
                    dim.as_str()
                },
            )
        };

        let mut left_hdr = format!(
            "{r1_rail}{icon} {BOLD}{white}[{}]{RESET} {tag_col}{eff_tag}{RESET}",
            ag.short_type
        );
        if let Some(bname) = Path::new(&ag.file)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
        {
            left_hdr.push_str(&format!(" {cyan}{bname}{RESET}"));
        }
        let right_close = if is_done {
            format!("{}×{RESET}", if is_cur { amber } else { dim })
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
    all_agent_rows
}
