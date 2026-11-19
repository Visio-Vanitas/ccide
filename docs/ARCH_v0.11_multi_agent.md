# cc-ide v0.11.0 — Multi-Agent Architecture (Claude Code · Antigravity CLI `agy` · Codex CLI)

Status: **FROZEN for Wave 2**. Baseline: `dev` @ `19c2fb8` (v0.10.0). Changing any signature, file
name, wire key or stdout rule below requires an `@arch-opus` amendment on the board.
Verified hosts: `agy` 1.4.7 (`~/.local/bin/agy`), `codex-cli` 0.144.6 (`/opt/homebrew/bin/codex`),
plus the agy `hooks.md`/`plugins.md` docs and on-disk state under `~/.gemini/antigravity-cli/` and `~/.codex/`.

## 0. Schema findings that shape this design (read first)
1. **agy model ≠ Gemini.** `~/.gemini/antigravity-cli/settings.json` `model` is a *display name* that can
   name any vendor (observed: `"Claude Opus 4.6 (Thinking)"`), and hook `modelName` can be `"auto"`.
   → The logo/pricing family always comes from the **model string**. `AgentKind` is only the fallback
   when the model is empty or `auto`/`default` (§2.3).
2. **Codex rollout schema drifts by version.** A Codex Desktop 0.167 rollout has
   `session_meta.payload.context_window = {"window_id":…}` (an object, not a number) and adds
   `token_usage_record` and `task_started.model_context_window`. Parsers must be **tolerant**: unknown
   record types are ignored, and numeric fields are read only when they are numbers (§3.2).
3. **`thread-writer-locks/<uuid>.lock` files are empty and pile up**, so they say nothing about
   liveness. Do not use them.
4. **agy `PreToolUse` is fail-closed and has no "no opinion" verdict** (`allow|deny|ask|force_ask`).
   Registering it means either auto-approving every matched tool (`allow`, which bypasses the user's
   permission policy) or adding prompts (`ask`). **v0.11 registers NO agy `PreToolUse`.** Opt-in
   routing exists only behind `CCIDE_AGY_ORCH=1` with matcher `invoke_subagent` (§4.3).
5. The pricing catalog is `assets/pricing.json`. `config/pricing.json` does not exist.
6. agy runs hooks synchronously in its loop (`sh -c` on Unix, `cmd /c` on Windows), with cwd = the
   directory containing `hooks.json` and `PLUGIN_ROOT` in env (not substituted into `command`).
   Runtime discovery loads `hooks.json` **only from the plugin root**.

## 1. Non-negotiable constraints
MSRV `1.70.0`, Edition 2021, zero crates (no serde/toml/rusqlite). JSON goes through `crate::json::*`;
TOML goes through the new `crate::json::toml_lite` (top-level scalars only). Every `.rs` file stays
≤ 300 lines after `rustfmt` (target ≤ 285). No `crate::commands` in `src/{session,tui,json,budget,
platform,ffi,orchestrator,telemetry}` or in `model_id.rs`/`proto.rs`. No orchestrator↔session cycle.
All shared state goes through `platform::atomic_write`, every signal through `ffi::wake_token`, and
every external CLI call is time-bounded. **Hooks and disk discovery are both required paths**: every
sidebar feature must work from disk discovery alone. Hooks only add low latency and exact binding.

## 2. `AgentKind` & the `session/` seam
### 2.1 `src/session/agent.rs` (NEW, ≤ 140 lines)
```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentKind { #[default] Claude, Agy, Codex }
impl AgentKind {
    pub fn as_str(self) -> &'static str;               // "claude" | "agy" | "codex"
    pub fn from_name(s: &str) -> Option<Self>;          // + aliases "claude-code", "antigravity", "openai-codex"
    pub fn default_family(self) -> &'static str;        // "claude" | "gemini" | "openai"
    pub fn supports_view_switch(self) -> bool;          // Claude only (SIGUSR2 patch / ready_pid)
    pub fn badge(self) -> &'static str;                 // "main" | "agy" | "codex"
}
pub fn read_tab_agent(home: &str, yid: &str) -> AgentKind;        // cache_dir/tab_agent_<yid>.txt; missing → Claude
pub fn write_tab_agent(home: &str, yid: &str, k: AgentKind);      // atomic, content "<as_str>\n"
pub fn agy_home() -> PathBuf;    // $CCIDE_AGY_HOME else ~/.gemini/antigravity-cli
pub fn codex_home() -> PathBuf;  // $CODEX_HOME else ~/.codex
```
`tab_agent_<yid>.txt` is written by the launchers before `workspace open`. The `remove_tab_state` sweep
(spec_io, @rust-core) also deletes `<cache_dir>/tab_agent_<yid>.txt`.

