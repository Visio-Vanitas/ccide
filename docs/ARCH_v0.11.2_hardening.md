# cc-ide v0.11.2 — Self-Check Hardening, Real Antigravity Schema & Release Pipeline Contract

Status: **FROZEN for Wave 2**. Baseline: `dev` @ `8ee0671` (v0.11.1). All global invariants remain in
force: MSRV `1.70.0`, Edition 2021, zero external crates, every `.rs` file `<= 280` raw / `<= 300`
post-`cargo fmt`, zero `crate::commands` imports in `src/{session,tui,json,budget,platform,ffi,
orchestrator,telemetry}` or leaf files, and zero `orchestrator <-> session` cycles.

## 1. Session Helpers & Coordinator Promotion (`src/session/{agy_swarm_io,agy_swarm,agent_bind,agent,scan,mod}.rs`)

- **Shared `members.json` cache & path helpers (`src/session/agy_swarm_io.rs`)**:
  - Move `MEMBERS_CACHE`, `pub(crate) fn read_members_cached(path: &Path) -> Option<String>`,
    `pub fn agy_sys_gen(transcript: &Path) -> Option<PathBuf>`, and
    `pub fn agy_brain_root(transcript: &Path) -> Option<PathBuf>` into `agy_swarm_io.rs`, and
    `pub use` them in `agy_swarm.rs` + `session/mod.rs` (zero duplicate logic; `agy_swarm.rs` drops ~35L).
  - Also in `agy_swarm.rs`: pass `&mut snap.worker_transcripts` into `append_plain_subagents` so plain
    `subagents/*.json` entries push `(cid, wt, String::new())` for click-to-inspect `CONTEXT` & `COST`,
    and prune stale `sub_trackers` (`sub_trackers.retain(|k, _| seen.contains(k))`).
- **Frozen public signatures in `src/session/agy_swarm_io.rs` (re-exported in `agy_swarm.rs` & `session/mod.rs`)**:
  ```rust
  pub fn agy_is_member_of(coord_transcript: &Path, candidate_cid: &str) -> bool;
  pub fn agy_parent_coordinator(cid: &str, transcript_path: &Path) -> Option<(String, PathBuf)>;
  pub fn agy_resolve_cid_detail(home: &str, yid: &str, cwd: &str) -> Option<(String, bool)>;
  pub fn agy_resolve_cid(home: &str, yid: &str, cwd: &str) -> Option<String>; // delegates to _detail
  ```
  - `agy_is_member_of`: requires `is_safe_id(candidate_cid)` and `sg = agy_sys_gen(coord_transcript)?`.
    Returns `true` if `read_members_cached(&sg.join("swarm/members.json"))` has any member in
    `"members"` with `"conversation_id" == candidate_cid`, OR if
    `sg.join("subagents").join(format!("{candidate_cid}.json"))` (`<= 64 KiB`) has
    `"conversationId" == candidate_cid`.
  - `agy_parent_coordinator`: returns `None` if `!is_safe_id(cid)` or if `agy_sys_gen(transcript_path)`
    already has `swarm/members.json` (`cid` is itself a coordinator). Otherwise scans `<= 128` sibling
    dirs across `agy_brain_root(transcript_path)` + `agy_brain_roots()` (deduped): for each sibling
    `cand_cid != cid` (`is_safe_id`), if `<root>/<cand_cid>/.system_generated/logs/transcript.jsonl`
    exists and `agy_is_member_of(&cand_tr, cid)` is true, returns `Some((cand_cid, cand_tr))`.
  - `agy_resolve_cid_detail`: returns `Some((cid, true))` when resolved from `tab_conv_<yid>.txt`
    (`is_safe_id(yid)`) or `$CCIDE_AGY_CONVERSATION`, and `Some((cid, false))` when resolved from
    `agy_cid_for_cwd(cwd)` or `scan_subagents_cid(&canon_cwd)`.
