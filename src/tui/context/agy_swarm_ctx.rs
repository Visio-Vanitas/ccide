//! Antigravity Swarm Multi-Model `CONTEXT` & Whole-Swarm `COST` Aggregator (`src/tui/context/agy_swarm_ctx.rs`).

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::session::{
    classify_model_family_for, codex_config, is_valid_model_name, AgentKind, AgentSession,
};
use crate::tui::context::agent_ctx::AgentCtxSample;
use crate::tui::context::agy_parser::update_agy_ctx;
use crate::tui::context::analyzer::{is_native_1m_model_id, resolve_model_window_hint};
use crate::tui::context::{ContextSnapshot, SessionContextAnalyzer};

const MAX_SWARM_WORKERS: usize = 48;
pub(crate) const TRACKER_IDLE_TTL: Duration = Duration::from_secs(600);

pub(crate) fn seed_agy_model(dst: &mut String, fb: &str) {
    if dst.is_empty() && is_valid_model_name(fb) && !fb.eq_ignore_ascii_case("auto") {
        *dst = fb.to_string();
    }
}

pub(crate) fn resolve_agent_window(kind: AgentKind, sample_win: u64, model: &str) -> u64 {
    match kind {
        AgentKind::Codex if sample_win > 0 => sample_win,
        AgentKind::Codex => codex_config()
            .context_window
            .filter(|&w| w > 0)
            .unwrap_or_else(|| resolve_model_window_hint("", model, false)),
        AgentKind::Agy => {
            if let Some(env_w) = env::var("CCIDE_AGY_CONTEXT_WINDOW")
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .filter(|&w| w >= 16_000)
            {
                return env_w;
            }
            if sample_win > 0 {
                return sample_win;
            }
            match classify_model_family_for(AgentKind::Agy, model) {
                "gemini" => 1_048_576,
                "claude" if is_native_1m_model_id(model) => 1_000_000,
                "openai" => 400_000,
                _ => 200_000,
            }
        }
        AgentKind::Claude => resolve_model_window_hint("", model, false),
    }
}

pub(crate) fn pick_eff_model(sample_model: &str, fallback_model: &str) -> String {
    if is_valid_model_name(sample_model) && !sample_model.eq_ignore_ascii_case("auto") {
        sample_model.to_string()
    } else if is_valid_model_name(fallback_model) && !fallback_model.eq_ignore_ascii_case("auto") {
        fallback_model.to_string()
    } else {
        "auto".to_string()
    }
}

