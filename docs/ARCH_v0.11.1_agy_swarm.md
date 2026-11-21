# cc-ide v0.11.1 — Native Antigravity Swarm Support (`agy` CLI + Antigravity 2.0 `native_swarm`)

Status: **FROZEN for Wave 2**. Baseline `dev` @ `7e10701` (v0.11.0). Changing any signature, status
string, wire key or line budget below requires an `@arch-opus` amendment. All v0.11 rules
(`docs/ARCH_v0.11_multi_agent.md` §1) still apply: MSRV 1.70, zero crates, `.rs` ≤ 285 lines after
`rustfmt` (hard cap 300), no `crate::commands` in `session`/`tui`/`json`, `is_safe_id` on every external
id, bounded I/O on the 160 ms tick, and `dispatch.rs` (299) is never touched.

## 0. Carry-over blocker (found during this design pass)
**`scripts/merge_agent_hooks.py` fails at import on macOS system Python 3.9.6.** Its PEP 604
annotations (`dict | None`, `str | None`, lines 39 and 67) are evaluated when each function is
defined, which raises `TypeError: unsupported operand type(s) for |`. As a result, `install.sh:805`
never merges Codex hooks on stock macOS, and `--uninstall` (`:185`) silently does nothing.
**Fix (@plugin-scripts):** add `from __future__ import annotations` as the first statement after the
docstring. **@tests-qa:** add a test that runs the script with `/usr/bin/python3` when it is < 3.10.

## 1. On-disk schema (all paths relative to the bound transcript; never derived from `agy_home()`)
```
transcript = <app>/brain/<cid>/.system_generated/logs/transcript.jsonl
sys_gen    = transcript.parent()?.parent()?   // must have file_name == ".system_generated"
brain_root = sys_gen.parent()?.parent()?      // <app>/brain
<sys_gen>/swarm/{swarm.json, members.json, board.jsonl}
<sys_gen>/subagents/<sub_cid>.json
<brain_root>/<worker_cid>/.system_generated/logs/transcript.jsonl
```
A swarm is **active** iff `<sys_gen>/swarm/members.json` parses and has at least one non-coordinator
member. `swarm.json` is informational only (`swarm_id == cid` is not required).

## 2. `src/session/agy_swarm.rs` (NEW, ≤ 280 lines) + `agy_swarm_tests.rs` (NEW, `#[cfg(test)] #[path]`)
```rust
#[derive(Clone, Default)]
pub struct AgySwarmSnapshot {
    pub active: bool,
    pub title: String,                                 // members.json "title", sanitized, ≤ 60 chars
    pub board_head: u64,                               // max seq seen in board.jsonl tail; 0 if absent
    pub coordinator_model: Option<String>,             // coordinator applied_model → model
    pub members: Vec<SubagentItem>,                    // sorted (§2.3); coordinator excluded
    pub worker_transcripts: Vec<(String, PathBuf, String)>, // (full_id = worker cid, transcript, model)
    pub board_latest: Vec<(String, u64, String)>,      // (roster_name, seq, 1-line summary) per agent
}
pub fn agy_sys_gen(transcript: &Path) -> Option<PathBuf>;
pub fn agy_brain_root(transcript: &Path) -> Option<PathBuf>;
pub fn swarm_member_status(member_json: &str) -> &'static str;      // §2.2
pub fn read_board_tail(board: &Path, max_bytes: u64) -> (u64, Vec<(String, u64, String)>);
pub fn load_agy_swarm(home: &str, main_sid: &str, transcript: &Path,
                      sub_trackers: &mut HashMap<String, JsonlTracker>,
                      fallback_tr: &mut JsonlTracker) -> AgySwarmSnapshot;
pub fn agy_resolve_cid(home: &str, yid: &str, cwd: &str) -> Option<String>;  // §2.5
```

### 2.1 Building `SubagentItem`s
**Swarm members:** one item per entry in `members.json` `members` whose `role != "coordinator"`:
- `full_id` = `conversation_id`. `is_safe_id` is required; a member that fails it is skipped.
- `desc` = roster name.
- `raw_type` = `role`, or `subagentDescriptor.typeName` for subagents. `short_type` = `badge(raw_type)`.
- `model_raw` = `applied_model` → `desired_model` → `model`; `model_short` = `short_model_label`.
- `effort` = `None`; `ctime` = transcript mtime (secs, 0 if missing); `footer_rank` = sorted index.
- `tool`/`file`/`line`/`span`/`file_op_seq` come from `agy_main_file_ops` on the worker transcript,
  using `sub_trackers[full_id]` (`JsonlTracker::from_tail(None)` on first sight; incremental afterwards).

