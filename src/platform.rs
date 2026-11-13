//! Cross-Platform OS Paths, UNC Normalization & Executable Resolution (`src/platform.rs`).
//!
//! Unifies environment & filesystem conventions across macOS, Linux, and Windows (`pwsh`/`cmd.exe`)
//! with zero external crate dependencies (MSRV `1.70.0`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolves the user's home directory across POSIX (`HOME`) and Windows (`USERPROFILE`).
pub fn user_home_dir() -> String {
    if let Ok(h) = env::var("HOME") {
        let t = h.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let Ok(up) = env::var("USERPROFILE") {
        let t = up.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let (Ok(drv), Ok(hp)) = (env::var("HOMEDRIVE"), env::var("HOMEPATH")) {
        let joined = format!("{}{}", drv.trim(), hp.trim());
        if !joined.is_empty() {
            return joined;
        }
    }
    ".".to_string()
}

/// Resolves the active OS username across POSIX (`USER`) and Windows (`USERNAME`).
pub fn current_username() -> String {
    env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_default()
}

/// Ephemeral IPC directory (`/tmp/.cc-ide` on Unix; `%TEMP%\.cc-ide` on Windows; `$CCIDE_IPC_DIR` override).
pub fn ipc_dir() -> PathBuf {
    if let Ok(custom) = env::var("CCIDE_IPC_DIR") {
        let t = custom.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    #[cfg(windows)]
    {
        env::temp_dir().join(".cc-ide")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/tmp/.cc-ide")
    }
}

pub fn ipc_path(name: &str) -> PathBuf {
    ipc_dir().join(name)
}

pub fn cache_dir(home: &str) -> PathBuf {
    Path::new(home).join(".cache").join("cc-ide")
}

/// Strips Windows `\\?\` extended-length UNC prefix produced by `std::fs::canonicalize()`.
pub fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", rest));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p
}

pub fn clean_canonicalize<P: AsRef<Path>>(p: P) -> Result<PathBuf, std::io::Error> {
    fs::canonicalize(p).map(strip_unc_prefix)
}

/// Normalizes path separators to forward slashes before hashing or prefix-filtering so
/// Windows `C:\a\b.rs` and `C:/a/b.rs` map identically.
pub fn normalize_slashes(raw: &str) -> String {
    let stripped = raw.strip_prefix(r"\\?\").unwrap_or(raw);
    stripped.replace('\\', "/")
}

/// Returns `true` if `abs_path` points inside Claude Code transcripts, cc-ide caches, or OS temp dirs.
pub fn is_internal_or_temp_path(abs_path: &str, home: &str) -> bool {
    let norm = normalize_slashes(abs_path);
    let home_norm = normalize_slashes(home).trim_end_matches('/').to_string();
    if !home_norm.is_empty() {
        let claude_proj = format!("{}/.claude/projects/", home_norm);
        let cc_cache = format!("{}/.cache/cc-ide/", home_norm);
        if norm.starts_with(&claude_proj) || norm.starts_with(&cc_cache) {
            return true;
        }
    }
    if norm.starts_with("/tmp/")
        || norm.starts_with("/private/tmp/")
        || norm.contains("/.cache/cc-ide/")
        || norm.contains("/.claude/projects/")
        || norm.contains("/.lib/")
        || norm.contains("/tmp/claude-")
    {
        return true;
    }
    let tmp_norm = normalize_slashes(&env::temp_dir().to_string_lossy())
        .trim_end_matches('/')
        .to_string();
    !tmp_norm.is_empty() && norm.starts_with(&format!("{}/", tmp_norm))
}

/// Resolves Yazi's configuration directory (`%APPDATA%\yazi\config` on Windows, `~/.config/yazi` on Unix).
pub fn yazi_config_dir(home: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            if !appdata.trim().is_empty() {
                return Path::new(appdata.trim()).join("yazi").join("config");
            }
        }
    }
    Path::new(home).join(".config").join("yazi")
}

/// Searches standard directories + `env::split_paths(PATH)` with Windows `.exe`/`.cmd` extensions,
/// skipping zero-byte Windows Store `WindowsApps\python*.exe` redirector stubs.
pub fn find_executable(bin_name: &str, home: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(not(windows))]
    {
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    if !home.is_empty() {
        for rel in [".local/bin", ".cargo/bin", ".claude/bin"] {
            dirs.push(Path::new(home).join(rel));
        }
        #[cfg(windows)]
        {
            if let Ok(lad) = env::var("LOCALAPPDATA") {
                dirs.push(Path::new(&lad).join("Microsoft/WinGet/Links"));
                dirs.push(Path::new(&lad).join("Programs/WezTerm"));
            }
            dirs.push(Path::new(home).join("scoop/shims"));
        }
        if let Ok(rd) = fs::read_dir(Path::new(home).join(".nvm/versions/node")) {
            for e in rd.flatten() {
                dirs.push(e.path().join("bin"));
            }
        }
    }
    if let Some(path_os) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path_os));
    }

    let exts: &[&str] = if cfg!(windows) && Path::new(bin_name).extension().is_none() {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };

    for dir in dirs {
        for ext in exts {
            let cand = dir.join(format!("{}{}", bin_name, ext));
            if let Ok(meta) = fs::metadata(&cand) {
                if meta.is_file() {
                    let s = cand.to_string_lossy();
                    if cfg!(windows) && s.contains("WindowsApps") && meta.len() == 0 {
                        continue;
                    }
                    return Some(cand);
                }
            }
        }
    }
    None
}

/// Resolves the `ccide-core` (`ccide-core.exe`) binary path for spawning `pane-runner` / `sidebar`.
pub fn current_engine_exe(home: &str) -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if exe.is_file() {
            return strip_unc_prefix(exe);
        }
    }
    let bin = if cfg!(windows) {
        "ccide-core.exe"
    } else {
        "ccide-core"
    };
    let cand = Path::new(home).join(".claude").join("bin").join(bin);
    if cand.is_file() {
        return cand;
    }
    find_executable("ccide-core", home).unwrap_or(cand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_slashes_and_detects_internal_paths() {
        assert_eq!(
            normalize_slashes(r"\\?\C:\Users\dev\src\main.rs"),
            "C:/Users/dev/src/main.rs"
        );
        assert!(is_internal_or_temp_path(
            r"C:\Users\dev\.claude\projects\abc\s1.jsonl",
            r"C:\Users\dev"
        ));
        assert!(!is_internal_or_temp_path(
            r"C:\Users\dev\repo\src\main.rs",
            r"C:\Users\dev"
        ));
    }
}
