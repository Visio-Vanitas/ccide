# `cc-ide` (`ccide-core`) — Architecture & Agent Maintenance Guide

## 1. Overview
`cc-ide` (v0.7.0) is a high-performance, cross-platform native engine for the 3-column Claude Code tiling workspace across **macOS**, **Linux**, and **Windows (`pwsh` / `cmd.exe`)**. It is written in **Rust Edition 2021** with a **minimum supported Rust version of `1.70`** (`rust-version = "1.70"`), uses zero external Cargo crates and no nightly features, and builds out of the box on LTS Linux distributions, macOS, and Windows MSVC:
- **Left Column (30%)**: `Yazi` File Tree (top, synced live on Claude Code `Read`/`Edit`/`Write`/`MultiEdit`/`NotebookEdit`/`LSP`/`Grep` ops — automatically revealing the file AND scrolling/highlighting the exact modified or inspected line range in preview via `assets/yazi-hscroll.lua` + `src/session/focus.rs`) + `Lazygit` Diff (bottom, unwrapped by default via `~/.config/cc-ide/viewer.json` with compact single-column `109│` line numbers via `scripts/lazygit-pager.sh`, full-stream zero-telemetry hash-free commit logs via `scripts/lazygit-log.py`, optional `vi`-style `↪` wrapping when `"wrap": true`, and hover-targeted content-bounds-clamped Magic Mouse scrolling via `scripts/pty-hscroll-bridge.py`), locked to zero terminal scrollback (`scrollback_lines = 0`).
- **Center Column (54%)**: `Claude Code` Main Workspace (with native `viewing-agent` switching, untouched native `/btw` overlay, and clean bottom chrome).
- **Right Column (16%)**: `ccide-core sidebar` 3-Tier Interactive TUI:
  1. **Top Panel**: Resolution-adaptive model sprite (`GEMINI`, `DEEPSEEK`, `CLAUDE`, `OPENAI`, `GROK` via native inline PNG `OSC 1337` / Kitty `APC _G` or standalone `.ansi` art), switching dynamically between the Main Model (`settings.model`) and the Subagent Model (`settings.env.CLAUDE_CODE_SUBAGENT_MODEL`) when viewing a subagent.
  2. **Middle Panel (`SUBAGENTS`)**: Permanent top `● [main]` item + session-isolated subagents. Clicking any item drives Claude Code's native `viewing-agent` state machine and jumps `Yazi` to the selected session's file and line range.
  3. **Bottom Region (`M4 PRO` / host CPU `⇄` Split `CONTEXT` + Per-Model `COST`)**: Multi-row (`8 × height` sub-pixel) oscilloscope curves for `CPU`, `GPU`, `MEM`, and `NET`, toggling (`t`/`Tab`, right-click, or clicking `[CTX xx%]`/`[HW]`) into a dual-box view: upper **`CONTEXT`** (`SYS+MD`, `TOOLS`, `TOOL IO`, `THINK`, `CHAT`, `FREE` headroom up to the exact auto-compact threshold, and cumulative/turn `CACHE` hit ratio) + lower **`COST`** (`tokscale`-aligned per-model USD spend, prompt-cache savings, burn rate, and token breakdown across Main + Subagents).

The workspace is **adaptive**: the same binary selects one of three split backends depending on the host terminal and transport — local iTerm2 daemon (Tier 1), SSH + client iTerm2 native split (Tier 2), or a universal `tmux` engine (Tier 3). See §4.

---

