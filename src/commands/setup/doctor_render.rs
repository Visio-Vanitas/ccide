//! TUI Box & Structured `--json` Renderers for `ccide-core doctor`.

use crate::commands::setup::doctor::{CheckStatus, DoctorReport};
use crate::tui::grid::{
    format_bottom_border, format_boxed_row, format_top_border, get_terminal_geometry, BoxRow,
};
use crate::tui::palette::Palette;
use crate::tui::width::trunc_to_width;
use crate::tui::{BOLD, RESET};

fn json_esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

pub fn format_doctor_json(report: &DoctorReport) -> String {
    let mut checks_json = String::new();
    for (idx, it) in report.items.iter().enumerate() {
        if idx > 0 {
            checks_json.push_str(",\n");
        }
        let st = match it.status {
            CheckStatus::Ok => "ok",
            CheckStatus::Warn => "warn",
            CheckStatus::Info => "info",
        };
        let slug: String = it
            .label
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let slug = slug.trim_matches('_');
        checks_json.push_str(&format!(
            "    {{\"id\":\"{}\",\"status\":\"{}\",\"label\":\"{}\",\"detail\":\"{}\"}}",
            json_esc(slug),
            st,
            json_esc(&it.label),
            json_esc(&it.detail)
        ));
    }
    let warn_count = report
        .items
        .iter()
        .filter(|i| i.status == CheckStatus::Warn)
        .count();
    format!(
        "{{\n  \"schema_version\": 1,\n  \"ccide_version\": \"{}\",\n  \"os_arch\": \"{}\",\n  \"backend_tier\": \"{}\",\n  \"healthy\": {},\n  \"warnings\": {},\n  \"checks\": [\n{}\n  ],\n  \"missing_hint\": \"{}\"\n}}",
        env!("CARGO_PKG_VERSION"),
        json_esc(&report.os_arch),
        json_esc(&report.backend_tier),
        if warn_count == 0 { "true" } else { "false" },
        warn_count,
        checks_json,
        json_esc(&report.missing_hint),
    )
}

pub fn print_doctor_tui(report: &DoctorReport, home: &str) {
    let pal = Palette::load(home);
    let g_cols = get_terminal_geometry().cols;
    let cols = if g_cols <= 48 {
        76
    } else {
        g_cols.clamp(68, 96)
    };

    println!(
        "{}",
        format_top_border(cols, "CC-IDE DOCTOR", &report.os_arch, &pal.cyan, &pal)
    );
    println!(
        "{}",
        format_boxed_row(
            cols,
            &BoxRow {
                text: format!(
                    " {}{}Backend:{RESET} {}{}{RESET}",
                    BOLD, pal.text, pal.mint, report.backend_tier
                ),
                bg: String::new(),
                tag: None,
            },
            &pal,
        )
    );
    for it in &report.items {
        let (icon, col) = match it.status {
            CheckStatus::Ok => ("✓", &pal.green),
            CheckStatus::Warn => ("!", &pal.amber),
            CheckStatus::Info => ("●", &pal.cyan),
        };
        println!(
            "{}",
            format_boxed_row(
                cols,
                &BoxRow {
                    text: trunc_to_width(
                        &format!(
                            " {col}{icon}{RESET} {BOLD}{}{:<14}{RESET} {}{}{RESET}",
                            pal.text, it.label, pal.dim, it.detail
                        ),
                        cols.saturating_sub(3),
                    ),
                    bg: String::new(),
                    tag: None,
                },
                &pal,
            )
        );
    }
    println!(
        "{}",
        format_boxed_row(
            cols,
            &BoxRow {
                text: trunc_to_width(
                    &format!(" {}{}{RESET}", pal.amber, report.missing_hint),
                    cols.saturating_sub(3),
                ),
                bg: String::new(),
                tag: None,
            },
            &pal,
        )
    );
    println!("{}", format_bottom_border(cols, &pal));
}
