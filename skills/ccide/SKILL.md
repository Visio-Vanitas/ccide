---
name: ccide
description: Open, rebalance, or reinstall the 3-column Claude Code tiling workspace (Left 30% Yazi/Lazygit, Center 54% Claude Code, Right 16% AI Companion & Hardware Telemetry) across its 3 adaptive backends (local iTerm2 daemon, SSH OSC 1337 native split, universal tmux).
---

# `/cc-ide:ccide` — 3-Column Adaptive Tiling Workspace Controller

`cc-ide` **v0.4.0** runs a zero-dependency Rust engine (`ccide-core`, **Rust Edition 2021, MSRV `1.70`**) and is installed through a `dev -> preview -> main` release pipeline. No `cargo` is required on the host: `scripts/ensure-ccide-core.sh` downloads and SHA256-verifies the matching GitHub Release binary on demand.

Manage the `cc-ide` workspace in the current tab or across all open windows/tabs. The launcher picks a backend automatically:

| Tier | When | Mechanism |
|------|------|-----------|
| **1** Local iTerm2 | macOS iTerm2, no SSH/tmux | Warm daemon over `/tmp/.cc-ide/iterm_daemon.sock` (`< 5ms` native Metal split, `< 2ms` subagent focus) |
| **2** SSH + client iTerm2 | Remote session inside iTerm2 | `OSC 1337;Custom=id=ccide:...` passthrough + SSH `ControlMaster` (`~/.ssh/ccide_config`) + `/tmp/.cc-ide/osc_ack_<yazi_id>` handshake (`< 450ms`) |
| **3** Universal `tmux` | `$TMUX`, Ghostty/Kitty/WezTerm/VS Code, or Tier 2 timeout | `tmux -L ccide` 3-column tiling engine |

## Commands

1. **Open / Reset the 3-column companion panes in the current tab**:
   ```bash
   ccide
   ```
   Useful flags: `--iterm` (force Tier 1), `--osc` (force Tier 2), `--tmux` (force Tier 3), `--ssh <host>` (ControlMaster SSH), `--no-git`, `--mon`, `--no-right`, `--width <pct>` (left column, default 30).

2. **Rebalance all open tabs to `30% | 54% | 16%` and lock TUI panes to zero scrollback**:
   ```bash
   ccide-core workspace rebalance
   ```

3. **Fast focus / subagent switch** (used by the right sidebar; no Python cold start):
   ```bash
   ccide-core workspace switch-subagent <yazi_id> <session_id> <view_id> [footer_rank]
   ccide-core workspace focus <query>
   ccide-core workspace close [<pane_spec>] [<yazi_id>]
   ```

4. **Rebuild, re-patch the Claude Code binary, and hot-reload all sidebars (from the cloned `ccide` repository)**:
   ```bash
   ./install.sh
   ```
   The Claude Code binary patch is version-independent (dynamic Bun trailer parsing + invariant string anchors) and is re-applied automatically by `ccide` on the first launch after any Claude Code update — `./install.sh --patch-only` is only needed for a forced re-patch.

5. **Fetch / refresh the prebuilt engine without `cargo`** (used automatically by marketplace installs and `install.sh` fallback):
   ```bash
   scripts/ensure-ccide-core.sh --help
   ```
   Resolves `$CLAUDE_PLUGIN_ROOT/bin`, `~/.claude/bin`, then `PATH`; otherwise downloads `ccide-core-<triple>.tar.gz` from the latest GitHub Release and verifies it against `SHA256SUMS.txt`.

## Release channels

- `dev` — development; CI gate only (`rustc 1.70.0` MSRV, fmt/clippy, 4-target matrix, patcher tests).
- `preview` — merges via PR from `dev`; publishes a **pre-release** (`v0.4.0-rc.<run_number>`) after live npm `@anthropic-ai/claude-code@latest` patch verification.
- `main` — merges via PR from `preview` only (enforced by CI); publishes the **immutable stable** `v0.4.0` release.
- A daily `cc-compat-watch.yml` probe (`03:17 UTC`) tests the patcher against the newest published Claude Code and opens a deduplicated `[Compat Alert]` issue if it breaks.

## Health checks

```bash
# Tier 1 daemon liveness (expect "pong")
python3 -c 'import socket;s=socket.socket(socket.AF_UNIX);s.connect("/tmp/.cc-ide/iterm_daemon.sock");s.sendall(b"{\"op\":\"ping\"}\n");print(s.recv(16))'

# AutoLaunch symlink + SSH ControlMaster config
ls -l "$HOME/Library/Application Support/iTerm2/Scripts/AutoLaunch/ccide_daemon.py"
grep -q ccide_config ~/.ssh/config && echo "ssh include ok"
```

If Tier 1 or 2 misbehaves, `ccide --tmux` (or the automatic fallback) always yields a working 3-column layout.