**Plain subagents:** `<sys_gen>/subagents/*.json`, at most 64 files, each read only if ≤ 64 KiB.
- `conversationId` must pass `is_safe_id` and must not already be a member.
- `SUBAGENT_STATE_ALIVE` → `"running"`, `SUBAGENT_STATE_KILLED` → `"killed"`, anything else → `"stopped"`.
- `desc` = `subagentDescriptor.role`; the transcript lives at `<brain_root>/<cid>/…` (may be missing).

**Fallback:** if neither `members.json` nor a `subagents/` dir exists, `members` =
`agy_subagents(transcript, fallback_tr)` (v0.11 behaviour) and `active = false`.

### 2.2 Status mapping (`swarm_member_status`, exact order)
1. `killed == true || phase == "KILLED" || desired_state == "KILLED"` → `"killed"`
2. `phase == "RUNNING" && wake_mode == "working"` → `"running"`
3. `phase == "RUNNING"` → `"waiting"`
4. otherwise → `"stopped"`

Status vocabulary for `SubagentItem.status` (`&'static str`), across all agents: `"running"`,
`"waiting"`, `"stopped"`, `"killed"`, `"done"`. NEW shared predicate in `src/session/subagents.rs`:
`pub fn is_subagent_alive(status: &str) -> bool { matches!(status, "running" | "waiting") }`.

### 2.3 Sorting & dismissal
- **Sort:** rank `running`=0, `waiting`=1, `stopped`=2, `killed`/`done`=3; then roster name/desc
  ascending; then `full_id`.
- **Dismissal:** `load_dismissed_subagents(home, main_sid)` hides an item only if
  `!is_subagent_alive(status)`. Live `running`/`waiting` members are **never** hidden, even if their id
  is in the dismissed set.

### 2.4 Bounded readers
- **`read_board_tail(board, 65_536)`:** seek to `len.saturating_sub(max)`. If the offset > 0, drop the
  first (partial) line. For each line, parse `seq` (u64), `agent` (≤ 64 chars) and `text`.
  `board_head` = max seq.
  - Summary = `text` with control chars replaced by space, whitespace collapsed, truncated on a char
    boundary to 96 chars plus `…`.
  - Keep the latest seq per agent, sorted by seq descending, at most 32 entries.
  - Missing file → `(0, vec![])`.
- **`members.json`:** one `fs::read_to_string` only when `(len, mtime)` changed. The cache lives in a
  `thread_local!` keyed by path, like the `scan.rs` session cache. Files > 256 KiB are ignored (treated
  as an inactive swarm).
- Iterating JSON object members needs a new leaf helper (@rust-core, `src/json/scan.rs`):
  `pub fn iter_object_members(obj_slice: &str) -> Vec<(String, &str)>` returns (unescaped key, raw
  value slice), is string/escape-aware, and stops at the closing `}`.

### 2.5 Conversation resolution `agy_resolve_cid` (replaces the `agy_cid_for_cwd` call in `agent_bind` disk fallback step 2)
1. `<cache_dir>/tab_conv_<yid>.txt` (written by the launcher; the most specific source).
2. `$CCIDE_AGY_CONVERSATION`. Second, because sidebar panes created by tmux/WezTerm may not inherit the
   launcher env, and an inherited shell value is less specific than the tab file.
3. `agy_cid_for_cwd(cwd)` (`last_conversations.json`, as in v0.11).
4. Bounded scan: for each brain root in `[agy_home()/brain, ~/.gemini/antigravity/brain]` (deduped,
   existing only), at most 200 `brain/*/.system_generated/subagents/*.json` files stat'ed, each ≤ 64 KiB.
   A file whose `workspaceUris` contains `file://<cwd>` (percent-decoded, `paths_equal_ci` after
   canonicalize) nominates its **parent** `<cid>`. The newest mtime wins.

Every result must pass `is_safe_id`, and its transcript must exist under one of those brain roots.
The hook record (`live_tab_<yid>.json`, resolution step 1 in v0.11) still wins over all of these.

## 3. Multi-model swarm CONTEXT & COST (`src/tui/context/`)
- **Refactor (@rust-core), no behaviour change.** Extract the snapshot assembly from
  `agent_ctx.rs::sample_agent` into:
  ```rust
  pub(crate) fn snapshot_from(&mut self, primary: &AgentCtxSample, kind: AgentKind, primary_model: &str,
      is_sub: bool, cost_sources: &[(AgentCtxSample, String /*fallback model*/)]) -> ContextSnapshot;
  ```
  `sample_agent` becomes `update tracker → snapshot_from(primary, kind, eff, is_sub, &[(primary.clone(), eff)])`.
  `agent_ctx.rs` must stay ≤ 270.