## 2. Unified Official Plugin & Project Structure (`ccide/`)
```text
ccide/
├── AGENT.md                  # Architecture & maintenance guide (symlinked as CLAUDE.md)
├── CONTRIBUTING.md           # Human & AI Agent Playbook, automation hooks & <300-line invariants
├── CHANGELOG.md              # Release history (Keep a Changelog / SemVer)
├── .github/
│   ├── ISSUE_TEMPLATE/       # Structured YAML Issue Forms (bug_report, feature_request, upstream_compat)
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── workflows/
│       ├── ci.yml            # CI Gate: 1.70.0 MSRV, fmt/clippy, 6-target matrix, scripts & patcher tests
│       ├── release.yml       # Release: gated preview RC (`v0.7.0-rc.<n>`) + immutable stable (`v0.7.0`)
│       ├── cc-compat-watch.yml # Daily 3-OS (`darwin`/`linux`/`win32`) npm Claude Code compatibility probe
│       └── issue-pr-lifecycle.yml # Auto-triage labels, PR stage bridge & multi-hop `main` issue closer
├── Cargo.toml                # Rust Edition 2021, MSRV 1.70, fat LTO, stripped release profile
├── install.sh                # POSIX Interactive TUI & Headless Installer, Doctor & Uninstaller
├── install.ps1 / install.cmd # Native PowerShell & cmd.exe Windows Installer, Doctor & Uninstaller
├── .claude-plugin/
│   ├── marketplace.json      # Official Claude Code Marketplace manifest ("cc-ide")
│   └── plugin.json           # Official Claude Code Plugin manifest ("cc-ide" v0.7.0)
├── hooks/
│   └── hooks.json            # Official Plugin Hooks (PreToolUse, PostToolUse, SubagentStart/Stop, PostModelSwitch, SessionStart)
├── docs/
│   └── CLAUDE_CODE_INTERFACES.md # Verified Claude Code plugin, hook, subagent & Workflow SDK reference
├── skills/
│   ├── ccide/SKILL.md        # `/cc-ide:ccide` workspace controller skill
│   ├── clear/SKILL.md        # `/cc-ide:clear` dismiss completed subagents skill
│   ├── orchestrate/SKILL.md  # `/cc-ide:orchestrate` model/effort & workflow orchestrator skill
│   ├── rebalance/SKILL.md    # `/cc-ide:rebalance` 30%|54%|16% layout rebalancer skill
│   └── theme/SKILL.md        # `/cc-ide:theme` live JSONC palette & preset switcher skill
├── scripts/
│   ├── ccide                 # Adaptive 3-Tier POSIX launcher (backend auto-detect, tmux bootstrap, SSH ControlMaster)
│   ├── ccide.ps1 / ccide.cmd # Native Windows PowerShell & cmd.exe workspace launchers
│   ├── ccide_workflow_runner.py # Native Workflow CLI runner (`ccide workflow list|run`)
│   ├── ccide_workflow_server.py # Local HTTP/SSE Workflow API server (`ccide workflow serve`)
│   ├── ensure-ccide-core.mjs # Cross-OS Node hook & prebuilt release binary resolver (macOS/Linux/Windows)
│   ├── ensure-ccide-core.sh  # POSIX standalone prebuilt GitHub Release binary resolver
│   ├── cc-maint.py           # 3-OS Claude Code symbol & schema maintenance workbench (`ccide --check-cc`)
│   ├── cc_schema_inspector.py# Upstream `statusLine` / `.jsonl` / `subagent_meta` / Hook schema extractor
│   ├── bun_pe.py             # Cross-OS Mach-O / ELF / Win32 PE (`B:/~BUN/` + Authenticode) Bun graph parser
│   ├── cc_patch_rules.py     # Minimal structural regex patch rules & invariant string anchors
│   ├── patch_claude_binary.py# Atomic binary patcher, Bun bytecode cache invalidator & `--verify` guard
│   ├── agent-task-context.py # Deterministic Issue -> JSON agent brief, PR promotion notes & line-budget gate
│   ├── sync-pricing.py       # Upstream pricing sync (`BerriAI/litellm` + `pydantic/genai-prices`)
│   ├── ccide_daemon.py       # iTerm2 AutoLaunch warm daemon (Unix socket + OSC 1337 SSH bridge + FocusMonitor)
│   ├── iterm2_workspace.py   # Cold fallback: iTerm2 Python API 3-column tiling & view-switch controller
│   ├── pane-runner.sh        # POSIX companion TUI pane runner (yazi, lazygit, macmon, sidebar)
│   ├── lazygit-pager.sh      # Compact single-column (`109│`) diff pager (unwrapped default, optional vi `↪` wrap)
│   ├── lazygit-log.py        # Full-stream hash-free `[Author] [Smooth DAG] [Subject]` commit log for Lazygit
│   ├── git_smooth_graph.py   # 4-direction bitmask Unicode box-drawing DAG commit tree renderer
│   ├── pty-hscroll-bridge.py # Hover-targeted & content-bounds-clamped PTY Magic Mouse SGR 66/67 & focus bridge
│   ├── md-render.py          # Yazi & Lazygit Markdown preview renderer (clean H1-H6 glyphs, no raw ###)
│   └── cc_tui_lib.py         # Python TUI & quadrant logo generator library
├── assets/
│   ├── sprites/              # Standalone transparent RGBA `.png` & `.ansi` model family sprites
│   ├── workflows/            # Built-in Workflow JS templates (`ccide-consensus.js`, `ccide-implement.js`, `ccide-review.js`)
│   ├── cc-schema.json        # Verified 3-OS Claude Code patch anchor & protocol field baseline
│   ├── orchestrator.json     # Default per-role subagent & Workflow model/effort routing config
│   ├── pricing.json          # Multi-vendor `tokscale`-aligned token & prompt-cache pricing catalog
│   ├── adaptive_quadrant_logos.json # Pre-rendered 2x2 Quadrant vector fallback logos
│   ├── glow-style.json       # Custom Glamour dark stylesheet (clean H1-H6 headings without ### prefixes)
│   ├── lazygit.yml           # Lazygit full-width, hash-free commits, unwrapped default & compact diff overlay
│   ├── viewer.json           # Line-wrapping, `sprite_mode` & `cost_lines` (`3` | `2` | `1`) JSONC config
│   ├── yazi-hscroll.lua      # Yazi 120Hz `:touch`, keyboard pan, blur & auto-scrolling hunk-highlight plugin
│   └── palette.json          # Commented JSONC color palette template
├── bin/                      # Generated by ./install.sh (gitignored)
│   └── ccide-core            # Compiled native Rust binary (`sidebar`, `yazi-hook`, `statusline`, `workspace`, `pane-runner`, `setup`, `doctor`)
└── src/
    ├── main.rs               # CLI dispatcher (`sidebar`, `yazi-hook`, `statusline`, `workspace`, `pane-runner`, `wake-pid`, `setup`, `doctor`)
    ├── platform.rs           # Cross-OS path, IPC directory, UNC normalization & executable discovery
    ├── ffi/                  # Zero-crate OS FFI (`mod`, `unix`, `macos`, `windows`, `win_sys`)
    ├── json.rs               # Zero-dependency JSON field & balanced-object extractors
    ├── tui/
    │   ├── mod.rs            # ANSI color palette & block/spinner constants
    │   ├── layout.rs         # Declarative cell typesetting engine (`RowLayout`, `ColSpec`, `ColumnWidths`)
    │   ├── context/          # Compact `CONTEXT` + scrollable `3/2/1`-line `COST` (`mod`, `analyzer`, `bar`, `cost_panel`, `json_scan`, `parser`, `perf`, `render`)
    │   ├── pricing/          # `tokscale`-aligned progressive multi-vendor pricing (`mod`, `lookup`)
    │   ├── logo/             # Inline Kitty `APC _G` / `OSC 1337` PNG & `.ansi` sprite renderer (`mod`, `image`)
    │   ├── palette.rs        # Customizable JSONC color palette & 6 built-in theme presets
    │   ├── width.rs          # ANSI- & BMP-wide-symbol-aware CJK display column width & safe truncation
    │   └── grid.rs           # TermGeom, BoxRow, PanelSpec, border focus, full-bleed row bg & row-level diffing
    ├── telemetry/            # Cross-OS 500ms shared snapshot (`mod`, `macos`, `linux`, `windows`, `chart`)
    ├── orchestrator/         # Zero-patch Subagent & Workflow router (`mod`, `presets`, `router`, `shims`, `workflow_router`)
    ├── session/
    │   ├── mod.rs            # Process-tree-isolated sessionId & live Yazi tab resolver
    │   ├── model.rs          # Per-session model family, alias & 3-tier width compactor
    │   ├── focus.rs          # Tool hunk line locator, Yazi `blur` & preview auto-scroll/highlight IPC
    │   ├── transcript.rs     # Incremental offset-cached .jsonl parser for main session
    │   └── subagents.rs      # Session-isolated subagent discovery & dismissal store
    └── commands/
        ├── mod.rs
        ├── pane_runner.rs    # Cross-platform native companion pane supervisor (`ccide-core pane-runner`)
        ├── pre_tool_hook.rs  # `PreToolUse` subagent & Workflow model/effort router (`ccide-core pre-tool-hook`)
        ├── setup/            # Interactive TUI installer, theme studio & `doctor [--json]` (`mod`, `doctor`, `doctor_render`, `input`, `runner`, `ui`)
        ├── sidebar/          # Event-driven right sidebar (`mod`, `bottom_panel`, `dispatch`, `hit_mask`, `input`, `ipc`, `mouse_actions`, `orch_panel`, `subagents_panel`)
        ├── statusline/       # Powerline statusline & exact auto-compact budget (`mod`, `compact`)
        ├── workspace/        # Adaptive workspace backends (`mod`, `actions`, `open`, `iterm`, `tmux`, `wezterm`, `wt`)
        └── yazi_hook.rs      # Official Plugin Hook handler (`PostToolUse`, `Subagent*`, `PostModelSwitch`, `SessionStart`)
```

