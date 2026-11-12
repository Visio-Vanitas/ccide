//! Live Session Task Progress Panel (`TASKS ⇄ HW`) for `ccide-core sidebar` (`Panel 3`).
//!
//! Reads Claude Code's native per-session task directory (`~/.claude/tasks/<sessionId>/*.json`)
//! with an `mtime`-gated cache (`1` `stat` syscall when idle, zero binary patches required):
//! - Automatically replaces `Panel 3` when the isolated session has incomplete (`pending` /
//!   `in_progress`) tasks, and automatically hands `Panel 3` back to the Hardware Telemetry
//!   oscilloscope as soon as all tasks reach `completed`.
//! - Supports manual click/keyboard toggling (`[HW]` ⇄ `[TASK]` on Panel 3's header/footer or `t`)
//!   and bounded mouse-wheel scrolling when task rows exceed `Panel 3`'s height.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::json::extract_json_str;
use crate::tui::grid::{BoxRow, PanelSpec};
use crate::tui::palette::Palette;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::{BOLD, RESET, SPINNERS};

pub const TAG_TASK_ROW: u8 = 10;
pub const TAG_TASK_TOGGLE: u8 = 11;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTask {
    pub id_num: u64,
    pub id: String,
    pub subject: String,
    pub active_form: String,
    pub status: String,
}

impl SessionTask {
    pub fn is_completed(&self) -> bool {
        self.status == "completed"
    }

    pub fn is_in_progress(&self) -> bool {
        self.status == "in_progress"
    }
}

pub struct SessionTaskCache {
    last_session_id: String,
    last_dir_mtime: Option<SystemTime>,
    tasks: Vec<SessionTask>,
}

impl SessionTaskCache {
    pub fn new() -> Self {
        Self {
            last_session_id: String::new(),
            last_dir_mtime: None,
            tasks: Vec::new(),
        }
    }

    pub fn refresh(&mut self, home: &str, session_id: &str, force: bool) -> &[SessionTask] {
        if session_id.is_empty() {
            self.last_session_id.clear();
            self.last_dir_mtime = None;
            self.tasks.clear();
            return &self.tasks;
        }

        let dir_path = format!("{}/.claude/tasks/{}", home, session_id);
        let Ok(meta) = fs::metadata(&dir_path) else {
            self.last_session_id = session_id.to_string();
            self.last_dir_mtime = None;
            self.tasks.clear();
            return &self.tasks;
        };

        let cur_mtime = meta.modified().ok();
        if !force
            && self.last_session_id == session_id
            && cur_mtime.is_some()
            && cur_mtime == self.last_dir_mtime
        {
            return &self.tasks;
        }

        self.last_session_id = session_id.to_string();
        self.last_dir_mtime = cur_mtime;
        self.tasks = read_tasks_from_dir(Path::new(&dir_path));
        &self.tasks
    }
}

pub fn read_tasks_from_dir(dir: &Path) -> Vec<SessionTask> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname.starts_with('.') || !fname.ends_with(".json") {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&p) else {
            continue;
        };
        let status = extract_json_str(&raw, "status").unwrap_or_else(|| "pending".to_string());
        if status == "deleted" {
            continue;
        }
        let subject = extract_json_str(&raw, "subject").unwrap_or_default();
        if subject.trim().is_empty() {
            continue;
        }
        let stem = fname.trim_end_matches(".json");
        let id = extract_json_str(&raw, "id").unwrap_or_else(|| stem.to_string());
        let id_num = id
            .parse::<u64>()
            .or_else(|_| stem.parse::<u64>())
            .unwrap_or(u64::MAX);
        let active_form = extract_json_str(&raw, "activeForm").unwrap_or_default();
        items.push(SessionTask {
            id_num,
            id,
            subject,
            active_form,
            status,
        });
    }
    items.sort_by(|a, b| a.id_num.cmp(&b.id_num).then_with(|| a.id.cmp(&b.id)));
    items
}

