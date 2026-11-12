# Changelog

All notable changes to **`ccide` (`cc-ide` / `ccide-core`)** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/Visio-Vanitas/ccide/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Visio-Vanitas/ccide/compare/v0.4.0-rc.1...v0.4.0
[0.3.0]: https://github.com/Visio-Vanitas/ccide/releases/tag/v0.4.0-rc.1
