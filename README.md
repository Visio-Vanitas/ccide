# `ccide` (`cc-ide`)

**Native 3-Column Tiling Terminal IDE & Real-Time Subagent / Hardware Telemetry Companion for Claude Code (`macOS` & `Linux`)**

`ccide` transforms **Claude Code** into a zero-polling, sub-millisecond 3-column tiling terminal IDE (`30% Left | 54% Center | 16% Right`) powered by a zero-dependency Rust engine (`ccide-core`) and a 3-tier adaptive workspace controller.

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
- **Permanent `● [main]` + All Session Subagents**: Unlike external wrappers that spawn separate heavy `claude` processes per pane, `ccide` hooks directly into Claude Code's internal `viewing-agent` state machine (`YU` / `EL`) via `SIGUSR2`.
- **Sub-Millisecond Switching (`< 0.05 ms`)**: Clicking `● [main]` or any running/completed `[subagent]` in the right sidebar switches the center Claude Code pane natively in place and keeps completed subagents inspectable for the entire session.
- **Strict Per-Tab Process Tree Isolation**: Resolves each tab's session via kernel process ancestry (`proc_pidinfo(PROC_PIDTBSDINFO)` on macOS, `/proc/<pid>/stat` on Linux) so `SUBAGENTS` lists never cross-contaminate across tabs.

### 2. Selected-Session-Linked `Yazi` File Tree Sync
- Driven by `ccide-core yazi-hook` (`PostToolUse` / `SubagentStart` / `SubagentStop`):
  - When `● [main]` is selected, `Yazi` automatically follows files read or edited by the main agent.
  - Clicking any `[subagent]` immediately reveals that subagent's active file in `Yazi` and follows its subsequent file operations.

### 3. 3-Tier Adaptive Workspace Engine (Native Metal Splits + Full SSH & Cross-Terminal Support)
`ccide-core workspace` automatically selects the fastest available layout backend:
- **Tier 1 — Local iTerm2 Warm Daemon (`/tmp/.cc-ide/iterm_daemon.sock`, `~0.07 ms` IPC)**:
  Persistent `AutoLaunch` daemon keeps a warm `iterm2.App` connection for instant native Metal splits (`30% | 54% | 16%`), zero-scrollback TUI pane locking, and `< 2 ms` focus restoration without Python startup overhead.
- **Tier 2 — SSH `OSC 1337` Passthrough + Client iTerm2 Native Split**:
  When running `ccide` inside a remote SSH session viewed from local iTerm2, `ccide-core` emits an invisible `\033]1337;Custom=id=ccide:...\007` sequence over the SSH stream. Your local Mac's `ccide_daemon.py` intercepts it, reuses the existing SSH `ControlMaster` socket (`~/.ssh/ccide-cm-%C`), and splits your **local iTerm2 window natively** into `30% | 54% | 16%` running the remote `yazi`, `lazygit`, and `ccide-core sidebar`.
- **Tier 3 — Universal `tmux` Engine (`tmux -L ccide`)**:
  Works in any terminal emulator (**Ghostty, WezTerm, Kitty, Alacritty, VS Code terminal**) or over multi-hop SSH jump hosts, automatically falling back if Tier 2 does not ACK within `450 ms`.

### 4. Zero-Subprocess Hardware Oscilloscope & Dynamic Model Sprites
- **Resolution-Adaptive 2×2 Quadrant Model Logos**: Dynamically renders vector logos (`GEMINI`, `DEEPSEEK`, `CLAUDE`, `OPENAI`, `GROK`), switching between your session's active Main Model (`/model`) and your configured Subagent Model (`CLAUDE_CODE_SUBAGENT_MODEL`) when inspecting a subagent.
- **Shared 500ms Hardware Telemetry Snapshot (`/tmp/.cc-ide/hw_telemetry_v1.bin`)**:
  - **macOS (`arm64` / `x86_64`)**: Direct C FFI against `Mach host_statistics64`, `IOKit IOAccelerator` (`AGX` GPU utilization & VRAM), and BSD `sysctl(NET_RT_IFLIST2)`.
  - **Linux (`x86_64` / `aarch64`)**: Zero-dependency `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, and `sysfs` / non-blocking `nvidia-smi`.
  - Sampled at most once per `500 ms` globally across the machine and shared across all open tabs in `< 2 µs` (`0.0%` idle CPU per tab).

---

## Requirements

- **Rust Toolchain** (`cargo` 1.91+ / Rust 2024 edition)
- **Zsh** (`zsh`) & **Python 3** (`python3`)
- **[Claude Code](https://docs.anthropic.com/en/docs/claude-code)** CLI
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

Or register the Claude Code plugin from the GitHub marketplace (Claude Code 2.1.275+, run `./install.sh` once to build the native `ccide-core` binary):
```text
/plugin install cc-ide --marketplace Visio-Vanitas/ccide
```

What `./install.sh` does:
1. Compiles `ccide-core` with `RUSTFLAGS="-C target-cpu=native" cargo build --release`.
2. Installs vector logo assets, helper scripts (`~/.claude/scripts/`), and the `ccide` launcher (`~/.local/bin/ccide`).
3. On macOS, installs `ccide_daemon.py` into `~/Library/Application Support/iTerm2/Scripts/AutoLaunch/`, configures `~/.ssh/ccide_config` (`ControlMaster auto`), and starts `/tmp/.cc-ide/iterm_daemon.sock`.
4. Registers the unified official Claude Code plugin (`cc-ide@cc-ide`) and statusline in `~/.claude/settings.json`.
5. Applies idempotent binary patches to `claude.exe` (with automatic backup at `claude.exe.bak`).

---

## Usage

```bash
# Launch the 3-column workspace in the current project directory (Auto-selects Tier 1 -> Tier 2 -> Tier 3)
ccide

# Force universal tmux 3-column mode (works in Ghostty, Kitty, WezTerm, Alacritty, or SSH)
ccide --tmux

# Connect to a remote server with SSH ControlMaster ready for native client iTerm2 splits
ccide --ssh user@remote-server

# Open without the bottom-left LazyGit pane (Yazi full height on left)
ccide --no-git

# Re-apply only the claude.exe binary patch after upgrading Claude Code
./install.sh --patch-only

# Uninstall binary patches and restore original claude.exe.bak
./install.sh --uninstall
```

Inside Claude Code, you can also invoke the built-in plugin skills:
- `/cc-ide:ccide` — Open or reset companion panes in the current tab
- `/cc-ide:rebalance` — Rebalance all open tabs to `30% | 54% | 16%` and lock zero scrollback

---

## Architecture Documentation

See [`AGENT.md`](./AGENT.md) for the complete module breakdown, FFI contracts, IPC file specifications (`/tmp/.cc-ide/*`), and binary patch offsets.

---

## License

Released under the [MIT License](./LICENSE).
