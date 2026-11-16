//! Ghostty Capability Probes (macOS AppleScript + Linux Opt-In CLI/D-Bus) & Bounded Runner.

use std::env;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::commands::workspace::ghostty::{is_ghostty_env, parse_ghostty_version_ok, run_osa};
use crate::commands::workspace::spec::{OpenRequest, PaneSpec};
use crate::platform::{atomic_write, find_executable, ipc_path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhosttyCap {
    None,
    MacOsAppleScript,
    LinuxCli,
    LinuxDbus,
}

impl GhosttyCap {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MacOsAppleScript => "macos-applescript",
            Self::LinuxCli => "linux-cli",
            Self::LinuxDbus => "linux-dbus",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim() {
            "none" => Some(Self::None),
            "macos-applescript" => Some(Self::MacOsAppleScript),
            "linux-cli" => Some(Self::LinuxCli),
            "linux-dbus" => Some(Self::LinuxDbus),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhosttyProbeEnv {
    pub osascript_bin: String,
    pub ghostty_bin: String,
    pub gdbus_bin: String,
    pub linux_opt_in: bool,
    pub in_ssh: bool,
    pub in_tmux: bool,
    pub is_ghostty: bool,
}

impl GhosttyProbeEnv {
    pub fn from_env() -> Self {
        let non_empty = |k: &str, default: &str| {
            env::var(k)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Self {
            osascript_bin: non_empty("CCIDE_OSASCRIPT_BIN", "/usr/bin/osascript"),
            ghostty_bin: non_empty("CCIDE_GHOSTTY_BIN", "ghostty"),
            gdbus_bin: non_empty("CCIDE_GDBUS_BIN", "gdbus"),
            linux_opt_in: env::var("CCIDE_GHOSTTY_LINUX")
                .ok()
                .map(|s| s.trim() == "1")
                .unwrap_or(false),
            in_ssh: env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok(),
            in_tmux: !env::var("TMUX").unwrap_or_default().trim().is_empty(),
            is_ghostty: is_ghostty_env(),
        }
    }
}

/// Runs `bin` with `args` bounded by `timeout_ms`, polling `try_wait` and killing on expiry.
pub fn run_bounded(bin: &str, args: &[&str], timeout_ms: u64) -> Option<String> {
    let mut ch = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out_pipe = ch.stdout.take();
    let reader = thread::spawn(move || {
        let mut buf = String::new();
        if let Some(ref mut p) = out_pipe {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(1));
    loop {
        match ch.try_wait() {
            Ok(Some(st)) => {
                let out = reader.join().unwrap_or_default();
                return st.success().then_some(out);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = ch.kill();
                    let _ = ch.wait();
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = ch.kill();
                let _ = ch.wait();
                return None;
            }
        }
    }
}

pub fn probe_linux(env: &GhosttyProbeEnv) -> GhosttyCap {
    if !env.linux_opt_in || env.in_ssh || env.in_tmux || !env.is_ghostty {
        return GhosttyCap::None;
    }
    if let Some(out) = run_bounded(&env.ghostty_bin, &["+list-actions"], 150) {
        if out.contains("new_split") && out.contains("resize_split") && out.contains("goto_split") {
            return GhosttyCap::LinuxCli;
        }
    }
    if let Some(out) = run_bounded(
        &env.gdbus_bin,
        &[
            "introspect",
            "--session",
            "--dest",
            "com.mitchellh.ghostty",
            "--object-path",
            "/com/mitchellh/ghostty",
        ],
        150,
    ) {
        if out.contains("org.gtk.Actions") && out.contains("new-split") {
            return GhosttyCap::LinuxDbus;
        }
    }
    GhosttyCap::None
}

fn file_mtime_secs(path: &str) -> Option<String> {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs().to_string())
}

fn macos_app_stamp(home: &str) -> Option<String> {
    [
        "/Applications/Ghostty.app/Contents/Info.plist".to_string(),
        format!("{home}/Applications/Ghostty.app/Contents/Info.plist"),
    ]
    .iter()
    .find_map(|p| file_mtime_secs(p))
}

fn read_cap_cache(cache: &std::path::Path, want_stamp: &str) -> Option<GhosttyCap> {
    let fresh = fs::metadata(cache)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs() < 3600)
        .unwrap_or(false);
    if !fresh {
        return None;
    }
    let raw = fs::read_to_string(cache).ok()?;
    let (cap_s, st_s) = raw.trim().split_once('|')?;
    if st_s != want_stamp {
        return None;
    }
    GhosttyCap::from_str(cap_s).filter(|&c| c != GhosttyCap::None)
}

pub fn probe_ghostty_with(home: &str, env: &GhosttyProbeEnv) -> GhosttyCap {
    if env.in_ssh || env.in_tmux || !env.is_ghostty {
        return GhosttyCap::None;
    }
    let has_osa = std::path::Path::new(&env.osascript_bin).exists();
    let osa_mocked = env.osascript_bin != "/usr/bin/osascript" && has_osa;
    let linux_mocked = env.ghostty_bin != "ghostty" || env.gdbus_bin != "gdbus";
    let linux_path =
        env.linux_opt_in && !osa_mocked && (!cfg!(target_os = "macos") || linux_mocked || !has_osa);
    if !linux_path && !osa_mocked && !cfg!(target_os = "macos") {
        return GhosttyCap::None;
    }
    let is_mocked = osa_mocked || linux_mocked;
    let cache = ipc_path("ghostty_cap.cache");
    let stamp = if is_mocked {
        None
    } else if linux_path {
        find_executable(&env.ghostty_bin, home).and_then(|p| p.to_str().and_then(file_mtime_secs))
    } else {
        macos_app_stamp(home)
    };
    if let Some(want) = stamp.as_deref() {
        if let Some(cached) = read_cap_cache(&cache, want) {
            return cached;
        }
    }
    let cap = if linux_path {
        probe_linux(env)
    } else if run_osa(
        &env.osascript_bin,
        "tell application \"Ghostty\" to get version",
        true,
    )
    .map(|s| parse_ghostty_version_ok(&s))
    .unwrap_or(false)
    {
        GhosttyCap::MacOsAppleScript
    } else {
        GhosttyCap::None
    };
    if !is_mocked {
        if let Some(want) = stamp.filter(|_| cap != GhosttyCap::None) {
            let _ = atomic_write(&cache, format!("{}|{want}", cap.as_str()).as_bytes());
        } else {
            let _ = fs::remove_file(&cache);
        }
    }
    cap
}

pub fn probe_ghostty(home: &str) -> GhosttyCap {
    probe_ghostty_with(home, &GhosttyProbeEnv::from_env())
}

pub fn probe_ghostty_native_capable(home: &str) -> bool {
    probe_ghostty(home) != GhosttyCap::None
}

pub fn run_ghostty_linux_open(_req: &OpenRequest, cap: GhosttyCap) -> Option<PaneSpec> {
    eprintln!(
        "ccide-core: Ghostty Linux probe detected ({}), falling back to tmux in v0.10 (external split surface targeting unsupported).",
        cap.as_str()
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_probe_gates_opt_in_and_missing_binary() {
        let mut env = GhosttyProbeEnv {
            osascript_bin: "/usr/bin/osascript".to_string(),
            ghostty_bin: "/nonexistent/ghostty_bin_check".to_string(),
            gdbus_bin: "/nonexistent/gdbus_bin_check".to_string(),
            linux_opt_in: false,
            in_ssh: false,
            in_tmux: false,
            is_ghostty: true,
        };
        assert_eq!(GhosttyCap::None.as_str(), "none");
        assert_eq!(GhosttyCap::LinuxCli.as_str(), "linux-cli");
        assert_eq!(GhosttyCap::LinuxDbus.as_str(), "linux-dbus");
        assert_eq!(GhosttyCap::MacOsAppleScript.as_str(), "macos-applescript");
        assert_eq!(probe_linux(&env), GhosttyCap::None);
        env.linux_opt_in = true;
        assert_eq!(probe_linux(&env), GhosttyCap::None);
        env.in_ssh = true;
        assert_eq!(probe_linux(&env), GhosttyCap::None);
    }
}
