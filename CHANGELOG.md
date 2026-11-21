# Changelog

All notable changes to **`ccide` (`cc-ide` / `ccide-core`)** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

---

## [0.11.3] - 2026-11-21

### Fixed
- **Lazygit CJK Horizontal Scrolling & Focus Auto-Sync (`scripts/pty-hscroll-bridge.py`, `tests/test_viewer_and_hscroll.py`)**:
  - Added `_row_plain_text` in `match_visible_main` so 2-column CJK/wide-character `""` trailing halves are not expanded into spaces (while keeping `find_boxes` and `has_popup_over_main` column coordinates exact).
  - Fixed `can_scroll_right` to treat trailing wide-char `""` halves at `edge_x` as overflowing content (`!= " "`), and tracked `cell_bold` in `VtScreen` to sync `focused_panel` automatically from active bold borders.
- **Lazygit `[0]-Log` 4-Column CJK Author Alignment (`scripts/lazygit-log.py`, `tests/test_viewer_and_hscroll.py`)**:
  - Tightened wide-character budget in `compact_author` to `<= 3` display columns and stripped trailing separator/ZWJ residue so CJK and ASCII rows align cleanly at `3 + 1 = 4` display columns alongside Lazygit `[4]-Commits`, verified across both unwrapped and `--wrap` continuation lines.
- **Right Sidebar `SUBAGENTS` / `SWARM` `Tab` Cursor Navigation (`src/commands/sidebar/{input,nav,dispatch,mod}.rs`)**:
  - Split `\t` into `InputEvent::Tab` and `\x1b[Z`/`\x1b[1;2Z` into `InputEvent::BackTab`; when `FocusZone::Subagents` is active (e.g., after clicking `SubagentFrame`), `Tab`/`Shift+Tab` cycles `sub_cursor` (or `orch_cursor` in orchestrator view) downward/upward with wrapping and synced scrolling, while keeping `Tab` bottom-panel toggle outside `Subagents` and `t`/`T` global toggle everywhere.

---

## [0.11.2] - 2026-11-20

### Fixed & Hardened
- **Real Antigravity `transcript.jsonl` Parsing, Coordinator Promotion & Path Filtering (`src/session/`, `src/platform/mod.rs`, `src/json/scan.rs`)**:
  - Supported both `"arguments"` and `"args"` (including double-encoded JSON string values via `extract_agy_arg_str` and quoted integers via `extract_agy_arg_u64`), preferred `agy_transcript_for_cid`, and filtered internal/temp paths (`/.gemini/`, `/.codex/`, `/.system_generated/`, `/.gemini/antigravity`, `/.codex/sessions/`) out of Yazi reveals.
  - Promoted implicit or hook-bound Agy worker `conversation_id`s to their Swarm coordinator via `agy_parent_coordinator` / `agy_is_member_of` while letting explicit `--conversation` / `tab_conv_<yid>.txt` overrides bypass the 24 h mtime cutoff (`agy_resolve_cid_detail`).
  - Pre-seeded empty transcript models from `members.json` / `sess.model_id` before `update_agy_ctx`, switched `agy_occupancy` to add `input + cache_read` for Claude models or when `input < cache_read`, decoded `file:///C:/...` across all OSes, and enforced `is_safe_id` in `agent.rs` and `scan.rs` plus `saturating_sub` / `unescape_key` in `json/scan.rs`.
- **Swarm Worker Hook Isolation & Sidebar Protection (`src/commands/{hook_input,hook_records,yazi_hook}.rs`, `src/commands/sidebar/{dismiss,hit_mask}.rs`, `src/commands/workspace/{spec_io,actions}.rs`)**:
  - Isolated Agy worker `PostToolUse` hooks (`agy_event_is_worker`, `should_reveal_agent_view`) so workers only write `live_sess_<sid>.json` and never overwrite coordinator `live_tab_<yid>.json`, `session_<coord>.yazi`, or `main_last_file_<yid>.json`, revealing Yazi only when the active view matches.
  - Cleaned `active_view_<yid>.txt`, `main_last_file_<yid>.json`, and `tab_shell_<yid>.txt` in `remove_tab_state`, aligned `clear-subagents` with `!is_subagent_alive`, and tested `waiting` protection vs `killed` dismissal.
- **Turn-Accurate Multi-Model `COST` & Expanded Pricing (`src/tui/context/`, `src/tui/pricing/`, `assets/pricing.json`)**:
  - Tracked per-model turn counts in 5-tuple `per_model` and computed remainder-preserving per-turn cost `(t - 1) * cost(q) + cost(q + r)` in `snapshot_from` so multi-turn sessions never trigger false single-turn `above_tokens` surcharges, plus 10-minute idle tracker pruning in `sample_agent`.
  - Added `gemini-4-argon`, `gemini-4-hydrogen`, `gemini-4-neon`, and `gpt-oss-120b` / `gpt-oss` pricing/fallback routing.
- **CI/Release Milestone Fallback, Bootstrap Skew & Launcher/Installer Hardening (`.github/workflows/{ci,release}.yml`, `scripts/`, `install.{sh,ps1}`)**:
  - Updated `ci.yml` and `release.yml` to promote `0.x.y` patches to full 6-target binary builds/releases when `v<M>.<m>.0` is absent on `origin`, plus 7-entry parity and `integrations/` validation in `ci.yml`.
  - Updated `ensure-ccide-core.{sh,mjs}` to compare `(major, minor, patch)` versions and fall back from `v<M>.<m>.0` to `v<M>.<m>.<p>`, fixed `install.sh` `EXTRA_ARGS` forwarding and `install.ps1` `sys.argv`/`Invoke-BestEffort` safety, and forwarded `CLAUDE_PLUGIN_ROOT`, `CCIDE_AGY_HOME`, and `--wt-agy-home` across `ccide` / `ccide.ps1`.

---

## [0.11.1] - 2026-11-19

### Added & Fixed
- **Native Antigravity Swarm Observability (`src/session/agy_swarm{,_io}.rs`, `src/tui/context/agy_swarm_ctx.rs`, `src/commands/sidebar/{agent_tab,agent_refresh,dismiss,subagents_rows}.rs`)**:
  - Auto-discovered `<sys_gen>/swarm/{swarm.json,members.json,board.jsonl}` and `subagents/*.json` relative to the active `agy` transcript, rendering `SWARM #<board_head> · <run>run·<wait>wait`, the swarm title, and the selected worker's latest `board.jsonl` snippet.
  - Mapped Swarm member states (`running`, `waiting` protected from dismissal, `stopped`, `killed` dismissible), enabled click-to-inspect worker Top Logo + `CONTEXT` + Yazi follow without `SIGUSR2`, aggregated Swarm `COST` across coordinator and unique worker transcripts, and added `--conversation <id>` / `CCIDE_AGY_CONVERSATION` (`^[A-Za-z0-9_-]{8,64}$`) and `CCIDE_AGY_HOME`.
  - Added `from __future__ import annotations` in `scripts/merge_agent_hooks.py` for stock macOS `/usr/bin/python3` (`3.9+`) compatibility.

