---
name: rebalance
description: Rebalance all open ccide tabs to deterministic 30% | 54% | 16% column widths across local iTerm2 (warm daemon), SSH OSC 1337 client splits, and tmux, refresh process-isolation bindings, and lock TUI side-panes to zero scrollback.
---

# `/cc-ide:rebalance`

Normalize every open `ccide` tab to the `30% | 54% | 16%` layout, re-bind each tab's shell PID for process isolation, and lock the Yazi/Lazygit/Companion panes to zero scrollback.

## Preferred command (cross-OS native resolver, no Python cold start)

```bash
node "${CLAUDE_PLUGIN_ROOT}/scripts/ensure-ccide-core.mjs" workspace rebalance
```

- On macOS iTerm2, if `/tmp/.cc-ide/iterm_daemon.sock` is alive, this is a single Unix-socket RPC; otherwise `ccide-core` starts `ccide_daemon.py` automatically.
- On macOS Ghostty `>= 1.3`, it equalizes and resizes native `Ghostty.sdef` splits to `30% | 54% | 16%`; inside `tmux` it ensures `mouse on` (tmux keeps creation-time pane geometry).

## Full rebalance + sidebar hot-reload

```bash
node "${CLAUDE_PLUGIN_ROOT}/scripts/ensure-ccide-core.mjs" workspace rebalance
```

Under Tier 2 (SSH), the change lands on the client Mac's native split; the remote side does not need `rebalance` because its panes are client-local. If a tab cannot be reached, re-open the tab with `ccide` (it will auto-fall back to `tmux -L ccide`).