### 2.2 Binding (`binding.rs`, `agent_bind.rs` NEW)
`SessionBinding` gains `pub agent: AgentKind, pub transcript_path: String` and stays
`#[derive(Clone, Debug, Default, PartialEq, Eq)]`. All 7 literal sites (`session/{mod,binding}.rs`) add
`..Default::default()`. Claude binding logic is unchanged and sets `agent: Claude, transcript_path: ""`.
```rust
// src/session/agent_bind.rs (NEW, ≤ 220 lines)
pub struct AgentSession { pub kind: AgentKind, pub session_id: String, pub agent_pid: u32,
                          pub transcript_path: PathBuf, pub model_id: String, pub cwd: String }
pub fn resolve_agent_session(home: &str, yid: &str, cwd: &str, shell_pid: u32, kind: AgentKind)
    -> Option<AgentSession>;
pub fn agent_session_to_binding(s: &AgentSession) -> SessionBinding; // ui_pid = data_pid = agent_pid
```
Resolution order (first hit wins; the result is cached by the caller per 160 ms tick):
1. **Hook record**: `<cache_dir>/live_tab_<yid>.json` with `"agent" == kind`, a `"transcript_path"` that
   exists, and an `"agent_pid"` token that passes `ProcToken::parse(..).is_alive_match()`, or a bare pid
   that passes `is_pid_alive`.
2. **Disk fallback (before the first hook has fired, or with hooks absent)**:
   - agy: read `cache/last_conversations.json` (object: workspace path → cid). Choose the key with
     `paths_equal_ci(canon(key), canon(cwd))`, else the longest ancestor-key of `cwd`. The transcript is
     `brain/<cid>/.system_generated/logs/transcript.jsonl` and must exist and have changed in the last
     24 h. `agent_pid = 0`.
   - codex: `codex::find_rollout_for_cwd(cwd)` (§2.4). `agent_pid = 0`.
3. **Sibling guard (F8 parity):** skip any candidate `session_id` claimed by another live tab
   (`session_<sid>.yazi` + live `sidebar_tab_<yid>.json` token). Commit with the existing `commit_binding`.

`resolve_isolated_session_binding()` stays Claude-only. The sidebar dispatches on
`read_tab_agent(home, yid)`: Claude goes through the existing path, Agy/Codex through `resolve_agent_session`.

### 2.3 `agy.rs` / `codex.rs` (NEW, ≤ 280 lines each, pure parsing + bounded fs walks)
```rust
// src/session/agy.rs
pub fn agy_cid_for_cwd(cwd: &str) -> Option<String>;
pub fn agy_transcript_path(cid: &str) -> PathBuf;
pub fn agy_settings_model() -> Option<String>;                      // settings.json "model"
pub fn agy_main_file_ops(path: &Path, tr: &mut JsonlTracker) -> Option<MainSessionOps>;
pub fn agy_subagents(path: &Path, tr: &mut JsonlTracker) -> Vec<SubagentItem>;
// src/session/codex.rs
pub fn find_rollout_by_id(session_id: &str) -> Option<PathBuf>;     // name suffix "-<id>.jsonl"
pub fn find_rollout_for_cwd(cwd: &str) -> Option<PathBuf>;
pub fn codex_config() -> CodexConfig; // { model: Option<String>, context_window: Option<u64>, effort: Option<String> }
pub fn codex_main_file_ops(path: &Path, tr: &mut JsonlTracker) -> Option<MainSessionOps>;
pub fn codex_subagents(home: &str, main_sid: &str, main_path: &Path) -> Vec<SubagentItem>;
```
- **Tail parsing** reuses `JsonlTracker` (incremental offset, partial-line safe). Each line is gated by
  cheap substring checks before any `extract_json_*` call.