---

## 3. Core Technical Invariants

### 3.1 Strict Per-Tab Process Isolation (Never Cross `SUBAGENTS` Lists)
- **Never use a global `live_latest.json` file**: Global files cause cross-tab contamination when multiple Claude sessions run simultaneously.
- **Process Tree Ancestry (`src/session/mod.rs` + `src/ffi.rs`)**:
  1. When the workspace opens or rebalances a tab, it queries the Center pane's shell PID and writes `~/.cache/cc-ide/tab_shell_<yazi_id>.txt` (Tier 1/2 via `async_get_variable("pid")`; Tier 3 writes `CCIDE_SHELL_PID` from the launcher).
  2. `resolve_isolated_session_id()` scans `~/.claude/sessions/<claude_pid>.json`, checks `is_pid_alive(claude_pid)`, and calls `is_descendant_of(claude_pid, shell_pid)` — macOS walks `pbi_ppid` in kernel memory via `proc_pidinfo(PROC_PIDTBSDINFO)` (`~0.002ms`), Linux parses `/proc/<pid>/stat` (`< 5 µs`).
  3. Fallback matching uses `std::fs::canonicalize(cwd)` (`realpath`) so symlinked project directories (e.g., `~/Projects/...` -> `/mnt/data/Projects/...`) match accurately.

