//! Masked Pointer Action & Subagent Dismissal Helpers (`src/commands/sidebar/mouse_actions.rs`).

use std::time::{Duration, Instant};

use crate::commands::sidebar::hit_mask::{
    FocusZone, MaskedAction, PointerCapture, SidebarGeomMask,
};
use crate::commands::sidebar::ipc::{trigger_native_view_switch, trigger_yazi_reveal};
use crate::session::emit_yazi_blur;
use crate::session::transcript::{append_dismissed_subagents, SubagentItem};

pub struct DispatchEnv<'a> {
    pub home: &'a str,
    pub yazi_id: &'a str,
    pub cwd: &'a str,
    pub sid: &'a str,
    pub cpid: u32,
    pub main_file: &'a str,
    pub main_line: usize,
    pub main_span: usize,
    pub viewport_h: usize,
    pub cost_max_scroll: usize,
    pub hit_mask: SidebarGeomMask,
}

impl<'a> DispatchEnv<'a> {
    #[allow(clippy::too_many_arguments)]
    fn apply_switch(
        &self,
        vid: &str,
        rtype: &str,
        desc: &str,
        done: bool,
        rank: usize,
        fp: &str,
        ln: usize,
        sp: usize,
        active_view: &mut String,
        last_click: &mut Instant,
    ) {
        *active_view = vid.to_string();
        *last_click = Instant::now();
        trigger_native_view_switch(
            self.home,
            self.yazi_id,
            self.sid,
            self.cpid,
            vid,
            rtype,
            desc,
            done,
            rank,
        );
        if fp.is_empty() {
            emit_yazi_blur(self.home, self.yazi_id);
        } else {
            trigger_yazi_reveal(self.home, self.yazi_id, self.cwd, fp, ln, sp);
        }
    }

    pub(crate) fn switch_to_main(&self, active_view: &mut String, last_click: &mut Instant) {
        self.apply_switch(
            "main",
            "",
            "",
            false,
            0,
            self.main_file,
            self.main_line,
            self.main_span,
            active_view,
            last_click,
        );
    }

    pub(crate) fn switch_to_agent(
        &self,
        ag: &SubagentItem,
        active_view: &mut String,
        last_click: &mut Instant,
    ) {
        self.apply_switch(
            &ag.full_id,
            &ag.raw_type,
            &ag.desc,
            ag.status != "running",
            ag.footer_rank,
            &ag.file,
            ag.line,
            ag.span,
            active_view,
            last_click,
        );
    }
}

pub struct SidebarSelectState {
    /// Position-driven panel selection (`None` until the user clicks a panel; cleared on `FocusOut`).
    pub focus_zone: Option<FocusZone>,
    pub pane_has_focus: bool,
    pub sub_cursor: Option<usize>,
    pub sub_scroll: usize,
    pub cost_scroll: usize,
    pub cost_lines: u8,
    pub show_context: bool,
    pub pointer: PointerCapture,
    pub last_click_instant: Instant,
}

impl Default for SidebarSelectState {
    fn default() -> Self {
        Self {
            focus_zone: None,
            pane_has_focus: false,
            sub_cursor: None,
            sub_scroll: 0,
            cost_scroll: 0,
            cost_lines: 3,
            show_context: false,
            pointer: PointerCapture::default(),
            last_click_instant: Instant::now() - Duration::from_secs(10),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn apply_masked_action(
    act: MaskedAction,
    env: &DispatchEnv<'_>,
    subagents: &[SubagentItem],
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    dismiss_idx: &mut Option<usize>,
    clear_all_done: &mut bool,
    restore_focus: &mut bool,
    switched_view: &mut bool,
) {
    match act {
        MaskedAction::SelectMain => {
            sel.pane_has_focus = false;
            sel.focus_zone = None;
            sel.sub_cursor = None;
            env.switch_to_main(active_view_id, &mut sel.last_click_instant);
            *switched_view = true;
        }
        MaskedAction::SelectSubagent(idx) => {
            if let Some(ag) = subagents.get(idx) {
                sel.pane_has_focus = false;
                sel.focus_zone = None;
                sel.sub_cursor = None;
                env.switch_to_agent(ag, active_view_id, &mut sel.last_click_instant);
                *switched_view = true;
            }
        }
        MaskedAction::DismissSubagent(idx) => {
            sel.pane_has_focus = true;
            sel.focus_zone = Some(FocusZone::Subagents);
            *dismiss_idx = Some(idx);
            emit_yazi_blur(env.home, env.yazi_id);
        }
        MaskedAction::ClearAllDone => {
            sel.pane_has_focus = true;
            sel.focus_zone = Some(FocusZone::Subagents);
            *clear_all_done = true;
            emit_yazi_blur(env.home, env.yazi_id);
        }
        MaskedAction::ToggleBottomPanel => {
            sel.pane_has_focus = true;
            sel.show_context = !sel.show_context;
            sel.sub_cursor = None;
            sel.focus_zone = Some(if sel.show_context {
                FocusZone::Context
            } else {
                FocusZone::Telemetry
            });
            emit_yazi_blur(env.home, env.yazi_id);
        }
        MaskedAction::FocusZoneOnly(z) => {
            sel.pane_has_focus = z.is_some();
            sel.focus_zone = z;
            if z != Some(FocusZone::Subagents) {
                sel.sub_cursor = None;
            }
            if z.is_some() {
                emit_yazi_blur(env.home, env.yazi_id);
            }
        }
        MaskedAction::RestoreCenterFocusOnly => {
            sel.pane_has_focus = false;
            sel.focus_zone = None;
            sel.sub_cursor = None;
            *restore_focus = true;
        }
    }
}

pub fn dismiss_single_completed(
    d_idx: usize,
    env: &DispatchEnv<'_>,
    subagents: &mut Vec<SubagentItem>,
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    switched_view: &mut bool,
) {
    if let Some(ag) = subagents.get(d_idx) {
        if ag.status != "running" {
            let dismissed_id = ag.full_id.clone();
            append_dismissed_subagents(env.home, env.sid, std::slice::from_ref(&dismissed_id));
            subagents.remove(d_idx);
            sel.sub_cursor = sel.sub_cursor.map(|c| c.min(subagents.len()));
            if *active_view_id == dismissed_id {
                env.switch_to_main(active_view_id, &mut sel.last_click_instant);
                *switched_view = true;
            }
        }
    }
}

pub fn dismiss_all_completed(
    env: &DispatchEnv<'_>,
    subagents: &mut Vec<SubagentItem>,
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    switched_view: &mut bool,
) {
    let done_ids: Vec<String> = subagents
        .iter()
        .filter(|a| a.status != "running")
        .map(|a| a.full_id.clone())
        .collect();
    if !done_ids.is_empty() {
        let reset_main = done_ids.iter().any(|id| id == active_view_id);
        append_dismissed_subagents(env.home, env.sid, &done_ids);
        subagents.retain(|a| a.status == "running");
        sel.sub_scroll = 0;
        sel.sub_cursor = sel.sub_cursor.map(|c| c.min(subagents.len()));
        if reset_main {
            env.switch_to_main(active_view_id, &mut sel.last_click_instant);
            *switched_view = true;
        }
    }
}