- **agy file ops:** `source == MODEL` lines carrying `tool_calls[*]` `{name, arguments}`. Path key
  precedence: `Path` → `AbsolutePath` → `TargetFile`. Tool → semantic map: `read_file|view_file` → Read
  (`StartLine/EndLine`); `edit_file` (`Target/Replacement`), `replace_file_content` /
  `multi_replace_file_content` (`TargetContent/ReplacementContent`) → Edit; `write_file|write_to_file` → Write.
- **agy subagents:** `invoke_subagent` calls become `SubagentItem{ id: <toolCall id>, desc: Role|TypeName,
  model_raw: Model }`, view-only (no transcript switching). `swarm` `add` gives `{name, model}`.
- **codex file ops:** `response_item` with `payload.type ∈ {function_call, custom_tool_call}` and
  `name == "apply_patch"`. Take the patch text from `input` (custom) or `arguments` → `input|patch` (JSON
  string, unescaped by `extract_json_str`), scan its lines for `*** Update File: ` / `*** Add File: `, and
  use the last such path as `file`. The line comes from the first `@@` hunk header if present, else 0.
- **codex discovery:** walk `sessions/YYYY/MM/DD` for today, yesterday and the day before (local date,
  newest first; at most 400 files stat'ed). Read only line 1 (`session_meta`). Choose
  `paths_equal_ci(canon(payload.cwd), canon(cwd))`, an empty `parent_thread_id`, and the newest mtime
  within 12 h. `find_rollout_by_id` checks the same 3 day dirs by file name, then
  `session_index.jsonl` `updated_at` → that date's dir.
- **codex subagents:** rollouts in the same day dirs whose line 1 has `parent_thread_id == main_sid`
  become `SubagentItem{ id, desc: agent_nickname|agent_path, model_raw: last turn_context.model,
  done: task_complete seen }`.
- `CodexConfig` is read with `toml_lite` (top-level keys before the first `[table]` only).

### 2.4 Models (`model_compact.rs`, `model.rs`, `model_id.rs`)
- `classify_model_family` adds: contains `gemini|argon|hydrogen|neon|antigravity` → `"gemini"`;
  contains `codex|gpt|openai` or an o-token → `"openai"`. Order: gemini, deepseek, grok, openai, else
  claude (`"Claude Opus 4.6 (Thinking)"` → claude, `"gpt-6-astra"` → openai).
- NEW `pub fn classify_model_family_for(agent: AgentKind, model: &str) -> &'static str`: an empty model or
  `auto|default|inherit` (case-insensitive) → `agent.default_family()`, else `classify_model_family`.
- NEW `pub fn load_config_models_for(agent, home, session_id, tab_yazi_id, live_model: Option<&str>) ->
  ConfigModels`. Claude delegates to the existing `load_config_models`. Agy: main = live `model_id`
  (if not `auto`), else `agy_settings_model()`, else `"auto"`. Codex: live `model_id`, else the last
  rollout `turn_context.model`, else `codex_config().model`. Sub model = main for both.
- `model_id::is_valid_model_name` already accepts spaces and parentheses. Add tests for
  `"Claude Opus 4.6 (Thinking)"`, `"gpt-6-astra"` and `"auto"` (valid string; family resolved via
  `_for`). `short_model_label` / `compact_model_for_width` must not panic on display names (tests at
  widths 6/10/16).

## 3. `tui/context/` parsers
### 3.1 Shared sample (`src/tui/context/agent_ctx.rs` NEW, ≤ 230 lines)
```rust
#[derive(Clone, Debug, Default)]
pub struct AgentCtxSample { pub model: String, pub ctx_window: u64,
    pub occupancy: u64, pub last_input: u64, pub last_cached: u64, pub last_output: u64, pub last_reasoning: u64,
    pub cum_input: u64, pub cum_cached: u64, pub cum_cache_write: u64, pub cum_output: u64,
    pub turns: usize, pub weights: [u64; 5] /* sys_md, tools, tool_io, think, chat bytes */,
    pub perf: Vec<RawPerfSample>, pub per_model: Vec<(String, u64, u64, u64)> /* in, cached, out */ }
#[derive(Default)] pub struct AgentCtxTracker { pub offset: u64, pub partial: String, pub s: AgentCtxSample }
impl SessionContextAnalyzer {   // child module of `context`, so it may touch private fields
    pub fn sample_agent(&mut self, home: &str, cwd: &str, sess: &AgentSession, view_id: &str) -> ContextSnapshot;
}
```
`SessionContextAnalyzer` gains one field, `agent_trackers: HashMap<PathBuf, AgentCtxTracker>`. To keep
`mod.rs` ≤ 285, `ModelCostEntry` + `ContextSnapshot` move to `src/tui/context/snapshot.rs` (NEW) and are
re-exported from `mod.rs`, so paths don't change. `sample_agent` builds the snapshot with the same
budget/pricing path as `sample`: `occupancy` drives `total_in_tokens`, `apportion_tokens(occupancy,
weights)` the category split, `cum_cached/cum_input` the hit %, `per_model` the costs, and `perf`
`ModelPerfAccum`. `rl_*`/`lines_*` stay `None`/0.

### 3.2 Parsers (`agy_parser.rs`, `codex_parser.rs` NEW, ≤ 200 lines each)
`pub fn update_agy_ctx(path: &Path, tr: &mut AgentCtxTracker)` / `pub fn update_codex_ctx(path: &Path, tr: &mut AgentCtxTracker)`.
- **agy** (`transcript.jsonl`): per `PLANNER_RESPONSE` read `input_tokens`, `output_tokens`,
  `cache_read_tokens`. **Occupancy rule (single function `agy_occupancy`):** if
  `classify_model_family(model)=="claude"` then `input + cache_read` (Anthropic counts them exclusively),
  else `input` (Gemini/OpenAI counts are inclusive). Weights: `thinking` bytes → think, `tool_calls` +
  `GENERIC`/tool results → tool_io, `USER_INPUT` + `content` → chat. Perf: TTFT ≈ first
  `PLANNER_RESPONSE.created_at − USER_INPUT.created_at`, duration = last step − `USER_INPUT`
  (`record_fallback`, i.e. marked approximate).
- **codex** (`rollout-*.jsonl`): `event_msg/token_count` → `info.last_token_usage` (last_*) and
  `info.total_token_usage` (cum_*). Occupancy = `last.input_tokens` (inclusive of cached). Window =
  `info.model_context_window`. `turn_context` → model and `turns += 1`. `event_msg/task_complete` →
  exact perf `{ttft: time_to_first_token_ms, dur: duration_ms, out: last output_tokens}`. Weights: message
  role user → chat, assistant `reasoning` → think, function_call(+_output) → tool_io. Ignore
  `token_usage_record`, `world_state` and any unknown record type.
- **Window precedence:** codex `token_count.info.model_context_window` → `task_started.model_context_window`
  → numeric `session_meta.context_window` → `codex_config().context_window` →
  `resolve_model_window_hint`. agy: `$CCIDE_AGY_CONTEXT_WINDOW` → family heuristic (gemini 1_048_576;
  claude 200_000, or 1M if `is_native_1m_model_id`; openai 400_000) → 200_000.
- **Pricing:** `tui/pricing/lookup.rs::normalize_model_key` also lowercases, drops parenthesized
  suffixes (`(thinking)`), maps spaces → `-` and `.` → `-` between digits (`claude-opus-4-6`), before
  the existing progressive lookup. `assets/pricing.json` gains only **aliases** for unknown display
  names. Rates come from `ccide --sync-pricing` (litellm/genai-prices); no hand-typed prices.

## 4. Multi-agent hook contracts
### 4.1 CLI & detection (`src/main.rs`, `src/commands/hook_input.rs` NEW ≤ 260 lines)
`ccide-core yazi-hook [--agent claude|agy|codex]`, `ccide-core pre-tool-hook [--agent …]`.
```rust
pub enum HookEventKind { SessionStart, PreToolUse, PostToolUse, SubagentStart, SubagentStop, PostModelSwitch, Stop, Other }
pub struct HookEvent { pub agent: AgentKind, pub event: HookEventKind, pub session_id: String, pub cwd: String,
    pub transcript_path: String, pub model: String, pub tool_name: String, pub tool_input: String /* raw JSON */,
    pub agent_id: String, pub perm_mode: String }
pub fn detect_agent(input: &str, forced: Option<AgentKind>) -> AgentKind;
pub fn parse_hook_event(input: &str, forced: Option<AgentKind>) -> HookEvent;
pub fn to_claude_shape(ev: &HookEvent) -> Option<(String, String)>; // (tool_name, tool_input JSON) for focus.rs
pub fn emit_required_stdout(agent: AgentKind, ev: HookEventKind);   // §4.2, printed FIRST, then flushed
```
- **Detection precedence:** `--agent`, then an unambiguous payload (`"conversationId"` or `"hookEventName"`
  → Agy; a `transcript_path` under `$CODEX_HOME`, or a `"workspace_dir"` key → Codex; a `transcript_path`
  under `~/.claude/` → Claude), then `$CCIDE_AGENT`, then Claude.
- **Field map:**
  - Agy: `hookEventName`, `conversationId`, `cwd` (else `workspacePaths[0]`), `transcriptPath`,
    `modelName`, `toolCall.name`, `toolCall.args`.
  - Codex: `hook_event_name|event`, `session_id`, `cwd|workspace_dir`, `tool_name`, `tool_input`.
  - Claude: unchanged.
- `to_claude_shape` rewrites agy/codex tools into the Claude `Read|Edit|Write` + `{file_path, offset,
  limit, old_string, new_string}` JSON (built with `json_escape`). That way
  `session::extract_tool_line_target` and the Yazi reveal path are reused unchanged.
- Codex `apply_patch` maps to `Edit` with `file_path` = the last `*** Update/Add File:` path and no strings.
- **Record writes (agy/codex, every event):** atomically write `live_sess_<sid>.json` and, when the tab
  resolves via `resolve_live_yazi_tab`, `live_tab_<yid>.json`. Keys: the existing statusline subset
  (`"model_id"`, `"model"`, `"cwd"`, `"updated"`) plus `"agent"`, `"transcript_path"`, and
  `"agent_pid":"<ProcToken>"` = the hook process's ancestor whose parent is the tab shell
  (`get_ppid_of` walk, ≤ 33 hops). Then `notify_sidebar(sid, yid)`.
  Also `write_tab_agent` if the file is missing.
- **Latency budget:** ≤ 100 ms per hook, no network, children detached (existing bounded `ya emit-to`).

### 4.2 Strict stdout table (exit code is always 0; diagnostics go to stderr only, never panics:
the entry is wrapped in `std::panic::catch_unwind`)
| Host | SessionStart | PreToolUse | PostToolUse | Stop / other |
|---|---|---|---|---|
| claude | unchanged (nothing) | unchanged (`hookSpecificOutput` JSON or nothing) | nothing | nothing |
| agy | `{}` | `{"decision":"ask"}` unless §4.3 | `{}` | `{}` |
| codex | **empty** | **empty** | **empty** | **empty** |

