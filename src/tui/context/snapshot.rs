//! Per-Model Cost Entries & Context Panel Snapshot Types (`src/tui/context/snapshot.rs`).

use crate::budget::ContextBudget;

#[derive(Clone, Debug)]
pub struct ModelCostEntry {
    pub model_short: String,
    pub family: &'static str,
    pub turns: usize,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub saved_usd: f64,
    pub share_pct: u64,
    pub tps: Option<f64>,
    pub ttft_ms: Option<u64>,
}

impl ModelCostEntry {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.output_tokens)
    }
}

#[derive(Clone, Debug)]
pub struct ContextSnapshot {
    pub is_subagent: bool,
    pub model_win_tokens: u64,
    pub budget: ContextBudget,
    pub total_in_tokens: u64,
    pub sys_md_tokens: u64,
    pub tools_tokens: u64,
    pub tool_io_tokens: u64,
    pub thinking_tokens: u64,
    pub chat_tokens: u64,
    pub cum_cache_read_tokens: u64,
    pub session_hit_pct: f64,
    pub last_hit_pct: f64,
    pub turns: usize,
    pub session_turns: usize,
    pub cost_usd: f64,
    pub saved_usd: f64,
    pub saved_pct: u64,
    pub burn_usd_per_hr: Option<f64>,
    pub rl_5h_pct: Option<u64>,
    pub rl_7d_pct: Option<u64>,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub model_costs: Vec<ModelCostEntry>,
}
