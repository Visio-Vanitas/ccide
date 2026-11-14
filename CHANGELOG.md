# Changelog

All notable changes to **`ccide` (`cc-ide` / `ccide-core`)** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/Visio-Vanitas/ccide/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.5.0...v0.7.0
[0.5.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.4.0-rc.1...v0.4.0
[0.3.0]: https://github.com/Visio-Vanitas/ccide/releases/tag/v0.4.0-rc.1