### 3.2 Config-Driven & Per-Session Real-Time Model Resolution (`src/session/mod.rs` + `src/commands/statusline.rs`)
- **Main Model (Per-Session Isolated, 160ms Real-Time Refresh)**:
  - Each tab resolves its own active Main Model by comparing the modification timestamps (`mtime`) of `~/.cache/cc-ide/live_sess_<sessionId>.json` (exported by `ccide-core statusline` for that session) and `~/.claude/projects/*/<sessionId>.jsonl` (extracting `/model` `Set model to ...` stdout and assistant message `"model"` fields), falling back to `~/.claude/settings.json` `"model"`.
  - When `/model` is executed in any session, that tab's Top Logo Panel (`GEMINI`, `DEEPSEEK`, `CLAUDE`, `OPENAI`, `GROK`) updates within **`160ms`** without affecting other sessions.
- **Subagent Model (Strictly Config-Driven)**:
  - Always reads `env.CLAUDE_CODE_SUBAGENT_MODEL` from `~/.claude/settings.json` (never from API/statusline payloads).
  - Clicking a subagent in `SUBAGENTS` switches the Top Logo Panel to the configured Subagent Model (`DEEPSEEK`), and clicking `● [main]` switches it back to that session's Main Model.

### 3.3 Shared Single-Instance Cross-Platform Hardware Telemetry (`/tmp/.cc-ide/hw_telemetry_v1.bin`)
- **Decoupled Refresh Cadence (`src/telemetry/mod.rs`)**:
  - Top Logo (`Panel 1`) and `SUBAGENTS` (`Panel 2`) refresh every **`160ms`** per tab for instant UI responsiveness.
  - Bottom Hardware Telemetry (`Panel 3`) shares a single fixed-layout binary snapshot `/tmp/.cc-ide/hw_telemetry_v1.bin` (`1,672` bytes `#[repr(C)]` `SharedTelemetrySnapshot`, magic `"CCIDE_HW"`) across all open windows and tabs.
  - At most **one** hardware query executes globally across the entire machine every **`500ms`**; all other tabs read the snapshot in `< 2 µs` (`0.002ms`), keeping CPU usage at **`~0.6%` per tab** and ensuring newly opened tabs immediately inherit the full 48-point oscilloscope curve history. Snapshots are written atomically (`.tmp.<pid>` + `rename`).
- **Cross-Platform Sources (`src/ffi.rs` + `src/telemetry/mod.rs`)**:
  - **macOS** (`arm64` / `x86_64`): `Mach host_statistics` / `host_statistics64` for CPU ticks, page counts & swap; `IOKit` `IOAccelerator` registry (`AGXAccelerator...`) for GPU utilization & VRAM; BSD `sysctl` `NET_RT_IFLIST2` for interface byte counters; `sysctl` `machdep.cpu.brand_string` / `hw.logicalcpu` / `hw.memsize` for identity.
  - **Linux** (`x86_64` / `aarch64`): `/proc/stat` (CPU ticks), `/proc/meminfo` (`MemTotal` / `MemAvailable`), `/proc/net/dev` (`rx_bytes` / `tx_bytes`), `/proc/cpuinfo` (model name), `/proc/loadavg`; GPU from `sysfs` `/sys/class/drm/card0/device/gpu_busy_percent`, else a cached non-blocking `nvidia-smi` probe (`/proc/driver/nvidia/version` gate, results cached in `/tmp/.cc-ide/nvidia_stat.txt`).
  - All sampling is in-process (`0` forks/sec on the hot path); the only child process is the rate-limited `nvidia-smi` probe on NVIDIA Linux hosts.

### 3.4 Event-Driven Hook & Signal Architecture (`SIGUSR2`, Zero Idle Polling)
- **Bidirectional Kernel Signal (`SIGUSR2 = 31` on macOS, `12` on Linux)**:
  1. **Right Sidebar -> Claude Code (`src/commands/sidebar.rs` -> Claude Code binary)**:
     - Clicking `● [main]` or any `[subagent]` writes `/tmp/.cc-ide/req_pid_<process.pid>.json` and calls `kill(claude_pid, SIGUSR2)`.
     - Inside Claude Code (patched via `~/.claude/scripts/patch_claude_binary.py`), a `process.on("SIGUSR2", ...)` listener wakes up in `< 0.05ms`, registers completed subagents on demand via the resolved `adoptAgent` function, and invokes the resolved `enterView` / `exitView` functions without any background timer polling.
  2. **Claude Code -> Right Sidebar (`claude.exe` / Hooks / `statusline` -> `ccide-core sidebar`)**:
     - `ccide-core sidebar` registers `signal(SIGUSR2, handle_sigusr2)` and writes its PID to `/tmp/.cc-ide/sb_pid_<process.pid>.pid`.
     - Whenever `/model` or `viewingAgentTaskId` changes inside Claude Code, or whenever `SubagentStart`, `SubagentStop`, `PostToolUse`, or `statusline` runs, `notify_sidebar()` sends `SIGUSR2` to wake `ccide-core sidebar` from `poll()` in `< 0.05ms`.
  3. **Row-Level Diffing (`src/tui/grid.rs`)**:
     - `render_multi_panel_grid()` diffs each row against `prev_rows` and only writes ANSI sequences for rows that actually changed (`0` bytes written for `Panel 1` and `Panel 2` when idle).

