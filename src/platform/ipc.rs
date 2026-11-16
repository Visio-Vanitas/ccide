//! Hardened Per-UID Ephemeral IPC Directory Resolution & Validation (`src/platform/ipc.rs`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::ffi::current_euid;
use crate::platform::{strip_unc_prefix, user_home_dir};

static IPC_MEMO: OnceLock<(String, Option<PathBuf>)> = OnceLock::new();
static WARNED_ENV: AtomicBool = AtomicBool::new(false);

/// Validates or prepares an IPC directory: rejects symlinks and foreign-UID directories,
/// creating missing directories with mode `0o700` and tightening `0o077` group/other bits on Unix.
pub fn validate_or_prepare_unix_ipc_dir(path: &Path, euid: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        if let Err(e) = fs::symlink_metadata(path) {
            if e.kind() == std::io::ErrorKind::NotFound {
                let mut db = fs::DirBuilder::new();
                db.recursive(true).mode(0o700);
                db.create(path).map_err(|e| e.to_string())?;
            } else {
                return Err(e.to_string());
            }
        }
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let ft = meta.file_type();
        if ft.is_symlink() || !ft.is_dir() {
            return Err("not a regular non-symlink directory".into());
        }
        if meta.uid() != euid {
            return Err(format!("owner uid {} != euid {}", meta.uid(), euid));
        }
        if (meta.mode() & 0o077) != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = euid;
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let ft = meta.file_type();
        if ft.is_symlink() || !ft.is_dir() {
            return Err("not a regular non-symlink directory".into());
        }
        Ok(())
    }
}

pub(crate) fn resolve_ipc_dir_inner(
    custom_env: Option<&str>,
    euid: u32,
    home: &str,
) -> Option<PathBuf> {
    if let Some(c) = custom_env.map(str::trim).filter(|s| !s.is_empty()) {
        let p = strip_unc_prefix(PathBuf::from(c));
        match validate_or_prepare_unix_ipc_dir(&p, euid) {
            Ok(()) => return Some(p),
            Err(err) => {
                if !WARNED_ENV.swap(true, Ordering::Relaxed) {
                    eprintln!(
                        "[cc-ide] warning: ignoring unsafe CCIDE_IPC_DIR {:?}: {}",
                        p, err
                    );
                }
            }
        }
    }
    #[cfg(unix)]
    {
        let primary = PathBuf::from(format!("/tmp/.cc-ide-{euid}"));
        if validate_or_prepare_unix_ipc_dir(&primary, euid).is_ok() {
            return Some(primary);
        }
        if !home.is_empty() && home != "." {
            let fallback = Path::new(home).join(".cache").join("cc-ide").join("ipc");
            if validate_or_prepare_unix_ipc_dir(&fallback, euid).is_ok() {
                return Some(fallback);
            }
        }
        None
    }
    #[cfg(windows)]
    {
        let _ = home;
        let base = env::var("TEMP")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| env::var("TMP").ok().filter(|s| !s.trim().is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        let win_dir = strip_unc_prefix(base.join(".cc-ide"));
        validate_or_prepare_unix_ipc_dir(&win_dir, euid)
            .ok()
            .map(|()| win_dir)
    }
}

/// Resolves and validates the active IPC directory, returning `None` if all candidates fail.
pub fn try_ipc_dir() -> Option<PathBuf> {
    let cur_env = env::var("CCIDE_IPC_DIR").unwrap_or_default();
    let (cached_env, cached_dir) = IPC_MEMO.get_or_init(|| {
        let dir = resolve_ipc_dir_inner(
            (!cur_env.is_empty()).then_some(cur_env.as_str()),
            current_euid(),
            &user_home_dir(),
        );
        (cur_env.clone(), dir)
    });
    if cached_env == &cur_env {
        cached_dir.clone()
    } else {
        resolve_ipc_dir_inner(
            (!cur_env.is_empty()).then_some(cur_env.as_str()),
            current_euid(),
            &user_home_dir(),
        )
    }
}

/// Ephemeral IPC directory (`/tmp/.cc-ide-<euid>` on Unix; `%TEMP%\.cc-ide` on Windows; `$CCIDE_IPC_DIR` override).
pub fn ipc_dir() -> PathBuf {
    try_ipc_dir().unwrap_or_else(|| {
        #[cfg(windows)]
        {
            strip_unc_prefix(env::temp_dir().join(".cc-ide"))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(format!("/tmp/.cc-ide-{}", current_euid()))
        }
    })
}

pub fn ipc_path(name: &str) -> PathBuf {
    ipc_dir().join(name)
}

/// Returns legacy `/tmp/.cc-ide` on Unix ONLY if it already exists, is a non-symlink directory
/// owned by `euid` (tightening permissions to `0o700`), and differs from `ipc_dir()`.
pub fn legacy_ipc_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let leg = PathBuf::from("/tmp/.cc-ide");
        if leg == ipc_dir() {
            return None;
        }
        let meta = fs::symlink_metadata(&leg).ok()?;
        let ft = meta.file_type();
        if ft.is_symlink() || !ft.is_dir() || meta.uid() != current_euid() {
            return None;
        }
        if (meta.mode() & 0o077) != 0 {
            fs::set_permissions(&leg, fs::Permissions::from_mode(0o700)).ok()?;
        }
        Some(leg)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ipc_dir_rejects_symlinks_and_enforces_0700() {
        let base = env::temp_dir().join(format!("ccide_ipc_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("valid_ipc");
        let euid = current_euid();
        assert!(validate_or_prepare_unix_ipc_dir(&dir, euid).is_ok());

        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(validate_or_prepare_unix_ipc_dir(&dir, euid).is_ok());
            let mode = fs::symlink_metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            assert!(validate_or_prepare_unix_ipc_dir(&dir, euid.wrapping_add(1)).is_err());

            let link = base.join("symlink_ipc");
            symlink(&dir, &link).unwrap();
            assert!(validate_or_prepare_unix_ipc_dir(&link, euid).is_err());
        }
        let _ = fs::remove_dir_all(&base);
    }
}