- **`resolve_agent_session` (`src/session/agent_bind.rs`)**:
  - For `AgentKind::Agy`:
    1. Check explicit override first (`agy_resolve_cid_detail` returning `Some((exp_cid, true))`).
       In the `live_tab_<yid>.json` branch, if an explicit `exp_cid` exists and `exp_cid != sid`, skip
       `live_tab_<yid>.json`; otherwise if `agy_parent_coordinator(&sid, &tpath)` returns
       `Some((c_cid, c_tp))`, promote `(sid, tpath)` to `(c_cid, c_tp)`.
    2. In the disk-fallback branch, call `let (mut cid, is_explicit) = agy_resolve_cid_detail(home, yid, cwd)?;`.
       Enforce the 24h (`86_400s`) `transcript.jsonl` mtime cutoff **only when `!is_explicit`**.
       When `!is_explicit`, if `agy_parent_coordinator(&cid, &tpath)` returns `Some((c_cid, c_tp))`,
       promote `(cid, tpath)` to `(c_cid, c_tp)`.
- **`is_safe_id` & URI hardening**:
  - `src/session/agent.rs:62,73`: require `is_safe_id(yid)` in `read_tab_agent` and `write_tab_agent`.
  - `src/session/scan.rs`: filter `session_id`, `bound_yid`, `sidebar_tab_*.json` `"yazi_id"`, and
    `$CC_YAZI_ID` through `is_safe_id`. (If `scan.rs` exceeds `280`L, trim comments/blank lines to stay `<= 280`L.)
  - `decode_file_uri` (`agy_swarm_io.rs`): strip leading `/` on `/[A-Za-z]:...` across all OSes (not gated on `cfg!(windows)`).

## 2. Real Antigravity `transcript.jsonl` Parsing & Internal Path Filtering (`src/session/agy.rs`, `src/tui/context/agy_parser.rs`, `src/platform/mod.rs`)

