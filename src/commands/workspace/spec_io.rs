//! `PaneSpec` Disk Persistence, State Cleanup & Live Tab Pruning (`src/commands/workspace/spec_io.rs`).

use std::fs;

use crate::commands::workspace::spec::PaneSpec;
use crate::ffi::{is_pid_alive, ProcToken};
use crate::platform::{atomic_write, cache_dir, ipc_dir, ipc_path, user_home_dir};

pub fn read_pane_spec(yid: &str) -> Option<PaneSpec> {
    let raw = fs::read_to_string(ipc_path(&format!("panes_{yid}.txt"))).ok()?;
    PaneSpec::parse(&raw, yid)
}

pub fn write_pane_spec(spec: &PaneSpec) -> std::io::Result<()> {
    atomic_write(
        &ipc_path(&format!("panes_{}.txt", spec.yid)),
        format!("{}\n", spec.encode()).as_bytes(),
    )
}

pub fn remove_tab_state(yid: &str) {
    let _ = fs::remove_file(ipc_path(&format!("panes_{yid}.txt")));
    let _ = fs::remove_file(ipc_path(&format!("ghostty_w_{yid}.txt")));
    let _ = fs::remove_file(ipc_path(&format!("reload_lg_{yid}")));
    for role in ["yazi", "lazygit", "mon", "sidebar"] {
        let _ = fs::remove_file(ipc_path(&format!("geom_{yid}_{role}.txt")));
    }
    let cdir = cache_dir(&user_home_dir());
    let _ = fs::remove_file(cdir.join(format!("tab_agent_{yid}.txt")));
    let _ = fs::remove_file(cdir.join(format!("tab_conv_{yid}.txt")));
    let _ = fs::remove_file(cdir.join(format!("live_tab_{yid}.json")));
}

pub fn prune_dead_specs(home: &str) -> usize {
    let (mut pruned, cdir) = (0usize, cache_dir(home));
    let Ok(rd) = fs::read_dir(ipc_dir()) else {
        return 0;
    };
    for ent in rd.flatten() {
        let fname = ent.file_name();
        let name = fname.to_string_lossy();
        let Some(yid) = name
            .strip_prefix("panes_")
            .and_then(|s| s.strip_suffix(".txt"))
        else {
            continue;
        };
        if yid.is_empty() {
            continue;
        }
        let Ok(raw) = fs::read_to_string(cdir.join(format!("tab_shell_{yid}.txt"))) else {
            continue;
        };
        let alive = match ProcToken::parse(&raw) {
            Some(t) if t.start_ms > 0 => t.is_alive_match(),
            Some(t) => is_pid_alive(t.pid),
            None => false,
        };
        if !alive {
            remove_tab_state(yid);
            let _ = fs::remove_file(cdir.join(format!("tab_agent_{yid}.txt")));
            let _ = fs::remove_file(cdir.join(format!("tab_conv_{yid}.txt")));
            let _ = fs::remove_file(cdir.join(format!("live_tab_{yid}.json")));
            pruned += 1;
        }
    }
    pruned
}