### 4.3 Orchestrator routing per host
- Claude: unchanged.
- Codex: `pre-tool-hook --agent codex` always prints nothing and is **not registered** in v0.11.
- agy: not registered by default. With `CCIDE_AGY_ORCH=1` and matcher `invoke_subagent`, it emits
  `{"decision":"allow","overwrite":{"Model":"<routed>"}}` when the orchestrator maps the call's
  `Role`/`TypeName` to a model, and `{"decision":"ask"}` otherwise. Documented as "auto-approves
  subagent spawns".

## 5. Sidebar UX & doctor
- **Agent per tab:** `sidebar/mod.rs` reads `read_tab_agent` at start and on mtime change (`tab_agent_*`).
  Agy/Codex tabs use `resolve_agent_session`, `load_config_models_for`, `sample_agent`, and
  `agy_/codex_main_file_ops` for the Yazi reveal (the same `main_last_file_<sid>.txt` writer).
- **No Claude-only behaviour on non-Claude tabs:** when `!agent.supports_view_switch()`, skip
  `claude_signalable`, `req_pid_*`, `wake_token(claude)` and the `"repatch needed"` notice
  (`mouse_actions.rs:152,161`, `bottom_panel.rs:241`).
  - A click on the main row still runs `focus_center_after_view_switch(...,"main")`.
  - A click on a subagent row reveals that subagent's last file op (if any) in Yazi and focuses the
    center, with no view switch.