impl SessionContextAnalyzer {
    /// Samples `CONTEXT` for the selected swarm member (`"main"` = coordinator, `<worker_cid>` = worker)
    /// while aggregating `COST` and per-model `TPS`/`TTFT` across the coordinator and all workers.
    pub fn sample_agent_swarm(
        &mut self,
        home: &str,
        _cwd: &str,
        sess: &AgentSession,
        view_id: &str,
        workers: &[(String, PathBuf, String)],
    ) -> ContextSnapshot {
        self.pricing.reload_if_changed(home);
        let now = Instant::now();
        self.agent_trackers.retain(|_, tr| {
            tr.last_seen.map_or(true, |t| {
                now.saturating_duration_since(t) <= TRACKER_IDLE_TTL
            })
        });

        let (coord_sample, coord_model) = {
            let tr = self
                .agent_trackers
                .entry(sess.transcript_path.clone())
                .or_default();
            tr.last_seen = Some(now);
            seed_agy_model(&mut tr.s.model, &sess.model_id);
            update_agy_ctx(&sess.transcript_path, tr);
            (tr.s.clone(), pick_eff_model(&tr.s.model, &sess.model_id))
        };

        let mut unique_workers: Vec<(&str, &PathBuf, &str, SystemTime)> = Vec::new();
        let mut seen_paths: HashSet<&PathBuf> = HashSet::new();
        seen_paths.insert(&sess.transcript_path);
        for (fid, wpath, wmodel) in workers {
            if !seen_paths.insert(wpath) {
                continue;
            }
            let Ok(meta) = fs::metadata(wpath) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            unique_workers.push((
                fid.as_str(),
                wpath,
                wmodel.as_str(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
        if unique_workers.len() > MAX_SWARM_WORKERS {
            unique_workers.sort_by_key(|b| std::cmp::Reverse(b.3));
            unique_workers.truncate(MAX_SWARM_WORKERS);
        }

        let mut cost_sources: Vec<(AgentCtxSample, String)> =
            Vec::with_capacity(1 + unique_workers.len());
        cost_sources.push((coord_sample.clone(), coord_model.clone()));

        let want_worker = !view_id.is_empty() && view_id != "main";
        let mut selected_worker: Option<(AgentCtxSample, String)> = None;

        for (fid, wpath, wmodel, _) in unique_workers {
            let tr = self.agent_trackers.entry(wpath.clone()).or_default();
            tr.last_seen = Some(now);
            seed_agy_model(&mut tr.s.model, wmodel);
            update_agy_ctx(wpath, tr);
            let eff = pick_eff_model(&tr.s.model, wmodel);
            let sample = tr.s.clone();
            if want_worker && fid == view_id {
                selected_worker = Some((sample.clone(), eff.clone()));
            }
            cost_sources.push((sample, eff));
        }

        if let Some((w_sample, w_model)) = selected_worker {
            self.snapshot_from(&w_sample, AgentKind::Agy, &w_model, true, &cost_sources)
        } else {
            self.snapshot_from(
                &coord_sample,
                AgentKind::Agy,
                &coord_model,
                false,
                &cost_sources,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COORD_JL: &str = include_str!(
        "../../../tests/fixtures/agy_swarm/brain/coord-001/.system_generated/logs/transcript.jsonl"
    );
    const OPUS_JL: &str = include_str!(
        "../../../tests/fixtures/agy_swarm/brain/worker-opus-001/.system_generated/logs/transcript.jsonl"
    );
    const ARGON_JL: &str = include_str!(
        "../../../tests/fixtures/agy_swarm/brain/worker-argon-002/.system_generated/logs/transcript.jsonl"
    );

    #[test]
    fn switches_context_between_coordinator_and_worker_while_aggregating_3_model_swarm_costs() {
        let dir = env::temp_dir().join(format!("ccide_swarm_ctx_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        let (p_coord, p_opus, p_argon) = (
            dir.join("c.jsonl"),
            dir.join("o.jsonl"),
            dir.join("a.jsonl"),
        );
        fs::write(&p_coord, COORD_JL).unwrap();
        fs::write(&p_opus, OPUS_JL).unwrap();
        fs::write(&p_argon, ARGON_JL).unwrap();

        let sess = AgentSession {
            kind: AgentKind::Agy,
            session_id: "coord-001".to_string(),
            agent_pid: 0,
            transcript_path: p_coord,
            model_id: "gemini-flash-worker-agy".to_string(),
            cwd: "/workspace/demo".to_string(),
        };
        let workers = vec![
            (
                "worker-opus-001".to_string(),
                p_opus,
                "claude-opus-5-5-high".to_string(),
            ),
            (
                "worker-argon-002".to_string(),
                p_argon,
                "gemini-4-argon-high".to_string(),
            ),
        ];

        let mut an = SessionContextAnalyzer::new();
        let snap_main =
            an.sample_agent_swarm("/nonexistent", "/workspace/demo", &sess, "main", &workers);
        assert!(!snap_main.is_subagent && snap_main.total_in_tokens == 18_000);
        assert_eq!(
            (snap_main.model_win_tokens, snap_main.model_costs.len()),
            (1_048_576, 3)
        );
        assert!(snap_main.cost_usd > 0.0);

        let snap_opus = an.sample_agent_swarm(
            "/nonexistent",
            "/workspace/demo",
            &sess,
            "worker-opus-001",
            &workers,
        );
        assert!(snap_opus.is_subagent && snap_opus.total_in_tokens == 25_000);
        assert_eq!(
            (snap_opus.model_win_tokens, snap_opus.model_costs.len()),
            (1_000_000, 3)
        );
        assert!((snap_opus.cost_usd - snap_main.cost_usd).abs() < 1e-9);

        let snap_argon = an.sample_agent_swarm(
            "/nonexistent",
            "/workspace/demo",
            &sess,
            "worker-argon-002",
            &workers,
        );
        assert!(snap_argon.is_subagent && snap_argon.total_in_tokens == 22_000);
        assert_eq!(
            (snap_argon.model_win_tokens, snap_argon.model_costs.len()),
            (1_048_576, 3)
        );

        let mut s20 = AgentCtxSample {
            turns: 20,
            ..Default::default()
        };
        for _ in 0..20 {
            s20.aggregate_per_model("gemini-4-carbon", 15_000, 0, 0);
        }
        let src20 = [(s20.clone(), "gemini-4-carbon".to_string())];
        let snap20 = an.snapshot_from(&s20, AgentKind::Agy, "gemini-4-carbon", false, &src20);
        assert_eq!(snap20.model_costs[0].turns, 20);
        assert!(
            (snap20.cost_usd - 0.60).abs() < 1e-6,
            "got {}",
            snap20.cost_usd
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
