//! Palette-Unified Syntax Highlighting Model & Atomic Sync (`<= 300` lines).

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::json::extract_json_str;
use crate::platform::atomic_write;

use super::artifacts::{
    build_ghostty_theme, build_glow_style_json, build_syntax_resolved_json, build_tm_theme,
};
use super::companions::sync_companion_themes;
use super::{ColorSpec, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    Current,
    Synced,
    Busy,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxPalette {
    pub keyword: ColorSpec,
    pub function: ColorSpec,
    pub type_name: ColorSpec,
    pub string: ColorSpec,
    pub number: ColorSpec,
    pub comment: ColorSpec,
    pub operator: ColorSpec,
    pub variable: ColorSpec,
    pub code_bg: ColorSpec,
    pub diff_add_fg: ColorSpec,
    pub diff_add_bg: ColorSpec,
    pub diff_del_fg: ColorSpec,
    pub diff_del_bg: ColorSpec,
}

impl SyntaxPalette {
    pub(crate) fn apply_overrides(&mut self, clean: &str) {
        const KEYS: &str = "syn_keyword syn_function syn_type syn_string syn_number syn_comment syn_operator syn_variable syn_code_bg syn_diff_add_fg syn_diff_add_bg syn_diff_del_fg syn_diff_del_bg";
        let mut fields = [
            &mut self.keyword,
            &mut self.function,
            &mut self.type_name,
            &mut self.string,
            &mut self.number,
            &mut self.comment,
            &mut self.operator,
            &mut self.variable,
            &mut self.code_bg,
            &mut self.diff_add_fg,
            &mut self.diff_add_bg,
            &mut self.diff_del_fg,
            &mut self.diff_del_bg,
        ];
        for (slot, k) in fields.iter_mut().zip(KEYS.split_ascii_whitespace()) {
            if let Some(v) = super::extract_color_field(clean, k) {
                **slot = v;
            }
        }
    }
}

fn compute_digest(pal: &Palette) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in b"v2:" {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    for &b in pal.preset.as_bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let s = &pal.syntax;
    let syn_all = [
        s.keyword,
        s.function,
        s.type_name,
        s.string,
        s.number,
        s.comment,
        s.operator,
        s.variable,
        s.code_bg,
        s.diff_add_fg,
        s.diff_add_bg,
        s.diff_del_fg,
        s.diff_del_bg,
    ];
    for c in pal.ui_specs.iter().chain(syn_all.iter()) {
        let (r, g, b) = c.to_rgb_approx();
        for x in [r, g, b] {
            h = (h ^ x as u64).wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:016x}")
}

struct SyncLockGuard(PathBuf);

impl Drop for SyncLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn acquire_sync_lock(cfg_dir: &Path) -> Option<SyncLockGuard> {
    let lock_path = cfg_dir.join(".sync.lock");
    for _ in 0..40 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                return Some(SyncLockGuard(lock_path));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&lock_path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .map(|d| d > Duration::from_secs(15))
                    .unwrap_or(false);
                if stale {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
    None
}

fn sync_cfg_dir(home: &str) -> PathBuf {
    let non_empty = |k| {
        env::var(k)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let default_dir = Path::new(home).join(".config/cc-ide");
    if let Some(pf) = non_empty("CCIDE_PALETTE_FILE") {
        return Path::new(&pf)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(default_dir);
    }
    if let Some(xdg) = non_empty("XDG_CONFIG_HOME") {
        return Path::new(&xdg).join("cc-ide");
    }
    default_dir
}

fn artifacts_current(resolved: &Path, files: &[&PathBuf; 5], digest: &str) -> bool {
    files.iter().all(|p| p.is_file())
        && fs::read_to_string(resolved)
            .ok()
            .and_then(|t| extract_json_str(&t, "digest"))
            .as_deref()
            == Some(digest)
}

pub fn sync_syntax_artifacts(home: &str, pal: &Palette) -> SyncOutcome {
    let cfg_dir = sync_cfg_dir(home);
    let digest = compute_digest(pal);
    let resolved_path = cfg_dir.join("syntax_resolved.json");
    let (tm_p, glow_p, gh_p, yz_p, lg_p) = (
        cfg_dir.join("ccide.tmTheme"),
        cfg_dir.join("glow-style.json"),
        cfg_dir.join("ghostty-theme.conf"),
        cfg_dir.join("yazi-theme.toml"),
        cfg_dir.join("lazygit.yml"),
    );
    let all_files = [&tm_p, &glow_p, &gh_p, &yz_p, &lg_p];
    if artifacts_current(&resolved_path, &all_files, &digest) {
        return SyncOutcome::Current;
    }
    if fs::create_dir_all(&cfg_dir).is_err() {
        return SyncOutcome::Failed;
    }
    let Some(_guard) = acquire_sync_lock(&cfg_dir) else {
        return SyncOutcome::Busy;
    };
    if artifacts_current(&resolved_path, &all_files, &digest) {
        return SyncOutcome::Current;
    }
    let tm_txt = build_tm_theme(pal);
    let core_ok = atomic_write(&tm_p, tm_txt.as_bytes()).is_ok()
        && atomic_write(&glow_p, build_glow_style_json(pal).as_bytes()).is_ok()
        && atomic_write(&gh_p, build_ghostty_theme(pal).as_bytes()).is_ok();
    if !core_ok {
        return SyncOutcome::Failed;
    }
    let (bat_theme, comp_ok) = sync_companion_themes(home, &cfg_dir, pal, &tm_p, &tm_txt);
    if !comp_ok {
        return SyncOutcome::Failed;
    }
    if atomic_write(
        &resolved_path,
        build_syntax_resolved_json(pal, &digest, bat_theme).as_bytes(),
    )
    .is_err()
    {
        return SyncOutcome::Failed;
    }
    SyncOutcome::Synced
}
