//! Middle Panel (`SUBAGENTS`) Renderer — Pinned `[main]` + Scrollable Subagent List.

use std::path::Path;

use crate::session::transcript::SubagentItem;
use crate::session::ConfigModels;
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::palette::Palette;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::{BOLD, RESET, SPINNERS};

pub struct SubagentPanelOutput {
    pub panel: PanelSpec,
    pub running_cnt: usize,
    pub viewport_h: usize,
}

#[allow(clippy::too_many_arguments)]
pub fn build_subagents_panel(
    cols: usize,
    mid_h: usize,
    target_cwd: &str,
    active_view_id: &str,
    cursor_idx: Option<usize>,
    sub_scroll_offset: &mut usize,
    main_tool: &str,
    main_file: &str,
    subagents: &[SubagentItem],
    cached_cfg: &ConfigModels,
    tick: usize,
    pal: &Palette,
) -> SubagentPanelOutput {
    let mid_interior = mid_h.saturating_sub(2);
    let inner_w = cols.saturating_sub(4).max(10);
    let box_inner_w = cols.saturating_sub(2).max(12);
    let white = &pal.text;
    let dim = &pal.dim;
    let cyan = &pal.cyan;
    let green = &pal.green;
    let amber = &pal.amber;

    let is_main_active = active_view_id == "main";
    let running_cnt = subagents.iter().filter(|a| a.status == "running").count();
    let done_cnt = subagents.len().saturating_sub(running_cnt);
    let mut sub_rows = Vec::with_capacity(mid_interior.max(4));

    let is_main_cursor = cursor_idx == Some(0);
    let main_bg: &str = match (is_main_active, is_main_cursor) {
        (true, true) => &pal.active_cursor_bg,
        (true, false) => &pal.active_view_bg,
        (false, true) => &pal.cursor_bg,
        (false, false) => "",
    };
    let main_ptr = if is_main_cursor {
        format!("{amber}❯{RESET}{main_bg}")
    } else if is_main_active {
        format!("{cyan}▸{RESET}{main_bg}")
    } else {
        " ".to_string()
    };
    let main_dot = if is_main_active {
        format!("{green}●{RESET}{main_bg}")
    } else {
        format!("{dim}○{RESET}{main_bg}")
    };
    let tool_lbl = if main_tool.is_empty() {
        "chat"
    } else {
        main_tool
    };
    let mut main_tail = if is_main_active {
        format!("{amber}{tool_lbl}{RESET}{main_bg}")
    } else {
        format!("{dim}{tool_lbl}{RESET}{main_bg}")
    };
    if !main_file.is_empty() {
        let bname = Path::new(main_file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        main_tail.push_str(&format!(
            " {cyan}{}{RESET}{main_bg}",
            trunc_to_width(bname, 15)
        ));
    }
    sub_rows.push(BoxRow {
        text: format!("{main_ptr}{main_dot} {BOLD}{white}[main]{RESET}{main_bg} {main_tail}"),
        bg: main_bg.to_string(),
        tag: Some((1, 0)),
    });

    let cwd_base = Path::new(target_cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("~");
    let main_sub_txt = if is_main_active {
        format!("{} · {}", cached_cfg.main_model_short, cwd_base)
    } else if is_main_cursor {
        format!("{} (Enter to switch)", cached_cfg.main_model_short)
    } else {
        format!("{} (click / ↑↓)", cached_cfg.main_model_short)
    };
    sub_rows.push(BoxRow {
        text: format!(
            "   {dim}{}",
            trunc_to_width(&main_sub_txt, inner_w.saturating_sub(1))
        ),
        bg: main_bg.to_string(),
        tag: Some((1, 0)),
    });

    let mut all_agent_rows = Vec::with_capacity(subagents.len() * 2);
    for (i_ag, ag) in subagents.iter().enumerate() {
        let is_viewed = ag.full_id == active_view_id;
        let is_cur = cursor_idx == Some(i_ag + 1);
        let row_bg: &str = match (is_viewed, is_cur) {
            (true, true) => &pal.active_cursor_bg,
            (true, false) => &pal.active_view_bg,
            (false, true) => &pal.cursor_bg,
            (false, false) => "",
        };
        let ptr = if is_cur {
            format!("{amber}❯{RESET}{row_bg}")
        } else if is_viewed {
            format!("{cyan}▸{RESET}{row_bg}")
        } else {
            " ".to_string()
        };
        let is_done = ag.status != "running";
        let (icon, mut tail_tag) = if !is_done {
            (
                format!("{cyan}{}{RESET}{row_bg}", SPINNERS[tick % SPINNERS.len()]),
                format!(
                    "{amber}{}{RESET}{row_bg}",
                    if ag.tool.is_empty() { "run" } else { &ag.tool }
                ),
            )
        } else {
            let ic = if is_viewed {
                format!("{green}●{RESET}{row_bg}")
            } else {
                format!("{green}✓{RESET}{row_bg}")
            };
            let tc: &str = if is_viewed || is_cur { amber } else { dim };
            (
                ic,
                format!(
                    "{tc}{}{RESET}{row_bg}",
                    if ag.tool.is_empty() { "done" } else { &ag.tool }
                ),
            )
        };

        let prefix_plain_w = 4
            + display_width(&ag.short_type)
            + 1
            + display_width(if ag.tool.is_empty() {
                if is_done {
                    "done"
                } else {
                    "run"
                }
            } else {
                &ag.tool
            });
        let right_reserve = if is_done { 3 } else { 1 };
        let max_file_w = box_inner_w.saturating_sub(prefix_plain_w + right_reserve + 1);

        if !ag.file.is_empty() && max_file_w >= 4 {
            let bname = Path::new(&ag.file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            tail_tag.push_str(&format!(
                " {cyan}{}{RESET}{row_bg}",
                trunc_to_width(bname, max_file_w.min(13))
            ));
        }

        let left_hdr = format!(
            "{ptr}{icon} {BOLD}{white}[{}]{RESET}{row_bg} {tail_tag}",
            ag.short_type
        );
        let hdr_text = if is_done {
            let lw = display_width(&left_hdr);
            if lw + 2 <= box_inner_w {
                let pad = box_inner_w - lw - 2;
                let x_col: &str = if is_cur { amber } else { dim };
                format!("{left_hdr}{}{x_col}×{RESET}{row_bg} ", " ".repeat(pad))
            } else {
                left_hdr
            }
        } else {
            left_hdr
        };

        all_agent_rows.push(BoxRow {
            text: hdr_text,
            bg: row_bg.to_string(),
            tag: Some((2, i_ag)),
        });
        let desc_line = format!("{} · {}", cached_cfg.sub_model_short, ag.desc);
        all_agent_rows.push(BoxRow {
            text: format!(
                "   {dim}{}",
                trunc_to_width(&desc_line, inner_w.saturating_sub(1))
            ),
            bg: row_bg.to_string(),
            tag: Some((5, i_ag)),
        });
    }

    let show_clear_bar = done_cnt > 0 && mid_interior >= 5;
    let reserved_rows = 2 + usize::from(show_clear_bar);
    let viewport_h = mid_interior.saturating_sub(reserved_rows);
    let max_scroll = all_agent_rows.len().saturating_sub(viewport_h);
    if *sub_scroll_offset > max_scroll {
        *sub_scroll_offset = max_scroll;
    }

    for r in all_agent_rows
        .into_iter()
        .skip(*sub_scroll_offset)
        .take(viewport_h)
    {
        sub_rows.push(r);
    }

    if show_clear_bar {
        while sub_rows.len() + 1 < mid_interior {
            sub_rows.push(BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            });
        }
        let clear_plain = format!(" ↑↓ ⏎ · x del · c clear {}", done_cnt);
        let clear_styled = if display_width(&clear_plain) <= box_inner_w {
            format!(
                " {dim}↑↓ ⏎ · [{amber}x{dim}] del · [{amber}c{dim}] clear {}{RESET}",
                done_cnt
            )
        } else {
            format!(" {dim}[{amber}c{dim}] × clear {} done{RESET}", done_cnt)
        };
        sub_rows.push(BoxRow {
            text: if display_width(&clear_styled) <= box_inner_w {
                clear_styled
            } else {
                trunc_to_width(&clear_plain, inner_w)
            },
            bg: String::new(),
            tag: Some((3, 0)),
        });
    }

    let sub_badge = if max_scroll > 0 {
        let first_item = (*sub_scroll_offset / 2) + 1;
        let vis_items = (viewport_h / 2).max(1);
        let last_item = (first_item + vis_items - 1).min(subagents.len());
        if running_cnt > 0 {
            format!(
                "{}run · ↕{}-{}/{}",
                running_cnt,
                first_item,
                last_item,
                subagents.len()
            )
        } else {
            format!("↕ {}-{}/{}", first_item, last_item, subagents.len())
        }
    } else if running_cnt == 0 {
        format!("main+{}", subagents.len())
    } else {
        format!("{} run/{}", running_cnt, subagents.len())
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
        },
        running_cnt,
        viewport_h,
    }
}