---

## [0.11.0] - 2026-11-19

### Added
- **Multi-Agent Support: Google Antigravity CLI (`agy`) & OpenAI Codex CLI (`codex`) (`src/session/{agent,agent_bind,agy,codex,codex_discover}.rs`, `src/tui/context/{agent_ctx,agy_parser,codex_parser,snapshot}.rs`, `src/json/toml_lite.rs`, `src/commands/{hook_input,hook_records}.rs`, `src/commands/setup/doctor_agents.rs`, `integrations/`)**:
  - Added `--agy`, `--codex`, `--agent <claude|agy|codex>`, and `CCIDE_AGENT` launcher support, non-destructive hook bundles (`~/.gemini/config/plugins/cc-ide/` and `$CODEX_HOME/hooks.json` via `scripts/merge_agent_hooks.py`), strict per-host hook stdout (`{}` for `agy`, empty stdout for `codex`, opt-in `CCIDE_AGY_ORCH=1`), and disk-only fallback discovery (`last_conversations.json` / `sessions/YYYY/MM/DD/rollout-*.jsonl`).
  - Added `agy` and `codex` context/cost parsers (tolerant of Codex Desktop `0.167` object `context_window`), display-name pricing normalization (`"Claude Opus 4.6 (Thinking)"` -> `claude-opus-4-6`), `[agy]`/`[codex]` sidebar badges, and `doctor --json` `"agents"` / `"agent_hooks"` diagnostics.

---

## [0.10.1] - 2026-11-18

### Fixed
- **iTerm2 `LayoutPlan` Column-Width Parity (`scripts/ccide_daemon.py`, `scripts/iterm2_workspace.py`, `tests/test_iterm_layout.py`)**:
  - Replaced legacy `56/80/38` floors (`sum=174`, which overflowed `< 175`-col windows and skewed `268`-col splits to `86|123|59`) with `compute_iterm_column_widths` matching `LayoutPlan` (`22/48/28` floors, deficit trimming, proportional narrow scaling, `user.cc_left_pct` persistence).

---

## [0.10.0] - 2026-11-16

