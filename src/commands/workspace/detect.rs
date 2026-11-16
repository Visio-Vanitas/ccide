//! Canonical Workspace Split-Engine Auto-Detection (`ccide-core workspace detect [--json]`).

use std::env;

use crate::commands::workspace::ghostty::{is_ghostty_env, probe_ghostty, GhosttyCap};
use crate::json::json_escape;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectResult {
    pub backend: &'static str,
    pub ghostty_cap: GhosttyCap,
    pub in_tmux: bool,
    pub in_ssh: bool,
    pub reasons: Vec<String>,
}

pub fn detect_backend(home: &str) -> DetectResult {
    let in_tmux = env::var("TMUX").map(|s| !s.is_empty()).unwrap_or(false);
    let in_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok();
    let in_wezterm = env::var("WEZTERM_PANE")
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let in_wt = env::var("WT_SESSION")
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let is_iterm = env::var("TERM_PROGRAM").as_deref() == Ok("iTerm.app")
        || env::var("LC_TERMINAL").as_deref() == Ok("iTerm2")
        || env::var("ITERM_SESSION_ID").is_ok();
    let is_ghostty = is_ghostty_env();
    let ghostty_cap = if !in_ssh && !in_tmux && is_ghostty {
        probe_ghostty(home)
    } else {
        GhosttyCap::None
    };

    let mut reasons = Vec::new();
    if in_tmux {
        reasons.push("TMUX environment active".to_string());
    }
    if in_ssh {
        reasons.push("SSH session detected".to_string());
    }

    let backend = if !cfg!(windows) && !in_ssh && !in_tmux && is_iterm {
        reasons.push("local iTerm2 terminal detected".to_string());
        "iterm"
    } else if in_ssh && !in_tmux && is_iterm {
        reasons.push("SSH with iTerm2 client OSC 1337 bridge".to_string());
        "osc"
    } else if !in_tmux && in_wezterm {
        reasons.push("WEZTERM_PANE active".to_string());
        "wezterm"
    } else if cfg!(windows) && in_wt {
        reasons.push("WT_SESSION active on Windows".to_string());
        "wt"
    } else if !in_ssh && !in_tmux && is_ghostty && ghostty_cap == GhosttyCap::MacOsAppleScript {
        reasons.push("Ghostty macOS AppleScript split engine capable".to_string());
        "ghostty"
    } else if in_tmux {
        if is_ghostty && ghostty_cap != GhosttyCap::None {
            reasons.push(format!(
                "Ghostty capability '{}' probed inside tmux",
                ghostty_cap.as_str()
            ));
        }
        "tmux"
    } else {
        if is_ghostty && ghostty_cap != GhosttyCap::None {
            reasons.push(format!(
                "Ghostty '{}' probed (falls back to tmux in v0.10)",
                ghostty_cap.as_str()
            ));
        } else {
            reasons.push("no native terminal split engine active outside tmux".to_string());
        }
        "none"
    };

    DetectResult {
        backend,
        ghostty_cap,
        in_tmux,
        in_ssh,
        reasons,
    }
}

pub fn format_detect_json(res: &DetectResult) -> String {
    let reasons_json: Vec<String> = res
        .reasons
        .iter()
        .map(|r| format!("\"{}\"", json_escape(r)))
        .collect();
    format!(
        "{{\"backend\":\"{}\",\"ghostty_cap\":\"{}\",\"in_tmux\":{},\"in_ssh\":{},\"reasons\":[{}]}}",
        res.backend,
        res.ghostty_cap.as_str(),
        if res.in_tmux { "true" } else { "false" },
        if res.in_ssh { "true" } else { "false" },
        reasons_json.join(",")
    )
}

pub fn run_detect(home: &str, args: &[String]) {
    let json_mode = args.iter().skip(3).any(|a| a == "--json" || a == "-j");
    let res = detect_backend(home);
    if json_mode {
        println!("{}", format_detect_json(&res));
    } else {
        println!("{}", res.backend);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_detect_json_with_required_schema() {
        let res = DetectResult {
            backend: "tmux",
            ghostty_cap: GhosttyCap::None,
            in_tmux: true,
            in_ssh: false,
            reasons: vec!["TMUX environment active".to_string()],
        };
        let js = format_detect_json(&res);
        assert!(js.contains("\"backend\":\"tmux\""));
        assert!(js.contains("\"ghostty_cap\":\"none\""));
        assert!(js.contains("\"in_tmux\":true"));
        assert!(js.contains("\"in_ssh\":false"));
    }
}