### 3.5 Version-Independent Structural Binary Patching (`scripts/patch_claude_binary.py`)
The patcher adapts to every Claude Code upgrade without hardcoded offsets or minified names:
1. **Dynamic Bun Standalone Trailer Parsing**: Locates the `\n---- Bun! ----\n` marker on both **macOS Mach-O** and **Linux ELF** binaries, reads the preceding 32-byte header (`<QII`: `byte_count`, `mod_off`, `mod_len`), derives the base offset, and walks the `StandaloneModuleGraph` table (`52`-byte entries, `<13I`) to enumerate JS modules under `/$bunfs/`. After patching, the `bytecode_*` fields and `0x100` flag are cleared only for affected modules, so Bun recompiles them instead of executing stale cached bytecode.
2. **Unminified Invariant String Anchors**: `discover_runtime_symbols()` regex-resolves `enterView` / `exitView` / `adoptAgent` from telemetry literals (`"tengu_transcript_view_enter"`, `"tengu_transcript_view_exit"`, `"local_agent"` + `"(resumed agent)"`). All other patches anchor on stable literals (`"in_process_teammate"`, `"quietlyParked"`, `".evictTerminal("`, `"local_workflow"`, `"PromptHintPart"`, `key:"mode"`). Only 4 patches are required (`subagent_retention_timers`, `transcript_view_retention`, `keep_completed_visible`, `sigusr2_view_switch_hook`); the UI cleanups are optional.
3. **Zero-Minifier-Dependency IPC**: The injected hook communicates via `process.pid`-scoped files — `/tmp/.cc-ide/req_pid_<pid>.json`, `cur_pid_<pid>.txt`, and `sb_pid_<pid>.pid` — so no minified variable name is ever hardcoded into the IPC contract.
4. **Idempotent Install & Auto-Repatch**: `apply_patches()` backs up the pristine binary once, writes atomically (`.tmp` + `rename`), and ad-hoc `codesign`s on macOS. `scripts/ccide`'s `auto_ensure_claude_patched()` fingerprints the binary (`inode:size:mtime` -> `~/.cache/cc-ide/claude_bin.stamp`) and silently re-runs the patcher on the first launch after an upstream update.
5. **Compatibility Guard**: `--verify` runs entirely in memory and prints a JSON report (`required_ok`, per-patch `applied`/`offset`, resolved symbols, cleared bytecode modules); it exits `1` when a required anchor fails to match, which is exactly what `ci.yml` and `cc-compat-watch.yml` consume.

### 3.6 Zero-Patch Subagent & Native `Workflow` Orchestrator (`src/orchestrator/`)
- **Config-Driven Role Routing (`~/.config/cc-ide/orchestrator.json`)**: Maps `Scout`, `Coder`, `Reviewer`, `Plan`, and `Workflow` roles to independent model and reasoning-effort overrides, edited interactively via the Right Sidebar `ORCHESTRATOR` toggle panel (`src/commands/sidebar/orch_panel.rs`) or `ccide orchestrator` (`status`, `preset`, `sync`).
- **PreToolUse & Workflow Dispatch**: `ccide-core pre-tool-hook` (`src/commands/pre_tool_hook.rs`) rewrites `Agent` / `Workflow` tool inputs via `hooks/hooks.json` without binary patching, while `assets/workflows/*.js`, `scripts/ccide_workflow_runner.py` (`ccide workflow list|run`), and `scripts/ccide_workflow_server.py` (`ccide workflow serve`) provide reusable multi-agent workflow templates and a local HTTP/SSE API.

---

## 4. 3-Tier Adaptive Workspace Architecture (`src/commands/workspace.rs`)

`ccide-core workspace <open|close|rebalance|focus|switch-subagent>` is the single entry point. The launcher `scripts/ccide` passes the resolved backend as the last `open` argument (or via `CCIDE_BACKEND`); `auto` selects at runtime:

