# `ccide` (`cc-ide`)

**Native 3-Column Tiling Terminal IDE & Real-Time Subagent / Hardware Telemetry Companion for Claude Code (`macOS` & `Linux`)**

`ccide` transforms **Claude Code** into a zero-polling, sub-millisecond 3-column tiling terminal IDE (`30% Left | 54% Center | 16% Right`) powered by a zero-dependency Rust engine (`ccide-core`, **v0.4.0**) and a 3-tier adaptive workspace controller.

> **v0.4.0 highlights:** Rust **Edition 2021** with an LTS **MSRV of `1.70`** (no nightly features, no post-1.70 compiler requirements), a **version-independent** Claude Code binary patcher that survives upstream releases, on-demand prebuilt binary downloads for hosts without `cargo`, and a `dev -> preview -> main` CI/CD promotion pipeline.

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

### 4. Zero-Subprocess Hardware Oscilloscope, Dynamic Model Sprites & Custom Palettes
- **Resolution-Adaptive 2×2 Quadrant Model Logos**: Dynamically renders vector logos (`GEMINI`, `DEEPSEEK`, `CLAUDE`, `OPENAI`, `GROK`), switching between your session's active Main Model (`/model`) and your configured Subagent Model (`CLAUDE_CODE_SUBAGENT_MODEL`) when inspecting a subagent.
- **Independent Color Palette (`~/.config/cc-ide/palette.json`)**: Customize every TUI border, row/cursor highlight, oscilloscope curve, model accent, optional monochrome logo tint (`"tint_logo": true`), and Powerline `statusline` segment via JSONC (`#RRGGBB`, `#RGB`, `rgb(r,g,b)`, or ANSI 256-color) with live hot-reload and 6 built-in presets (`ccide --theme <default|catppuccin-mocha|tokyo-night|nord|gruvbox-dark|dracula>`).
- **Shared 500ms Hardware Telemetry Snapshot (`/tmp/.cc-ide/hw_telemetry_v1.bin`)**:
  - **macOS (`arm64` / `x86_64`)**: Direct C FFI against `Mach host_statistics64`, `IOKit IOAccelerator` (`AGX` GPU utilization & VRAM), and BSD `sysctl(NET_RT_IFLIST2)`.
  - **Linux (`x86_64` / `aarch64`)**: Zero-dependency `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, and `sysfs` / non-blocking `nvidia-smi`.
  - Sampled at most once per `500 ms` globally across the machine and shared across all open tabs in `< 2 µs` (`0.0%` idle CPU per tab).

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

Or register the Claude Code plugin from the GitHub marketplace (Claude Code 2.1.275+). No local clone or Rust toolchain is required: the plugin resolves `ccide-core` through `scripts/ensure-ccide-core.sh`, which downloads and SHA256-verifies the matching GitHub Release binary on first use.
```text
/plugin install cc-ide --marketplace Visio-Vanitas/ccide
```

What `./install.sh` does:
1. Builds `ccide-core` with `cargo build --release` (honoring `CCIDE_RUSTFLAGS`, default `-C target-cpu=native`). If `cargo` is not installed, it falls back to `scripts/ensure-ccide-core.sh` to fetch the prebuilt release binary.
2. Installs vector logo assets, helper scripts (`~/.claude/scripts/`), and the `ccide` launcher (`~/.local/bin/ccide`).
3. On macOS, installs `ccide_daemon.py` into `~/Library/Application Support/iTerm2/Scripts/AutoLaunch/`, configures `~/.ssh/ccide_config` (`ControlMaster auto`), and starts `/tmp/.cc-ide/iterm_daemon.sock`.
4. Registers the unified official Claude Code plugin (`cc-ide@cc-ide`) and statusline in `~/.claude/settings.json`.
5. Applies idempotent, version-independent binary patches to the Claude Code binary (with automatic backup at `.bak`). Subsequent `ccide` launches re-patch automatically whenever Claude Code is updated.

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

# Force a binary re-patch (normally automatic on the first `ccide` launch after a Claude Code update)
./install.sh --patch-only

# Dry-run the patcher and print a JSON compatibility report (exit 1 if a required anchor fails)
python3 scripts/patch_claude_binary.py --verify

# Uninstall binary patches and restore original claude.exe.bak
./install.sh --uninstall
```

Inside Claude Code, you can also invoke the built-in plugin skills:
- `/cc-ide:ccide` — Open or reset companion panes in the current tab
- `/cc-ide:rebalance` — Rebalance all open tabs to `30% | 54% | 16%` and lock zero scrollback

---

## Branch & CI/CD Promotion Flow (`dev -> preview -> main`)

Releases are promoted through a strict two-stage pull-request pipeline. Direct pushes to `main` are rejected by the `Enforce dev -> preview -> main Flow` CI guard.

| Stage | Branch | Trigger | CI Gate | Release Output |
|-------|--------|---------|---------|----------------|
| **Development** | `dev` | `push` / PR | `ci.yml`: Rust `1.70.0` MSRV check + tests, stable `rustfmt` + `clippy -D warnings`, 4-target build matrix (`aarch64`/`x86_64` × macOS/Linux), shell/Python/JSON manifest validation, patcher mutation tests | none |
| **Preview RC** | `preview` | PR from `dev` merges (push) | Same gate **plus** live npm `@anthropic-ai/claude-code@latest` patch verification | Pre-release `v0.4.0-rc.<run_number>` marked *not latest* |
| **Stable Release** | `main` | PR from `preview` merges (push) | Same gate **plus** live npm CC verification | Immutable `v0.4.0` marked *latest* |

- **CI Gate (`.github/workflows/ci.yml`)**: Runs on every push and PR to `dev`, `preview`, and `main`. The MSRV job builds and tests with a real `1.70.0` toolchain, then runs formatting and Clippy against stable. The `scripts-and-patcher` job validates `ensure-ccide-core.sh`, `install.sh`, `scripts/ccide`, and `pane-runner.sh`, byte-compiles the Python scripts, parses all plugin/hook/asset JSON manifests, runs the patcher mutation/structural unit tests, and — on `preview`/`main` — downloads the latest published Claude Code and runs `patch_claude_binary.py --verify` against it.
- **Release Pipeline (`.github/workflows/release.yml`)**: Triggered on push to `preview` or `main`. Builds `ccide-core` for all four targets, packages `ccide-core-<triple>.tar.gz`, generates `SHA256SUMS.txt`, and publishes the GitHub Release. On `preview` the tag is a moving pre-release (`v0.4.0-rc.<run>`); on `main` it is the immutable stable `v0.4.0` and is marked `make_latest`.
- **Daily Upstream Compatibility Probe (`.github/workflows/cc-compat-watch.yml`)**: Runs at `03:17 UTC` every day (plus manual `workflow_dispatch`). It installs the current `@anthropic-ai/claude-code@latest` from npm, runs the patcher in `--verify` mode, and opens a deduplicated `[Compat Alert]` GitHub issue if the upstream bundle layout changes enough to break a required patch.

---

## Roadmap

- **v1.0 — First-class PowerShell (`pwsh` / Windows Terminal / WezTerm) support**: native Windows packaging and a PowerShell pane runner, extending the adaptive workspace beyond the current macOS + Linux matrix while keeping the same `30% | 54% | 16%` layout contract.

---

## Architecture Documentation

See [`AGENT.md`](./AGENT.md) for the complete module breakdown, FFI contracts, IPC file specifications (`/tmp/.cc-ide/*`), and the structural binary-patching strategy.

---

## License

Released under the [MIT License](./LICENSE).