### Added & Changed
- **Unified `PaneSpec` v2, `LayoutPlan` & Real Multi-Backend `rebalance` (`src/commands/workspace/{spec,layout,rebalance,open,mod,tmux,wezterm,wt,iterm}.rs`)**:
  - Replaced ad-hoc `<backend>:<sides>` strings with typed `PaneSpec` v2 (`v2;b=...;k=...;c=...;y=...;l=...;m=...;s=...;lp=...;f=gmr`) plus v1 backwards-compatible parsing and automatic dead-tab spec pruning (`prune_dead_specs`), written exclusively by `workspace/open.rs`.
  - Introduced `LayoutPlan::compute` (`22 / 48 / 28` column floors, right collapse `< 100` cols, left collapse `< 76` cols, `left_pct` `12..=45`) and `run_rebalance` with deterministic exit codes (`0` Applied/Reopened, `3` Unsupported, `1` error) — enabling live `tmux resize-pane -x` and WezTerm `adjust-pane-size` rebalances plus accurate `/rebalance-panes` exit-code toasts in `hooks/register.ts` (iTerm's Python daemon retains `56/80/38` floors until `v1.0`).
- **Ghostty Caller Targeting, Live Width Rebalance & Opt-In Linux Probe (`src/commands/workspace/{ghostty,ghostty_script,ghostty_linux}.rs`)**:
  - Fixed macOS Ghostty targeting (`F1`) via `OSC 2` `ccide-<yid>` title markers and `$GHOSTTY_SURFACE_ID` lookup (gated front-window fallback behind `CCIDE_GHOSTTY_FRONT_FALLBACK=1`), removed the `"1"` sentinel cache bug (`F3`), and drove `rebalance_ghostty` (`F2`) from live `geom_<yid>_*` pixel widths + `spec.left_pct`/`spec.show_right`.
  - Added `GhosttyCap` and `ghostty_linux.rs` (`CCIDE_GHOSTTY_LINUX=1` opt-in `150 ms`-bounded `+list-actions` CLI and `gdbus` D-Bus probes), continuing to fall through cleanly to `tmux` in `v0.10.0`.
- **Canonical `workspace detect`, `src/proto.rs`, Milestone-Pinned Bootstrap & Cross-OS Companion Reload (`src/proto.rs`, `src/main.rs`, `src/commands/{pane_runner,setup/doctor*}.rs`, `scripts/`)**:
  - Added `ccide-core workspace detect [--json]` (replacing inline env sniffing in `scripts/ccide` and `scripts/ccide.ps1`), `ccide-core --version`, and `src/proto.rs` (`ccide-core proto [--json]`, `ipc_proto=1`, `patch_proto=1`, `patch_rules_version="2"`, `spec_version=2`).
  - Added live `geom_<yid>_<role>.txt` reporting and cross-OS `reload_lg_<yid>` Lazygit theme hot-reload in `pane_runner.rs` + `workspace/reload.rs`, `doctor` `ipc_dir`/`ipc_proto`/`patch_proto`/`ghostty_cap`/`legacy_patch` (`IPC Patch Proto`) reporting, and milestone-tag pinning (`v<major>.<minor>.0`, dropping `releases/latest`) with `.ccide-core.version` skew detection in `scripts/ensure-ccide-core.{mjs,sh}`.

### Compatibility Notes
- **Pre-0.10 Running Sidebars**: Sidebars already running a pre-`0.10.0` binary cannot parse `v2` `panes_<yid>.txt` specs, so click-to-focus on newly opened tabs degrades until those tabs are reopened (`close` and `rebalance` use the new binary and are unaffected).
- **`legacy_ipc_dir()` Retention**: Read-only fallback to legacy `/tmp/.cc-ide` (`claude_signalable`, `cur_pid`, `req_pid`) is retained in `v0.10.0` (surfaced via `doctor`'s `IPC Patch Proto` warning) and scheduled for removal in `v0.11.0`.

---

## [0.9.4] - 2026-11-15

### Security & IPC Hardening
- **Generation-Guarded `ProcToken` (`<pid>:<start_ms>`) (`src/ffi/`, `src/commands/sidebar/`, `src/session/`, `scripts/`)**:
  - Replaced bare-PID signalling and ownership checks with `ProcToken { pid, start_ms }` backed by kernel start timestamps (`macOS proc_pidinfo`, Linux `/proc/<pid>/stat` + `btime`, Win32 `GetProcessTimes`), eliminating PID-reuse mis-signalling hazards (`SIGUSR2` & cleanup).
  - Updated `wake-pid`, `notify_sidebar`, `sb_*.pid`, `sidebar_tab_*.json`, `tab_shell_*.txt`, `wake_all_sidebars`, `hooks/register.ts`, and `ccide_daemon.py` with startup and 30 s stale-token sweeps via `platform::atomic_write`.
- **`ready_pid_<pid>` Patch Handshake & `ipc_guard` (`scripts/cc_patch_rules.py`, `assets/cc-schema.json`, `src/commands/sidebar/ipc_guard.rs`)**:
  - Bumped `PATCH_RULES_VERSION = "2"` and `claude_bin.stamp` (`r2:`) so patched `claude.exe` binaries atomically write `ready_pid_<pid>`, enforce Linux `/proc/<k>/stat` generation verification before `SIGUSR2`, and pass full tokens on Windows.
  - Added `claude_signalable(pid)` proof verification (`lstat` regular file, same UID, `mtime + 1000 >= start_ms`) before firing `SIGUSR2`, surfacing `repatch needed (ccide doctor)` in the sidebar when unverified.
- **Per-UID `0700` `ipc_dir()` Convergence (`src/platform/ipc.rs`, `src/main.rs`, `scripts/`, `hooks/register.ts`, `assets/yazi-hscroll.lua`)**:
  - Split `src/platform.rs` into `src/platform/{mod,ipc}.rs` with `OnceLock`-memoized `ipc_dir()` resolving `$CCIDE_IPC_DIR` -> Unix `/tmp/.cc-ide-<euid>` (`0700`, symlink/ownership checked, fallback `~/.cache/cc-ide/ipc`) -> Windows `%TEMP%`/`%TMP%`/`.cc-ide`, plus legacy `/tmp/.cc-ide` fallback reader and `ccide-core ipc-dir` CLI subcommand exported by `ccide` / `ccide.ps1` and aligned across all companion scripts.

---

## [0.9.3] - 2026-11-15

### Changed & Fixed
- **Zero-Cycle Layered Architecture & 100% `<= 300`-Line Rust Modules (`src/budget/`, `src/model_id.rs`, `src/json/`, `src/session/`, `src/tui/`, `src/commands/`)**:
  - Extracted leaf modules `src/budget/{mod,config}.rs` and `src/model_id.rs`, split `src/json.rs` into `src/json/{mod,extract,scan,jsonc}.rs` (deduplicating 2 JSONC strippers and 5 JSON escapers), and eliminated every `crate::commands` reverse dependency and `orchestrator <-> session` cycle, guarded by CI-grade assertions in `tests/test_agent_infra.py`.
  - Split every oversized Rust source file below `300` lines (`src/session/{mod,binding,scan,model,model_compact,subagents,subagent_state,transcript,transcript_main,focus}.rs`, `src/tui/{grid,grid_render}.rs`, `src/tui/logo/{mod,ansi}.rs`, `src/commands/setup/{doctor,doctor_probes}.rs`, `src/commands/statusline/{mod,git_info}.rs`, `src/commands/yazi_hook.rs`).
- **Session Isolation, Theme Leader Election & Telemetry Hardening (`src/platform.rs`, `src/session/`, `src/tui/palette/`, `src/telemetry/`)**:
  - Unified atomic writes into `platform::atomic_write` and added case-insensitive Windows cwd matching via `platform::paths_equal_ci`, separated pure `choose_binding()` + `commit_binding()` with `dir_mtime` session-scan caching, and fixed `compact_model_for_width` non-ASCII tail glyph measurement.
  - Added `SyncOutcome` / `PaletteReload { changed, synced_leader }` so only the sidebar winning `.sync.lock` triggers companion Yazi preview refreshes, bounded `ya emit-to` waiter threads (`<= 4`, `2 s` timeout), bounded Linux/Windows `nvidia-smi` waits to `5 s`, and handled 32-bit `MibIfRow` `wrapping_sub` rollover on Windows.

---

## [0.9.2] - 2026-11-15

### Fixed
- **In-Process Mod (`hooks/register.ts`), Manifests & Skills Drift (`.claude-plugin/{plugin,marketplace}.json`, `skills/*/SKILL.md`)**:
  - Aligned Windows IPC directory lookup in `register.ts` to `TMP || TEMP` (matching Win32 `GetTempPathW` / Rust `std::env::temp_dir()`), bounded `perfBuffers` to the last `256` lines and capped per-session caches at `64` entries for `O(1)` memory/I/O, made `/rebalance-panes` toast reflect the real exit code/support status, and made `syncSessionModel` parse leading numeric PIDs from future `pid:start_time` tokens.
  - Added `"workflows": "./assets/workflows"` to `.claude-plugin/plugin.json`, added explicit `0.9.2` version fields to `.claude-plugin/marketplace.json` (checked via 5-way CI parity), and updated `skills/{ccide,clear,rebalance,theme}/SKILL.md` to invoke cross-OS `ensure-ccide-core.mjs` without stale `v0.5.0` or Unix-only references.
- **Python `-I` Isolation, Windows Companion Config & Launcher Parity (`scripts/`, `install.ps1`, `install.sh`, `assets/yazi-hscroll.lua`, `src/commands/workspace/iterm.rs`)**:
  - Added `import tempfile` (fixing Windows `NameError`) and `TMP`/`TEMP` alignment in `scripts/lazygit-log.py`, prepended script directory to `sys.path` in `scripts/cc_patch_rules.py` and `scripts/cc_schema_inspector.py` for standalone `python3 -I`, caught `OSError` in `ccide_workflow_runner.py` `discover_workflows()`, enforced `-I` across `pane-runner.sh`, `yazi-hscroll.lua`, and `iterm.rs`, and deleted dead `scripts/cc_tui_lib.py`.
  - Added `yazi.toml` (`touch` mouse events + `hscroll` previewer) and `keymap.toml` setup to `install.ps1`, added `--mon`/`--no-mon` and `CLAUDECODE` session binding to `scripts/ccide.ps1`, added `--wt` and backend help flags to `scripts/ccide`, and updated `area:workspace` mapping to `src/commands/workspace/` in CI/triage helpers.

---

## [0.9.1] - 2026-11-15

### Fixed
- **FleetView Dual-PID `SessionBinding` & Model Resolution (`src/session/{mod,model}.rs`, `src/commands/sidebar/`, `src/commands/statusline/mod.rs`, `src/session/transcript.rs`, `scripts/ccide_daemon.py`)**:
  - Replaced the `(sessionId, claude_pid)` tuple with `SessionBinding { session_id, ui_pid, data_pid }`, so a FleetView (`/subtask`) tab keeps the interactive viewer `ui_pid` (`SIGUSR2` view switching) while reading `CONTEXT`/`COST` from the attached `bg` session's `data_pid`, and never steals a sibling tab's active session (`is_claimed_by_live_sibling`).
  - Preserved raw model strings end-to-end (`ConfigModels.main_model_raw` instead of the compacted `main_model_short`), propagated the `[1M]` suffix through alias mapping and `live_sess_<id>.json` resolution, narrowed `o1`/`o3`/`o4` OpenAI detection to whole tokens (`nemo1-turbo3` stays `claude`), and restricted `Set model to` parsing to `local-command-stdout` lines with `(`-safe truncation.
  - Bounded the `tmux` pane-title probe to `150 ms` (killing the child on timeout), atomically rewrote the statusline `live_sess_*`/`live_tab_*`/`session_*.yazi` caches (PID temp + `rename`), hardened `read_git_head_branch` for `ref: refs/tags/*`, and released iTerm2 per-session watcher tasks on pane termination.
- **`register.ts` `perfQueues` Serialization & Deduplicated Model Sync (`hooks/register.ts`)**:
  - Serialized `turn.step` perf appends through a per-session `perfQueues` promise chain so concurrent samples can no longer interleave and truncate `pf_sid_<sessionId>.jsonl`, removed the unbounded `512`-line buffer growth, and deduplicated `syncSessionModel` via a `lastModels` cache plus a single `sb_<sid>.pid` wake instead of scanning every `sb_yazi_*.pid`.
- **Palette `.sync.lock`, `bat` Timeout & `base16-256` Auto-Fallback (`src/tui/palette/{syntax,companions}.rs`, `scripts/ccide_syntax_pal.py`, `assets/yazi-hscroll.lua`, `scripts/lazygit-pager.sh`, `scripts/md-render.py`)**:
  - Guarded concurrent artifact generation with a stale-aware `~/.config/cc-ide/.sync.lock` (`15 s` expiry, RAII release) and bounded every `bat cache --build`/`--config-dir` shell-out to `5 s`, so a hung `bat` (or a multi-tab theme switch) can no longer pin a sidebar cycle.
  - Added a universal `base16-256` fallback when the generated `ccide` theme is missing: `yazi-hscroll.lua` retries `base16-256` and requests `syn.sgr` remapping, `lazygit-pager.sh` re-runs `delta` without an inherited `BAT_THEME`, `.tmTheme`/`theme.toml` paths are Windows UNC-safe, and compound SGR (`1;38;5;117`, `38;5;115;48;5;236`) is upgraded to 24-bit RGB without touching backgrounds.
  - Added a one-shot Kitty image deletion (`APC _G a=d,i=71`) when the sprite renderer transitions to ASCII/`OSC 1337`, made explicit terminal identity win over inherited `KITTY_WINDOW_ID`/`ITERM_SESSION_ID`, and gated image blits below `12×12` cells.
  - Window-scrolled the `ccide setup` configuration panel so all 9 rows plus `[ ▶ Apply ... ]` stay reachable and clickable at `80x24`, and compacted the key hints below 86 columns.
- **Ghostty Equalized-Split Rebalance, Ghostty/TMUX/WT Backend Hardening (`src/commands/workspace/`, `scripts/ccide`, `scripts/ccide.ps1`)**:
  - Rewrote Ghostty rebalance around `compute_ghostty_equalized_deltas` (per-tab `ghostty_w_<yid>.txt` width + right-column presence) after `equalize_splits`, cached the native capability against the `Ghostty.app` `Info.plist` stamp, and disabled the macOS-only native path on Linux so Ghostty always falls back to tmux.
  - Added a local `tmux -L ccide` fallback in `scripts/ccide` whenever the auto backend returns no pane spec, passed `--tmux` last so launcher arguments survive, made `--backend`/`--width` tolerant of a missing value, clamped tmux side columns on narrow windows, targeted `TMUX_PANE` explicitly, and cleared stale `close_<yazi_id>` sentinels before splitting.
  - Escaped `;` in `wt.exe` directory arguments, closed partially-created WT panes on failure, reaped short-lived `wezterm`/`wt` focus children, and joined the bounded (`< 320 ms`) Yazi reveal-restore thread so `ccide --theme` never drops a reveal.
- **Linux 64-bit `poll` `NfdsT` & PID Bounds, macOS/Windows FFI & Telemetry (`src/ffi/`, `src/telemetry/`)**:
  - Typed Linux `poll(2)` `nfds` as `c_ulong` (`NfdsT`) and `ioctl` requests as `c_ulong`, replaced the `0x40087468`/`0x5413` literals, and rejected `pid > i32::MAX` before `kill(2)` casts.
  - Guarded `NVIDIA_IN_FLIGHT` with an atomic compare-exchange + `Drop` guard and PIPED stdout committed through a PID temp + `rename` on both Linux and Windows, clamped `dt` to `>= 1 ms` so the first `NET` sample cannot divide by zero, made macOS `sample_net_sysctl` return `Option` instead of zeroing counters, aligned the `host_statistics64` buffer to 8 bytes, rejected short `proc_pidinfo` reads, and widened the ancestry walk to 33 levels.
- **`PreToolUse` Shim Model Guard & Workflow Server Hardening (`src/orchestrator/`, `scripts/ccide_workflow_{runner,server}.py`)**:
  - Kept `model` off `Agent` rewrites for existing `ccide-{gp,ro}--*` shims (only emitting real aliases), added `target_model` to the shim pool on demand, and rejected path-traversal shim names while escaping control characters in generated `.md`/JSON.
  - Bounded the workflow CLI dispatch with guaranteed child termination, validated `Content-Type`, body shape and `Origin` (localhost only), compared bearer tokens with `hmac.compare_digest`, and resolved workflow sources through realpath containment so `../` escapes fail with `404`.
- **Binary Patcher, Installers & Companion Scripts (`scripts/patch_claude_binary.py`, `install.sh`, `install.ps1`, `scripts/`)`**:
  - Verified the patched `claude` binary with `--version` before install/`codesign` and rolled back from the pristine `.bak` on failure, wired the injected `cur_pid_<pid>.txt` write through a PID temp + `rename`, and made the resolver's `.ensure-ccide-core.lock` owner-PID scoped (released before a long-running session starts).
  - Restored every `*.ccide.bak` on uninstall (Yazi, Lazygit, `theme.toml`, `init.lua`), removed managed blocks only when a backup is absent, bounded `glow`/`delta`/`git log` to `5–10 s` timeouts, and re-assembled coalesced multi-key bursts plus split `ESC` sequences in `pty-hscroll-bridge.py`.
- **CI & Release Policy for `0.x.y` Patches (`.github/workflows/{ci,release}.yml`)**:
  - Detect `0.X.0` vs `0.x.y` from `Cargo.toml`: patch releases still run the MSRV/clippy/patch-anchor gate but skip the 6-OS build matrix, while `release.yml` skips binary compilation and tags `v0.9.1` on `main` without publishing archives. Added a `pwsh` parser gate for `install.ps1`/`scripts/ccide.ps1`.

---

## [0.9.0] - 2026-11-14

### Added
- **In-Process Claude Code `mod` Extension (`hooks/register.ts`, `hooks/hooks.json`, `scripts/cc_patch_rules.py`)**:
  - Registered collision-safe `immediate: true` slash commands (`/palette`, `/rebalance-panes`, `/clear-agents`, `/orch`, `/ccide-doctor`) via `hooks.json` `modules` (`$.command.register` + `command.run` + `$.ui.toast`), executing locally in `< 15 ms` with zero LLM token consumption.
  - Migrated Binary Patch #7 (`export_api_perf_metrics`) to the official `on('turn.step')` streaming hook (`pf_sid_<sessionId>.jsonl` via `$.clock.now()` + `$.fs.write`), reducing binary patches from 12 to 11 and leaving an entire Bun module untouched, plus added `< 0.2 ms` in-process `session.start` / `classic.PostModelSwitch` model state sync (`mod_model_<sessionId>.txt`).
- **Palette-Unified Companion UI & `syntect` Syntax Highlighting (`src/tui/palette/`, `scripts/ccide_syntax_pal.py`, `assets/yazi-hscroll.lua`, `scripts/lazygit-pager.sh`, `scripts/md-render.py`)**:
  - Split `src/tui/palette/` into `<= 300`-line modules (`mod`, `presets`, `syntax`, `artifacts`, `companions`, `template`) with harmonized 8-channel syntax & diff defaults across all 6 presets plus granular `"syntax"` overrides in `~/.config/cc-ide/palette.json`.
  - Atomically generates `ccide.tmTheme` (15 TextMate scopes + `bat cache --build` so Yazi `syntect_theme`, `bat --theme=ccide`, and `delta --syntax-theme=ccide` share identical highlighting), full Yazi UI `theme.toml`, Lazygit `gui.theme` (`lazygit.yml`), `glow-style.json`, and `ghostty-theme.conf`, with `< 120 ms` companion pane hot-reload on theme switch.
- **First-Class Ghostty Workspace Backend (`src/commands/workspace/{ghostty,ghostty_script}.rs`, `scripts/ccide`, `src/commands/setup/doctor.rs`)**:
  - Added native macOS Ghostty `>= 1.3` workspace controller (`Ghostty.sdef` AppleScript `surface configuration`, `split`, pixel-accurate `30% | 54% | 16%` `resize_split`, `focus`, and `close`) preserving native Kitty Graphics Protocol (`APC _G`) Retina sprites, with automatic capability probing (`ccide-core workspace probe-ghostty`) and seamless fallback to `tmux -L ccide`.

### Fixed
- **Pure Streaming Decode `TPS` Calibration (`src/tui/context/perf.rs`, `src/tui/context/parser.rs`)**:
  - Removed the `gen_ms < 250 ms -> dur_ms` fallback that mixed TTFT/thinking wait into short-turn decode durations, exported `firstContentMs` (`f`) alongside `ttftMs` (`t`), and upgraded `.jsonl` fallback `TPS` to self-calibrating inter-block pure streaming windows (`last_blk_ms - first_blk_ms`).
- **FleetView (`/subtask`) Multi-Conversation Model Isolation, New-Session Refresh & iTerm2 Retina Sprites (`src/session/{mod,model,transcript}.rs`, `scripts/ccide_daemon.py`, `src/tui/logo/image.rs`)**:
  - Added live Center-pane title monitoring (`tab_title_<yazi_id>.txt` + `parkedJobId` awareness) so tabs attached to FleetView `bg` sessions resolve the active attached conversation instead of the outer idle viewer shell, prevented `PostModelSwitch` `.jsonl` writes from reviving stale assistant turn models, and restored `OSC 1337` Retina sprite scaling on iTerm2.

---

## [0.8.0] - 2026-11-14

### Added
- **Zero-Patch Subagent & Native `Workflow` Model/Effort Orchestrator (`src/orchestrator/`, `src/commands/pre_tool_hook.rs`, `src/commands/sidebar/orch_panel.rs`, `hooks/hooks.json`, `assets/orchestrator.json`, `skills/orchestrate/SKILL.md`)**:
  - Added `PreToolUse` model/effort routing across `Scout`, `Coder`, `Reviewer`, `Plan`, and `Workflow` roles via `~/.config/cc-ide/orchestrator.json`, an interactive Right Sidebar `ORCHESTRATOR` toggle panel, and `ccide orchestrator` (`status`, `preset`, `sync`).
- **Native `Workflow` Templates, CLI Runner & Local HTTP/SSE Server (`assets/workflows/*.js`, `scripts/ccide_workflow_runner.py`, `scripts/ccide_workflow_server.py`)**:
  - Added built-in workflow scripts (`ccide-consensus.js`, `ccide-implement.js`, `ccide-review.js`), `ccide workflow` (`list`, `run <name>`, `serve`), and a local HTTP/SSE API server.
- **Official Claude Code Extension & Orchestration Reference (`docs/CLAUDE_CODE_INTERFACES.md`)**:
  - Documented plugin/marketplace manifests, hooks, subagent/workflow schemas, and `sdk-tools.d.ts` interfaces verified against `@anthropic-ai/claude-code` `2.1.304` / `2.1.323`.

### Fixed
- **Full-Repo Audit & Hardening (`src/`, `scripts/`, `assets/yazi-hscroll.lua`, `install.sh`, `install.ps1`)**:
  - Hardened partial-line `.jsonl` incremental offset reads, JSONC UTF-8 escape handling, macOS `IfMsghdr2` unaligned reads & IOKit/Mach resource cleanup, child-process zombie reaping, interrupted/`TaskStop` subagent status tracking, and `COST` `I`/`R` 4-sig-fig & `O` 3-sig-fig token formatting.
  - Fixed Lua 5.4 string escape sequences & PTY input drain, iTerm2 per-pane wallpaper preservation, `ensure-ccide-core` lockfile + atomic staging, and Windows Lazygit companion config wiring.

---

## [0.7.0] - 2026-11-14

### Added
- **Full Native Windows (`pwsh` / `cmd.exe`) Engine, Installers & Workspace Backends (`src/ffi/windows.rs`, `src/telemetry/windows.rs`, `src/commands/workspace/{wezterm,wt}.rs`, `install.ps1`, `install.cmd`, `scripts/ccide.{ps1,cmd}`, `scripts/ensure-ccide-core.mjs`)**:
  - Zero-dependency Win32 FFI (`kernel32.dll` + `iphlpapi.dll` at MSRV `1.70.0`) with `Local\ccide_wake_<pid>` Named-Event `< 0.05 ms` sidebar wakeups, raw VT/SGR console input, `CreateToolhelp32Snapshot` process-tree isolation, and live CPU/RAM/NIC/GPU telemetry.
  - Native **WezTerm** (`wezterm cli`) and **Windows Terminal** (`wt.exe -w 0`) `30% | 54% | 16%` 3-column workspace backends backed by `ccide-core pane-runner`, cross-platform Node hook bootstrap (`ensure-ccide-core.mjs`), and a 6-target CI/Release matrix (`x86_64`/`aarch64` × `macOS`/`Linux`/`Windows MSVC`).
- **Semi-Automated Cross-OS Symbol & Schema Maintenance Workbench (`scripts/cc-maint.py`, `scripts/bun_pe.py`, `scripts/cc_schema_inspector.py`, `assets/cc-schema.json`)**:
  - `ccide --check-cc` (`--local` or `--fetch-npm [ver]`) simultaneously probes `darwin-arm64`, `linux-x64`, and `win32-x64` Bun PE binaries (including `B:/~BUN/` module graphs and trailing Authenticode tables), prints `±220`-byte anchor context on drift, and diffs `statusLine`, `.jsonl`, `subagent_meta`, and Hook schemas.
- **Declarative Terminal Cell Typesetting Engine, `3/2/1`-Line `COST` Telemetry (`TPS` · `HIT%` · `TTFT`) & `0.1%` `CACHE` Precision (`src/tui/layout.rs`, `src/tui/context/`)**:
  - Added `RowLayout`, `ColSpec`, and `ColumnWidths` so `SUBAGENTS`, `CONTEXT`, `COST`, and Hardware Telemetry headers align flush inside box borders without manual string padding, full-bleed `BoxRow.bg` rectangles, and 3-tier lossless model name compaction (`compact_model_for_width`).
  - Tightened split `CONTEXT` to a compact fixed `11`-row height (`0.1%` `CACHE` hit ratios strictly capped at `99.9%` so `100%` rounding never occurs) and expanded scrollable `COST` (`WheelUp`/`WheelDown` + `↑`/`↓`/`PgUp`/`PgDn`) with per-model `TPS`, prompt-cache `HIT%`, and right-aligned `TTFT` (first-token latency), configurable across `3` / `2` / `1` lines via `~/.config/cc-ide/viewer.json` (`"cost_lines"`) and `ccide setup`.
  - Full Claude Code `2.1.3xx` plugin-first telemetry: official `PostModelSwitch` & `SessionStart` hooks (`hooks/hooks.json`), `5h`/`7d` rate-limit quotas, `+add/-del` diff counters, native-1M Opus 5.5 / Fable 5.1 `context_window_size`, linked git worktrees, `usage.iterations` advisor/context unwrapping, fast-mode / US-geo / web-search billing, per-subagent `model`/`effort`/`stoppedByUser`, and `yazi-hook` `agent_id` isolation + hunk auto-scroll/highlight.
- **Smooth Connected Unicode DAG Commit Graph for Lazygit `[0]-Log` (`scripts/git_smooth_graph.py`, `scripts/lazygit-log.py`)**:
  - Replaced raw ASCII `git log --graph` slashes (`*`, `|`, `/`, `\`) in `[0]-Log` (`branchLogCmd` / `allBranchesLogCmds`) with a 4-direction bitmask Unicode box-drawing DAG renderer (`○`, `⏣`, `│`, `╭`, `╮`, `╰`, `╯`, `├`, `┤`, `┬`, `┴`, `┼`) matching `[4]-Commits`.

### Fixed
- **Single-Click Responsiveness, Mutually-Exclusive 3-Column Highlight Clearing & Non-Draggable GPU Sprites (`src/commands/sidebar/`, `scripts/ccide_daemon.py`, `scripts/pty-hscroll-bridge.py`, `assets/yazi-hscroll.lua`)**:
  - Activated sidebar controls on `MouseDown` (handling iTerm2 unfocused `reportUnreportedFocusClick:` single-packet clicks) with `0`-RPC `YAZI_SESSION_CACHE` focus switching, position-driven border focus, and automatic cross-column highlight clearing (`DECSET 1004` `\x1b[O` in Sidebar & Lazygit PTY bridge + `FocusMonitor` `ya emit-to ... blur` in Yazi).
  - Upgraded iTerm2 `>= 3.5` and WezTerm PNG sprite rendering to Kitty Graphics Protocol (`APC _G`, `q=2,C=1`) with `HitZone::InertSprite` shielding, BMP wide-symbol column guards (`src/tui/width.rs`), and subagent `viewing-agent` locks (`keep_viewing_agent_on_turn_update`, `lock_hidden_footer_tasks_nav`, `safe_escape_from_subagent_view`).

---

## [0.5.0] - 2026-11-12

### Added
- **Live Session Task Progress Board `TASKS ⇄ Hardware Telemetry` (`src/tui/tasks.rs`, `src/commands/sidebar.rs`, `hooks/hooks.json`)**:
  - Added an interactive per-session `TASKS` board in the Bottom-Right sidebar slot (`Panel 3`) backed directly by `~/.claude/tasks/<sessionId>/*.json` (`mtime`-cached, zero binary patching — leaving Claude Code's native `/btw` overlay untouched in the `54%` Center pane).
  - Automatically swaps `Panel 3` to `TASKS` (`✓` completed, animated `⠋` `in_progress` with `└─ activeForm` subtitle, `○` pending, scrollable viewport, and pinned progress bar `━━━░░░ 50% [HW]`) whenever incomplete tasks exist, and automatically hands `Panel 3` back to the continuous `500 ms` Hardware Telemetry oscilloscope once all tasks complete.
  - Supports instant manual toggling (`t` / `Tab`, clicking the `Panel 3` header or `[HW]`/`[TASK]` badge, or secondary click) and mouse wheel scrolling over `Panel 3`.
- **Configurable Line-Wrapping (`~/.config/cc-ide/viewer.json`, Default Unwrapped) & Compact Diff/Log Renderers (`assets/viewer.json`, `scripts/lazygit-pager.sh`, `scripts/lazygit-log.py`, `assets/lazygit.yml`)**:
  - Added `~/.config/cc-ide/viewer.json` (`"wrap": false` by default, `"wrap_style": "vi"`) so diffs, commit graphs, and Markdown previews render in full without line breaks by default for horizontal panning via Magic Mouse or keyboard (`H`/`L`, `Shift`/`Alt`+`←`/`→`), while preserving exact `vi`-style (`↪` / `↩` `showbreak`) full-width wrapping whenever `"wrap": true` (or `CCIDE_WRAP=1` / `--wrap`) is enabled.
  - Collapsed dual old/new diff line numbers (`1 ⋮ 1 │`) into a single compact column (`  1│`) in `scripts/lazygit-pager.sh`, removed legacy CPU/MEM/DSK telemetry from Lazygit `[1]-Status`, hid SHA hashes in unfocused commit lists (`gui.commitHashLength: 0`), and added a full-stream hash-free `[Author] [Graph] [Ref] [Subject]` commit graph renderer (`scripts/lazygit-log.py`) across `[1]-Status` and `[3]-Local branches`.
- **Ergonomic Hover-Targeted & Content-Bounds-Clamped Navigation for `Lazygit` and `Yazi` (`scripts/pty-hscroll-bridge.py`, `assets/yazi-hscroll.lua`, `assets/lazygit.yml`)**:
  - **Mouse Mode vs Keyboard Mode Highlight Discipline**: Suppresses reversed selection highlight bars in `Yazi` (`Entity:style` in `mouse` mode) and unfocused selection bars in `Lazygit` (`inactiveViewSelectedLineBgColor: ["default"]`) while scrolling with mouse/trackpad, routing vertical and horizontal scroll strictly to the hovered panel (`Parent`, `Current`, `Preview` in Yazi; `[0]` and `[2]..[5]` in Lazygit) and immediately restoring selection highlights upon any keyboard navigation, click, or tool file reveal.
  - **macOS Natural-Scrolling Apple Magic Mouse X-Axis Polarity & Pre-Fill Bounds**: Fixed horizontal swipe polarity (`SGR 66` left-swipe -> `+X` reveal right content; `SGR 67` right-swipe -> `-X` return toward column 0) and enforced strict content bounds (`gui.scrollPastBottom: false` + `VtScreen` visible-viewport overflow bounds in `pty-hscroll-bridge.py`; `Entity` / `Linemode` widths, `bat`/`md-render` viewport widths, and `#folder.files` vs `area.h` in `hscroll.yazi`) so axes whose content already fits never overscroll into blank space.
  - **Selective Non-Text GUI Selection Interception (`src/commands/sidebar.rs`, `scripts/ccide_daemon.py`)**: Intercepts click, drag (`?1002h`), and cross-pane unfocused text selection (`iterm2.VariableMonitor(SESSION, "selection")` -> `async_set_selection([])`) strictly on decorative GUI panes (`[AI] Companion` Top Model Sprite/Logo & Bottom Hardware Telemetry, and `[SYS] macmon`), keeping native text/line selection completely intact in `Lazygit`, `Yazi`, and Center `Claude Code`.

### Fixed
- **120Hz Zero-Subprocess Horizontal Scrolling, PTY Stream Integrity & Off-Screen ANSI Syntax Preservation (`scripts/pty-hscroll-bridge.py`, `assets/yazi-hscroll.lua`, `scripts/lazygit-pager.sh`, `scripts/lazygit-log.py`, `install.sh`)**:
  - Replaced subprocess-based refresh scrolling in `pty-hscroll-bridge.py` with a `< 0.1 ms` in-memory pre-tokenized ANSI viewport blitter (`±2` cols per 120 Hz Magic Mouse event) wrapped in DEC Mode 2026 (`?2026h`/`?2026l`) synchronized VSync frames, backed by `blake2s` cache deduplication (preserving `pan_x` across `refreshInterval` ticks), an incremental UTF-8 + VT100/CSI/OSC-8 (`\x1b]8;;\x1b\\`) state machine, `write_all()` Darwin PTY short-write retry handling, canonical compound `delta` SGR state tracking, and `[4]-Commits` visual branch-graph locking (`showGraph: always`, blocking accidental `Reflog` switches).
  - Fixed `Yazi`'s off-screen left ANSI control-code syntax-highlighting drop via a canonical SGR state machine (`slice_ansi_preserve_sgr`) with exact `Yazi` `Highlighter` theme parity (`bat --theme=ansi` / `th.mgr.syntect_theme`, `tab_size = 2`), background `bat` cache pre-warming on `px == 0`, 60fps-paced 120 Hz delta accumulation (eliminating `PeekError::Cancelled` worker starvation), and Main-UI-VM `folder:peek` directory preview horizontal panning.
- **Real-Time `Yazi` File-Tree Sync on `Update` (`Edit`), `Read` & `LSP` (`src/commands/yazi_hook.rs`, `src/session/mod.rs`, `src/session/transcript.rs`, `src/commands/sidebar.rs`, `assets/yazi-hscroll.lua`)**:
  - Fixed Yazi 26.x `plugin` single-string argument passing (`plugin hscroll "reveal '<path>'"`), removed the 900-byte `.jsonl` truncation window that cut off `file_path` on large `Edit`/`Update`/`Write` payloads, added live `sidebar_tab_<yazi_id>.json` + `bg-pty-host` session resolution, and normalized symlinked CWD paths (`normalize_path_under_cwd`).
  - Enabled live `Yazi` file-tree reveal, cursor highlight activation, and preview sync across **`Update` (`Edit`)**, **`Write`**, **`MultiEdit`**, **`NotebookEdit`**, **`Read`**, and **`LSP`**.
- **Top-Left `Yazi` Markdown `###` Heading Preview Rendering (`scripts/md-render.py`, `assets/glow-style.json`, `install.sh`)**:
  - Replaced `glow -s=dark` (whose default Glamour stylesheet emits literal `"## "` / `"### "` / `"#### "` prefixes on H2–H6 headings and per-cell trailing ANSI space padding) with a dedicated Glamour stylesheet (`assets/glow-style.json`) and renderer (`scripts/md-render.py`) featuring distinct hierarchical glyphs (`▌`, `▍`, `▸`, `•`, `›`, `·`), ATX heading space normalization (`###标题` -> `### 标题` outside code fences), trailing ANSI padding cleanup, and a zero-dependency pure-Python fallback.

---

## [0.4.0] - 2026-11-11

### Added
- **Independent JSONC Color Palette & Live Theme Switcher (`src/tui/palette.rs`, `assets/palette.json`, `skills/theme/SKILL.md`)**:
  - User-customizable JSONC palette configuration (`~/.config/cc-ide/palette.json`, `$XDG_CONFIG_HOME/cc-ide/palette.json`, or `$CCIDE_PALETTE_FILE`) supporting `//` and `/* ... */` comments plus `#RRGGBB`, `#RGB`, `rgb(r,g,b)`, and ANSI 256-color (`0..255`) overrides across TUI borders, cursor/view highlights, oscilloscope curves, model accents, monochrome logo tinting (`"tint_logo": true`), and Powerline `statusline` segments.
  - 6 built-in named presets (`default`, `catppuccin-mocha`, `tokyo-night`, `nord`, `gruvbox-dark`, `dracula`) with `ccide --theme <preset|list|reset>`, `/cc-ide:theme`, and `mtime`-driven live hot-reload across all open sidebars.
- **Keyboard-Navigable & Scrollable `SUBAGENTS` Panel (`src/commands/sidebar.rs`, `skills/clear/SKILL.md`)**:
  - Scrollable `SUBAGENTS` viewport keeping `● [main]` pinned at the top with mouse wheel / trackpad scrolling and full keyboard navigation (`↑`/`↓` or `j`/`k`, `PgUp`/`PgDn`, `g`/`G`, `Enter`/`Space` to switch views).
  - Single-item completed subagent dismissal (`x`/`d`/`Backspace`/`Delete`, left-clicking the right-aligned `×`, or secondary click) and bulk dismissal (`c`, clicking `[c] clear N`, `ccide --clear-subagents`, or `/cc-ide:clear`).
- **Version-Independent Structural Binary Patcher (`scripts/patch_claude_binary.py`, `tests/test_patch_claude_binary.py`)**:
  - Dynamic `\n---- Bun! ----\n` standalone trailer and `StandaloneModuleGraph` parser supporting both macOS Mach-O and Linux ELF binaries without hardcoded file offsets.
  - Unminified invariant telemetry/string regex symbol discovery (`enterView`, `exitView`, `adoptAgent`), selective per-module Bun bytecode cache invalidation, V8 `node --check` syntax verification, and `process.pid`-scoped `/tmp/.cc-ide/*` IPC.
  - `--verify` in-memory dry-run JSON compatibility reporting and automatic binary re-patching on `ccide` launch via `~/.cache/cc-ide/claude_bin.stamp`.
- **On-Demand Prebuilt Binary Resolver (`scripts/ensure-ccide-core.sh`)**:
  - POSIX shell resolver for zero-Rust and Claude Code Marketplace installations that auto-detects `aarch64`/`x86_64` × `macOS`/`Linux`, downloads `ccide-core-<triple>.tar.gz` from GitHub Releases, verifies `SHA256SUMS.txt`, and ad-hoc `codesign`s on macOS.
- **3-Stage CI/CD Promotion Pipeline & Daily Upstream Watch (`.github/workflows/ci.yml`, `release.yml`, `cc-compat-watch.yml`)**:
  - Enforced `dev -> preview -> main` branch flow guard, Rust `1.70.0` MSRV verification, strict `rustfmt` & `clippy -D warnings`, 4-target cross-compilation matrix, shell/Python/JSON manifest checks, and synthetic Bun mutation unit tests.
  - Automated preview RC (`v0.4.0-rc.<run_number>`) and immutable stable (`v0.4.0`) GitHub Release publishing with `SHA256SUMS.txt`.
  - Scheduled daily (`03:17 UTC`) compatibility probe against `@anthropic-ai/claude-code@latest` on npm with deduplicated `[Compat Alert]` issue creation.

### Changed
- **Rust Edition 2021 & LTS MSRV `1.70` (`Cargo.toml`, `Cargo.lock`, `src/tui/width.rs`)**:
  - Pinned `edition = "2021"` and `rust-version = "1.70"` with `Cargo.lock` v3 and zero external crate dependencies so `ccide-core` builds cleanly on LTS Linux distributions and macOS.
- **Marketplace Hook Bootstrap (`hooks/hooks.json`)**:
  - Routed `PostToolUse`, `SubagentStart`, and `SubagentStop` hooks through `scripts/ensure-ccide-core.sh` for automatic first-run binary provisioning.

### Fixed
- **Linux `aarch64` Cross-Compilation (`22d8a41`)**:
  - Added `libc6-dev-arm64-cross` alongside `gcc-aarch64-linux-gnu` in CI and Release workflows so `aarch64-unknown-linux-gnu` links cleanly on Ubuntu runners.

---

## [0.3.0] - 2026-11-11

### Added
- **Zero-Dependency Native Rust Engine (`ccide-core`)**:
  - Unified native binary providing `sidebar`, `yazi-hook`, `statusline`, and `workspace` subcommands with fat LTO and stripped release symbols.
- **3-Tier Adaptive Workspace Engine (`src/commands/workspace.rs`, `scripts/ccide_daemon.py`, `scripts/ccide`)**:
  - **Tier 1 (Local iTerm2 Daemon)**: Persistent `AutoLaunch` daemon over `/tmp/.cc-ide/iterm_daemon.sock` (`< 5 ms` native Metal splits, `< 2 ms` focus restoration, zero-scrollback TUI pane locking).
  - **Tier 2 (SSH `OSC 1337` Passthrough + `ControlMaster`)**: Zero-GUI-dependency remote trigger splitting client iTerm2 windows natively over multiplexed SSH sockets with a `450 ms` `/tmp/.cc-ide/osc_ack_<yazi_id>` handshake.
  - **Tier 3 (Universal `tmux` Engine)**: Dedicated `tmux -L ccide` `30% | 54% | 16%` 3-column tiling across Ghostty, WezTerm, Kitty, Alacritty, VS Code, and SSH jump hosts.
- **In-Process Native Subagent View Switching & Session Isolation (`src/session/`, `src/commands/yazi_hook.rs`)**:
  - Bidirectional `SIGUSR2` view switching into Claude Code's internal `viewing-agent` state machine, full-session completed subagent retention, kernel process-tree ancestry isolation (`proc_pidinfo` on macOS, `/proc/<pid>/stat` on Linux), and active-view-linked `Yazi` file-tree reveal.
- **Shared Cross-Platform Hardware Oscilloscope & Quadrant Model Logos (`src/telemetry/`, `src/tui/logo.rs`)**:
  - Single-instance `500 ms` `/tmp/.cc-ide/hw_telemetry_v1.bin` binary snapshot shared across all open tabs (`< 2 µs` read) rendering sub-pixel `CPU`, `GPU` (`IOKit AGX` / Linux `sysfs` & `nvidia-smi`), `MEM`, and `NET` curves alongside dynamic 2×2 Quadrant vector model logos (`CLAUDE`, `GEMINI`, `DEEPSEEK`, `OPENAI`, `GROK`).

[Unreleased]: https://github.com/Visio-Vanitas/ccide/compare/v0.11.3...HEAD
[0.11.3]: https://github.com/Visio-Vanitas/ccide/compare/v0.11.2...v0.11.3
[0.11.2]: https://github.com/Visio-Vanitas/ccide/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/Visio-Vanitas/ccide/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/Visio-Vanitas/ccide/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.9.4...v0.10.0
[0.9.4]: https://github.com/Visio-Vanitas/ccide/compare/v0.9.3...v0.9.4
[0.9.3]: https://github.com/Visio-Vanitas/ccide/compare/v0.9.2...v0.9.3
[0.9.2]: https://github.com/Visio-Vanitas/ccide/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/Visio-Vanitas/ccide/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.5.0...v0.7.0
[0.5.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.4.0-rc.1...v0.4.0
[0.3.0]: https://github.com/Visio-Vanitas/ccide/releases/tag/v0.4.0-rc.1
