//! Cross-Platform OS Paths, UNC Normalization & Executable Resolution (`src/platform.rs`).
//!
//! Unifies environment & filesystem conventions across macOS, Linux, and Windows (`pwsh`/`cmd.exe`)
//! with zero external crate dependencies (MSRV `1.70.0`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Resolves the user's home directory across POSIX (`HOME`) and Windows (`USERPROFILE`).
pub fn user_home_dir() -> String {
    for k in ["HOME", "USERPROFILE"] {
        if let Ok(v) = env::var(k) {
            let t = v.trim();
            if !t.is_empty() {
                return t.to_string();
            }
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

/// Normalizes path separators to forward slashes before hashing or prefix-filtering.
pub fn normalize_slashes(raw: &str) -> String {
    raw.strip_prefix(r"\\?\").unwrap_or(raw).replace('\\', "/")
}

/// Atomically writes `bytes` to `path` via a sibling `.{name}.tmp.{pid}.{seq}` file and rename.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = fs::create_dir_all(parent);
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("art");
    let tmp = path.with_file_name(format!(".{name}.tmp.{}.{seq}", std::process::id()));
    if let Err(e) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if fs::rename(&tmp, path).is_ok() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
        if fs::rename(&tmp, path).is_ok() {
            return Ok(());
        }
    }
    let res = fs::write(path, bytes);
    let _ = fs::remove_file(&tmp);
    res
}

/// Lexical path comparison without I/O: strips extended UNC prefixes, normalizes `\` to `/`,
/// trims trailing `/`, and compares case-insensitively on Windows (exact on Unix).
pub fn paths_equal_ci(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| -> String {
        let raw = strip_unc_prefix(p.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let s = match raw.strip_prefix(r"\\?\UNC\") {
            Some(rest) => format!("//{}", rest.replace('\\', "/")),
            None => normalize_slashes(&raw),
        };
        let t = s.trim_end_matches('/');
        if t.is_empty() && s.starts_with('/') {
            "/".to_string()
        } else {
            t.to_string()
        }
    };
    let (na, nb) = (norm(a), norm(b));
    if cfg!(windows) {
        na.to_lowercase() == nb.to_lowercase()
    } else {
        na == nb
    }
}

/// Returns `true` if `abs_path` points inside Claude Code transcripts, cc-ide caches, or OS temp dirs.
pub fn is_internal_or_temp_path(abs_path: &str, home: &str) -> bool {
    let norm = normalize_slashes(abs_path);
    let home_norm = normalize_slashes(home).trim_end_matches('/').to_string();
    if !home_norm.is_empty() {
        for sub in ["/.claude/projects/", "/.cache/cc-ide/", "/.lib/"] {
            if norm.starts_with(&format!("{home_norm}{sub}")) {
                return true;
            }
        }
    }
    if norm.starts_with("/tmp/")
        || norm.starts_with("/private/tmp/")
        || norm.contains("/.cache/cc-ide/")
        || norm.contains("/.claude/projects/")
        || norm.contains("/tmp/claude-")
    {
        return true;
    }
    let tmp_norm = normalize_slashes(&env::temp_dir().to_string_lossy())
        .trim_end_matches('/')
        .to_string();
    !tmp_norm.is_empty() && norm.starts_with(&format!("{tmp_norm}/"))
}

/// Resolves Yazi's configuration directory (`%APPDATA%\yazi\config` on Windows, `~/.config/yazi` on Unix).
pub fn yazi_config_dir(home: &str) -> PathBuf {
    #[cfg(windows)]
    if let Ok(appdata) = env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            return Path::new(appdata.trim()).join("yazi").join("config");
        }
    }
    Path::new(home).join(".config").join("yazi")
}

/// Searches standard directories + `env::split_paths(PATH)` with Windows `.exe`/`.cmd` extensions.
pub fn find_executable(bin_name: &str, home: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(not(windows))]
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        dirs.push(PathBuf::from(d));
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
            let cand = dir.join(format!("{bin_name}{ext}"));
            if let Ok(meta) = fs::metadata(&cand) {
                if meta.is_file() {
                    if cfg!(windows)
                        && cand.to_string_lossy().contains("WindowsApps")
                        && meta.len() == 0
                    {
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
        assert!(!is_internal_or_temp_path("/repo/.lib/foo.rs", "/Users/dev"));
        assert!(is_internal_or_temp_path(
            "/Users/dev/.lib/foo.rs",
            "/Users/dev"
        ));
        assert!(paths_equal_ci(Path::new("/a/b/"), Path::new("/a/b")));
    }
}
