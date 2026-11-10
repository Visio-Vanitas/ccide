---
name: rebalance
description: Rebalance all open ccide tabs to deterministic 30% | 54% | 16% column widths across local iTerm2 (warm daemon), SSH OSC 1337 client splits, and tmux, refresh process-isolation bindings, and lock TUI side-panes to zero scrollback.
---

# `/cc-ide:rebalance`

Normalize every open `ccide` tab to the `30% | 54% | 16%` layout, re-bind each tab's shell PID for process isolation, and lock the Yazi/Lazygit/Companion panes to zero scrollback.

## Preferred command (Tier 1 warm daemon, no Python cold start)

```bash
ccide-core workspace rebalance
```

- If `/tmp/.cc-ide/iterm_daemon.sock` is alive, this is a single Unix-socket RPC.
- If the socket is dead, it starts `ccide_daemon.py` and falls back to `iterm2_workspace.py rebalance`.
- Inside `tmux` it only re-enables `mouse on`; tmux keeps its own geometry (percentages apply at pane creation).

## Full rebalance + sidebar hot-reload

```bash
ccide-core workspace rebalance && pkill -USR2 -f "ccide-core sidebar" || true
```

Under Tier 2 (SSH), the change lands on the client Mac's native split; the remote side does not need `rebalance` because its panes are client-local. If a tab cannot be reached, re-open the tab with `ccide` (it will auto-fall back to `tmux -L ccide`).