- **NEW `src/tui/context/agy_swarm_ctx.rs`** (≤ 180 lines; a child module, so it can reach the private
  `agent_trackers`):
  ```rust
  impl SessionContextAnalyzer {
      pub fn sample_agent_swarm(&mut self, home: &str, cwd: &str, sess: &AgentSession, view_id: &str,
                                workers: &[(String, PathBuf, String)]) -> ContextSnapshot;
  }
  ```
  - Update the coordinator tracker and every unique worker tracker (keyed by `PathBuf`; skip missing
    files; at most 48 workers, newest mtime first).
  - **Primary:** `view_id == "main"` (or not found) → the coordinator sample, `is_sub = false`.
    Otherwise → the worker whose `full_id == view_id`, with `is_sub = true`. Model =
    `sample.model` (if valid and not `auto`) → the tuple's applied model → `"auto"`. The window comes from
    `resolve_agent_window(Agy, sample.ctx_window, model)`.
  - **Cost sources:** always coordinator + all workers, each tagged with its fallback model, so
    `per_model`, per-model TPS/TTFT (`ModelPerfAccum`) and USD aggregate across the whole swarm in
    every view.
  - Prune `agent_trackers` entries not seen for 10 min, so the map stays bounded.
- **`per_model` invariant:** `AgentCtxSample.per_model` holds at most one entry per model (aggregate in
  place; never push per event). Fix `codex_parser.rs:104` to obey it as well.
- **Pricing (`tui/pricing/lookup.rs`):** add golden tests for `claude-opus-5-5-high`,
  `gemini-4-argon-high`, `gemini-3.8-flash-low`, `gemini-4-hydrogen-high` and `gpt-6-astra`. Lookup
  order: raw lowercase, then `normalize_model_key`, then progressive suffix strip, which must strip
  effort suffixes `-low|-medium|-high|-xhigh|-max`. Unknown models give cost 0 with no panic.
  Existing keys such as `gemini-2.5-pro` and `gpt-4.1` must still resolve (test).

## 4. Sidebar `SWARM` panel & click-to-inspect (`src/commands/sidebar/`)
### 4.1 Mandatory splits BEFORE feature edits (budgets after `rustfmt`)
| File | Now | Action | After |
|---|---|---|---|
| `mouse_actions.rs` | 299 | move `dismiss_single_completed` + `dismiss_all_completed` → **`dismiss.rs`** (NEW) | ≤ 260 |
| `agent_tab.rs` | 284 | move `refresh_non_claude_tab` → **`agent_refresh.rs`** (NEW, ≤ 240) | ≤ 160 |
| `sidebar/mod.rs` | 283 | move the ctx-sampling `match` (`mod.rs:194-215`) → `agent_refresh.rs::sample_ctx(...)` | ≤ 270 |
| `session/agy.rs` | 294 | move tests → `agy_tests.rs` (exists); move `agy_cid_for_cwd` + `last_conversations` helpers → `agy_swarm.rs` | ≤ 250 |
| `hit_mask.rs` | 256 | +title-row offset only | ≤ 270 |

### 4.2 Data flow
```rust
// agent_refresh.rs
pub struct TabRefresh { pub rebound: bool, pub tool: String, pub file: String, pub line: usize, pub span: usize,
                        pub cfg: ConfigModels, pub subs: Vec<SubagentItem>, pub swarm: Option<AgySwarmSnapshot> }
pub fn refresh_non_claude_tab(/* v0.11 params */, active_view_id: &mut String,
                              seen_sub_seq: &mut HashMap<String, usize>) -> TabRefresh;
pub fn sample_ctx(an: &mut SessionContextAnalyzer, kind: AgentKind, home: &str, cwd: &str,
                  sess: Option<&AgentSession>, swarm: Option<&AgySwarmSnapshot>, view: &str,
                  /* claude path args unchanged */) -> ContextSnapshot;
```
- `refresh_tab_dispatch` (agent_tab.rs) returns `TabRefresh`. The Claude branch fills `swarm: None`.
- For Agy, `subs = swarm.members`. `sample_ctx` calls `sample_agent_swarm` when `swarm.active`, else
  `sample_agent`.
