# `ccide` (`cc-ide`)

**Native 3-Column Tiling Terminal IDE & Real-Time Multi-Agent / Hardware Telemetry Companion (`macOS`, `Linux` & `Windows`)**

`ccide` transforms **Claude Code**, **Google Antigravity CLI (`agy`)**, and **OpenAI Codex CLI (`codex`)** into a zero-polling, 3-column tiling terminal IDE (`30% Left | 54% Center | 16% Right`) powered by a zero-dependency Rust engine (`ccide-core`, **v0.11.3**) and a 3-tier adaptive workspace controller.

> **v0.11.3 highlights:** Fixed Lazygit CJK horizontal scrolling & bold-border `focused_panel` auto-sync (`pty-hscroll-bridge.py`), aligned `[0]-Log` CJK/ASCII author prefixes to `3 + 1 = 4` display columns matching `[4]-Commits` (`lazygit-log.py`), and added `Tab`/`Shift+Tab` cursor cycling inside the focused `SUBAGENTS`/`SWARM` sidebar panel.
>
> **v0.11.2 highlights:** Hardened Antigravity `transcript.jsonl` `"args"` & double-encoded string parsing, Swarm worker hook isolation & coordinator promotion, turn-accurate multi-model `COST` + `gemini-4-{argon,hydrogen,neon}`/`gpt-oss-120b` pricing, `is_safe_id` / internal-path filtering, and `(major, minor, patch)` bootstrap & missing-milestone release fallbacks.
>
> **v0.11.1 highlights:** Full native **Antigravity Swarm** observability (`SWARM #<board_head> · <run>run·<wait>wait`, swarm title & selected-worker latest `board.jsonl` activity row, `running`/`waiting`/`stopped`/`killed` states with `waiting` protected from dismissal and `killed` dismissible, click-to-inspect worker switching Logo + `CONTEXT` + Yazi follow without `SIGUSR2`, and multi-model Swarm `COST` aggregation across coordinator + unique worker transcripts), validated `--conversation <id>` / `CCIDE_AGY_CONVERSATION` (`^[A-Za-z0-9_-]{8,64}$`), `CCIDE_AGY_HOME`, and Python 3.9+ `merge_agent_hooks.py`.
>
> **v0.11.0 highlights:** Multi-agent support across **Claude Code (`[main]`)**, **Google Antigravity CLI (`[agy]`)**, and **OpenAI Codex CLI (`[codex]`)** via `--agy`, `--codex`, `--agent <claude|agy|codex>`, or `CCIDE_AGENT` — complete with live `Yazi` file-tree reveal (`read_file`/`edit_file`/`apply_patch`), disk-only fallback discovery (`last_conversations.json` / `rollout-*.jsonl`), model-family sprite & `CONTEXT`/`COST` telemetry, and non-destructive hook installation (`install.sh --with-agy --with-codex`).

```text
┌──────────────────────┬────────────────────────────────────────┬──────────────┐
│  [TREE] Yazi (Top)   │                                        │  MODEL LOGO  │
│  Session-linked file │                                        │ (2x2 Vector) │
│  reveal on tool ops  │             CLAUDE CODE                ├──────────────┤
├──────────────────────┤            Main Workspace              │  SUBAGENTS   │
│  [GIT] LazyGit (Bot) │     (54% Width · Native View-Switch    │ ● [main]     │
│  Live working tree & │      between [main] & [subagents])     │ ✓ [explore]  │
│  diff inspector      │                                        ├──────────────┤
│                      ├────────────────────────────────────────┤  TELEMETRY   │
│     (30% Width)      │  bypass  model  CTX  branch  HW   │ CPU/GPU/MEM  │
└──────────────────────┴────────────────────────────────────────┴──────────────┘
```

---

## Key Features

