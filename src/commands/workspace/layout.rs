//! Unified 3-Column Adaptive Workspace `LayoutPlan` & `RebalanceOutcome` (`src/commands/workspace/layout.rs`).

pub const LEFT_MIN: usize = 22;
pub const CENTER_MIN: usize = 48;
pub const RIGHT_MIN: usize = 28;
pub const RIGHT_PCT: usize = 16;
pub const COLLAPSE_RIGHT_BELOW: usize = 100;
pub const COLLAPSE_LEFT_BELOW: usize = 76;
pub const LEFT_PCT_MIN: u8 = 12;
pub const LEFT_PCT_MAX: u8 = 45;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutPlan {
    pub total_cols: usize,
    pub left_cols: usize,
    pub center_cols: usize,
    pub right_cols: usize,
    pub collapse_left: bool,
    pub collapse_right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebalanceOutcome {
    Applied,
    #[allow(dead_code)]
    Reopened,
    Unsupported {
        backend: &'static str,
        reason: &'static str,
    },
}

fn pct_round(total: usize, p: usize) -> usize {
    total.saturating_mul(p).saturating_add(50) / 100
}

impl LayoutPlan {
    pub fn compute(total_cols: usize, left_pct: u8, show_right: bool) -> Self {
        let lp = left_pct.clamp(LEFT_PCT_MIN, LEFT_PCT_MAX) as usize;
        let collapse_right = !show_right || total_cols < COLLAPSE_RIGHT_BELOW;
        let collapse_left = total_cols < COLLAPSE_LEFT_BELOW;

        let mut right_cols = if collapse_right {
            0
        } else {
            pct_round(total_cols, RIGHT_PCT).max(RIGHT_MIN)
        };
        let mut left_cols = if collapse_left {
            0
        } else {
            pct_round(total_cols, lp).max(LEFT_MIN)
        };

        let used = left_cols + right_cols;
        if total_cols < used + CENTER_MIN {
            let mut deficit = (used + CENTER_MIN).saturating_sub(total_cols);
            if !collapse_right && right_cols > RIGHT_MIN {
                let cut = deficit.min(right_cols - RIGHT_MIN);
                right_cols -= cut;
                deficit -= cut;
            }
            if deficit > 0 && !collapse_left && left_cols > LEFT_MIN {
                let cut = deficit.min(left_cols - LEFT_MIN);
                left_cols -= cut;
            }
        }
        let center_cols = total_cols.saturating_sub(left_cols + right_cols);
        Self {
            total_cols,
            left_cols,
            center_cols,
            right_cols,
            collapse_left,
            collapse_right,
        }
    }

    pub fn tmux_lengths(&self) -> (usize, usize) {
        (self.left_cols, self.right_cols)
    }

    pub fn wezterm_percents(&self) -> (u8, u8) {
        if self.total_cols == 0 {
            return (0, 0);
        }
        let l_pct = ((self.left_cols * 100 + self.total_cols / 2) / self.total_cols) as u8;
        let rem = self.center_cols + self.right_cols;
        let r_pct = if rem == 0 || self.right_cols == 0 {
            0
        } else {
            ((self.right_cols * 100 + rem / 2) / rem) as u8
        };
        (l_pct, r_pct)
    }

    pub fn wt_fractions(&self) -> (f32, f32) {
        let total = self.total_cols.max(1) as f32;
        let rem = (self.center_cols + self.right_cols) as f32;
        let f1 = if self.left_cols == 0 {
            0.0
        } else {
            (rem / total).clamp(0.10, 0.90)
        };
        let f2 = if self.right_cols == 0 || rem <= 0.0 {
            0.0
        } else {
            ((self.right_cols as f32) / rem).clamp(0.10, 0.90)
        };
        (f1, f2)
    }

    pub fn scale_px(&self, total_px: usize) -> (usize, usize, usize) {
        if self.total_cols == 0 {
            return (0, total_px, 0);
        }
        let l_px = self.left_cols.saturating_mul(total_px) / self.total_cols;
        let r_px = self.right_cols.saturating_mul(total_px) / self.total_cols;
        let c_px = total_px.saturating_sub(l_px + r_px);
        (l_px, c_px, r_px)
    }

    pub fn ghostty_open_deltas(&self, total_px: usize) -> (isize, isize) {
        let (left_px, _, right_px) = self.scale_px(total_px);
        let d_left = (total_px / 2) as isize - (left_px as isize);
        let d_right = (total_px.saturating_sub(left_px) / 2) as isize - (right_px as isize);
        (d_left, d_right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_golden_table_and_sum_invariants() {
        type GoldenCase = (usize, u8, bool, (usize, usize, usize));
        let cases: &[GoldenCase] = &[
            (70, 30, true, (0, 70, 0)),
            (80, 30, true, (24, 56, 0)),
            (100, 30, true, (24, 48, 28)),
            (120, 30, true, (36, 56, 28)),
            (160, 30, true, (48, 84, 28)),
            (200, 30, true, (60, 108, 32)),
            (240, 30, true, (72, 130, 38)),
            (320, 30, true, (96, 173, 51)),
            (120, 30, false, (36, 84, 0)),
            (100, 45, true, (24, 48, 28)),
        ];
        for &(total, lp, sr, expected) in cases {
            let p = LayoutPlan::compute(total, lp, sr);
            assert_eq!(
                (p.left_cols, p.center_cols, p.right_cols),
                expected,
                "failed for total={total} lp={lp} show_right={sr}"
            );
            assert_eq!(p.left_cols + p.center_cols + p.right_cols, total);
        }
        assert_eq!(
            LayoutPlan::compute(240, 30, true).ghostty_open_deltas(2400),
            (480, 460)
        );
        for total in 60..=320 {
            for lp in [10, 12, 25, 30, 45, 60] {
                for sr in [false, true] {
                    let p = LayoutPlan::compute(total, lp, sr);
                    assert_eq!(p.left_cols + p.center_cols + p.right_cols, total);
                    assert!(p.center_cols >= CENTER_MIN);
                }
            }
        }
    }
}
