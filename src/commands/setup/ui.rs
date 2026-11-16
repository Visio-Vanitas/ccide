//! TUI Panel Layout & Live Powerline Statusbar Preview for `ccide-core setup`.

use crate::commands::setup::doctor::{CheckStatus, DoctorReport};
use crate::commands::setup::input::{TAG_APPLY_BTN, TAG_SETUP_ROW, TOTAL_SETUP_ROWS};
use crate::commands::setup::runner::{SetupOptions, PRESETS, SPRITE_MODES};
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::palette::{ColorSpec, Palette};
use crate::tui::width::trunc_to_width;
use crate::tui::{BOLD, RESET};

fn trow(s: &str, w: usize, bg: &str, tag: Option<(u8, usize)>) -> BoxRow {
    BoxRow {
        text: trunc_to_width(s, w),
        bg: bg.to_string(),
        tag,
    }
}

/// Builds a panel; `None` height selects the flexible, focused (weight `1.0`) pane.
fn panel(t: &str, b: String, a: &str, r: Vec<BoxRow>, h: Option<usize>) -> PanelSpec {
    PanelSpec {
        title: t.to_string(),
        badge: b,
        accent: a.to_string(),
        rows: r,
        fixed_height: h,
        weight: 1.0,
        raw_interior: false,
        focused: h.is_none(),
    }
}

fn render_statusline_preview(pal: &Palette, max_w: usize) -> String {
    let sl = &pal.statusline;
    let seg = |bg: ColorSpec, fg: ColorSpec| (bg.bg_ansi(), fg.fg_ansi(), bg.fg_ansi());
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
    let f = |c: ColorSpec| c.fg_ansi();
    let (kw, fn_c, ty, op) = (f(s.keyword), f(s.function), f(s.type_name), f(s.operator));
    let (st, nm, cm, vr) = (f(s.string), f(s.number), f(s.comment), f(s.variable));
    let bg = s.code_bg.bg_ansi();
    let line = format!("   {bg}{kw}pub fn{RESET}{bg} {fn_c}route{RESET}{bg}({vr}cfg{RESET}{bg}{op}:{RESET}{bg} {op}&{RESET}{bg}{ty}Config{RESET}{bg}) {op}->{RESET}{bg} {ty}usize{RESET}{bg} {{ {st}\"ok\"{RESET}{bg}; {nm}42{RESET}{bg} }} {cm}// syntax{RESET}");
    trunc_to_width(&line, max_w)
}

/// Toggle-row titles; the cursor index is `4 + position`.
const TOG_T: [&str; 4] = [
    "Claude Plugin & Powerline Statusline",
    "claude.exe SIGUSR2 Patch",
    "Yazi & Lazygit 120Hz Configs",
    "iTerm2 Daemon & SSH Multiplexer",
];