| Tier | Condition (`auto`) | Transport | Typical latency |
|------|--------------------|-----------|-----------------|
| **1 — Local iTerm2 Daemon** | `!SSH && !$TMUX && iTerm2` | Unix socket -> warm `iterm2.App` | **`< 5ms`** split, **`< 2ms`** focus |
| **2 — SSH + Client iTerm2** | `SSH && !$TMUX && iTerm2` | `OSC 1337;Custom=id=ccide:` passthrough -> client daemon | **`< 450ms`** handshake budget |
| **3 — Universal `tmux`** | `$TMUX` set, or non-iTerm2 terminal | `tmux` server (`-L ccide`) | Native tmux split commands |

### 4.1 Tier 1 — Local iTerm2 AutoLaunch Daemon (`< 5ms` native Metal split panes)
- `scripts/ccide_daemon.py` runs inside iTerm2's Python environment (`iterm2.run_forever`) as an `AutoLaunch` script, symlinked to `~/Library/Application Support/iTerm2/Scripts/AutoLaunch/ccide_daemon.py`.
- It owns a **Unix domain socket `/tmp/.cc-ide/iterm_daemon.sock`** (mode `0600`, PID at `/tmp/.cc-ide/iterm_daemon.pid`) and serves newline-delimited JSON RPC: `ping`, `open`, `close`, `rebalance`, `focus`, `switch-subagent`.
- Because `iterm2.App` state is already warm, splits avoid the ~350ms Python interpreter + WebSocket cold start. `send_daemon_ipc()` in `workspace.rs` connects, writes one line, reads one line, and treats an empty or `ERR:` reply as failure.
- Pane creation uses `async_split_pane` with a write-only profile that forces zero scrollback, unlimited-scrollback off, mouse reporting on, and (optionally) a custom working directory. Widths are then normalized by `apply_consistent_tab_widths()` to `left_pct | remainder | 16%` (minimum 56 / 80 / 38 cells; left column rows are split evenly between yazi and lazygit/macmon).
- Session bookkeeping: side pane IDs are stored on the Center session as `user.cc_side_panes`, and the tab's yazi id as `user.cc_yazi_id`.
- **Cold fallback**: if the socket is missing or dead, `workspace.rs` starts the daemon (`ensure_daemon_started`) and shells out to `iterm2_workspace.py open ...` for that call.
- **Focus / subagent switching** (`focus_center_after_view_switch`, used by the sidebar) sends `{"op":"switch-subagent",...}` with a 150ms timeout and returns as soon as the daemon acknowledges; the Python `iterm2_workspace.py` path is only a fallback.

### 4.2 Tier 2 — SSH + Client iTerm2 Native Split (`OSC 1337` + ControlMaster + ACK)
- The remote host gains **zero** GUI dependency. `workspace.rs` writes
  `\x1b]1337;Custom=id=ccide:<json>\x07` to `/dev/tty` (falling back to stdout), which iTerm2 passes through to the **client** Mac's daemon.
- The client daemon subscribes globally via `async_subscribe_to_custom_escape_sequence_notification` and only accepts sequences whose `sender_identity == "ccide"`.
- The payload is `{"op":"open_ssh","dir","git_root","yid","git","mon","left_pct","right","shell_pid","user","ssh_conn"}`. The daemon discovers the local `ssh` client process under the Center pane's shell PID (`detect_ssh_transport_for_pid`, extracting `-p`, `-i`, `-J`, `-F`, `-l`, `-o`, `-S`) and reuses/establishes a multiplexed connection:
  `ssh -o ControlMaster=auto -o ControlPath=~/.ssh/ccide-cm-%C -o ControlPersist=10m ...`.
  If the process tree yields nothing, it falls back to `user@server_ip:port` from the `ssh_conn` field (`SSH_CONNECTION`).
- Each remote pane runs `~/.claude/scripts/pane-runner.sh <yazi|lazygit|macmon|sidebar> ...` over that shared socket, so the client iTerm2 tab is split natively into local Metal panes at `30% | 54% | 16%`. The remote `claude` keeps running in the original tab.
- **Handshake**: the remote side needs proof the client finished. `pane-runner.sh` writes `ok` to `/tmp/.cc-ide/osc_ack_<yazi_id>` on startup; `workspace.rs` polls that path every `15ms` for up to `450ms`. On success it records `osc:<yazi_id>` in `/tmp/.cc-ide/panes_<yazi_id>.txt`. On timeout the launcher falls back to bootstrapping Tier 3.
- **SSH config** is generated by `install.sh` at `~/.ssh/ccide_config` (ControlMaster/ControlPath/ControlPersist/SendEnv `LC_TERMINAL`) and `Include`d from `~/.ssh/config`, so remote `claude` sessions inherit the multiplexed socket.
- Focus events use `{"op":"focus","yid","sid","view"}` and are emitted when `SSH_CONNECTION`/`SSH_TTY` is set and `$TMUX` is not.