- **Labels and sprites:** the main row label is `● [<badge>]` (`[main]` for Claude, `[agy]`, `[codex]`).
  The logo family is `classify_model_family_for(agent, model)`. Existing sprites
  `assets/sprites/{claude,gemini,openai,deepseek,grok}.{png,ansi}` cover every family; no new assets.
- **Line budget:** `subagents_panel.rs` (298) and `sidebar/mod.rs` (293) MUST shed rows/helpers before
  any edit. Extract row formatting into `subagents_rows.rs` (NEW) and agent dispatch into
  `agent_tab.rs` (NEW). Do not touch `dispatch.rs` (299).
- **Doctor:** new `src/commands/setup/doctor_agents.rs`. `doctor_probes.rs` (292) must not grow; it
  gets only one call line, and `doctor.rs` adds the items.
  - CLI probes: `claude|agy|codex --version` through
    `crate::commands::workspace::ghostty_linux::run_bounded(bin, args, 2000)`. A missing CLI is `Ok`
    ("not installed"), not `Warn`.
  - Hook registration: agy `~/.gemini/config/plugins/cc-ide/hooks.json` contains `ccide-core`; codex
    `$CODEX_HOME/hooks.json` contains `ccide-core`.
  - JSON additive keys: `"agents":{"claude":"<ver|null>","agy":…,"codex":…}` and
    `"agent_hooks":{"agy":bool,"codex":bool}`.

