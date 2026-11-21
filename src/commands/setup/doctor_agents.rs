//! Multi-Agent (`claude` · `agy` · `codex`) CLI & Hook Registration Probes for `ccide-core doctor`.

use std::fs;
use std::path::Path;

use crate::commands::setup::doctor::{item, CheckStatus, DoctorItem, DoctorReport};
use crate::commands::workspace::ghostty_linux::run_bounded;
use crate::json::json_escape;
use crate::platform::find_executable;
use crate::session::codex_home;

pub fn probe_cli_version(bin: &Path) -> Option<String> {
    let out = run_bounded(&bin.to_string_lossy(), &["--version"], 2000)?;
    out.lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(String::from)
}

pub fn probe_named_cli_version(name: &str, home: &str) -> Option<String> {
    let bin = find_executable(name, home)?;
    Some(probe_cli_version(&bin).unwrap_or_else(|| "installed".to_string()))
}

pub fn is_agy_hooks_registered(home: &str) -> bool {
    let p = Path::new(home).join(".gemini/config/plugins/cc-ide/hooks.json");
    fs::read_to_string(p)
        .map(|s| s.contains("ccide-core") || s.contains("yazi-hook") || s.contains("cc-ide"))
        .unwrap_or(false)
}

pub fn is_codex_hooks_registered() -> bool {
    let p = codex_home().join("hooks.json");
    fs::read_to_string(p)
        .map(|s| s.contains("ccide-core") || s.contains("yazi-hook") || s.contains("cc-ide"))
        .unwrap_or(false)
}

/// Appends `Antigravity` and `Codex CLI` check items to `DoctorReport.items`.
/// Missing optional agent CLIs report `CheckStatus::Ok` (`"not installed"`), never `Warn`.
pub fn probe_extra_agents(home: &str, items: &mut Vec<DoctorItem>) {
    let agy_hooks = is_agy_hooks_registered(home);
    let agy_detail = match probe_named_cli_version("agy", home) {
        Some(ver) => format!(
            "{ver} · hooks {}",
            if agy_hooks {
                "registered"
            } else {
                "unregistered"
            }
        ),
        None => "not installed".to_string(),
    };
    items.push(item(CheckStatus::Ok, "Antigravity", agy_detail));

    let codex_hooks = is_codex_hooks_registered();
    let codex_detail = match probe_named_cli_version("codex", home) {
        Some(ver) => format!(
            "{ver} · hooks {}",
            if codex_hooks {
                "registered"
            } else {
                "unregistered"
            }
        ),
        None => "not installed".to_string(),
    };
    items.push(item(CheckStatus::Ok, "Codex CLI", codex_detail));
}

fn fmt_opt_str(v: Option<&str>) -> String {
    match v {
        Some(s) => format!("\"{}\"", json_escape(s)),
        None => "null".to_string(),
    }
}

fn extract_ver_from_item(items: &[DoctorItem], label: &str) -> Option<String> {
    let detail = items.iter().find(|i| i.label == label)?.detail.trim();
    if detail.starts_with("not installed") || detail.starts_with("not found") {
        return None;
    }
    let ver = detail.split(" · ").next().unwrap_or(detail).trim();
    (!ver.is_empty()).then(|| ver.to_string())
}

/// Injects additive `"agents"` and `"agent_hooks"` keys into `format_doctor_json` output.
pub fn inject_multi_agent_json(
    base_json: &str,
    claude_ver: Option<&str>,
    agy_ver: Option<&str>,
    codex_ver: Option<&str>,
    agy_hooks: bool,
    codex_hooks: bool,
) -> String {
    let extra = format!(
        "  \"agents\": {{\"claude\": {}, \"agy\": {}, \"codex\": {}}},\n  \"agent_hooks\": {{\"agy\": {}, \"codex\": {}}},\n  \"missing_hint\":",
        fmt_opt_str(claude_ver),
        fmt_opt_str(agy_ver),
        fmt_opt_str(codex_ver),
        if agy_hooks { "true" } else { "false" },
        if codex_hooks { "true" } else { "false" },
    );
    if base_json.contains("  \"missing_hint\":") {
        base_json.replacen("  \"missing_hint\":", &extra, 1)
    } else {
        base_json.to_string()
    }
}

pub fn format_doctor_json_with_agents(
    report: &DoctorReport,
    base_json: &str,
    home: &str,
) -> String {
    let claude_v = extract_ver_from_item(&report.items, "Claude Code");
    let agy_v = extract_ver_from_item(&report.items, "Antigravity");
    let codex_v = extract_ver_from_item(&report.items, "Codex CLI");
    inject_multi_agent_json(
        base_json,
        claude_v.as_deref(),
        agy_v.as_deref(),
        codex_v.as_deref(),
        is_agy_hooks_registered(home),
        is_codex_hooks_registered(),
    )
}