### 4.3 Tier 3 — Universal `tmux` 3-Column Tiling Engine (`tmux -L ccide`)
- Used whenever `$TMUX` is set or the terminal is not iTerm2 (Ghostty, Kitty, WezTerm, Alacritty, VS Code, bare SSH jump hosts). `scripts/ccide` bootstraps `tmux -L ccide new-session -A -s ccide-<dir>-<pid>` unless already inside tmux or running under `CLAUDECODE`.
- `run_tmux_open()` measures `#{window_width}`, computes `left = max(46, 30%)` and `right = max(38, 16%)` (the remaining ~54% stays with the Claude pane), then:
  1. `split-window -hb` for the left column (yazi), then `-v 50%` for lazygit (if a `.git` root was found) and optionally `-v 35%` for macmon;
  2. `split-window -h` for the right `ccide-core sidebar`;
  3. `select-pane` back to the Center Claude pane.
- It enables `mouse on` for the window and records `tmux:<center_pane>:<side1,side2,...>` in `/tmp/.cc-ide/panes_<yazi_id>.txt`. `close` replays `kill-pane` for the recorded sides; `switch-subagent`/focus simply `tmux select-pane` on the recorded center pane (`< 2ms`).
- `rebalance` under tmux only guarantees `mouse on`; tmux keeps its own pane geometry, so the exact `30% | 54% | 16%` percentages apply at creation time (separator columns can shift the realized widths by a cell or two).
- **Layout caveat**: the `-hb` split takes from the active pane, so realized columns at 240 cells are 72 / 128 / 38 (30.0% / 53.3% / 15.8%), with two 1-cell separators accounting for the remainder.

---

## 5. Runtime Paths & Hand-off Files
All shared state lives under `/tmp/.cc-ide/` (mode-restricted socket; plain files otherwise), with grouped snapshots in `~/.cache/cc-ide/`:

| Path | Writer -> Reader | Purpose |
|------|------------------|---------|
| `/tmp/.cc-ide/iterm_daemon.sock` | daemon -> `workspace.rs`, sidebar | Tier 1 JSON RPC |
| `/tmp/.cc-ide/iterm_daemon.pid` | daemon | daemon liveness |
| `/tmp/.cc-ide/panes_<yazi_id>.txt` | `workspace.rs` | `iterm:` / `osc:` / `tmux:` pane spec |
| `/tmp/.cc-ide/osc_ack_<yazi_id>` | `pane-runner.sh` -> `workspace.rs` | Tier 2 completion handshake |
| `/tmp/.cc-ide/hw_telemetry_v1.bin` | any sidebar -> all sidebars | shared 500ms telemetry snapshot |
| `/tmp/.cc-ide/sys_telemetry.cache` | sampler -> `statusline` | `CPU ..%  GPU ..%  RAM ..%` |
| `/tmp/.cc-ide/cur_pid_<pid>.txt` | Claude Code -> sidebar | current viewing-agent view |
| `/tmp/.cc-ide/req_pid_<pid>.json` | sidebar -> Claude Code | view-switch request |
| `/tmp/.cc-ide/sb_pid_<pid>.pid` | sidebar | `SIGUSR2` target |
| `/tmp/.cc-ide/lg_geom_<sid>.txt` | `pty-hscroll-bridge.py` -> pagers | live Lazygit `rows cols main_inner_w` |
| `/tmp/.cc-ide/nvidia_stat.txt` | Linux NVIDIA probe | cached GPU utilization/VRAM |
| `~/.claude/projects/*/<sessionId>.jsonl` | Claude Code -> sidebar | live session/subagent `CONTEXT` breakdown |
| `~/.cache/cc-ide/tab_shell_<yazi_id>.txt` | workspace/daemon | tab shell PID for isolation |
| `~/.cache/cc-ide/live_sess_<sessionId>.json` | `statusline` | per-session main model |

---

## 6. Interactive TUI Setup, Doctor & One-Click Maintenance (`./install.sh`)
All build, environment diagnostics, live theme preview, modular configuration, binary patching, validation, and hot-reload steps are unified in `./install.sh` + `ccide-core setup|doctor`:

```bash
# Launch interactive TUI Setup, Live Theme Studio & Doctor (auto-falls back to --yes in non-TTY):
./install.sh
ccide setup

# Non-interactive headless one-click build, plugin install, claude.exe patch & hot-reload:
./install.sh --yes

# Run environment, companion tool & patch-anchor health diagnostics (human TUI or JSON):
./install.sh --doctor [--json]
ccide doctor [--json]

# Apply or roll back reversible iTerm2 display & Python API preferences:
./install.sh --iterm-preferences-only
./install.sh --restore-iterm-preferences

# Re-apply only the Claude Code binary patch (usually automatic on the next `ccide` launch):
./install.sh --patch-only

# Cleanly restore claude.exe.bak, stop daemon, and strip managed SSH/Yazi/plugin entries:
./install.sh --uninstall
```

