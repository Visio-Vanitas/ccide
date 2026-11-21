//! OpenAI Codex CLI (`codex`) TOML Config & Rollout Discovery (`src/session/codex_discover.rs`).

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::json::{extract_json_str, extract_toml_scalar};
use crate::platform::{clean_canonicalize, paths_equal_ci};
use crate::session::agent::codex_home;
use crate::session::is_valid_model_name;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodexConfig {
    pub model: Option<String>,
    pub context_window: Option<u64>,
    pub effort: Option<String>,
}

pub fn codex_config() -> CodexConfig {
    let Ok(txt) = fs::read_to_string(codex_home().join("config.toml")) else {
        return CodexConfig::default();
    };
    CodexConfig {
        model: extract_toml_scalar(&txt, "model").filter(|m| is_valid_model_name(m)),
        context_window: extract_toml_scalar(&txt, "model_context_window")
            .and_then(|s| s.parse().ok()),
        effort: extract_toml_scalar(&txt, "model_reasoning_effort").filter(|s| !s.is_empty()),
    }
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

pub(crate) fn recent_day_dirs(base: &Path) -> Vec<PathBuf> {
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let days = now_s / 86_400;
    [1, 0, -1, -2, -3]
        .iter()
        .map(|off| {
            let (y, m, d) = civil_from_days(days + off);
            base.join("sessions").join(format!("{y:04}/{m:02}/{d:02}"))
        })
        .filter(|p| p.is_dir())
        .collect()
}

pub(crate) fn read_first_line(path: &Path) -> Option<String> {
    let mut line = String::new();
    BufReader::new(File::open(path).ok()?)
        .read_line(&mut line)
        .ok()?;
    Some(line)
}

pub(crate) fn rollout_session_id(path: &Path) -> String {
    read_first_line(path)
        .and_then(|l| extract_json_str(&l, "session_id").or_else(|| extract_json_str(&l, "id")))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        })
}

fn valid_iso_date_prefix(ts: &str) -> Option<&str> {
    let b = ts.as_bytes();
    if b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
    {
        Some(&ts[..10])
    } else {
        None
    }
}

pub fn find_rollout_by_id(session_id: &str) -> Option<PathBuf> {
    if !crate::session::is_safe_id(session_id) {
        return None;
    }
    let (home, suffix) = (codex_home(), format!("-{}.jsonl", session_id));
    let mut dirs = recent_day_dirs(&home);
    if let Ok(idx_txt) = fs::read_to_string(home.join("session_index.jsonl")) {
        for line in idx_txt.lines().rev() {
            if line.contains(session_id) {
                if let Some(ymd) = extract_json_str(line, "updated_at")
                    .and_then(|t| valid_iso_date_prefix(&t).map(str::to_string))
                {
                    dirs.push(home.join("sessions").join(ymd.replace('-', "/")));
                    break;
                }
            }
        }
    }
    for dir in dirs {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(&suffix))
            {
                return Some(p);
            }
        }
    }
    None
}

pub fn find_rollout_for_cwd(cwd: &str) -> Option<PathBuf> {
    let canon_target = clean_canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    let now = SystemTime::now();
    let (mut best, mut budget): (Option<(SystemTime, PathBuf)>, usize) = (None, 400);
    for dir in recent_day_dirs(&codex_home()) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            if budget == 0 {
                return best.map(|(_, p)| p);
            }
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            budget -= 1;
            let Ok(mt) = fs::metadata(&p).and_then(|m| m.modified()) else {
                continue;
            };
            if now.duration_since(mt).unwrap_or_default() > Duration::from_secs(43_200) {
                continue;
            }
            let Some(l1) = read_first_line(&p) else {
                continue;
            };
            let is_child = extract_json_str(&l1, "parent_thread_id").is_some_and(|s| !s.is_empty());
            if !l1.contains("\"session_meta\"") || is_child {
                continue;
            }
            let Some(rcwd) = extract_json_str(&l1, "cwd") else {
                continue;
            };
            let rcanon = clean_canonicalize(&rcwd).unwrap_or_else(|_| PathBuf::from(&rcwd));
            let newer = best.as_ref().map_or(true, |(bmt, _)| mt > *bmt);
            if paths_equal_ci(&rcanon, &canon_target) && newer {
                best = Some((mt, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::valid_iso_date_prefix;

    #[test]
    fn rejects_traversal_and_non_ascii_dates_in_index() {
        assert_eq!(valid_iso_date_prefix("../../etc/foo"), None);
        assert_eq!(valid_iso_date_prefix("2026-11-1é"), None);
        assert_eq!(valid_iso_date_prefix("2026-11"), None);
        assert_eq!(
            valid_iso_date_prefix("2026-11-19T10:00:00Z"),
            Some("2026-11-19")
        );
    }
}
