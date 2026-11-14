---
name: theme
description: Inspect, switch, or customize the independent cc-ide color palette (~/.config/cc-ide/palette.json) across all open sidebars and Powerline statuslines with live hot-reload.
---

# `/cc-ide:theme` — Independent Color Palette, Syntax Highlighting & Live Theme Switcher

`cc-ide` loads TUI borders, `SUBAGENTS`/`ORCHESTRATOR` highlights, hardware oscilloscope curves, model accents, Powerline `statusline` segments, **and companion syntax highlighting** (Yazi code previews, Lazygit `delta`/fallback diffs, Markdown `glow`/fallback fenced code blocks, and optional Ghostty theme include) from an independent JSONC configuration file:

- **Default config path**: `~/.config/cc-ide/palette.json` (override via `$CCIDE_PALETTE_FILE` or `$XDG_CONFIG_HOME/cc-ide/palette.json`)
- **Generated syntax artifacts**: `~/.config/cc-ide/{ccide.tmTheme, glow-style.json, syntax_resolved.json, ghostty-theme.conf}`
- **Live hot-reload**: Saving `~/.config/cc-ide/palette.json` automatically repaints all open sidebars and regenerates companion syntax themes within `< 1s` (or `< 1ms` when switched via `ccide --theme`).

## Commands

```bash
# List active palette file, current preset, and all built-in preset swatches
~/.claude/bin/ccide-core workspace theme list

# Switch to a built-in preset while preserving any custom slot overrides
# Presets: default | catppuccin-mocha | tokyo-night | nord | gruvbox-dark | dracula
~/.claude/bin/ccide-core workspace theme catppuccin-mocha

# Force-regenerate companion syntax themes (.tmTheme, glow-style.json, syntax_resolved.json)
~/.claude/bin/ccide-core workspace theme sync

# Reset ~/.config/cc-ide/palette.json back to the commented default template
~/.claude/bin/ccide-core workspace theme reset
```

## Customizing Individual Colors (`~/.config/cc-ide/palette.json`)

Supports `//` and `/* ... */` comments plus `#RRGGBB`, `#RGB`, `rgb(r,g,b)`, or ANSI 256-color (`0..255`) values across `ui`, `highlights`, `models`, `tint_logo`, `statusline`, and `syntax` (`syn_keyword`, `syn_function`, `syn_type`, `syn_string`, `syn_number`, `syn_comment`, `syn_operator`, `syn_variable`, `syn_code_bg`, `syn_diff_add_fg`, `syn_diff_add_bg`, `syn_diff_del_fg`, `syn_diff_del_bg`) sections.
