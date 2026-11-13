//! TUI Panel Layout & Live Powerline Statusbar Preview for `ccide-core setup`.

use crate::commands::setup::doctor::{CheckStatus, DoctorReport};
use crate::commands::setup::input::{TAG_APPLY_BTN, TAG_SETUP_ROW};
use crate::commands::setup::runner::{SetupOptions, PRESETS, SPRITE_MODES};
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::palette::Palette;
use crate::tui::width::trunc_to_width;
use crate::tui::{BOLD, RESET};

fn render_statusline_preview(pal: &Palette, max_w: usize) -> String {
    let sl = &pal.statusline;
    let (s0_bg, s0_fg, s0_sep) = (
        sl.bypass_bg.bg_ansi(),
        sl.bypass_fg.fg_ansi(),
        sl.bypass_bg.fg_ansi(),
    );
    let (s1_bg, s1_fg, s1_sep) = (
        sl.model_bg.bg_ansi(),
        sl.model_fg.fg_ansi(),
        sl.model_bg.fg_ansi(),
    );
    let (s2_bg, s2_fg, s2_sep) = (
        sl.ctx_ok_bg.bg_ansi(),
        sl.ctx_ok_fg.fg_ansi(),
        sl.ctx_ok_bg.fg_ansi(),
    );
    let (s3_bg, s3_fg, s3_sep) = (
        sl.git_bg.bg_ansi(),
        sl.git_fg.fg_ansi(),
        sl.git_bg.fg_ansi(),
    );
    let (s4_bg, s4_fg, s4_sep) = (
        sl.sys_bg.bg_ansi(),
        sl.sys_fg.fg_ansi(),
        sl.sys_bg.fg_ansi(),
    );

    if max_w >= 64 {
        format!(
            "   {s0_bg}{s0_fg} {BOLD}bypass \x1b[22m{s1_bg}{s0_sep}\u{e0b0}{s1_bg}{s1_fg} {BOLD}claude-fable-5-1 \x1b[22m{s2_bg}{s1_sep}\u{e0b0}{s2_bg}{s2_fg} CTX 238k/837k (28%) {s3_bg}{s2_sep}\u{e0b0}{s3_bg}{s3_fg} \u{e0a0} dev {s4_bg}{s3_sep}\u{e0b0}{s4_bg}{s4_fg} CPU 15% {RESET}{s4_sep}\u{e0b0}{RESET}"
        )
    } else {
        format!(
            "   {s0_bg}{s0_fg} {BOLD}bypass \x1b[22m{s1_bg}{s0_sep}\u{e0b0}{s1_bg}{s1_fg} {BOLD}fable \x1b[22m{s2_bg}{s1_sep}\u{e0b0}{s2_bg}{s2_fg} CTX 28% {RESET}{s2_sep}\u{e0b0}{RESET}"
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_setup_panels(
    cols: usize,
    rows_total: usize,
    report: &DoctorReport,
    opts: &SetupOptions,
    cursor: usize,
    logs: &[String],
    finished_msg: Option<&str>,
    pal: &Palette,
) -> Vec<PanelSpec> {
    let content_w = cols.saturating_sub(4).max(20);
    let (white, dim, cyan, green, amber, mint, lavender) = (
        &pal.text,
        &pal.dim,
        &pal.cyan,
        &pal.green,
        &pal.amber,
        &pal.mint,
        &pal.lavender,
    );

    let top_spec = PanelSpec {
        title: "CC-IDE v0.5.0 · SETUP & DOCTOR".to_string(),
        badge: report.os_arch.clone(),
        accent: cyan.clone(),
        rows: vec![
            BoxRow {
                text: trunc_to_width(
                    &format!(
                        " {mint}◆{RESET} {BOLD}{white}3-Column Claude Code Tiling IDE{RESET} {dim}· {}{RESET}",
                        report.backend_tier
                    ),
                    content_w,
                ),
                bg: String::new(),
                tag: None,
            },
            BoxRow {
                text: trunc_to_width(
                    &format!(
                        " {dim}[{amber}↑↓{dim}] Select  [{amber}Space/←→{dim}] Toggle  [{amber}1-6{dim}] Theme  [{amber}⏎/i{dim}] Install  [{amber}u{dim}] Uninstall  [{amber}q{dim}] Quit{RESET}"
                    ),
                    content_w,
                ),
                bg: String::new(),
                tag: None,
            },
        ],
        fixed_height: Some(4),
        weight: 0.2,
        raw_interior: false,
    };

    let mut doc_rows = Vec::with_capacity(report.items.len() + 1);
    for it in &report.items {
        let (ic, col) = match it.status {
            CheckStatus::Ok => ("✓", green),
            CheckStatus::Warn => ("!", amber),
            CheckStatus::Info => ("●", cyan),
        };
        doc_rows.push(BoxRow {
            text: trunc_to_width(
                &format!(
                    " {col}{ic}{RESET} {BOLD}{white}{:<13}{RESET} {dim}{}{RESET}",
                    it.label, it.detail
                ),
                content_w,
            ),
            bg: String::new(),
            tag: None,
        });
    }
    doc_rows.push(BoxRow {
        text: trunc_to_width(
            &format!(" {amber}↳ {}{RESET}", report.missing_hint),
            content_w,
        ),
        bg: String::new(),
        tag: None,
    });
    let doc_h = (doc_rows.len() + 2).min(rows_total.saturating_sub(15).max(6));
    let doc_spec = PanelSpec {
        title: "ENVIRONMENT DOCTOR".to_string(),
        badge: "[r] re-scan".to_string(),
        accent: mint.clone(),
        rows: doc_rows,
        fixed_height: Some(doc_h),
        weight: 0.35,
        raw_interior: false,
    };

    if !logs.is_empty() {
        let avail_log_h = rows_total.saturating_sub(4 + doc_h + 2).max(4);
        let skip_n = logs.len().saturating_sub(avail_log_h.saturating_sub(1));
        let mut lrows: Vec<BoxRow> = logs
            .iter()
            .skip(skip_n)
            .map(|ln| BoxRow {
                text: trunc_to_width(&format!(" {white}{ln}{RESET}"), content_w),
                bg: String::new(),
                tag: None,
            })
            .collect();
        if let Some(done_txt) = finished_msg {
            lrows.push(BoxRow {
                text: trunc_to_width(&format!(" {green}{BOLD}{done_txt}{RESET}"), content_w),
                bg: pal.active_view_bg.clone(),
                tag: Some((TAG_APPLY_BTN, 0)),
            });
        }
        return vec![
            top_spec,
            doc_spec,
            PanelSpec {
                title: "INSTALLATION PROGRESS".to_string(),
                badge: if finished_msg.is_some() {
                    "done · press ⏎ or q".to_string()
                } else {
                    "running...".to_string()
                },
                accent: green.clone(),
                rows: lrows,
                fixed_height: None,
                weight: 1.0,
                raw_interior: false,
            },
        ];
    }

    let preset_name = PRESETS[opts.preset_idx % PRESETS.len()];
    let sprite_name = SPRITE_MODES[opts.sprite_mode_idx % SPRITE_MODES.len()];
    let wrap_desc = if opts.wrap_lines {
        "vi ↪ Soft Wrap (wrap: true)"
    } else {
        "Unwrapped + 120Hz HScroll (Recommended)"
    };
    let mk_ptr = |idx: usize| -> (&str, String) {
        if cursor == idx {
            (&pal.cursor_bg, format!("{amber}❯{RESET}{}", pal.cursor_bg))
        } else {
            ("", " ".to_string())
        }
    };

    let mut opt_rows = Vec::with_capacity(11);
    let (bg0, ptr0) = mk_ptr(0);
    opt_rows.push(BoxRow {
        text: trunc_to_width(
            &format!(
                " {ptr0} {lavender}Theme Preset:{RESET}{bg0}   {amber}◀{RESET}{bg0} {BOLD}{white}{preset_name}{RESET}{bg0} {dim}({}/{}){RESET}{bg0} {amber}▶{RESET}",
                opts.preset_idx + 1,
                PRESETS.len()
            ),
            content_w,
        ),
        bg: bg0.to_string(),
        tag: Some((TAG_SETUP_ROW, 0)),
    });
    opt_rows.push(BoxRow {
        text: render_statusline_preview(pal, cols.saturating_sub(4)),
        bg: String::new(),
        tag: Some((TAG_SETUP_ROW, 0)),
    });

    let (bg1, ptr1) = mk_ptr(1);
    opt_rows.push(BoxRow {
        text: trunc_to_width(
            &format!(
                " {ptr1} {mint}Sprite Mode:{RESET}{bg1}    {amber}◀{RESET}{bg1} {BOLD}{white}{sprite_name}{RESET}{bg1} {dim}(PNG/JPG vs .ansi){RESET}{bg1} {amber}▶{RESET}"
            ),
            content_w,
        ),
        bg: bg1.to_string(),
        tag: Some((TAG_SETUP_ROW, 1)),
    });

    let (bg2, ptr2) = mk_ptr(2);
    opt_rows.push(BoxRow {
        text: trunc_to_width(
            &format!(
                " {ptr2} {cyan}Viewer Wrap:{RESET}{bg2}    {amber}◀{RESET}{bg2} {BOLD}{white}{wrap_desc}{RESET}{bg2} {amber}▶{RESET}"
            ),
            content_w,
        ),
        bg: bg2.to_string(),
        tag: Some((TAG_SETUP_ROW, 2)),
    });

    for (idx, enabled, title, sub) in [
        (
            3usize,
            opts.install_plugin,
            "Claude Plugin & Powerline Statusline",
            "~/.claude/plugins + statusLine",
        ),
        (
            4usize,
            opts.patch_claude,
            "Event-Driven claude.exe SIGUSR2 Patch",
            "instant <0.05ms subagent switch (.bak)",
        ),
        (
            5usize,
            opts.config_companions,
            "Yazi & Lazygit 120Hz Configs",
            "hscroll.yazi + compact diff (.ccide.bak)",
        ),
        (
            6usize,
            opts.setup_daemon_ssh,
            "iTerm2 Metal Daemon & SSH Multiplexer",
            "warm socket + ~/.ssh/ccide_config",
        ),
    ] {
        let (bg, ptr) = mk_ptr(idx);
        let box_ic = if enabled {
            format!("{green}[✓]{RESET}{bg}")
        } else {
            format!("{dim}[ ]{RESET}{bg}")
        };
        opt_rows.push(BoxRow {
            text: trunc_to_width(
                &format!(" {ptr} {box_ic} {BOLD}{white}{title}{RESET}{bg} {dim}— {sub}{RESET}"),
                content_w,
            ),
            bg: bg.to_string(),
            tag: Some((TAG_SETUP_ROW, idx)),
        });
    }

    let is_apply_cur = cursor == 7;
    let btn_bg: &str = if is_apply_cur {
        &pal.active_cursor_bg
    } else {
        &pal.active_view_bg
    };
    let btn_ptr = if is_apply_cur {
        format!("{amber}❯{RESET}{btn_bg}")
    } else {
        " ".to_string()
    };
    opt_rows.push(BoxRow {
        text: trunc_to_width(
            &format!(
                " {btn_ptr} {green}{BOLD}[ ▶ Apply Configuration, Install & Hot-Reload Workspace (⏎ / i) ]{RESET}{btn_bg}"
            ),
            content_w,
        ),
        bg: btn_bg.to_string(),
        tag: Some((TAG_APPLY_BTN, 7)),
    });

    vec![
        top_spec,
        doc_spec,
        PanelSpec {
            title: "CONFIGURATION & LIVE THEME STUDIO".to_string(),
            badge: format!("preset: {preset_name}"),
            accent: lavender.clone(),
            rows: opt_rows,
            fixed_height: None,
            weight: 1.0,
            raw_interior: false,
        },
    ]
}
