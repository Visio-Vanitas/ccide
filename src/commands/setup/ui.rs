//! TUI Panel Layout & Live Powerline Statusbar Preview for `ccide-core setup`.

use crate::commands::setup::doctor::{CheckStatus, DoctorReport};
use crate::commands::setup::input::{TAG_APPLY_BTN, TAG_SETUP_ROW};
use crate::commands::setup::runner::{SetupOptions, PRESETS, SPRITE_MODES};
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::palette::Palette;
use crate::tui::width::trunc_to_width;
use crate::tui::{BOLD, RESET};

fn trow(s: &str, w: usize, bg: &str, tag: Option<(u8, usize)>) -> BoxRow {
    BoxRow {
        text: trunc_to_width(s, w),
        bg: bg.to_string(),
        tag,
    }
}

fn panel(
    title: &str,
    badge: String,
    accent: String,
    rows: Vec<BoxRow>,
    fixed_height: Option<usize>,
    weight: f64,
    focused: bool,
) -> PanelSpec {
    PanelSpec {
        title: title.to_string(),
        badge,
        accent,
        rows,
        fixed_height,
        weight,
        raw_interior: false,
        focused,
    }
}

fn render_statusline_preview(pal: &Palette, max_w: usize) -> String {
    let sl = &pal.statusline;
    let seg = |bg: crate::tui::palette::ColorSpec, fg: crate::tui::palette::ColorSpec| {
        (bg.bg_ansi(), fg.fg_ansi(), bg.fg_ansi())
    };
    let (s0b, s0f, s0s) = seg(sl.bypass_bg, sl.bypass_fg);
    let (s1b, s1f, s1s) = seg(sl.model_bg, sl.model_fg);
    let (s2b, s2f, s2s) = seg(sl.ctx_ok_bg, sl.ctx_ok_fg);
    let (s3b, s3f, s3s) = seg(sl.git_bg, sl.git_fg);
    let (s4b, s4f, s4s) = seg(sl.sys_bg, sl.sys_fg);
    let line = if max_w >= 64 {
        format!("   {s0b}{s0f} {BOLD}bypass \x1b[22m{s1b}{s0s}\u{e0b0}{s1b}{s1f} {BOLD}claude-fable-5-1 \x1b[22m{s2b}{s1s}\u{e0b0}{s2b}{s2f} CTX 238k/837k (28%) {s3b}{s2s}\u{e0b0}{s3b}{s3f} \u{e0a0} dev {s4b}{s3s}\u{e0b0}{s4b}{s4f} CPU 15% {RESET}{s4s}\u{e0b0}{RESET}")
    } else {
        format!("   {s0b}{s0f} {BOLD}bypass \x1b[22m{s1b}{s0s}\u{e0b0}{s1b}{s1f} {BOLD}fable \x1b[22m{s2b}{s1s}\u{e0b0}{s2b}{s2f} CTX 28% {RESET}{s2s}\u{e0b0}{RESET}")
    };
    trunc_to_width(&line, max_w)
}

