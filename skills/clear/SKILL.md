---
name: clear
description: Dismiss all completed subagents from the Right Sidebar SUBAGENTS list across active sessions while preserving running subagents and .jsonl transcripts.
---

# `/cc-ide:clear` — Clear Completed Subagents

Run the native `ccide-core` command to immediately dismiss all completed (`✓ done`) subagents from the Right Sidebar (`SUBAGENTS` panel) and wake all open sidebars via `SIGUSR2`:

```bash
~/.claude/bin/ccide-core workspace clear-subagents
```

Interactive sidebar shortcuts (no command needed):
- **Arrow keys `↑` / `↓` (`j` / `k`)** move the `❯` cursor with automatic viewport scrolling; **`Enter` / `Space` / `→`** switches view to the selected item
- **Press `x`, `d`, `Backspace`, `Delete`, or `←`** (or **left-click `×`** at the right edge of a completed subagent row) to dismiss only that completed item
- **Press `c`** or **left-click the bottom `[c] clear N` bar** to dismiss all completed subagents at once
- **Mouse wheel / Trackpad scroll** (or `PgUp` / `PgDn`, `g` / `G`) to scroll through long `SUBAGENTS` lists while keeping `● [main]` pinned at top
