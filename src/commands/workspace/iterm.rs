//! Tier 1 (Local iTerm2 Warm Daemon Socket) & Tier 2 (SSH OSC 1337 Passthrough) Backends.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::platform::{find_executable, ipc_path};

/// Sends a newline-delimited JSON request to the warm local iTerm2 daemon over Unix Domain Socket.
#[cfg(unix)]
pub fn send_daemon_ipc(json_line: &str, timeout_ms: u64) -> Option<String> {
    use std::os::unix::net::UnixStream;
    let sock_path = ipc_path("iterm_daemon.sock");
    let stream = UnixStream::connect(sock_path).ok()?;
    let dur = Some(Duration::from_millis(timeout_ms));
    let _ = stream.set_read_timeout(dur);
    let _ = stream.set_write_timeout(dur);
    let mut writer = stream.try_clone().ok()?;
    writer.write_all(json_line.as_bytes()).ok()?;
    if !json_line.ends_with('\n') {
        writer.write_all(b"\n").ok()?;
    }
    writer.flush().ok()?;

    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).ok()?;
    let trimmed = resp.trim().to_string();
    if trimmed.is_empty() || trimmed.starts_with("ERR:") {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(not(unix))]
pub fn send_daemon_ipc(_json_line: &str, _timeout_ms: u64) -> Option<String> {
    let _ = (
        BufReader::<std::io::Empty>::new,
        Duration::from_millis,
        ipc_path,
    );
    let _ = <std::io::Empty as BufRead>::read_line;
    None
}

/// Emits an `OSC 1337;Custom=id=ccide:<payload>` escape sequence directly to the controlling TTY.
pub fn emit_osc1337(json_payload: &str) -> bool {
    let seq = format!("\x1b]1337;Custom=id=ccide:{}\x07", json_payload);
    #[cfg(unix)]
    if let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") {
        if tty.write_all(seq.as_bytes()).is_ok() {
            let _ = tty.flush();
            return true;
        }
    }
    #[cfg(windows)]
    if let Ok(mut con) = OpenOptions::new().write(true).open("CONOUT$") {
        if con.write_all(seq.as_bytes()).is_ok() {
            let _ = con.flush();
            return true;
        }
    }
    let mut out = std::io::stdout();
    if out.write_all(seq.as_bytes()).is_ok() {
        let _ = out.flush();
        return true;
    }
    false
}

/// Unified Python interpreter resolver shared with `install.sh`, `install.ps1` & `iterm-pyenv.py`.
pub fn find_python_bin(home: &str) -> String {
    let rec_file = format!("{}/.cache/cc-ide/python_bin", home);
    if let Ok(raw) = fs::read_to_string(&rec_file) {
        let cand = raw.trim();
        if !cand.is_empty() && Path::new(cand).is_file() {
            return cand.to_string();
        }
    }
    for cand in [
        format!("{}/.local/share/cc-ide/pyenv/bin/python3", home),
        format!("{}/.local/share/cc-ide/pyenv/Scripts/python.exe", home),
        format!("{}/miniconda3/bin/python3", home),
        format!("{}/miniconda3/python.exe", home),
    ] {
        if Path::new(&cand).is_file() {
            return cand;
        }
    }
    if let Some(p) = find_executable("python3", home).or_else(|| find_executable("python", home)) {
        return p.to_string_lossy().into_owned();
    }
    if cfg!(windows) {
        "python.exe".to_string()
    } else {
        "/usr/bin/python3".to_string()
    }
}

pub fn ensure_daemon_started(home: &str) {
    if cfg!(windows) {
        return;
    }
    if ipc_path("iterm_daemon.sock").exists() && send_daemon_ipc("{\"op\":\"ping\"}", 80).is_some()
    {
        return;
    }
    let py = find_python_bin(home);
    let daemon_script = format!("{}/.claude/scripts/ccide_daemon.py", home);
    if Path::new(&py).exists() && Path::new(&daemon_script).exists() {
        let _ = Command::new(&py)
            .arg(&daemon_script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}
