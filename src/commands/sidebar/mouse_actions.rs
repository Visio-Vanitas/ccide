//! Masked Pointer Action Dispatcher (`src/commands/sidebar/mouse_actions.rs`).

use std::cell::Cell;
use std::time::{Duration, Instant};

pub use crate::commands::sidebar::dismiss::{dismiss_all_completed, dismiss_single_completed};
use crate::commands::sidebar::hit_mask::{
    FocusZone, MaskedAction, PointerCapture, SidebarGeomMask,
};
use crate::commands::sidebar::ipc::{
    spawn_center_focus_restore, trigger_native_view_switch, trigger_yazi_reveal,
    write_active_view_state,
};
use crate::orchestrator::{OrchestratorConfig, ALL_ROLES};
use crate::session::transcript::SubagentItem;
use crate::session::{emit_yazi_blur, is_subagent_alive, AgentKind};

pub struct DispatchEnv<'a> {
    pub home: &'a str,
    pub yazi_id: &'a str,
    pub cwd: &'a str,
    pub sid: &'a str,
    pub cpid: u32,
    pub agent: AgentKind,
    pub main_file: &'a str,
    pub main_line: usize,
    pub main_span: usize,
    pub viewport_h: usize,
    pub cost_max_scroll: usize,
    pub hit_mask: SidebarGeomMask,
    pub switch_signal_ok: Cell<Option<bool>>,
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
        if !self.agent.supports_view_switch() {
            *active_view = vid.to_string();
            write_active_view_state(self.home, self.yazi_id, vid);
            *last_click = Instant::now();
            self.switch_signal_ok.set(Some(true));
            if !fp.is_empty() {
                trigger_yazi_reveal(self.home, self.yazi_id, self.cwd, fp, ln, sp);
            }
            spawn_center_focus_restore(self.home, self.yazi_id, self.sid, "main", 0);
            return;
        }
        *active_view = vid.to_string();
        *last_click = Instant::now();
        let ok = trigger_native_view_switch(
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
        self.switch_signal_ok.set(Some(ok));
        if !fp.is_empty() {
            trigger_yazi_reveal(self.home, self.yazi_id, self.cwd, fp, ln, sp);
        }
    }

    pub(crate) fn sync_notice(&self, notice: &mut Option<String>) {
        if self.agent.supports_view_switch() {
            if let Some(ok) = self.switch_signal_ok.get() {
                *notice = (!ok).then(|| "repatch needed (ccide doctor)".to_string());
            }
        } else {
            *notice = None;
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
            !is_subagent_alive(ag.status),
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
    pub focus_zone: Option<FocusZone>,
    pub pane_has_focus: bool,
    pub sub_cursor: Option<usize>,
    pub sub_scroll: usize,
    pub cost_scroll: usize,
    pub cost_lines: u8,
    pub show_context: bool,
    pub show_orchestrator: bool,
    pub orch_cursor: Option<usize>,
    pub pointer: PointerCapture,
    pub last_click_instant: Instant,
    pub status_notice: Option<String>,
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
            show_orchestrator: false,
            orch_cursor: None,
            pointer: PointerCapture::default(),
            last_click_instant: Instant::now() - Duration::from_secs(10),
            status_notice: None,
        }
    }
}

impl SidebarSelectState {
    fn focus_sub(&mut self, env: &DispatchEnv<'_>) {
        self.pane_has_focus = true;
        self.focus_zone = Some(FocusZone::Subagents);
        emit_yazi_blur(env.home, env.yazi_id);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn apply_masked_action(
    act: MaskedAction,
    env: &DispatchEnv<'_>,
    subagents: &[SubagentItem],
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    orch_cfg: &mut OrchestratorConfig,
    dismiss_idx: &mut Option<usize>,
    clear_all_done: &mut bool,
    restore_focus: &mut bool,
    switched_view: &mut bool,
) {
    match act {
        MaskedAction::SelectMain => {
            (sel.pane_has_focus, sel.focus_zone, sel.sub_cursor) = (false, None, None);
            env.switch_to_main(active_view_id, &mut sel.last_click_instant);
            env.sync_notice(&mut sel.status_notice);
            *switched_view = true;
        }
        MaskedAction::SelectSubagent(idx) => {
            if let Some(ag) = subagents.get(idx) {
                (sel.pane_has_focus, sel.focus_zone, sel.sub_cursor) = (false, None, None);
                env.switch_to_agent(ag, active_view_id, &mut sel.last_click_instant);
                env.sync_notice(&mut sel.status_notice);
                *switched_view = true;
            }
        }
        MaskedAction::DismissSubagent(idx) => {
            sel.focus_sub(env);
            *dismiss_idx = Some(idx);
        }
        MaskedAction::ClearAllDone => {
            sel.focus_sub(env);
            *clear_all_done = true;
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
        MaskedAction::ToggleOrchestrator => {
            sel.focus_sub(env);
            sel.show_orchestrator = !sel.show_orchestrator;
            (sel.sub_cursor, sel.orch_cursor) = (None, None);
        }
        MaskedAction::OrchCyclePreset(fwd) => {
            sel.focus_sub(env);
            sel.orch_cursor = Some(0);
            orch_cfg.cycle_preset(fwd);
            orch_cfg.save(env.home);
        }
        MaskedAction::OrchCycleModel(idx, fwd) => {
            sel.focus_sub(env);
            if let Some(&role) = ALL_ROLES.get(idx) {
                sel.orch_cursor = Some(idx + 1);
                orch_cfg.cycle_role_model(role, fwd);
                orch_cfg.save(env.home);
            }
        }
        MaskedAction::OrchCycleEffort(idx, fwd) => {
            sel.focus_sub(env);
            if let Some(&role) = ALL_ROLES.get(idx) {
                sel.orch_cursor = Some(idx + 1);
                orch_cfg.cycle_role_effort(role, fwd);
                orch_cfg.save(env.home);
            }
        }
        MaskedAction::FocusZoneOnly(z) => {
            (sel.pane_has_focus, sel.focus_zone) = (z.is_some(), z);
            if z != Some(FocusZone::Subagents) {
                (sel.sub_cursor, sel.orch_cursor) = (None, None);
            }
            if z.is_some() {
                emit_yazi_blur(env.home, env.yazi_id);
            }
        }
        MaskedAction::RestoreCenterFocusOnly => {
            (sel.pane_has_focus, sel.focus_zone, sel.sub_cursor) = (false, None, None);
            *restore_focus = true;
        }
    }
}
