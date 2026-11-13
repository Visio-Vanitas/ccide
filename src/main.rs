//! `ccide-core` — Cross-Platform High-Frequency Native Engine for `cc-ide`.
//!
//! Provides four sub-millisecond, zero-dependency subcommands:
//! - `ccide-core sidebar <target_dir> <yazi_id> [shell_pid]`:
//!   3-Tier Right Sidebar TUI daemon (`Model Logo` + `SUBAGENTS` + `Hardware Telemetry Curves`).
//! - `ccide-core yazi-hook`:
//!   `PostToolUse` / `SubagentStart` / `SubagentStop` hook that syncs `Yazi` and wakes the sidebar.
//! - `ccide-core statusline`:
//!   Powerline status bar renderer reading shared telemetry cache and exporting per-session model state.
//! - `ccide-core workspace <open|close|rebalance|focus|switch-subagent> ...`:
//!   3-Tier Adaptive Workspace Controller (`Local iTerm2 Daemon Socket` -> `SSH OSC 1337 Passthrough` -> `tmux`).

mod commands;
mod ffi;
mod json;
mod session;
mod telemetry;
mod tui;

use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let subcmd = args.get(1).map(|s| s.as_str()).unwrap_or("sidebar");

    match subcmd {
        "sidebar" => {
            let target_dir = args.get(2).cloned().unwrap_or_else(|| {
                env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(String::from))
                    .unwrap_or_default()
            });
            let yazi_id = args.get(3).map(|s| s.as_str()).unwrap_or("");
            let shell_pid = args.get(4).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            commands::sidebar::run(&target_dir, yazi_id, shell_pid);
        }
        "yazi-hook" => commands::yazi_hook::run(),
        "statusline" => commands::statusline::run(),
        "workspace" => commands::workspace::run(&args),
        "setup" | "install" => commands::setup::run_setup(&args),
        "doctor" => commands::setup::run_doctor(&args),
        _ => {}
    }
}
