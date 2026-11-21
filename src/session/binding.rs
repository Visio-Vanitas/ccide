//! Pure Session Binding Selection & Atomic Binding Persistence (`src/session/binding.rs`).

use std::fs;
use std::path::Path;

use crate::platform::{atomic_write, paths_equal_ci};
use crate::session::agent::AgentKind;
use crate::session::scan::SessEntry;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionBinding {
    pub session_id: String,
    pub ui_pid: u32,
    pub data_pid: u32,
    pub agent: AgentKind,
    pub transcript_path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BindingChoice {
    pub binding: SessionBinding,
    pub should_commit: bool,
}

fn matches_title(cand: &SessEntry, title: &str) -> bool {
    cand.name == title
        || (!cand.job_id.is_empty() && cand.job_id == title)
        || (title.len() >= 8 && cand.session_id.starts_with(title))
}

/// Pure binding selector: resolves FleetView (`ui_pid` vs `data_pid`), title heuristics,
/// sibling claim guard (`F8`), and case-insensitive cwd fallback (`F9`) without side effects.
#[allow(clippy::too_many_arguments)]
pub fn choose_binding<FAnc, FTitle, FClaim, FBound>(
    entries: &[SessEntry],
    tab_yazi_id: &str,
    canon_target: &Path,
    shell_pid: u32,
    shell_pid_alive: bool,
    ancestor_dist: FAnc,
    read_title: FTitle,
    is_claimed: FClaim,
    is_bound_to_tab: FBound,
) -> BindingChoice
where
    FAnc: Fn(u32) -> Option<usize>,
    FTitle: Fn(bool) -> Option<String>,
    FClaim: Fn(&str) -> bool,
    FBound: Fn(&str) -> bool,
{
    if shell_pid > 0 {
        if let Some((_dist, e)) = entries
            .iter()
            .filter_map(|e| ancestor_dist(e.pid).map(|d| (d, e)))
            .min_by_key(|(d, e)| {
                (
                    *d,
                    !e.parked_job_id.is_empty(),
                    std::cmp::Reverse((e.is_busy, e.updated_at, e.started_at)),
                )
            })
        {
            let mut chosen = e;
            let is_viewer = !e.parked_job_id.is_empty();
            if let Some(title) = read_title(is_viewer) {
                if !(e.parked_job_id.is_empty() && matches_title(e, &title)) {
                    if let Some(bg_same_cwd) = entries
                        .iter()
                        .find(|c| {
                            c.kind == "bg"
                                && paths_equal_ci(&c.cwd, &e.cwd)
                                && matches_title(c, &title)
                                && !is_claimed(&c.session_id)
                        })
                        .or_else(|| {
                            entries.iter().find(|c| {
                                c.kind == "bg"
                                    && paths_equal_ci(&c.cwd, &e.cwd)
                                    && matches_title(c, &title)
                            })
                        })
                    {
                        chosen = bg_same_cwd;
                    } else if is_viewer {
                        if let Some(bg_any) = entries
                            .iter()
                            .find(|c| {
                                c.kind == "bg"
                                    && matches_title(c, &title)
                                    && !is_claimed(&c.session_id)
                            })
                            .or_else(|| {
                                entries
                                    .iter()
                                    .find(|c| c.kind == "bg" && matches_title(c, &title))
                            })
                        {
                            chosen = bg_any;
                        }
                    }
                }
            }
            if chosen.pid == e.pid && is_viewer {
                if let Some(bg_fb) = entries.iter().find(|c| {
                    c.kind == "bg" && paths_equal_ci(&c.cwd, &e.cwd) && !is_claimed(&c.session_id)
                }) {
                    chosen = bg_fb;
                }
            }
            return BindingChoice {
                binding: SessionBinding {
                    session_id: chosen.session_id.clone(),
                    ui_pid: e.pid,
                    data_pid: chosen.pid,
                    ..Default::default()
                },
                should_commit: !tab_yazi_id.is_empty(),
            };
        }

        if shell_pid_alive {
            if let Some(title) = read_title(false) {
                if let Some(bg) = entries.iter().find(|c| {
                    c.kind == "bg"
                        && paths_equal_ci(&c.cwd, canon_target)
                        && matches_title(c, &title)
                        && !is_claimed(&c.session_id)
                }) {
                    return BindingChoice {
                        binding: SessionBinding {
                            session_id: bg.session_id.clone(),
                            ui_pid: bg.pid,
                            data_pid: bg.pid,
                            ..Default::default()
                        },
                        should_commit: false,
                    };
                }
            }
            return BindingChoice::default();
        }
    }

    if !tab_yazi_id.is_empty() {
        for e in entries {
            if e.parked_job_id.is_empty() && is_bound_to_tab(&e.session_id) {
                return BindingChoice {
                    binding: SessionBinding {
                        session_id: e.session_id.clone(),
                        ui_pid: e.pid,
                        data_pid: e.pid,
                        ..Default::default()
                    },
                    should_commit: false,
                };
            }
        }
    }

    for e in entries {
        if paths_equal_ci(&e.cwd, canon_target)
            && e.parked_job_id.is_empty()
            && !is_claimed(&e.session_id)
        {
            return BindingChoice {
                binding: SessionBinding {
                    session_id: e.session_id.clone(),
                    ui_pid: e.pid,
                    data_pid: e.pid,
                    ..Default::default()
                },
                should_commit: !tab_yazi_id.is_empty(),
            };
        }
    }

    BindingChoice::default()
}

/// Persists `session_<sid>.yazi = tab_yazi_id` atomically only when the value changed.
pub fn commit_binding(cache_dir: &Path, session_id: &str, tab_yazi_id: &str) {
    if session_id.is_empty() || tab_yazi_id.is_empty() {
        return;
    }
    let path = cache_dir.join(format!("session_{}.yazi", session_id));
    if let Ok(existing) = fs::read_to_string(&path) {
        if existing.trim() == tab_yazi_id {
            return;
        }
    }
    let _ = fs::create_dir_all(cache_dir);
    let _ = atomic_write(&path, tab_yazi_id.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk_entry(pid: u32, sid: &str, cwd: &str, kind: &str, parked: &str, name: &str) -> SessEntry {
        SessEntry {
            pid,
            session_id: sid.to_string(),
            cwd: PathBuf::from(cwd),
            kind: kind.to_string(),
            name: name.to_string(),
            job_id: String::new(),
            parked_job_id: parked.to_string(),
            is_busy: false,
            updated_at: 100,
            started_at: 50,
        }
    }

    #[test]
    fn choose_binding_covers_fleetview_sibling_guard_cwd_and_title() {
        let entries = vec![
            mk_entry(101, "viewer-sid", "/repo", "fg", "job-1", ""),
            mk_entry(202, "bg-claimed", "/repo", "bg", "", "worker-a"),
            mk_entry(303, "bg-free-1234", "/repo", "bg", "", "worker-b"),
        ];
        // FleetView: viewer (101) under shell 10 switches data_pid to matching bg session (303)
        let c1 = choose_binding(
            &entries,
            "y1",
            Path::new("/repo"),
            10,
            true,
            |pid| (pid == 101).then_some(1),
            |qt| qt.then(|| "worker-b".to_string()),
            |sid| sid == "bg-claimed",
            |_| false,
        );
        assert_eq!(
            c1.binding,
            SessionBinding {
                session_id: "bg-free-1234".into(),
                ui_pid: 101,
                data_pid: 303,
                ..Default::default()
            }
        );
        assert!(c1.should_commit);

        // Sibling guard (F8) + CWD fallback (F9) skips claimed entry and viewer entry
        let fg_entries = vec![
            mk_entry(101, "viewer-sid", "/repo", "fg", "job-1", ""),
            mk_entry(404, "fg-claimed", "/repo", "fg", "", ""),
            mk_entry(505, "fg-free", "/repo/", "fg", "", ""),
        ];
        let c2 = choose_binding(
            &fg_entries,
            "y2",
            Path::new("/repo"),
            0,
            false,
            |_| None,
            |_| None,
            |sid| sid == "fg-claimed",
            |_| false,
        );
        assert_eq!(c2.binding.session_id, "fg-free");
        assert_eq!(c2.binding.ui_pid, 505);
        assert!(c2.should_commit);

        // Live shell without descendant uses title heuristic + prefix match
        let c3 = choose_binding(
            &entries,
            "y3",
            Path::new("/repo"),
            99,
            true,
            |_| None,
            |_| Some("bg-free-".to_string()),
            |sid| sid == "bg-claimed",
            |_| false,
        );
        assert_eq!(c3.binding.session_id, "bg-free-1234");
        assert!(!c3.should_commit);
    }
}