## 6. Launchers, integration manifests, installers
- **`scripts/ccide`, `scripts/ccide.ps1`:**
  - Flags `--agent <claude|agy|codex>`, `--agy`, `--codex`, `$CCIDE_AGENT` (flag wins). Unknown agent → error, exit 2.
  - Export `CCIDE_AGENT` and write `tab_agent_<yid>.txt` (`ccide-core` is not needed; plain
    write-then-mv) BEFORE `workspace open`.
  - Run `auto_ensure_claude_patched` / `patch_claude_binary.py` ONLY for `claude`.
  - Exec `claude …` (unchanged), `agy …` or `codex …`, forwarding remaining args verbatim.
    `--continue` etc. belong to the agent.
  - Missing agent binary → message and exit 127, before any panes open.
- **`integrations/antigravity/`** (native plugin; `hooks.json` at the plugin root):
  - `plugin.json` `{"name":"cc-ide","author":{"name":"Visio-Vanitas"},"homepage":"https://github.com/Visio-Vanitas/ccide"}`.
  - `hook.sh` / `hook.cmd`: exec `${CCIDE_CORE_BIN:-$HOME/.claude/bin/ccide-core} "$@"`. If the binary
    is missing, print `{}` and exit 0.
  - `hooks.json` (Unix; the installer substitutes `hooks.windows.json` with `hook.cmd` on Windows):
    `{"cc-ide":{"SessionStart":[{"type":"command","command":"./hook.sh yazi-hook --agent agy","timeout":5}],
    "PostToolUse":[{"matcher":"read_file|view_file|edit_file|replace_file_content|multi_replace_file_content|write_file|write_to_file","hooks":[{"type":"command","command":"./hook.sh yazi-hook --agent agy","timeout":5}]}]}}`.
    No `PreToolUse` and no `Stop`.