- **Follow the selected worker:** if `active_view_id != "main"` and that member's `file_op_seq >
  seen_sub_seq[id]`, call `trigger_yazi_reveal` with its file/line/span and update `seen_sub_seq`.
- **Vanished worker:** if the selected `active_view_id` is no longer in `subs`, reset it to `"main"` and
  `write_active_view_state(home, yid, "main")`.

### 4.3 Click semantics when `!agent.supports_view_switch()` (`mouse_actions.rs::apply_switch`)
1. `*active_view = vid` (not forced to `"main"`).
2. `write_active_view_state(home, yid, vid)`.
3. If `fp` is non-empty → `trigger_yazi_reveal`.
4. `spawn_center_focus_restore(..., "main", 0)`.
5. `switch_signal_ok.set(Some(true))`.

There is **zero** `trigger_native_view_switch`, `wake_token(claude)`, `req_pid_*` or
`"repatch needed"`. Clicking `● [agy]` sets `vid = "main"`.

### 4.4 Rendering
- **Liveness everywhere:** every `status != "running"` / `== "running"` liveness test becomes
  `!is_subagent_alive(..)` / `is_subagent_alive(..)` at `hit_mask.rs:118`, `mouse_actions.rs:111,263`,
  `dismiss.rs` (ex-`:285,291`), `subagents_rows.rs:126`, and the counts at `subagents_panel.rs:51,202`.
  The `run` count stays `status == "running"`; a new `wait` count is `status == "waiting"`.
- **Title and badge:** when `swarm.active`, the panel title is `"SWARM"` and the badge is
  `#<board_head> · <run>run·<wait>wait`, degrading to `<run>r·<wait>w` below 24 cols.
  Otherwise the title is the existing `"SUBAGENTS"`.
- **Optional title row:** `pub fn swarm_title_rows(active: bool, mid_interior: usize) -> usize`
  (`subagents_rows.rs`) returns 1 iff `active && mid_interior >= 6 && !title.is_empty()`, else 0. The row
  shows the swarm `title`; when a member is selected, it shows that member's latest `board_latest`
  summary instead. **The same value MUST be passed to both the row builder and `hit_mask`**, so
  pointer→row mapping cannot drift.
- **Icons and tags:** waiting `○` / `wait` (dim); killed `×` / `kill` (coral); running/stopped/done
  unchanged. The right-edge dismiss `×` is shown only when `!is_subagent_alive`.

## 5. Hooks, launchers, cleanup
- **`integrations/antigravity/hooks.json` + `hooks.windows.json`:** the PostToolUse matcher appends
  `|swarm|post_message|invoke_subagent|swarm_wait`. Still no PreToolUse and no Stop.
- **`hook_input.rs` / `hook_records.rs` (@backends):** agy tools in `{swarm, post_message,
  invoke_subagent, swarm_wait}` are **wake-only**. They refresh `live_sess_*`/`live_tab_*` (model,
  transcript, agent_pid) and run `notify_sidebar`, but produce no `to_claude_shape` and no Yazi reveal.
  Stdout stays `{}` (§4.2 v0.11).
- **`scripts/ccide`, `scripts/ccide.ps1` (@plugin-scripts):**
  - Accept `--conversation <id>`, `--conversation=<id>` and `$CCIDE_AGY_CONVERSATION` (the flag wins).
  - Validate `^[A-Za-z0-9_-]{8,64}$` (invalid → error, exit 2, before any panes open).
  - When agent == agy, atomically write `<cache_dir>/tab_conv_<yid>.txt` before `workspace open`.
  - **Always forward** the original `--conversation` argument to `agy` unchanged.
- **`spec_io.rs` (@rust-core):** `remove_tab_state` also removes `<cache_dir>/tab_conv_<yid>.txt`.

## 6. Wave 2 file ownership (exclusive)
| Worker | Files (NEW in bold) |
|---|---|
| `@session-mod` | **`session/agy_swarm.rs`**, **`session/agy_swarm_tests.rs`**, `session/{agy,agy_tests,agent_bind,subagents,mod}.rs` |
| `@rust-core` | `json/scan.rs` (`iter_object_members` + test), **`tui/context/agy_swarm_ctx.rs`**, `tui/context/{agent_ctx,codex_parser,agy_parser,mod}.rs`, `tui/pricing/lookup.rs`, `commands/workspace/spec_io.rs` |
| `@backends` | **`sidebar/{dismiss,agent_refresh}.rs`**, `sidebar/{mod,agent_tab,mouse_actions,hit_mask,subagents_panel,subagents_rows}.rs`, `commands/{hook_input,hook_records}.rs` |
| `@plugin-scripts` | `scripts/{ccide,ccide.ps1}`, `integrations/antigravity/{hooks.json,hooks.windows.json}`, `scripts/merge_agent_hooks.py` (§0 fix) |
| `@tests-qa` | `tests/fixtures/agy_swarm/**` (#123), `tests/test_multi_agent.py` (+ swarm manifest matcher, `--conversation` validation, py3.9 import test) |

Rust tests `include_str!`/copy `tests/fixtures/agy_swarm/brain/**`. The required assertions:
- statuses: `arch-opus`→`waiting`, `backends`→`running`, `audit-old`→`killed`, `stale-worker`→`stopped`
- `sub-extra-001`→`running`
- `board_head == 6`
- sort order
- dismissed killed hidden, but dismissed waiting kept
- `sample_agent_swarm` main vs worker view
- COST spans 3 models

**Order:** @tests-qa fixtures (#123) first, then @session-mod `agy_swarm.rs` signatures. Everyone else
codes against this document immediately.