pub fn build_tasks_panel(
    tasks: &[SessionTask],
    cols: usize,
    bot_h: usize,
    scroll_offset: &mut usize,
    tick: usize,
    pal: &Palette,
) -> PanelSpec {
    let box_inner_w = cols.saturating_sub(2).max(12);
    let content_w = cols.saturating_sub(4).max(10);
    let interior_h = bot_h.saturating_sub(2).max(3);

    let total = tasks.len();
    let done_cnt = tasks.iter().filter(|t| t.is_completed()).count();
    let run_cnt = tasks.iter().filter(|t| t.is_in_progress()).count();

    let dim = &pal.dim;
    let white = &pal.text;
    let cyan = &pal.cyan;
    let green = &pal.green;
    let amber = &pal.amber;

    let mut body_rows: Vec<BoxRow> = Vec::with_capacity(total * 2);
    for (idx, t) in tasks.iter().enumerate() {
        if t.is_completed() {
            let subj = trunc_to_width(&t.subject, content_w.saturating_sub(2));
            body_rows.push(BoxRow {
                text: format!(" {green}✓{RESET} {dim}{subj}{RESET}"),
                bg: String::new(),
                tag: Some((TAG_TASK_ROW, idx)),
            });
        } else if t.is_in_progress() {
            let spin = SPINNERS[tick % SPINNERS.len()];
            let bg = &pal.active_view_bg;
            let subj = trunc_to_width(&t.subject, content_w.saturating_sub(2));
            body_rows.push(BoxRow {
                text: format!(" {cyan}{spin}{RESET}{bg} {BOLD}{white}{subj}{RESET}{bg}"),
                bg: bg.clone(),
                tag: Some((TAG_TASK_ROW, idx)),
            });
            if !t.active_form.trim().is_empty() && t.active_form.trim() != t.subject.trim() {
                let sub = trunc_to_width(t.active_form.trim(), content_w.saturating_sub(5));
                body_rows.push(BoxRow {
                    text: format!("   {amber}└─ {sub}{RESET}{bg}"),
                    bg: bg.clone(),
                    tag: Some((TAG_TASK_ROW, idx)),
                });
            }
        } else {
            let subj = trunc_to_width(&t.subject, content_w.saturating_sub(2));
            body_rows.push(BoxRow {
                text: format!(" {dim}○{RESET} {white}{subj}{RESET}"),
                bg: String::new(),
                tag: Some((TAG_TASK_ROW, idx)),
            });
        }
    }

    // Reserve 1 bottom row for the progress bar + `[HW]` toggle action
    let body_viewport_h = interior_h.saturating_sub(1).max(1);
    let max_scroll = body_rows.len().saturating_sub(body_viewport_h);
    if *scroll_offset > max_scroll {
        *scroll_offset = max_scroll;
    }

    let mut rows: Vec<BoxRow> = Vec::with_capacity(interior_h);
    for r in body_rows
        .into_iter()
        .skip(*scroll_offset)
        .take(body_viewport_h)
    {
        rows.push(r);
    }

    while rows.len() + 1 < interior_h {
        rows.push(BoxRow {
            text: String::new(),
            bg: String::new(),
            tag: None,
        });
    }

    // Bottom progress bar + `[HW]` switch button
    let pct = (done_cnt * 100).checked_div(total).unwrap_or(100);
    let tail_plain = format!(" {:>3}% [HW]", pct);
    let tail_w = display_width(&tail_plain);
    let bar_w = box_inner_w.saturating_sub(tail_w + 2).max(4);
    let filled = ((done_cnt * bar_w) + (total / 2))
        .checked_div(total)
        .unwrap_or(bar_w)
        .min(bar_w);
    let empty = bar_w.saturating_sub(filled);
    let bar_col = if done_cnt == total { green } else { cyan };
    let prog_line = format!(
        " {bar_col}{}{dim}{}{RESET} {cyan}{:>3}%{RESET} {dim}[{amber}HW{dim}]{RESET}",
        "━".repeat(filled),
        "░".repeat(empty),
        pct
    );
    rows.push(BoxRow {
        text: if display_width(&prog_line) <= box_inner_w {
            prog_line
        } else {
            format!(" {cyan}{done_cnt}/{total}{RESET} {dim}[{amber}HW{dim}]{RESET}")
        },
        bg: String::new(),
        tag: Some((TAG_TASK_TOGGLE, 0)),
    });

    let badge = if max_scroll > 0 {
        format!("{done_cnt}/{total} ↕ · [HW]")
    } else {
        format!("{done_cnt}/{total} · [HW]")
    };
    let accent = if run_cnt > 0 {
        cyan.clone()
    } else if done_cnt == total && total > 0 {
        green.clone()
    } else {
        amber.clone()
    };

    PanelSpec {
        title: "TASKS".to_string(),
        badge,
        accent,
        rows,
        fixed_height: Some(bot_h),
        weight: 1.12,
        raw_interior: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn loads_and_sorts_session_tasks_skipping_deleted() {
        let uniq = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1);
        let tmp_dir = std::env::temp_dir().join(format!("ccide_task_test_{}", uniq));
        let _ = fs::create_dir_all(&tmp_dir);
        let _ = fs::write(
            tmp_dir.join("10.json"),
            r#"{"id":"10","subject":"Second task","activeForm":"Running second","status":"in_progress"}"#,
        );
        let _ = fs::write(
            tmp_dir.join("2.json"),
            r#"{"id":"2","subject":"First task","activeForm":"","status":"completed"}"#,
        );
        let _ = fs::write(
            tmp_dir.join("3.json"),
            r#"{"id":"3","subject":"Removed","status":"deleted"}"#,
        );
        let _ = fs::write(tmp_dir.join(".highwatermark"), "10");

        let loaded = read_tasks_from_dir(&tmp_dir);
        let _ = fs::remove_dir_all(&tmp_dir);

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "2");
        assert!(loaded[0].is_completed());
        assert_eq!(loaded[1].id, "10");
        assert!(loaded[1].is_in_progress());
    }

    #[test]
    fn builds_tasks_panel_fitting_exact_interior_height() {
        let pal = Palette::default_palette();
        let tasks = vec![
            SessionTask {
                id_num: 1,
                id: "1".to_string(),
                subject: "Clone & analyze upstream repos".to_string(),
                active_form: String::new(),
                status: "completed".to_string(),
            },
            SessionTask {
                id_num: 2,
                id: "2".to_string(),
                subject: "Implement live TASKS ⇄ HW panel".to_string(),
                active_form: "Building tasks.rs".to_string(),
                status: "in_progress".to_string(),
            },
            SessionTask {
                id_num: 3,
                id: "3".to_string(),
                subject: "Run CI gate".to_string(),
                active_form: String::new(),
                status: "pending".to_string(),
            },
        ];
        let mut sc = 0usize;
        let spec = build_tasks_panel(&tasks, 38, 14, &mut sc, 0, &pal);
        assert_eq!(spec.title, "TASKS");
        assert_eq!(spec.rows.len(), 12);
        assert_eq!(
            spec.rows.last().and_then(|r| r.tag),
            Some((TAG_TASK_TOGGLE, 0))
        );
    }
}
