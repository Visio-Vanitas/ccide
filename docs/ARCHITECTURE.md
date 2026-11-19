# `cc-ide` (`ccide-core`) Architecture Overview (`v0.11.1`)

`ccide-core` is a zero-crate, MSRV `1.70.0` Rust binary (Edition 2021) orchestrating a 3-column adaptive terminal IDE (`30% | 54% | 16%`) across **Claude Code**, **Google Antigravity CLI (`agy`)**, **Antigravity Swarm**, and **OpenAI Codex CLI (`codex`)** on macOS, Linux, and Windows.

For frozen per-release interface contracts, see [`ARCH_v0.10.md`](./ARCH_v0.10.md), [`ARCH_v0.11_multi_agent.md`](./ARCH_v0.11_multi_agent.md), and [`ARCH_v0.11.1_agy_swarm.md`](./ARCH_v0.11.1_agy_swarm.md); for full maintenance notes, see [`../AGENT.md`](../AGENT.md).

---

## 1. Layering & Engineering Invariants

- **Strict Layering**: Leaf and domain modules `src/{ffi,telemetry,session,tui,orchestrator,json,budget,platform,model_id,proto}` must **never** import `crate::commands`, and `src/session` and `src/orchestrator` have zero mutual imports (enforced by `tests/test_agent_infra.py`).
- **Modular `<= 300`-Line Cap**: Every `.rs` source file stays `<= 300` lines post-`rustfmt`.
- **IPC & Signal Safety**:
  - All shared IPC files live under `platform::ipc_dir()` (`$CCIDE_IPC_DIR`, else `/tmp/.cc-ide-<euid>` mode `0700` on Unix or `%TEMP%\.cc-ide` on Windows), written via `platform::atomic_write`.
  - Process liveness and signals use generation-guarded `ffi::ProcToken` (`<pid>:<start_ms>`) and `ffi::wake_token` (plus `ready_pid_<pid>` proof gating for Claude `SIGUSR2` view switching).

---

## 2. Rust Module Summary (`src/`)

### Leaf & Infrastructure Modules
- `src/main.rs` — CLI entrypoint (`--version`, `proto`, `ipc-dir`, `wake-pid`, `sidebar`, `yazi-hook [--agent ...]`, `pre-tool-hook [--agent ...]`, `statusline`, `workspace`, `pane-runner`, `setup`, `doctor`). Hook commands wrap execution in `std::panic::catch_unwind` to exit `0` safely.
- `src/platform/{mod,ipc}.rs` — Cross-OS home/cache/executable paths, `0700` per-UID `ipc_dir()` (`legacy_ipc_dir()` retained read-only until removed), `atomic_write`, and `paths_equal_ci`.
- `src/ffi/{mod,unix,macos,windows,win_sys}.rs` — Kernel process ancestry (`proc_pidinfo`, `/proc/<pid>/stat`, `CreateToolhelp32Snapshot` + `GetProcessTimes`), `ProcToken`, and `wake_token`.
- `src/json/{mod,extract,scan,jsonc,toml_lite}.rs` — Zero-dependency JSON extractor, balanced array scanner, JSONC stripper/escaper, and top-level TOML scalar parser (`extract_toml_scalar` for `$CODEX_HOME/config.toml`).
- `src/budget/{mod,config}.rs`, `src/model_id.rs`, `src/proto.rs` — Context window & auto-compact math, model-ID validation & shim slugs, and protocol version constants (`IPC_PROTO = 1`, `PATCH_PROTO = 1`, `SPEC_VERSION = 2`).

### Multi-Agent Session & Discovery (`src/session/`)
- `src/session/agent.rs` — `AgentKind::{Claude, Agy, Codex}`, `tab_agent_<yid>.txt` persistence, `agy_home()` (`$CCIDE_AGY_HOME` / `~/.gemini/antigravity-cli`), and `codex_home()` (`$CODEX_HOME` / `~/.codex`).
- `src/session/agent_bind.rs` & `binding.rs` — `AgentSession`, `resolve_agent_session` (hook `live_tab_<yid>.json` with `ProcToken` liveness -> disk fallbacks -> sibling claim guard), and `SessionBinding { session_id, ui_pid, data_pid, agent, transcript_path }`.
- `src/session/agy.rs`, `agy_tests.rs`, `agy_swarm.rs`, & `agy_swarm_io.rs` — Agy workspace/conversation lookup (`tab_conv_<yid>.txt`, `$CCIDE_AGY_CONVERSATION`, `last_conversations.json`, bounded brain scan), `settings.json` model fallback, tool-op extraction (`Path`/`AbsolutePath`/`TargetFile`), and Antigravity Swarm discovery (`swarm.json`, `members.json`, bounded `board.jsonl` tail, `subagents/*.json`, `running`/`waiting`/`stopped`/`killed` status mapping, and worker transcript/file-op resolution).
- `src/session/codex.rs` & `codex_discover.rs` — Codex TOML config reader, bounded 3-day `sessions/YYYY/MM/DD/rollout-*.jsonl` & `session_index.jsonl` discovery, `apply_patch` file-op extraction, and `parent_thread_id` subagent rollouts.
- `src/session/{mod,scan,model,model_compact,focus,transcript,transcript_main,subagents,subagent_state}.rs` — Claude process-tree isolation, family classification (`classify_model_family_for`), and Yazi reveal IPC.

