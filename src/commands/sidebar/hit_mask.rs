//! Position-Based Control-Partition Hit-Mask & Single-Click Dispatcher (`src/commands/sidebar/hit_mask.rs`).
//!
//! Panel focus (`Option<FocusZone>`) is strictly driven by the clicked/scrolled coordinate:
//! - `1..=top_h` -> `InertSprite` (clears panel focus, masks drags).
//! - `top_h+1..panel3_top_row` -> `FocusZone::Subagents`.
//! - `panel3_top_row..` -> `Telemetry` (when HW active), `Context`, or `Cost` (based on `row`).

use std::collections::HashMap;

use crate::commands::sidebar::orch_panel::{TAG_ORCH_PRESET, TAG_ORCH_ROLE};
use crate::session::transcript::SubagentItem;
use crate::tui::context::TAG_CTX_TOGGLE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusZone {
    Subagents,
    Context,
    Cost,
    Telemetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitZone {
    InertSprite,
    ToggleOrchestrator,
    OrchPreset,
    OrchRoleModel(usize),
    OrchRoleEffort(usize),
    SubagentFrame,
    SubagentMain,
    SubagentItem(usize),
    SubagentDismiss(usize),
    ClearDoneFooter,
    BottomToggle(FocusZone),
    BottomFrame(FocusZone),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskedAction {
    SelectMain,
    SelectSubagent(usize),
    DismissSubagent(usize),
    ClearAllDone,
    ToggleBottomPanel,
    ToggleOrchestrator,
    OrchCyclePreset(bool),
    OrchCycleModel(usize, bool),
    OrchCycleEffort(usize, bool),
    FocusZoneOnly(Option<FocusZone>),
    RestoreCenterFocusOnly,
}

#[derive(Clone, Copy, Debug)]
pub struct SidebarGeomMask {
    pub cols: usize,
    pub top_h: usize,
    pub panel3_top_row: usize,
    pub cost_top_row: usize,
    pub show_context_panel: bool,
    pub split_ctx_cost: bool,
}

impl SidebarGeomMask {
    pub fn bottom_zone_at(&self, row: usize) -> FocusZone {
        if !self.show_context_panel {
            FocusZone::Telemetry
        } else if self.split_ctx_cost && row >= self.cost_top_row {
            FocusZone::Cost
        } else {
            FocusZone::Context
        }
    }

    pub fn classify(
        &self,
        col: usize,
        row: usize,
        row_tag_map: &HashMap<usize, (u8, usize)>,
        subagents: &[SubagentItem],
    ) -> HitZone {
        if row <= self.top_h {
            return HitZone::InertSprite;
        }
        if row >= self.panel3_top_row {
            let bz = self.bottom_zone_at(row);
            let on_hw_badge = !self.show_context_panel
                && row == self.panel3_top_row
                && col >= self.cols.saturating_sub(14);
            let on_ctx_toggle = self.show_context_panel
                && ((row == self.panel3_top_row && col >= self.cols.saturating_sub(12))
                    || (self.split_ctx_cost
                        && row == self.cost_top_row
                        && col >= self.cols.saturating_sub(10))
                    || row_tag_map
                        .get(&row)
                        .map(|&(k, _)| k == TAG_CTX_TOGGLE)
                        .unwrap_or(false));
            return if on_hw_badge || on_ctx_toggle {
                HitZone::BottomToggle(bz)
            } else {
                HitZone::BottomFrame(bz)
            };
        }

        if row == self.top_h + 1 && col >= self.cols.saturating_sub(18) {
            return HitZone::ToggleOrchestrator;
        }
        if col <= 1 || col >= self.cols {
            return HitZone::SubagentFrame;
        }

        match row_tag_map.get(&row).copied() {
            Some((1, _)) => HitZone::SubagentMain,
            Some((2, idx)) => {
                let can_dismiss = col >= self.cols.saturating_sub(4)
                    && subagents
                        .get(idx)
                        .map(|a| a.status != "running")
                        .unwrap_or(false);
                if can_dismiss {
                    HitZone::SubagentDismiss(idx)
                } else {
                    HitZone::SubagentItem(idx)
                }
            }
            Some((5, idx)) => HitZone::SubagentItem(idx),
            Some((3, _)) => HitZone::ClearDoneFooter,
            Some((TAG_ORCH_PRESET, _)) => HitZone::OrchPreset,
            Some((TAG_ORCH_ROLE, i)) if col >= self.cols.saturating_sub(9) => {
                HitZone::OrchRoleEffort(i)
            }
            Some((TAG_ORCH_ROLE, i)) => HitZone::OrchRoleModel(i),
            _ => HitZone::SubagentFrame,
        }
    }
}

#[derive(Default)]
pub struct PointerCapture {
    sprite_down: Option<(usize, usize)>,
}

impl PointerCapture {
    pub fn on_mouse_down(
        &mut self,
        btn: u8,
        col: usize,
        row: usize,
        zone: HitZone,
    ) -> Option<MaskedAction> {
        if zone == HitZone::InertSprite {
            if btn == 0 {
                self.sprite_down = Some((col, row));
            }
            return Some(MaskedAction::FocusZoneOnly(None));
        }
        self.sprite_down = None;

        if btn == 1 || btn == 2 {
            return match zone {
                HitZone::BottomToggle(_) | HitZone::BottomFrame(_) => {
                    Some(MaskedAction::ToggleBottomPanel)
                }
                HitZone::ToggleOrchestrator => Some(MaskedAction::ToggleOrchestrator),
                HitZone::OrchPreset => Some(MaskedAction::OrchCyclePreset(false)),
                HitZone::OrchRoleModel(i) => Some(MaskedAction::OrchCycleModel(i, false)),
                HitZone::OrchRoleEffort(i) => Some(MaskedAction::OrchCycleEffort(i, false)),
                HitZone::SubagentItem(idx) | HitZone::SubagentDismiss(idx) => {
                    Some(MaskedAction::DismissSubagent(idx))
                }
                HitZone::SubagentMain | HitZone::ClearDoneFooter | HitZone::SubagentFrame => {
                    Some(MaskedAction::FocusZoneOnly(Some(FocusZone::Subagents)))
                }
                HitZone::InertSprite => None,
            };
        }

        if btn != 0 {
            return None;
        }

        match zone {
            HitZone::InertSprite => None,
            HitZone::ToggleOrchestrator => Some(MaskedAction::ToggleOrchestrator),
            HitZone::OrchPreset => Some(MaskedAction::OrchCyclePreset(true)),
            HitZone::OrchRoleModel(i) => Some(MaskedAction::OrchCycleModel(i, true)),
            HitZone::OrchRoleEffort(i) => Some(MaskedAction::OrchCycleEffort(i, true)),
            HitZone::SubagentMain => Some(MaskedAction::SelectMain),
            HitZone::SubagentDismiss(idx) => Some(MaskedAction::DismissSubagent(idx)),
            HitZone::SubagentItem(idx) => Some(MaskedAction::SelectSubagent(idx)),
            HitZone::ClearDoneFooter => Some(MaskedAction::ClearAllDone),
            HitZone::BottomToggle(_) => Some(MaskedAction::ToggleBottomPanel),
            HitZone::SubagentFrame => Some(MaskedAction::FocusZoneOnly(Some(FocusZone::Subagents))),
            HitZone::BottomFrame(bz) => Some(MaskedAction::FocusZoneOnly(Some(bz))),
        }
    }

    pub fn on_mouse_up(
        &mut self,
        btn: u8,
        col: usize,
        row: usize,
        up_zone: HitZone,
    ) -> Option<MaskedAction> {
        let (down_col, down_row) = self.sprite_down.take()?;
        if btn == 0
            && up_zone == HitZone::InertSprite
            && down_row.abs_diff(row) <= 1
            && down_col.abs_diff(col) <= 3
        {
            return Some(MaskedAction::RestoreCenterFocusOnly);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_exact_clicked_panel_zone_instead_of_defaulting_to_subagents() {
        let mask = SidebarGeomMask {
            cols: 40,
            top_h: 12,
            panel3_top_row: 32,
            cost_top_row: 42,
            show_context_panel: true,
            split_ctx_cost: true,
        };
        let tags = HashMap::new();
        let mut cap = PointerCapture::default();

        // Clicking inside COST body selects `FocusZone::Cost`, NOT `Subagents`!
        let z_cost = mask.classify(10, 44, &tags, &[]);
        assert_eq!(
            cap.on_mouse_down(0, 10, 44, z_cost),
            Some(MaskedAction::FocusZoneOnly(Some(FocusZone::Cost)))
        );

        // Clicking inside CONTEXT body selects `FocusZone::Context`!
        let z_ctx = mask.classify(10, 35, &tags, &[]);
        assert_eq!(
            cap.on_mouse_down(0, 10, 35, z_ctx),
            Some(MaskedAction::FocusZoneOnly(Some(FocusZone::Context)))
        );

        // Clicking Sprite clears panel focus without bouncing Center focus on MouseDown!
        let z_sp = mask.classify(10, 5, &tags, &[]);
        assert_eq!(
            cap.on_mouse_down(0, 10, 5, z_sp),
            Some(MaskedAction::FocusZoneOnly(None))
        );
        assert_eq!(cap.on_mouse_up(0, 20, 18, z_ctx), None);
    }
}