fn render_syntax_preview(pal: &Palette, max_w: usize) -> String {
    let s = &pal.syntax;
    let (kw, fn_c, ty, op) = (
        s.keyword.fg_ansi(),
        s.function.fg_ansi(),
        s.type_name.fg_ansi(),
        s.operator.fg_ansi(),
    );
    let (st, nm, cm, vr) = (
        s.string.fg_ansi(),
        s.number.fg_ansi(),
        s.comment.fg_ansi(),
        s.variable.fg_ansi(),
    );
    let bg = s.code_bg.bg_ansi();
    let line = format!("   {bg}{kw}pub fn{RESET}{bg} {fn_c}route{RESET}{bg}({vr}cfg{RESET}{bg}{op}:{RESET}{bg} {op}&{RESET}{bg}{ty}Config{RESET}{bg}) {op}->{RESET}{bg} {ty}usize{RESET}{bg} {{ {st}\"ok\"{RESET}{bg}; {nm}42{RESET}{bg} }} {cm}// syntax{RESET}");
    trunc_to_width(&line, max_w)
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
    let w = cols.saturating_sub(4).max(20);
    let (white, dim, cyan, green, amber, mint, lavender) = (
        &pal.text,
        &pal.dim,
        &pal.cyan,
        &pal.green,
        &pal.amber,
        &pal.mint,
        &pal.lavender,
    );
    let top_spec = panel(
        "CC-IDE v0.9.0 · SETUP & DOCTOR", report.os_arch.clone(), cyan.clone(),
        vec![
            trow(&format!(" {mint}◆{RESET} {BOLD}{white}3-Column Claude Code Tiling IDE{RESET} {dim}· {}{RESET}", report.backend_tier), w, "", None),
            trow(&format!(" {dim}[{amber}↑↓{dim}] Select  [{amber}Space/←→{dim}] Toggle  [{amber}1-6{dim}] Theme  [{amber}⏎/i{dim}] Install  [{amber}u{dim}] Uninstall  [{amber}q{dim}] Quit{RESET}"), w, "", None),
        ],
        Some(4), 0.2, false,
    );

    let mut doc_rows = Vec::with_capacity(report.items.len() + 1);
    for it in &report.items {
        let (ic, col) = match it.status {
            CheckStatus::Ok => ("✓", green),
            CheckStatus::Warn => ("!", amber),
            CheckStatus::Info => ("●", cyan),
        };
        doc_rows.push(trow(
            &format!(
                " {col}{ic}{RESET} {BOLD}{white}{:<13}{RESET} {dim}{}{RESET}",
                it.label, it.detail
            ),
            w,
            "",
            None,
        ));
    }
    doc_rows.push(trow(
        &format!(" {amber}↳ {}{RESET}", report.missing_hint),
        w,
        "",
        None,
    ));
    let doc_h = (doc_rows.len() + 2).min(rows_total.saturating_sub(15).max(6));
    let doc_spec = panel(
        "ENVIRONMENT DOCTOR",
        "[r] re-scan".to_string(),
        mint.clone(),
        doc_rows,
        Some(doc_h),
        0.35,
        false,
    );

    if !logs.is_empty() {
        let avail_log_h = rows_total.saturating_sub(4 + doc_h + 2).max(4);
        let skip_n = logs.len().saturating_sub(avail_log_h.saturating_sub(1));
        let mut lrows: Vec<BoxRow> = logs
            .iter()
            .skip(skip_n)
            .map(|ln| trow(&format!(" {white}{ln}{RESET}"), w, "", None))
            .collect();
        if let Some(done_txt) = finished_msg {
            lrows.push(trow(
                &format!(" {green}{BOLD}{done_txt}{RESET}"),
                w,
                &pal.active_view_bg,
                Some((TAG_APPLY_BTN, 0)),
            ));
        }
        let badge = if finished_msg.is_some() {
            "done · press ⏎ or q"
        } else {
            "running..."
        }
        .to_string();
        return vec![
            top_spec,
            doc_spec,
            panel(
                "INSTALLATION PROGRESS",
                badge,
                green.clone(),
                lrows,
                None,
                1.0,
                true,
            ),
        ];
    }

    let preset_name = PRESETS[opts.preset_idx % PRESETS.len()];
    let sprite_name = SPRITE_MODES[opts.sprite_mode_idx % SPRITE_MODES.len()];
    let wrap_desc = if opts.wrap_lines {
        "vi ↪ Soft Wrap (wrap: true)"
    } else {
        "Unwrapped + 120Hz HScroll (Recommended)"
    };
    let cost_desc = match opts.cost_lines {
        1 => "1 Line · Compact (USD + % Only)",
        2 => "2 Lines · Brief (USD + Tokens)",
        _ => "3 Lines · Detailed (USD + Tokens + TPS/TTFT)",
    };
    let mk_ptr = |idx: usize| -> (&str, String) {
        if cursor == idx {
            (&pal.cursor_bg, format!("{amber}❯{RESET}{}", pal.cursor_bg))
        } else {
            ("", " ".to_string())
        }
    };

    let mut opt_rows = Vec::with_capacity(12);
    let (bg0, ptr0) = mk_ptr(0);
    opt_rows.push(trow(&format!(" {ptr0} {lavender}Theme Preset:{RESET}{bg0}   {amber}◀{RESET}{bg0} {BOLD}{white}{preset_name}{RESET}{bg0} {dim}({}/{}){RESET}{bg0} {amber}▶{RESET}", opts.preset_idx + 1, PRESETS.len()), w, bg0, Some((TAG_SETUP_ROW, 0))));
    opt_rows.push(BoxRow {
        text: render_statusline_preview(pal, w),
        bg: String::new(),
        tag: Some((TAG_SETUP_ROW, 0)),
    });
    opt_rows.push(BoxRow {
        text: render_syntax_preview(pal, w),
        bg: String::new(),
        tag: Some((TAG_SETUP_ROW, 0)),
    });

    for (idx, lbl, col, val) in [
        (
            1usize,
            "Sprite Mode: ",
            mint,
            format!("{sprite_name} {dim}(PNG/JPG vs .ansi){RESET}"),
        ),
        (2usize, "Viewer Wrap: ", cyan, wrap_desc.to_string()),
        (3usize, "COST Density:", amber, cost_desc.to_string()),
    ] {
        let (bg, ptr) = mk_ptr(idx);
        opt_rows.push(trow(&format!(" {ptr} {col}{lbl}{RESET}{bg}   {amber}◀{RESET}{bg} {BOLD}{white}{val}{RESET}{bg} {amber}▶{RESET}"), w, bg, Some((TAG_SETUP_ROW, idx))));
    }

    for (idx, enabled, title, sub) in [
        (
            4usize,
            opts.install_plugin,
            "Claude Plugin & Powerline Statusline",
            "~/.claude/plugins + statusLine",
        ),
        (
            5usize,
            opts.patch_claude,
            "Event-Driven claude.exe SIGUSR2 Patch",
            "instant <0.05ms subagent switch (.bak)",
        ),
        (
            6usize,
            opts.config_companions,
            "Yazi & Lazygit 120Hz Configs",
            "hscroll.yazi + compact diff (.ccide.bak)",
        ),
        (
            7usize,
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
        opt_rows.push(trow(
            &format!(" {ptr} {box_ic} {BOLD}{white}{title}{RESET}{bg} {dim}— {sub}{RESET}"),
            w,
            bg,
            Some((TAG_SETUP_ROW, idx)),
        ));
    }

    let is_apply_cur = cursor == 8;
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
    opt_rows.push(trow(&format!(" {btn_ptr} {green}{BOLD}[ ▶ Apply Configuration, Install & Hot-Reload Workspace (⏎ / i) ]{RESET}{btn_bg}"), w, btn_bg, Some((TAG_APPLY_BTN, 8))));

    vec![
        top_spec,
        doc_spec,
        panel(
            "CONFIGURATION & LIVE THEME STUDIO",
            format!("preset: {preset_name}"),
            lavender.clone(),
            opt_rows,
            None,
            1.0,
            true,
        ),
    ]
}