### TUI, Pricing & Multi-Agent Context Parsers (`src/tui/`)
- `src/tui/context/{mod,snapshot,agent_ctx,agy_swarm_ctx,agy_parser,codex_parser,analyzer,bar,cost_panel,json_scan,parser,perf,render}.rs`:
  - `snapshot.rs` holds `ContextSnapshot` and `ModelCostEntry`.
  - `agent_ctx.rs` implements `SessionContextAnalyzer::sample_agent` with per-transcript `AgentCtxTracker` state, 10-minute idle tracker pruning, and agent/family context-window precedence.
  - `agy_swarm_ctx.rs` implements `sample_agent_swarm`, keeping `CONTEXT` focused on the coordinator or clicked worker while aggregating `COST` across the coordinator and unique worker transcripts.
  - `agy_parser.rs` parses `PLANNER_RESPONSE` token/cache counts (`agy_occupancy` adds `input + cache_read` for Claude models vs inclusive `input` for Gemini/OpenAI) and `created_at` step deltas.
  - `codex_parser.rs` parses `token_count`, `task_started`, `turn_context`, and `task_complete` (`time_to_first_token_ms`, `duration_ms`) while tolerating schema drift (e.g., object `context_window` in Codex Desktop 0.167).
- `src/tui/pricing/{mod,lookup}.rs` — Display-name normalization (`"Claude Opus 4.6 (Thinking)"` -> `claude-opus-4-6`) against `assets/pricing.json`.
- `src/tui/{grid,grid_render,layout,width,mod}.rs`, `src/tui/logo/{mod,image,ansi}.rs`, `src/tui/palette/*`.

### Commands, Workspace & Host Hooks (`src/commands/`)
- `src/commands/hook_*.rs` (`hook_input.rs` + `hook_records.rs`), `yazi_hook.rs`, `pre_tool_hook.rs`:
  - Auto-detects host (`--agent` flag -> payload keys -> `$CCIDE_AGENT` -> `Claude`), normalizes Agy/Codex tool calls (`to_claude_shape`) for Yazi hunk reveal, writes `live_sess_<sid>.json` + `live_tab_<yid>.json`, and flushes strict host-specific stdout (`{}` for Agy, empty stdout for Codex, `CCIDE_AGY_ORCH=1` opt-in `invoke_subagent` routing).
- `src/commands/sidebar/{mod,agent_tab,agent_refresh,bottom_panel,dismiss,dispatch,hit_mask,input,ipc,ipc_guard,mouse_actions,orch_panel,subagents_panel,subagents_rows}.rs`:
  - `agent_tab.rs` & `agent_refresh.rs` dispatch `Agy`/`Codex` tabs and Swarm state (`TabRefresh`, `sample_ctx`) cleanly without Claude-only `SIGUSR2` view-switching or `"repatch needed"` warnings; `dismiss.rs` guards `waiting` Swarm workers against dismissal while allowing `killed`/`done` cleanup; `subagents_rows.rs` renders `[main]` / `[agy]` / `[codex]` badges, `SWARM #<board_head>` header, title, and selected-worker board snippet.
- `src/commands/setup/{mod,doctor,doctor_probes,doctor_agents,doctor_render,input,runner,ui}.rs`:
  - `doctor_agents.rs` probes `claude`, `agy`, `codex` `--version` (`run_bounded`, `2s`), checks `~/.gemini/config/plugins/cc-ide/hooks.json` and `$CODEX_HOME/hooks.json`, and adds `"agents"` / `"agent_hooks"` to `doctor --json`.
- `src/commands/workspace/{mod,spec,spec_io,layout,open,detect,rebalance,reload,actions,iterm,ghostty,ghostty_script,ghostty_linux,tmux,wezterm,wt}.rs` — `PaneSpec` v2, `LayoutPlan` (`22/48/28`), `workspace detect`, `rebalance` (`0`/`3`/`1`), Ghostty `OSC 2` targeting, and Lazygit `reload_lg_<yid>`.
- `src/commands/pane_runner.rs`, `src/commands/statusline/{mod,git_info}.rs`.

---

## 3. Host Integrations & Launchers

- **`scripts/ccide` & `scripts/ccide.ps1`**: Parse `--agent <claude|agy|codex>`, `--agy`, `--codex`, `$CCIDE_AGENT`, `--conversation <id>` / `CCIDE_AGY_CONVERSATION` (`^[A-Za-z0-9_-]{8,64}$`), and `CCIDE_AGY_HOME` (invalid -> exit `2`, missing CLI -> exit `127`), write `tab_agent_<yid>.txt` / `tab_conv_<yid>.txt`, run `patch_claude_binary.py` only for `claude`, and exec the selected CLI.
- **`integrations/antigravity/`**: Native plugin bundle (`plugin.json`, `hooks.json`, `hooks.windows.json`, `hook.sh`, `hook.cmd`) registering `SessionStart` and `PostToolUse` (no `PreToolUse` by default). Installed non-destructively to `~/.gemini/config/plugins/cc-ide/` (`.ccide-managed`).
- **`integrations/codex/`**: Codex plugin & hook bundle (`.codex-plugin/plugin.json`, `hooks.json`, `hooks.windows.json`, `hook.sh`, `hook.cmd`) using `command`/`commandWindows`/`timeoutSec: 5` (no `async` flag). Merged non-destructively into `$CODEX_HOME/hooks.json` by `scripts/merge_agent_hooks.py`.
