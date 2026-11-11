---
name: theme
description: Inspect, switch, or customize the independent cc-ide color palette (~/.config/cc-ide/palette.json) across all open sidebars and Powerline statuslines with live hot-reload.
---

# `/cc-ide:theme` — Independent Color Palette & Live Theme Switcher

`cc-ide` loads all TUI borders, `SUBAGENTS` row/cursor highlights, hardware oscilloscope curves, model accents, and Powerline `statusline` segments from an independent JSONC configuration file:

- **Default config path**: `~/.config/cc-ide/palette.json` (override via `$CCIDE_PALETTE_FILE` or `$XDG_CONFIG_HOME/cc-ide/palette.json`)
- **Live hot-reload**: Saving `~/.config/cc-ide/palette.json` automatically repaints all open sidebars within `< 1s` (or `< 1ms` when switched via `ccide --theme`).

## Commands

```bash
# List active palette file, current preset, and all built-in preset swatches
~/.claude/bin/ccide-core workspace theme list

# Switch to a built-in preset while preserving any custom slot overrides
# Presets: default | catppuccin-mocha | tokyo-night | nord | gruvbox-dark | dracula
~/.claude/bin/ccide-core workspace theme catppuccin-mocha

# Reset ~/.config/cc-ide/palette.json back to the commented default template
~/.claude/bin/ccide-core workspace theme reset
```

## Customizing Individual Colors (`~/.config/cc-ide/palette.json`)

Supports `//` and `/* ... */` comments plus `#RRGGBB`, `#RGB`, `rgb(r,g,b)`, or ANSI 256-color (`0..255`) values across `ui`, `highlights`, `models`, `tint_logo`, and `statusline` sections.