/// Toggle-row subtitles, parallel to [`TOG_T`].
const TOG_S: [&str; 4] = [
    "~/.claude/plugins + statusLine",
    "instant <0.05ms subagent switch (.bak)",
    "hscroll.yazi + compact diff (.ccide.bak)",
    "warm socket + ~/.ssh/ccide_config",
];

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
    // Compact key hints below 86 columns so `[q] Quit` still fits an 80-column terminal.
    let keys = if cols < 86 {
        format!(" {dim}[{amber}↑↓{dim}] Move [{amber}␣{dim}] Toggle [{amber}1-6{dim}] Theme [{amber}i{dim}] Install [{amber}u{dim}] Uninstall [{amber}q{dim}] Quit{RESET}")
    } else {
        format!(" {dim}[{amber}↑↓{dim}] Select [{amber}Space/←→{dim}] Toggle [{amber}1-6{dim}] Theme [{amber}⏎/i{dim}] Install [{amber}u{dim}] Uninstall [{amber}q{dim}] Quit{RESET}")
    };
    let be = report.backend_tier.as_str();
    let ti = format!(
        " {mint}◆{RESET} {BOLD}{white}3-Column Claude Code Tiling IDE{RESET} {dim}· {be}{RESET}"
    );
    let top = panel(
        concat!("CC-IDE v", env!("CARGO_PKG_VERSION"), " · SETUP & DOCTOR"),
        report.os_arch.clone(),
        cyan,
        vec![trow(&ti, w, "", None), trow(&keys, w, "", None)],
        Some(4),
    );

    let mut doc_rows = Vec::with_capacity(report.items.len() + 1);
    for it in &report.items {
        let (ic, col) = match it.status {
            CheckStatus::Ok => ("✓", green),
            CheckStatus::Warn => ("!", amber),
            CheckStatus::Info => ("●", cyan),
        };
        let (lbl, det) = (it.label.as_str(), it.detail.as_str());
        doc_rows.push(trow(
            &format!(" {col}{ic}{RESET} {BOLD}{white}{lbl:<13}{RESET} {dim}{det}{RESET}"),
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
    let doc = panel(
        "ENVIRONMENT DOCTOR",
        "[r] re-scan".into(),
        mint,
        doc_rows,
        Some(doc_h),
    );

    // Interior rows left after the header and doctor box (plus borders). The live theme
    // previews are dropped below this height so all 9 selectable rows still fit.
    let view_h = rows_total.saturating_sub(4 + doc_h + 2).max(1);
    if !logs.is_empty() {
        let skip_n = logs.len().saturating_sub(view_h.max(4).saturating_sub(1));
        let mut lrows: Vec<BoxRow> = logs
            .iter()
            .skip(skip_n)
            .map(|l| trow(&format!(" {white}{l}{RESET}"), w, "", None))
            .collect();
        if let Some(txt) = finished_msg {
            lrows.push(trow(
                &format!(" {green}{BOLD}{txt}{RESET}"),
                w,
                &pal.active_view_bg,
                Some((TAG_APPLY_BTN, 0)),
            ));
        }
        let badge = if finished_msg.is_some() {
            "done · press ⏎ or q"
        } else {
            "running..."
        };
        return vec![
            top,
            doc,
            panel("INSTALLATION PROGRESS", badge.into(), green, lrows, None),
        ];
    }

    let preset_name = PRESETS[opts.preset_idx % PRESETS.len()];
    let sprite_name = SPRITE_MODES[opts.sprite_mode_idx % SPRITE_MODES.len()];
    let wrap = if opts.wrap_lines {
        "vi ↪ Soft Wrap (wrap: true)"
    } else {
        "Unwrapped + 120Hz HScroll (Recommended)"
    };
    let cost = match opts.cost_lines {
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

    let mut items: Vec<(usize, BoxRow)> = Vec::with_capacity(TOTAL_SETUP_ROWS + 2);
    let (bg0, ptr0) = mk_ptr(0);
    let ptxt = format!(" {ptr0} {lavender}Theme Preset:{RESET}{bg0}   {amber}◀{RESET}{bg0} {BOLD}{white}{preset_name}{RESET}{bg0} {dim}({}/{}){RESET}{bg0} {amber}▶{RESET}", opts.preset_idx + 1, PRESETS.len());
    items.push((0, trow(&ptxt, w, bg0, Some((TAG_SETUP_ROW, 0)))));
    if view_h >= TOTAL_SETUP_ROWS + 2 {
        for t in [
            render_statusline_preview(pal, w),
            render_syntax_preview(pal, w),
        ] {
            items.push((0, trow(&t, w, "", Some((TAG_SETUP_ROW, 0)))));
        }
    }
    for (idx, lbl, col, val) in [
        (
            1,
            "Sprite Mode: ",
            mint,
            format!("{sprite_name} {dim}(PNG/JPG vs .ansi){RESET}"),
        ),
        (2, "Viewer Wrap: ", cyan, wrap.to_string()),
        (3, "COST Density:", amber, cost.to_string()),
    ] {
        let (bg, ptr) = mk_ptr(idx);
        let txt = format!(" {ptr} {col}{lbl}{RESET}{bg}   {amber}◀{RESET}{bg} {BOLD}{white}{val}{RESET}{bg} {amber}▶{RESET}");
        items.push((idx, trow(&txt, w, bg, Some((TAG_SETUP_ROW, idx)))));
    }
    let tg = [
        opts.install_plugin,
        opts.patch_claude,
        opts.config_companions,
        opts.setup_daemon_ssh,
    ];
    for (i, on) in tg.into_iter().enumerate() {
        let (bg, ptr) = mk_ptr(4 + i);
        let ic = if on {
            format!("{green}[✓]{RESET}{bg}")
        } else {
            format!("{dim}[ ]{RESET}{bg}")
        };
        let txt = format!(
            " {ptr} {ic} {BOLD}{white}{}{RESET}{bg} {dim}— {}{RESET}",
            TOG_T[i], TOG_S[i]
        );
        items.push((4 + i, trow(&txt, w, bg, Some((TAG_SETUP_ROW, 4 + i)))));
    }
    let cur = &pal.active_cursor_bg;
    let (btn_bg, bp) = if cursor == 8 {
        (cur, format!("{amber}❯{RESET}{cur}"))
    } else {
        (&pal.active_view_bg, " ".to_string())
    };
    let btn = format!(" {bp} {green}{BOLD}[ ▶ Apply Configuration, Install & Hot-Reload Workspace (⏎ / i) ]{RESET}{btn_bg}");
    items.push((8, trow(&btn, w, btn_bg, Some((TAG_APPLY_BTN, 8)))));

    // Window-scroll around the cursor so every row stays reachable in short terminals.
    let pos = items.iter().position(|(i, _)| *i == cursor).unwrap_or(0);
    let start = pos
        .saturating_sub(view_h / 2)
        .min(items.len().saturating_sub(view_h));
    let opt_rows: Vec<BoxRow> = items.drain(start..).take(view_h).map(|(_, r)| r).collect();
    let studio = panel(
        "CONFIGURATION & LIVE THEME STUDIO",
        format!("preset: {preset_name}"),
        lavender,
        opt_rows,
        None,
    );
    vec![top, doc, studio]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::setup::doctor::DoctorItem;

    /// `80x24` must still render the `[ ▶ Apply ... ]` row, keeping it visible and clickable.
    #[test]
    fn renders_apply_row_at_80x24() {
        let pal = Palette::default_palette();
        let items = vec![DoctorItem {
            status: CheckStatus::Ok,
            label: "T".into(),
            detail: "o".into(),
        }];
        let rep = DoctorReport {
            os_arch: "d".into(),
            backend_tier: "t".into(),
            missing_hint: "OK".into(),
            ipc_dir: "/tmp/.cc-ide-501".into(),
            ghostty_cap: "none",
            legacy_patch: false,
            items,
        };
        for c in 0..TOTAL_SETUP_ROWS {
            let p = build_setup_panels(80, 24, &rep, &SetupOptions::default(), c, &[], None, &pal);
            assert!(
                p[2].rows.iter().any(|r| r.text.contains("Apply")),
                "cursor={c}"
            );
        }
    }
}