Installer side effects worth knowing:
- **Stage 3a (`scripts/iterm-pyenv.py`)**: locates `iTerm.app` across `/Applications`, `~/Applications`, and `mdfind`; respects `PIP_INDEX_URL` / `pip config list` or concurrently probes `pypi.org`, TUNA, Aliyun, and USTC (cached 24h in `~/.cache/cc-ide/pypi_mirror.json`); provisions an isolated PEP 668-safe venv at `~/.local/share/cc-ide/pyenv` with `iterm2` installed and records `~/.cache/cc-ide/python_bin` (falling back cleanly to Tier 3 `tmux` with copy-pasteable commands when offline).
- **Stage 3b (`scripts/iterm-preferences.py`)**: backs up `com.googlecode.iterm2.plist` + original key snapshot under `~/.config/cc-ide/backups/iterm2/`, idempotently applies `ShowPaneTitles=false`, `ShowPaneTitlesEvenIfOnlyOnePane=false`, `PerPaneBackgroundImage=false`, `DimInactiveSplitPanes=false`, and `EnableAPIServer=true` (leaving `SplitPaneDimmingAmount` untouched and clearing per-pane wallpapers via `LocalWriteOnlyProfile`), and supports exact rollback via `./install.sh --restore-iterm-preferences`.
- **Scripts, SSH & Plugin Cache**: symlinks `scripts/*` into `~/.claude/scripts/`, links `ccide`/`cc-ide` into `~/.local/bin/` (and `ccide.cmd` + `ccide.ps1` on Windows via `install.ps1` / `install.cmd`), (re)writes `~/.ssh/ccide_config` + `Include` in `~/.ssh/config`, (re)starts `ccide_daemon.py`, and packages `cc-ide` into `~/.claude/plugins/cache/cc-ide/cc-ide/0.7.0/` (fetching prebuilt release binaries via `scripts/ensure-ccide-core.mjs` / `ensure-ccide-core.sh` when `cargo` is absent).

---

## 7. Toolchain, Release & CI/CD (`dev -> preview -> main`)

### 7.1 MSRV & Toolchain Policy
- `Cargo.toml` pins `edition = "2021"` and `rust-version = "1.70"`. There is **zero** dependency on nightly or post-1.70 compiler features, and the crate has **no external dependencies** (the only `[[package]]` in `Cargo.lock` is `ccide-core` itself), so it builds out of the box on LTS Linux distributions, macOS, and Windows MSVC.
- CI proves the floor with a real toolchain: `dtolnay/rust-toolchain@master` with `toolchain: 1.70.0`, then `cargo +1.70.0 check --locked --all-targets` and `cargo +1.70.0 test --locked` against the v3 `Cargo.lock`. Stable is used only for `rustfmt` and `clippy --all-targets -- -D warnings`.

### 7.2 Promotion Flow (enforced by `.github/workflows/ci.yml`)
1. **`dev`** — feature work; pushes and PRs run the full gate (no release).
2. **`preview`** — a PR from `dev`; on merge, `ci.yml` runs a live 3-OS `cc-maint.py --fetch-npm latest` probe, and `release.yml` publishes a pre-release tag `v0.7.0-rc.<run_number>` (`prerelease: true`, `make_latest: false`).
3. **`main`** — a PR from `preview` only; the `branch-flow-guard` job fails any PR whose `head_ref` is not `preview`. On merge, `release.yml` publishes the immutable stable tag `v0.7.0` (`make_latest: true`).

`release.yml` builds six archives (`ccide-core-<triple>.tar.gz` for `aarch64`/`x86_64` × macOS/Linux/Windows MSVC), computes `SHA256SUMS.txt`, and attaches both to the release — the same assets `scripts/ensure-ccide-core.mjs` resolves and verifies at install/hook time.

### 7.3 Daily Upstream Compatibility Probe (`.github/workflows/cc-compat-watch.yml`)
Runs daily at `03:17 UTC` (and on `workflow_dispatch`). It resolves the newest `@anthropic-ai/claude-code` version from npm, runs `patch_claude_binary.py --verify` against the downloaded bundle, and on failure opens a `[Compat Alert] Claude Code v<ver> upstream bundle layout changed` issue with the JSON report — deduplicated against existing open issues for the same version.

---

## 8. Roadmap
- **v0.6 (Next Release)**:
  1. **Palette-Unified Custom Syntax Highlighting**: blend syntax highlighting directly with `~/.config/cc-ide/palette.json` so Yazi code previews, Lazygit diffs (`delta` & fallback), and Markdown fenced code blocks share the active theme preset and per-slot color overrides seamlessly.
  2. **First-Class Ghostty Workspace Support**: native Ghostty split/IPC workspace backend alongside Local iTerm2 Metal Daemon, SSH OSC 1337, and `tmux` (complementing Kitty Graphics Protocol `APC _G` PNG sprite rendering already supported in Ghostty).
- **v1.0**: first-class **PowerShell (`pwsh`) support** for Windows Terminal and WezTerm, including a native PowerShell pane runner and Windows packaging, while preserving the `30% | 54% | 16%` layout contract.