### 1. In-Process Native Subagent View Switching & Full-Session Retention
- **Permanent `● [main]` + All Session Subagents**: Unlike external wrappers that spawn separate heavy `claude` processes per pane, `ccide` hooks directly into Claude Code's internal `viewing-agent` state machine via `SIGUSR2`.
- **Sub-Millisecond Switching (`< 0.05 ms`)**: Clicking `● [main]` or any running/completed `[subagent]` in the right sidebar switches the center Claude Code pane natively in place and keeps completed subagents inspectable for the entire session.
- **Scrollable List, Keyboard Cursor & One-Key Cleanup**: Supports mouse wheel / trackpad scrolling with `● [main]` pinned at the top, full keyboard navigation (`↑`/`↓` cursor, `Enter` to switch, `x`/`Backspace`/`Delete` or left-click `×` to dismiss a single completed subagent, `c` or `/cc-ide:clear` to clear all completed).
- **Strict Per-Tab Process Tree Isolation**: Resolves each tab's session via kernel process ancestry (`proc_pidinfo(PROC_PIDTBSDINFO)` on macOS, `/proc/<pid>/stat` on Linux) so `SUBAGENTS` lists never cross-contaminate across tabs.

### 2. Selected-Session-Linked `Yazi` File Tree Sync
- **Selected-Session-Linked `Yazi` (`Read`/`LSP`/`Edit` Sync + Clean Markdown Preview) & Full-Stream `Lazygit` Diffs**:
  - When `● [main]` is selected, `Yazi` (`ccide-core yazi-hook` + `sidebar`) automatically jumps to, highlights, and previews files read (`Read`, `LSP`) or modified (`Edit`, `Write`, `MultiEdit`, `NotebookEdit`) by the main agent; clicking any `[subagent]` reveals and tracks that subagent's active files. `scripts/md-render.py` + `assets/glow-style.json` render all H1–H6 Markdown headings cleanly without raw `###` markers.
  - `Lazygit` runs inside `scripts/pty-hscroll-bridge.py` (`assets/lazygit.yml`) with **unwrapped full-line rendering by default** (configurable via `~/.config/cc-ide/viewer.json`, which preserves `vi`-style `↪` wrapping when `"wrap": true`), **zero-telemetry hash-free `[Author] [Graph] [Subject]` commit logs** (`scripts/lazygit-log.py` + `commitHashLength: 0`), **compact single-column diff line numbers** (`109│` via `scripts/lazygit-pager.sh --col-width={{columnWidth}}`), and **hover-targeted, content-bounds-clamped Apple Magic Mouse + keyboard (`H`/`L`, `Shift`/`Option`+`←`/`→`) scrolling** without unfocused selection bars (mirrored in `Yazi` via `assets/yazi-hscroll.lua`, which suppresses selection highlights in mouse mode and restores them on keyboard input, click, or tool reveal).

### 3. 3-Tier Adaptive Workspace Engine (Native Metal Splits + Full SSH & Cross-Terminal Support)
`ccide-core workspace` automatically selects the fastest available layout backend:
- **Tier 1 — Native GPU Split Backends (`iTerm2` Warm Daemon, `Ghostty 1.3+`, `WezTerm`, `Windows Terminal`)**:
  Persistent iTerm2 `AutoLaunch` daemon (`~0.07 ms` IPC), native macOS Ghostty `Ghostty.sdef` (`surface configuration` + pixel-accurate `resize_split`), WezTerm (`wezterm cli`), and Windows Terminal (`wt.exe`) deliver native `30% | 54% | 16%` splits with Retina inline model sprites (`OSC 1337` / Kitty `APC _G`).
- **Tier 2 — SSH `OSC 1337` Passthrough + Client iTerm2 Native Split**:
  When running `ccide` inside a remote SSH session viewed from local iTerm2, `ccide-core` emits an invisible `\033]1337;Custom=id=ccide:...\007` sequence over the SSH stream. Your local Mac's `ccide_daemon.py` intercepts it, reuses the existing SSH `ControlMaster` socket (`~/.ssh/ccide-cm-%C`), and splits your **local iTerm2 window natively** into `30% | 54% | 16%` running the remote `yazi`, `lazygit`, and `ccide-core sidebar`.
- **Tier 3 — Universal `tmux` Engine (`tmux -L ccide`)**:
  Works in any terminal emulator (**Ghostty `< 1.3` / Linux, Kitty, Alacritty, VS Code terminal**) or over multi-hop SSH jump hosts, automatically falling back if Tier 2 does not ACK within `450 ms`.