- **`integrations/codex/`:**
  - `.codex-plugin/plugin.json` with keys `name, version, description, author, homepage, repository,
    license, keywords, hooks:"./hooks.json"`.
  - Claude-compatible `hooks.json` `{"hooks":{…}}`: `SessionStart` + `PostToolUse` matcher `apply_patch`, each
    `{"type":"command","command":"<abs>/hook.sh yazi-hook --agent codex","commandWindows":"<abs>\\hook.cmd yazi-hook --agent codex","timeoutSec":5}`.
  - **Amendment #82 (verified by @coordinator, codex 0.144.6 `ConfiguredHookHandler`):** the handler keys
    are `command`, `commandWindows`, `timeoutSec` and `statusMessage`. There is NO `timeout` key, and
    `async` is unsupported (never set it). One `hooks.json` serves both OSes, so `hooks.windows.json`
    is optional for codex.
- **`install.sh` / `install.ps1`** (non-destructive, idempotent, reversible):
  - agy: copy to `~/.gemini/config/plugins/cc-ide/` plus a `.ccide-managed` marker. If the directory
    exists without the marker, skip and warn.
  - codex: merge with NEW `scripts/merge_agent_hooks.py` (`python3 -I`). It inserts or removes only
    entries whose command contains `ccide-core`/`cc-ide`, backs up `hooks.json.ccide.bak` once, and
    never rewrites foreign entries.
  - `--uninstall` reverses both.
  - Skip each integration when its CLI is absent, unless forced with `--with-agy` / `--with-codex`.

## 7. Wave 2 file ownership (exclusive; budget = post-`rustfmt` lines)
| Worker | Files (NEW in bold) | Budget notes |
|---|---|---|
| `@session-mod` | **`session/{agent,agent_bind,agy,codex}.rs`**, `session/{binding,model,model_compact,mod}.rs` | mod.rs ≤ 290 (re-exports only); binding ≤ 285 |
| `@rust-core` | **`json/toml_lite.rs`**, `json/mod.rs`, **`tui/context/{agent_ctx,agy_parser,codex_parser,snapshot}.rs`**, `tui/context/mod.rs`, `tui/pricing/lookup.rs`, `assets/pricing.json`, `model_id.rs` (tests), `commands/workspace/spec_io.rs` (+`tab_agent_` cleanup) | context/mod.rs must DROP to ≤ 250 via snapshot.rs |
| `@backends` | **`commands/hook_input.rs`**, `commands/{yazi_hook,pre_tool_hook}.rs`, `main.rs`, `commands/sidebar/{mod,ipc,mouse_actions,bottom_panel,subagents_panel}.rs`, **`sidebar/{subagents_rows,agent_tab}.rs`**, **`setup/doctor_agents.rs`**, `setup/doctor.rs` (+1 call in `doctor_probes.rs`) | yazi_hook ≤ 270; subagents_panel ≤ 260; sidebar/mod ≤ 285 |
| `@plugin-scripts` | `scripts/{ccide,ccide.ps1}`, `install.sh`, `install.ps1`, **`integrations/antigravity/*`**, **`integrations/codex/**`**, **`scripts/merge_agent_hooks.py`** | py ≤ 285 |
| `@tests-qa` | **`tests/fixtures/{agy,codex}/*.jsonl`** (sanitized, ≤ 40 lines each, includes the 0.167 object-`context_window` variant), **`tests/test_multi_agent.py`** (manifest JSON validity, hook wrapper `{}`/empty-stdout behaviour, `merge_agent_hooks.py` idempotence, launcher `zsh -n` / flag parsing) | Rust unit tests `include_str!` these fixtures |

**Order:** @tests-qa posts the fixtures first (Rust tests depend on them). @session-mod lands
`agent.rs` + `AgentSession` signatures. Everyone else codes against this document immediately.