- **`src/session/agy.rs`**:
  - `pub fn extract_agy_arg_str(args: &str, key: &str) -> Option<String>`: extracts `extract_json_str(args, key)`,
    trims whitespace, and if wrapped in outer quotes (`s.len() >= 2 && s.starts_with('"') && s.ends_with('"')`),
    strips outer `"..."` and unescapes `\"` -> `"` and `\\` -> `\`; returns `(!s.is_empty()).then_some(s)`.
  - `pub fn extract_agy_arg_u64(args: &str, key: &str) -> Option<u64>`:
    `extract_json_u64(args, key).or_else(|| extract_agy_arg_str(args, key)?.trim().parse::<u64>().ok())`.
  - In `map_agy_tool_call` and `parse_agy_sub_call`: resolve
    `let args = extract_json_object(tc, "arguments").or_else(|| extract_json_object(tc, "args")).unwrap_or(tc);`
    and use `extract_agy_arg_str` / `extract_agy_arg_u64`.
  - In `map_agy_tool_call`: reject internal/temp paths via `if is_internal_or_temp_path(&file, "") { return None; }`.
  - In `agy_transcript_path(cid)`: return
    `agy_transcript_for_cid(cid).unwrap_or_else(|| agy_home().join("brain").join(cid).join(".system_generated/logs/transcript.jsonl"))`.
- **`src/platform/mod.rs` (`is_internal_or_temp_path`)**:
  - Add `"/.gemini/"` and `"/.codex/"` to `home_norm` checks, and `"/.system_generated/"`,
    `"/.gemini/antigravity"`, `"/.codex/sessions/"` to general checks.
- **Model pre-seeding & exclusive cache occupancy (`src/tui/context/{agy_parser,agy_swarm_ctx,agent_ctx}.rs`)**:
  - Before calling `update_agy_ctx(path, tr)` in `agent_ctx.rs` and `agy_swarm_ctx.rs`, pre-seed:
    `if tr.s.model.is_empty() && is_valid_model_name(fb) && !fb.eq_ignore_ascii_case("auto") { tr.s.model = fb.to_string(); }`.
  - `pub fn agy_occupancy(model: &str, input_tokens: u64, cache_read_tokens: u64) -> u64`:
    returns `input_tokens.saturating_add(cache_read_tokens)` when
    `classify_model_family_for(AgentKind::Agy, model) == "claude" || input_tokens < cache_read_tokens`,
    else `input_tokens`.

## 3. Turn-Accurate Per-Model `COST` & Pricing (`src/tui/context/*`, `src/tui/pricing/*`, `assets/pricing.json`)

- **5-tuple `per_model` (`src/tui/context/agent_ctx.rs`)**:
  - `pub per_model: Vec<(String, u64, u64, u64, usize)>` — `(model, uncached_in, cached_in, out_tok, turns)`.
  - `pub fn aggregate_per_model(&mut self, model: &str, uncached_in: u64, cached_in: u64, out_tok: u64)`:
    increments existing entry's tokens and `entry.4 = entry.4.saturating_add(1)`, or pushes `(..., 1)`.
  - Update `codex_parser.rs:185` test assertion to `("gpt-5.4".to_string(), 6_500, 12_000, 340, 1)`.
  - Also run the 10-minute `TRACKER_IDLE_TTL` `agent_trackers.retain(...)` in `sample_agent`.
- **Single frozen turn-averaged cost formula in `snapshot_from` (`src/tui/context/agent_ctx.rs`)**:
  ```rust
  for (m_raw, uncached_in, cached_in, out_tok, m_turns) in &src.per_model {
      let t = (*m_turns).max(1) as u64;
      let (q_in, r_in) = (*uncached_in / t, *uncached_in % t);
      let (q_cr, r_cr) = (*cached_in / t, *cached_in % t);
      let (q_out, r_out) = (*out_tok / t, *out_tok % t);
      let rate = self.pricing.resolve(&m_use);
      let (act_q, unc_q) = rate.compute_turn_cost(q_in, 0, 0, q_cr, q_out);
      let (act_r, unc_r) = rate.compute_turn_cost(q_in + r_in, 0, 0, q_cr + r_cr, q_out + r_out);
      let (act, unc) = (act_q * ((t - 1) as f64) + act_r, unc_q * ((t - 1) as f64) + unc_r);
      // accumulate cost_usd, uncached_usd, and entry.turns += (*m_turns).max(1)
  }
  ```
- **Pricing catalog (`assets/pricing.json`, `src/tui/pricing/{mod,lookup}.rs`)**:
  - Add `"gemini-4-argon"`, `"gemini-4-hydrogen"`, `"gemini-4-neon"`, `"gpt-oss-120b"`, `"gpt-oss"` to `assets/pricing.json`.
  - In `src/tui/pricing/mod.rs:210-220`: route `norm.contains("gpt-oss") -> "gpt-oss-120b"` before `"gpt"`,
    and include `norm.contains("argon") || norm.contains("hydrogen") || norm.contains("neon")` in the Gemini fallback.
  - In `src/tui/pricing/lookup.rs:252`: assert non-zero cost on `compute_turn_cost(10_000, 0, 0, 0, 1_000)`.
- **JSON scanner (`src/json/scan.rs`)**: use `depth = depth.saturating_sub(1)` in `find_balanced_span` and unescape strings in `extract_string_list_in_array`.

## 4. Hook Worker Isolation (`src/commands/{hook_records,yazi_hook}.rs`)

- **Frozen signatures in `src/commands/hook_records.rs` (re-exported in `hook_input.rs`)**:
  ```rust
  pub fn agy_event_is_worker(cdir: &Path, yid: &str, ev: &HookEvent) -> bool;
  pub fn should_reveal_agent_view(cdir: &Path, yid: &str, ev: &HookEvent, is_worker: bool) -> bool;
  ```
  - `agy_event_is_worker`: returns `false` unless `ev.agent == AgentKind::Agy && !ev.session_id.is_empty()`.
    Returns `true` if: (a) `tab_conv_<yid>.txt` has a valid `tc_cid != ev.session_id`; OR
    (b) `live_tab_<yid>.json` has `"agent" == "agy"`, `"session_id": prev_sid != ev.session_id`, and
    `"transcript_path": prev_tp` with `agy_is_member_of(Path::new(&prev_tp), &ev.session_id)`; OR
    (c) `agy_parent_coordinator(&ev.session_id, Path::new(&resolved_tp)).is_some()`.
  - `write_agent_hook_records`: always writes `live_sess_<ev.session_id>.json`; writes
    `live_tab_<yid>.json` (and `tab_agent_<yid>.txt`) **only when `!is_worker`** (for `Agy`,
    `!agy_event_is_worker`; for `Codex`, when rollout line 1 has empty `parent_thread_id`).
  - `should_reveal_agent_view`: reads `active_view_<yid>.txt` (default `"main"`). For `Agy`, if
    `is_worker` returns `active_view == ev.session_id`, else `active_view == "main" || active_view == ev.session_id`.
    For `Codex`, returns `!is_worker`.
  - In `yazi_hook.rs` (`<= 288`L): compute `is_worker = hook_input::agy_event_is_worker(&cdir, &target_yazi_id, &ev)`;
    skip `session_<sid>.yazi` and `main_last_file_<yid>.json` writes when `is_worker`; gate non-Claude
    `emit_yazi_reveal_at` on `hook_input::should_reveal_agent_view(&cdir, &target_yazi_id, &ev, is_worker)`.

## 5. Exact Fixture Updates & Wave 2 Disjoint File Ownership

- **`tests/fixtures/agy/main.jsonl`**: in L4 `PLANNER_RESPONSE`, change `replace_file_content` to use
  `"args":{"TargetFile":"\"src/lib.rs\"","StartLine":"12","EndLine":"15",...}` (keeping token counts
  `14500/620/18000` and `write_to_file` `/workspace/demo/src/helper.rs` unchanged).
- **`tests/fixtures/agy_swarm/brain/worker-opus-001/.system_generated/logs/transcript.jsonl`**: in L2
  `PLANNER_RESPONSE`, remove `"model":"claude-opus-5-5-high"` and use
  `"args":{"Path":"\"src/arch.rs\"","StartLine":"1","EndLine":"25"}` (verifying `members.json` model
  pre-seeding -> `25,000` occupancy / `1,000,000` window + double-quoted `"args"`).

| Track / Worker | Exclusive Owned Files |
|---|---|
| **Track A (`@session-mod`)** | `src/session/{agy_swarm_io,agy_swarm,agy,agent_bind,agent,scan,mod,agy_swarm_tests,agy_tests}.rs` |
| **Track B (`@rust-core`)** | `src/tui/context/{agent_ctx,agy_parser,agy_swarm_ctx,codex_parser}.rs`, `src/tui/pricing/{mod,lookup}.rs`, `src/json/scan.rs`, `src/platform/mod.rs`, `assets/pricing.json` |
| **Track C (`@backends`)** | `src/commands/{hook_input,hook_records,yazi_hook}.rs`, `src/commands/workspace/{spec_io,actions}.rs`, `src/commands/sidebar/{hit_mask,dismiss}.rs` |
| **Track D (`@plugin-scripts`)** | `scripts/{ccide,ccide.ps1,ensure-ccide-core.sh,ensure-ccide-core.mjs,merge_agent_hooks.py}`, `install.{sh,ps1}`, `.github/workflows/{ci,release}.yml`, `AGENT.md`, `CHANGELOG.md` |
| **Track E (`@tests-qa`)** | `tests/fixtures/agy/main.jsonl`, `tests/fixtures/agy_swarm/brain/worker-opus-001/.system_generated/logs/transcript.jsonl`, `tests/{test_multi_agent,test_iterm_layout}.py` |