### 4. Zero-Subprocess Hardware Oscilloscope, Dynamic Model Sprites & Palette-Unified Syntax Highlighting
- **Resolution-Adaptive Retina RGBA & 2×2 Quadrant Model Logos**: Dynamically renders model family sprites (`GEMINI`, `DEEPSEEK`, `CLAUDE`, `OPENAI`, `GROK`), switching between your session's active Main Model (`/model`, including FleetView `/subtask` attached sessions) and your configured Subagent Model (`CLAUDE_CODE_SUBAGENT_MODEL`) when inspecting a subagent.
- **Palette-Unified Companion UI & Syntax Highlighting (`~/.config/cc-ide/palette.json`)**: Customize TUI borders, row/cursor highlights, oscilloscope curves, Powerline `statusline` segments, and `"syntax"` token overrides via JSONC (`#RRGGBB`, `#RGB`, `rgb(r,g,b)`, or ANSI 256-color) across 6 built-in presets (`ccide --theme <default|catppuccin-mocha|tokyo-night|nord|gruvbox-dark|dracula>` or `/palette <preset>`), atomically syncing `ccide.tmTheme` (`bat` / `delta` / `Yazi` `syntect_theme`), Yazi `theme.toml`, Lazygit `gui.theme`, `glow-style.json`, and `ghostty-theme.conf` with `< 120 ms` live reload.
- **Shared 500ms Hardware Telemetry & Auto-Swapping `TASKS` Progress Board**:
  - **macOS (`arm64` / `x86_64`)**: Direct C FFI against `Mach host_statistics64`, `IOKit IOAccelerator` (`AGX` GPU utilization & VRAM), and BSD `sysctl(NET_RT_IFLIST2)`.
  - **Linux (`x86_64` / `aarch64`)**: Zero-dependency `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, and `sysfs` / non-blocking `nvidia-smi`.
  - Sampled at most once per `500 ms` globally across the machine and shared across all open tabs in `< 2 µs` (`0.0%` idle CPU per tab).
  - **Live Session `TASKS ⇄ HW` Panel**: Whenever incomplete tasks exist in `~/.claude/tasks/<sessionId>/`, the Bottom-Right slot automatically switches to a scrollable `TASKS` progress board (`✓` done, `⠋` in-progress with `activeForm`, `○` pending, plus `━━━░░░ % [HW]` footer), supports instant `t`/`Tab` or `[HW]`/`[TASK]` click toggling, and automatically restores Hardware Telemetry once all tasks finish.

### 5. Version-Independent Claude Code Binary Patcher
- **Dynamic `\n---- Bun! ----\n` Trailer Parsing**: Instead of hardcoded file offsets, the patcher locates Bun's standalone trailer dynamically on both **macOS Mach-O** and **Linux ELF** binaries, parses the `StandaloneModuleGraph` table (`byte_count` / `mod_off` / `mod_len`), and clears the bytecode cache only for the JS modules it actually modified, so Bun recompiles the patched source instead of running stale cached bytecode.
- **Unminified Invariant String Anchors**: Internal functions (`enterView`, `exitView`, `adoptAgent`) and component closures are discovered through stable string literals (`"tengu_transcript_view_enter"`, `"tengu_transcript_view_exit"`, `"(resumed agent)"`, `"local_workflow"`, `".evictTerminal("`, `"PromptHintPart"`, `key:"mode"`) matched with regex — never by minified identifier name.
- **Zero-Minifier-Dependency IPC via `process.pid`**: Runtime chat/agent handles are addressed through `/tmp/.cc-ide/{req,cur,sb}_pid_<process.pid>.*` and `SIGUSR2`, so the patch keeps working no matter how upstream renames its minified variables.
- **Automatic Re-Patching on Launch**: `ccide` fingerprints the resolved `claude` binary (`inode:size:mtime`, cached in `~/.cache/cc-ide/claude_bin.stamp`) and transparently re-applies the patch the first time it runs after a Claude Code update. `python3 scripts/patch_claude_binary.py --verify` is a dry-run that emits a JSON compatibility report and exits non-zero if any required patch fails to match.

### 6. On-Demand Prebuilt Binaries (`scripts/ensure-ccide-core.sh`)
- **Marketplace & Zero-Rust Hosts**: `/plugin install cc-ide --marketplace Visio-Vanitas/ccide` and any machine without `cargo` resolve `ccide-core` automatically. The POSIX `scripts/ensure-ccide-core.sh` resolver checks `$CLAUDE_PLUGIN_ROOT/bin`, `~/.claude/bin`, and `PATH`, then downloads the matching `ccide-core-<triple>.tar.gz` from the latest GitHub Release.
- **Platform Detection**: Maps `uname -s`/`uname -m` to `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, or `aarch64-unknown-linux-gnu`; unknown platforms are skipped gracefully.
- **SHA256 Verified**: Downloads are checked against the release's `SHA256SUMS.txt` (via `shasum -a 256` or `sha256sum`) and rejected on mismatch before extraction, and macOS binaries are ad-hoc `codesign`ed after unpacking. `./install.sh` uses the same resolver as its fallback when `cargo` is absent.

