//! `ccide-core` — Cross-Platform High-Frequency Native Engine for `cc-ide`.
//!
//! Subcommands (`macOS`, `Linux`, `Windows MSVC`, zero external Cargo dependencies):
//! - `ccide-core sidebar <target_dir> <yazi_id> [shell_pid]`
//! - `ccide-core pane-runner <yazi|lazygit|macmon|sidebar> <target_dir> [yazi_id] [shell_pid]`
//! - `ccide-core yazi-hook`
//! - `ccide-core statusline`
//! - `ccide-core workspace <open|close|rebalance|focus|switch-subagent|clear-subagents|theme> ...`
//! - `ccide-core setup [project_root]` / `ccide-core doctor [--json] [project_root]`
//! - `ccide-core ipc-dir` / `ccide-core wake-pid <pid[:start_ms]>`

mod budget;
mod commands;
mod ffi;
mod json;
mod model_id;
mod orchestrator;
mod platform;
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
        "pane-runner" => commands::pane_runner::run(&args),
        "pre-tool-hook" => commands::pre_tool_hook::run_pre_tool_hook(),
        "orchestrator" => commands::pre_tool_hook::run_orchestrator_cli(&args),
        "yazi-hook" => commands::yazi_hook::run(),
        "statusline" => commands::statusline::run(),
        "workspace" => commands::workspace::run(&args),
        "setup" | "install" => commands::setup::run_setup(&args),
        "doctor" => commands::setup::run_doctor(&args),
        "ipc-dir" => match platform::try_ipc_dir() {
            Some(dir) => println!("{}", dir.display()),
            None => std::process::exit(1),
        },
        "wake-pid" => {
            if let Some(tok) = args.get(2).and_then(|s| ffi::ProcToken::parse(s)) {
                if tok.start_ms > 0 {
                    let _ = ffi::wake_token(&tok);
                } else if let Ok(rd) = std::fs::read_dir(platform::ipc_dir()) {
                    for ent in rd.flatten() {
                        let p = ent.path();
                        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if fname.starts_with("sb_") && fname.ends_with(".pid") {
                            if let Ok(txt) = std::fs::read_to_string(&p) {
                                if let Some(t) = ffi::ProcToken::parse(&txt)
                                    .filter(|t| t.pid == tok.pid && t.is_alive_match())
                                {
                                    let _ = ffi::wake_token(&t);
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}