---

## Requirements

- **Rust Toolchain** — **Edition 2021, MSRV `1.70`+** (`rust-version = "1.70"` in `Cargo.toml`). No nightly features and no post-1.70 compiler requirements, and no external crate dependencies, so it builds out of the box on LTS Linux distributions and macOS. CI's MSRV job runs `cargo +1.70.0 check --all-targets` and `cargo +1.70.0 test` (see [`AGENT.md` §7.1](./AGENT.md) for the `Cargo.lock` note). If you have no `cargo` at all, `scripts/ensure-ccide-core.sh` fetches a prebuilt binary instead.
- **Zsh** (`zsh`) & **Python 3** (`python3`)
- **CLI Hosts**: [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (`claude`), and optionally [Google Antigravity CLI](https://antigravity.google) (`agy`) or [OpenAI Codex CLI](https://github.com/openai/codex) (`codex`)
- **[Yazi](https://github.com/sxyazi/yazi)** (`yazi` + `ya` CLI)
- **[Lazygit](https://github.com/jesseduffield/lazygit)** (`lazygit`)
- **Terminal**:
  - **iTerm2** (with Python API enabled & `pip install iterm2` for Tier 1/2 native Metal splits), **or**
  - **Any Terminal + `tmux`** (Ghostty, WezTerm, Kitty, Alacritty, etc. for Tier 3)

---

## One-Click Installation

```bash
git clone https://github.com/Visio-Vanitas/ccide.git
cd ccide
./install.sh
```

Or register the Claude Code plugin from the GitHub marketplace (Claude Code 2.1.275+). No local clone or Rust toolchain is required: the plugin resolves `ccide-core` through `scripts/ensure-ccide-core.sh`, which downloads and SHA256-verifies the matching GitHub Release binary on first use.
```text
/plugin install cc-ide --marketplace Visio-Vanitas/ccide
```

What `./install.sh` does:
1. Builds `ccide-core` with `cargo build --release` (honoring `CCIDE_RUSTFLAGS`, default `-C target-cpu=native`). If `cargo` is not installed, it falls back to `scripts/ensure-ccide-core.{mjs,sh}` to fetch the milestone-pinned prebuilt release binary.
2. Installs vector logo assets, helper scripts (`~/.claude/scripts/`), and the `ccide` launcher (`~/.local/bin/ccide`, or `ccide.cmd` + `ccide.ps1` via `install.ps1`).
3. On macOS, installs `ccide_daemon.py` into `~/Library/Application Support/iTerm2/Scripts/AutoLaunch/`, configures `~/.ssh/ccide_config` (`ControlMaster auto`), and starts `$CCIDE_IPC_DIR/iterm_daemon.sock`.
4. Registers the unified official Claude Code plugin (`cc-ide@cc-ide`) and statusline in `~/.claude/settings.json`. When `agy` / `codex` are present (or forced via `--with-agy` / `--with-codex`, `-WithAgy` / `-WithCodex` on Windows), installs the Antigravity plugin into `~/.gemini/config/plugins/cc-ide/` (stamped `.ccide-managed`) and merges `ccide-core` hooks into `$CODEX_HOME/hooks.json` via `scripts/merge_agent_hooks.py` (preserving foreign entries with a single-time `hooks.json.ccide.bak`). `--uninstall` cleanly reverses both.
5. Applies idempotent, version-independent binary patches to the Claude Code binary (with automatic backup at `.bak`). Subsequent `ccide` launches re-patch automatically whenever Claude Code is updated (`agy` and `codex` are never binary-patched).

---

## Usage

```bash
# Launch the 3-column workspace in the current project directory (Auto-selects Tier 1 -> Tier 2 -> Tier 3)
ccide

# Launch with Google Antigravity CLI (`agy`) or OpenAI Codex CLI (`codex`)
ccide --agy
ccide --agy --conversation <id>        # Validated ^[A-Za-z0-9_-]{8,64}$; or CCIDE_AGY_CONVERSATION / CCIDE_AGY_HOME
ccide --codex
CCIDE_AGENT=agy ccide --agent codex   # CLI flag overrides CCIDE_AGENT

# Force non-destructive integration setup for agy / codex (reversible via --uninstall)
./install.sh --with-agy --with-codex
./install.sh --uninstall

# Force universal tmux 3-column mode (works in Ghostty, Kitty, WezTerm, Alacritty, or SSH)
ccide --tmux

# Connect to a remote server with SSH ControlMaster ready for native client iTerm2 splits
ccide --ssh user@remote-server

# Open without the bottom-left LazyGit pane (Yazi full height on left)
ccide --no-git

# Force a binary re-patch (normally automatic on the first `ccide` launch after a Claude Code update)
./install.sh --patch-only

# Dry-run the patcher and print a JSON compatibility report (exit 1 if a required anchor fails)
python3 -I scripts/patch_claude_binary.py --verify

# Inspect environment, backends, agents & hooks diagnostics
ccide doctor --json
```

### Multi-Agent Discovery, Antigravity Swarm & Opt-In `agy` Routing
- **Hooks + Disk Fallback**: Both `agy` (`SessionStart` / `PostToolUse` emitting `{}`) and `codex` (`SessionStart` / `PostToolUse` emitting empty stdout) report live sessions to `ccide-core yazi-hook`, while every sidebar feature also works without hooks via disk-only discovery (`~/.gemini/antigravity-cli/cache/last_conversations.json` -> `transcript.jsonl`, and `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`). `scripts/merge_agent_hooks.py` supports Python 3.9+ (including stock macOS `/usr/bin/python3`).
- **Native Antigravity Swarm Sidebar (`SWARM`)**: Automatically detects `<sys_gen>/swarm/{swarm.json,members.json,board.jsonl}` and `subagents/*.json` relative to the active `agy` transcript, displaying `SWARM #<board_head> · <run>run·<wait>wait`, the swarm title, and the selected worker's latest `board.jsonl` snippet. Worker statuses map to `running`, `waiting` (protected from dismissal), `stopped`, and `killed` (dismissible); clicking a worker switches the Top Logo, `CONTEXT` panel, and Yazi preview target without `SIGUSR2`, while `COST` aggregates across the coordinator and unique worker transcripts.
- **Opt-In `agy` Subagent Routing (`CCIDE_AGY_ORCH=1`)**: `integrations/antigravity/hooks.json` registers **no `PreToolUse` hook by default** because Antigravity's `PreToolUse` gate is fail-closed (`allow`/`deny`/`ask`/`force_ask`). Only enable `CCIDE_AGY_ORCH=1` with an `invoke_subagent` `PreToolUse` matcher if you intentionally want `pre-tool-hook` to return `{"decision":"allow","overwrite":{"Model":"..."}}` (**warning: this auto-approves matched `invoke_subagent` calls**).

Inside Claude Code, you can also invoke the built-in plugin skills:
- `/cc-ide:ccide` — Open or reset companion panes in the current tab
- `/cc-ide:rebalance` — Rebalance all open tabs to `30% | 54% | 16%` and lock zero scrollback

---

## Branch & CI/CD Promotion Flow (`dev -> preview -> main`)

Releases are promoted through a strict two-stage pull-request pipeline. Direct pushes to `main` are rejected by the `Enforce dev -> preview -> main Flow` CI guard.

| Stage | Branch | Trigger | CI Gate | Release Output |
|-------|--------|---------|---------|----------------|
| **Development** | `dev` | `push` / PR | `ci.yml`: Rust `1.70.0` MSRV check + tests, stable `rustfmt` + `clippy -D warnings`, 6-target build matrix (`aarch64`/`x86_64` × macOS/Linux/Windows MSVC), shell/Python/JSON manifest validation, patcher & multi-agent tests | none |
| **Preview RC** | `preview` | PR from `dev` merges (push) | Same gate **plus** live npm `@anthropic-ai/claude-code@latest` patch verification | Pre-release `v0.X.0-rc.<run_number>` (`0.X.0` milestones) |
| **Stable Release** | `main` | PR from `preview` merges (push) | Same gate **plus** live npm CC verification | Immutable `v0.X.0` marked *latest* (`0.x.y` tags only) |

- **CI Gate (`.github/workflows/ci.yml`)**: Runs on every push and PR to `dev`, `preview`, and `main`. The MSRV job builds and tests with a real `1.70.0` toolchain, then runs formatting and Clippy against stable. The `scripts-and-patcher` job validates `ensure-ccide-core.{sh,mjs}`, `install.{sh,ps1}`, `scripts/ccide*`, and `pane-runner.sh`, byte-compiles the Python scripts, checks 5-way version parity, runs all `tests/` unit tests, and — on `preview`/`main` — verifies `@anthropic-ai/claude-code@latest`.
- **Release Pipeline (`.github/workflows/release.yml`)**: Triggered on push to `preview` or `main`. Builds `ccide-core` across all 6 targets on `0.X.0` minor milestones, packages `ccide-core-<triple>.tar.gz`, generates `SHA256SUMS.txt`, and publishes the GitHub Release (on `0.x.y` patches, skips binary compilation and creates tag `v0.x.y` on `main`).
- **Daily Upstream Compatibility Probe (`.github/workflows/cc-compat-watch.yml`)**: Runs at `03:17 UTC` every day (plus manual `workflow_dispatch`). It installs the current `@anthropic-ai/claude-code@latest` from npm, runs the patcher in `--verify` mode, and opens a deduplicated `[Compat Alert]` GitHub issue if the upstream bundle layout changes enough to break a required patch.

---

## Roadmap

- **v0.11.3 (Current)**: Lazygit CJK horizontal scrolling & bold-border focus sync, `[0]-Log` 4-column CJK author alignment, and `SUBAGENTS`/`SWARM` `Tab`/`Shift+Tab` cursor navigation.
- **v0.11.2**: Hardened Antigravity `transcript.jsonl` parsing, Swarm worker hook isolation & coordinator promotion, turn-accurate multi-model `COST` + `gemini-4-*`/`gpt-oss-120b` pricing, `is_safe_id` / internal-path filtering, and `(M,m,p)` bootstrap / CI release fallbacks.
- **v0.11.1**: Native Antigravity Swarm observability (`SWARM` header, board activity row, click-to-inspect worker Logo/`CONTEXT`/Yazi, multi-model Swarm `COST` aggregation, `--conversation` / `CCIDE_AGY_HOME`), plus Python 3.9+ compatibility in `scripts/merge_agent_hooks.py`.
- **v0.11.0**: Multi-agent workspace, sidebar, telemetry & hook integration across **Claude Code**, **Google Antigravity CLI (`agy`)**, and **OpenAI Codex CLI (`codex`)**.
- **v1.0**: First-class PowerShell (`pwsh`) Windows Terminal reopen-based rebalance, WezTerm packaging & marketplace distribution, and passing `LayoutPlan` column widths to the iTerm2 daemon (`30% | 54% | 16%`).

---

## Architecture Documentation

See [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md), [`docs/ARCH_v0.11_multi_agent.md`](./docs/ARCH_v0.11_multi_agent.md), [`docs/ARCH_v0.11.1_agy_swarm.md`](./docs/ARCH_v0.11.1_agy_swarm.md), [`docs/ARCH_v0.11.2_hardening.md`](./docs/ARCH_v0.11.2_hardening.md), and [`AGENT.md`](./AGENT.md) for the complete module breakdown, multi-agent & Swarm contracts, FFI/IPC specifications, and structural binary-patching strategy.

---

## License

Released under the [MIT License](./LICENSE).
