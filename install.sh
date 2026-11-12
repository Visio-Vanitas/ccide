#!/usr/bin/env zsh
# ==============================================================================
# cc-ide One-Click Installer, Adaptive Daemon Configurator & Binary Patcher
# ==============================================================================
# Usage:
#   ./install.sh               # Full build, plugin registration, daemon setup, binary patch & hot-reload
#   ./install.sh --patch-only  # Only re-apply claude.exe binary patches (e.g. after npm update)
#   ./install.sh --uninstall   # Restore claude.exe.bak and stop ccide_daemon.py
# ==============================================================================
set -euo pipefail

PROJECT_ROOT="${0:A:h}"
PLUGIN_VERSION="0.5.0"
HOME_DIR="$HOME"
CLAUDE_DIR="$HOME_DIR/.claude"
CACHE_DIR="$HOME_DIR/.cache/cc-ide"
PLUGIN_CACHE_DIR="$CLAUDE_DIR/plugins/cache/cc-ide/cc-ide/$PLUGIN_VERSION"
OS_NAME="$(uname -s)"

NVM_NODE_BIN=""
for d in "$HOME_DIR"/.nvm/versions/node/*/bin(N); do
  NVM_NODE_BIN="$d"
done
export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME_DIR/.cargo/bin:$HOME_DIR/.local/bin:$CLAUDE_DIR/bin${NVM_NODE_BIN:+:$NVM_NODE_BIN}:$PATH"

PYTHON_BIN="$HOME_DIR/miniconda3/bin/python3"
[[ -x "$PYTHON_BIN" ]] || PYTHON_BIN="$(command -v python3 || echo /usr/bin/python3)"

CARGO_BIN="$(command -v cargo || echo "$HOME_DIR/.cargo/bin/cargo")"
[[ -x "$CARGO_BIN" ]] || CARGO_BIN="$HOME_DIR/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo"

discover_claude_exe() {
  local c_cli
  c_cli="$(command -v claude || true)"
  if [[ -n "$c_cli" ]]; then
    local real_c
    real_c="$(realpath "$c_cli" 2>/dev/null || readlink -f "$c_cli" 2>/dev/null || echo "$c_cli")"
    if [[ -f "$(dirname "$real_c")/bin/claude.exe" ]]; then
      echo "$(dirname "$real_c")/bin/claude.exe"
      return
    elif [[ -f "$real_c" ]]; then
      echo "$real_c"
      return
    fi
  fi
  for cand in "$HOME_DIR"/.nvm/versions/node/*/lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe(N); do
    echo "$cand"
  done | tail -n 1
}

MODE="${1:-install}"

log_step() {
  printf "\033[1;36m==>\033[0m \033[1m%s\033[0m\n" "$1"
}

log_ok() {
  printf "  \033[1;32m[OK]\033[0m %s\n" "$1"
}

if [[ "$MODE" == "--uninstall" ]]; then
  log_step "Uninstalling cc-ide patches and restoring claude.exe.bak..."
  CLAUDE_BIN="$(discover_claude_exe)"
  if [[ -n "$CLAUDE_BIN" && -f "${CLAUDE_BIN}.bak" ]]; then
    cp -f "${CLAUDE_BIN}.bak" "$CLAUDE_BIN"
    [[ "$OS_NAME" == "Darwin" ]] && codesign -f -s - "$CLAUDE_BIN"
    log_ok "Restored original claude.exe from backup"
  fi
  pkill -f "ccide_daemon.py" >/dev/null 2>&1 || true
  rm -f "/tmp/.cc-ide/iterm_daemon.sock" "/tmp/.cc-ide/iterm_daemon.pid"
  exit 0
fi

if [[ "$MODE" == "--patch-only" ]]; then
  log_step "Applying event-driven SIGUSR2 & UI retention patches to claude.exe..."
  "$PYTHON_BIN" -I "$PROJECT_ROOT/scripts/patch_claude_binary.py"
  exit 0
fi

# ------------------------------------------------------------------------------
# 1. Build or Fetch Native Rust Engine (`ccide-core`)
# ------------------------------------------------------------------------------
log_step "1/7 Building native Rust engine (ccide-core) for $OS_NAME..."
mkdir -p "$PROJECT_ROOT/bin" "$CLAUDE_DIR/bin" "/tmp/.cc-ide"
if [[ -x "$CARGO_BIN" ]]; then
  (
    cd "$PROJECT_ROOT"
    RUSTFLAGS="${CCIDE_RUSTFLAGS:--C target-cpu=native}" "$CARGO_BIN" build --release --locked --quiet
  )
  cp -f "$PROJECT_ROOT/target/release/ccide-core" "$PROJECT_ROOT/bin/ccide-core"
  [[ "$OS_NAME" == "Darwin" ]] && codesign -f -s - "$PROJECT_ROOT/bin/ccide-core" >/dev/null 2>&1 || true
  ln -sf "$PROJECT_ROOT/bin/ccide-core" "$CLAUDE_DIR/bin/ccide-core"
  log_ok "Compiled $PROJECT_ROOT/bin/ccide-core ($(du -h "$PROJECT_ROOT/bin/ccide-core" | awk '{print $1}'))"
else
  log_step "cargo not found locally; fetching prebuilt ccide-core from GitHub Releases..."
  chmod +x "$PROJECT_ROOT/scripts/ensure-ccide-core.sh"
  sh "$PROJECT_ROOT/scripts/ensure-ccide-core.sh" --help >/dev/null 2>&1 || true
  if [[ -x "$CLAUDE_DIR/bin/ccide-core" ]]; then
    cp -f "$CLAUDE_DIR/bin/ccide-core" "$PROJECT_ROOT/bin/ccide-core"
    log_ok "Downloaded prebuilt ccide-core -> $CLAUDE_DIR/bin/ccide-core"
  else
    echo "Error: cargo not found and no prebuilt release binary available yet." >&2
    exit 1
  fi
fi

# ------------------------------------------------------------------------------
# 2. Install Assets, Scripts, CLI Launchers & Yazi Configuration
# ------------------------------------------------------------------------------
log_step "2/7 Installing vector logo assets, scripts & CLI launchers..."
mkdir -p "$CACHE_DIR/logos" "$CACHE_DIR/status" "$CLAUDE_DIR/scripts" "$HOME_DIR/.local/bin" "$HOME_DIR/.config/yazi" "$HOME_DIR/.config/cc-ide"
cp -f "$PROJECT_ROOT/assets/adaptive_quadrant_logos.json" "$CACHE_DIR/logos/adaptive_quadrant_logos.json"
cp -f "$PROJECT_ROOT/assets/lazygit.yml" "$HOME_DIR/.config/cc-ide/lazygit.yml"
cp -f "$PROJECT_ROOT/assets/glow-style.json" "$HOME_DIR/.config/cc-ide/glow-style.json"
if [[ ! -f "$HOME_DIR/.config/cc-ide/palette.json" ]]; then
  cp -f "$PROJECT_ROOT/assets/palette.json" "$HOME_DIR/.config/cc-ide/palette.json"
fi
if [[ ! -f "$HOME_DIR/.config/cc-ide/viewer.json" ]]; then
  cp -f "$PROJECT_ROOT/assets/viewer.json" "$HOME_DIR/.config/cc-ide/viewer.json"
fi

for script_name in ccide_daemon.py iterm2_workspace.py pane-runner.sh lazygit-pager.sh lazygit-log.py pty-hscroll-bridge.py md-render.py patch_claude_binary.py cc_tui_lib.py ccide ensure-ccide-core.sh; do
  chmod +x "$PROJECT_ROOT/scripts/$script_name"
  ln -sf "$PROJECT_ROOT/scripts/$script_name" "$CLAUDE_DIR/scripts/$script_name"
done
ln -sf "$PROJECT_ROOT/scripts/ccide" "$HOME_DIR/.local/bin/cc-ide"
ln -sf "$PROJECT_ROOT/scripts/ccide" "$HOME_DIR/.local/bin/ccide"
ln -sf "$PROJECT_ROOT/scripts/lazygit-log.py" "$HOME_DIR/.local/bin/lazygit-log.py"

YAZI_TOML="$HOME_DIR/.config/yazi/yazi.toml"
if [[ ! -f "$YAZI_TOML" ]] || ! grep -q "mouse_events" "$YAZI_TOML"; then
  cat <<'EOF' > "$YAZI_TOML"
[mgr]
mouse_events = ["click", "scroll", "touch", "move", "drag"]
scrolloff = 4
EOF
fi

# Install Yazi horizontal-panning plugin (`hscroll.yazi` — Magic Mouse swipe + Shift/Alt+Left/Right)
HSCROLL_PLUGIN_DIR="$HOME_DIR/.config/yazi/plugins/hscroll.yazi"
mkdir -p "$HSCROLL_PLUGIN_DIR"
cp -f "$PROJECT_ROOT/assets/yazi-hscroll.lua" "$HSCROLL_PLUGIN_DIR/main.lua"

# Configure Yazi (`yazi.toml`, `init.lua`, `keymap.toml`) and Lazygit user config upgrades cleanly
"$PYTHON_BIN" -I - "$YAZI_TOML" "$HOME_DIR" <<'PYEOF'
import os, re, sys

yazi_toml, home_dir = sys.argv[1], sys.argv[2]
yazi_dir = os.path.join(home_dir, ".config", "yazi")

with open(yazi_toml, "r", encoding="utf-8") as f:
    src = f.read()

# Strip any previous cc-ide managed previewers or legacy piper glow/md-render blocks
clean_yazi = re.sub(
    r"\n?# Managed by cc-ide:[^\n]*\n(?:.*\n)*?(?=\Z|\n\[(?!\[plugin\.prepend_previewers\]\]))",
    "",
    src,
)
clean_yazi = re.sub(
    r"\n*\[\[plugin\.prepend_previewers\]\]\nurl\s*=\s*\"\*\.(?:md|markdown|MD)\"\nrun\s*=\s*'piper --[^'\n]*(?:glow|md-render\.py)[^'\n]*'\n?",
    "",
    clean_yazi,
).rstrip()

# Ensure `mouse_events` includes `"touch"` so Magic Mouse horizontal swipes reach `hscroll.yazi`
if re.search(r"(?m)^mouse_events\s*=\s*\[([^\]]*)\]", clean_yazi):
    def _ensure_touch(m):
        inner = m.group(1)
        if '"touch"' in inner or "'touch'" in inner:
            return m.group(0)
        items = [x.strip() for x in inner.split(",") if x.strip()]
        items.append('"touch"')
        return f"mouse_events   = [{', '.join(items)}]"
    clean_yazi = re.sub(r"(?m)^mouse_events\s*=\s*\[([^\]]*)\]", _ensure_touch, clean_yazi, count=1)
elif "[mgr]" in clean_yazi:
    clean_yazi = clean_yazi.replace(
        "[mgr]",
        '[mgr]\nmouse_events   = ["click", "scroll", "touch", "move", "drag"]',
        1,
    )

previewer_block = (
    "\n\n# Managed by cc-ide: 120Hz cached & bounds-clamped horizontal previewers\n"
    "[[plugin.prepend_previewers]]\n"
    'url = "*.{md,markdown,MD}"\n'
    'run = "hscroll -- md"\n\n'
    "[[plugin.prepend_previewers]]\n"
    'mime = "application/{json,ndjson}"\n'
    'run = "hscroll -- json"\n\n'
    "[[plugin.prepend_previewers]]\n"
    'mime = "text/*"\n'
    'run = "hscroll -- code"\n\n'
    "[[plugin.prepend_previewers]]\n"
    'mime = "application/{mbox,javascript,wine-extension-ini}"\n'
    'run = "hscroll -- code"\n'
)

updated = clean_yazi + previewer_block
if updated != src:
    with open(yazi_toml, "w", encoding="utf-8") as f:
        f.write(updated)

# Wire `require("hscroll"):setup()` into `~/.config/yazi/init.lua` without clobbering existing plugins
init_lua = os.path.join(yazi_dir, "init.lua")
init_src = ""
if os.path.isfile(init_lua):
    with open(init_lua, "r", encoding="utf-8") as f:
        init_src = f.read()
if 'require("hscroll")' not in init_src and "require('hscroll')" not in init_src:
    sep = "\n" if init_src and not init_src.endswith("\n") else ""
    with open(init_lua, "w", encoding="utf-8") as f:
        f.write(init_src + sep + 'require("hscroll"):setup()\n')

# Wire keyboard navigation highlight activation & horizontal scroll bindings in `keymap.toml`
keymap_toml = os.path.join(yazi_dir, "keymap.toml")
km_src = ""
if os.path.isfile(keymap_toml):
    with open(keymap_toml, "r", encoding="utf-8") as f:
        km_src = f.read()

# Strip any previous cc-ide managed block before writing the current canonical block
km_clean = re.sub(
    r"\n?# Managed by cc-ide:[^\n]*\n(?:.*\n)*?(?=\Z|\n# (?!Managed by cc-ide))",
    "",
    km_src,
).rstrip()

block = (
    "\n# Managed by cc-ide: ergonomic keyboard highlight activation & bounded horizontal panning\n"
    "[[mgr.prepend_keymap]]\n"
    'on   = "<Up>"\n'
    'run  = "plugin hscroll -- key arrow -1"\n'
    'desc = "Move cursor up (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "k"\n'
    'run  = "plugin hscroll -- key arrow -1"\n'
    'desc = "Move cursor up (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "<Down>"\n'
    'run  = "plugin hscroll -- key arrow 1"\n'
    'desc = "Move cursor down (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "j"\n'
    'run  = "plugin hscroll -- key arrow 1"\n'
    'desc = "Move cursor down (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "<Left>"\n'
    'run  = "plugin hscroll -- key leave"\n'
    'desc = "Go to parent directory (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "h"\n'
    'run  = "plugin hscroll -- key leave"\n'
    'desc = "Go to parent directory (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "<Right>"\n'
    'run  = "plugin hscroll -- key enter"\n'
    'desc = "Enter child directory (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "l"\n'
    'run  = "plugin hscroll -- key enter"\n'
    'desc = "Enter child directory (activate highlight)"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "<S-Left>"\n'
    'run  = "plugin hscroll -- -4"\n'
    'desc = "Pan file tree & preview left"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "<S-Right>"\n'
    'run  = "plugin hscroll -- 4"\n'
    'desc = "Pan file tree & preview right"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "<A-Left>"\n'
    'run  = "plugin hscroll -- -4"\n'
    'desc = "Pan file tree & preview left"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = "<A-Right>"\n'
    'run  = "plugin hscroll -- 4"\n'
    'desc = "Pan file tree & preview right"\n\n'
    "[[mgr.prepend_keymap]]\n"
    'on   = ["z", "0"]\n'
    'run  = "plugin hscroll -- reset"\n'
    'desc = "Reset horizontal pan offset"\n'
)
km_out = (km_clean + "\n" if km_clean else "") + block
if km_out != km_src:
    with open(keymap_toml, "w", encoding="utf-8") as f:
        f.write(km_out)

# Upgrade existing user Lazygit config in place if present so open/restarted instances inherit
# `--col-width={{columnWidth}}`, vertical full-width main split, 2-column branchLogCmd, and zero status telemetry.
for lg_cfg in (
    os.path.join(home_dir, "Library", "Application Support", "lazygit", "config.yml"),
    os.path.join(home_dir, ".config", "lazygit", "config.yml"),
):
    if os.path.isfile(lg_cfg):
        with open(lg_cfg, "r", encoding="utf-8") as f:
            lg_src = f.read()
        lg_new = re.sub(
            r"(pager:\s*[^\n]*lazygit-pager\.sh)(?:\s+--(?:col-)?width=\{\{columnWidth\}\})?",
            r"\1 --col-width={{columnWidth}}",
            lg_src,
        )
        lg_new = re.sub(r"mainPanelSplitMode:\s*flexible", "mainPanelSplitMode: vertical", lg_new)
        lg_new = re.sub(r"splitDiff:\s*auto", "splitDiff: never", lg_new)
        lg_new = re.sub(r"expandFocusedSidePanel:\s*true", "expandFocusedSidePanel: false", lg_new)
        lg_new = re.sub(
            r"branchLogCmd:\s*[^\n]*lazygit-log\.py[^\n]*",
            "branchLogCmd: lazygit-log.py --branch={{branchName}}",
            lg_new,
        )
        lg_new = re.sub(
            r"-\s*[^\n]*(?:lazygit-status\.sh|lazygit-log\.py[^\n]*)",
            "- lazygit-log.py --all",
            lg_new,
        )
        if "scrollPastBottom:" not in lg_new and "gui:" in lg_new:
            lg_new = lg_new.replace("gui:\n", "gui:\n  scrollPastBottom: false\n", 1)
        if "wrapLinesInStagingView:" not in lg_new and "gui:" in lg_new:
            lg_new = lg_new.replace("gui:\n", "gui:\n  wrapLinesInStagingView: false\n", 1)
        if "commitHashLength:" not in lg_new and "gui:" in lg_new:
            lg_new = lg_new.replace("gui:\n", "gui:\n  commitHashLength: 0\n", 1)
        if "showGraph:" not in lg_new and "git:\n" in lg_new:
            lg_new = lg_new.replace("git:\n", "git:\n  log:\n    order: topo-order\n    showGraph: always\n", 1)
        elif "showGraph:" in lg_new:
            lg_new = re.sub(r"showGraph:\s*\S+", "showGraph: always", lg_new)
        if lg_new != lg_src:
            with open(lg_cfg, "w", encoding="utf-8") as f:
                f.write(lg_new)

# Strip legacy hardware telemetry from `~/.claude/scripts/lazygit-status.sh` if present
lg_status = os.path.join(home_dir, ".claude", "scripts", "lazygit-status.sh")
if os.path.isfile(lg_status) and not os.path.islink(lg_status):
    with open(lg_status, "r", encoding="utf-8") as f:
        st_src = f.read()
    if "M4 PRO TELEMETRY" in st_src:
        with open(lg_status, "w", encoding="utf-8") as f:
            f.write(
                "#!/usr/bin/env sh\n"
                'exec "$HOME/.claude/scripts/lazygit-log.py" --all "$@"\n'
            )
PYEOF
if command -v ya >/dev/null 2>&1; then
  ya cache --clear >/dev/null 2>&1 || true
fi
log_ok "Synced assets & linked scripts into ~/.claude/scripts/ and ~/.local/bin/ccide"

# ------------------------------------------------------------------------------
# 3. Configure iTerm2 AutoLaunch Daemon & SSH ControlMaster Bridge (macOS)
# ------------------------------------------------------------------------------
log_step "3/7 Configuring iTerm2 AutoLaunch Daemon & SSH ControlMaster multiplexing..."
if [[ "$OS_NAME" == "Darwin" ]]; then
  AUTOLAUNCH_DIR="$HOME_DIR/Library/Application Support/iTerm2/Scripts/AutoLaunch"
  mkdir -p "$AUTOLAUNCH_DIR"
  ln -sf "$PROJECT_ROOT/scripts/ccide_daemon.py" "$AUTOLAUNCH_DIR/ccide_daemon.py"

  # Ensure SSH ControlMaster is configured so remote SSH `ccide` can split local iTerm2 panes in < 20ms
  mkdir -p "$HOME_DIR/.ssh"
  chmod 700 "$HOME_DIR/.ssh"
  cat <<'EOF' > "$HOME_DIR/.ssh/ccide_config"
# Managed by cc-ide: enables zero-latency SSH multiplexing & LC_TERMINAL forwarding for native iTerm2 splits
Host *
    ControlMaster auto
    ControlPath ~/.ssh/ccide-cm-%C
    ControlPersist 10m
    SendEnv LC_TERMINAL
EOF
  chmod 600 "$HOME_DIR/.ssh/ccide_config"
  if [[ ! -f "$HOME_DIR/.ssh/config" ]]; then
    printf "Include ~/.ssh/ccide_config\n" > "$HOME_DIR/.ssh/config"
    chmod 600 "$HOME_DIR/.ssh/config"
  elif ! grep -q "ccide_config" "$HOME_DIR/.ssh/config"; then
    tmp_ssh="$(mktemp)"
    printf "Include ~/.ssh/ccide_config\n\n" > "$tmp_ssh"
    cat "$HOME_DIR/.ssh/config" >> "$tmp_ssh"
    mv "$tmp_ssh" "$HOME_DIR/.ssh/config"
    chmod 600 "$HOME_DIR/.ssh/config"
  fi

  # Restart warm iTerm2 daemon (`ccide_daemon.py`) in background
  pkill -f "ccide_daemon.py" >/dev/null 2>&1 || true
  rm -f "/tmp/.cc-ide/iterm_daemon.sock"
  nohup "$PYTHON_BIN" "$PROJECT_ROOT/scripts/ccide_daemon.py" >/dev/null 2>&1 &
  for _ in {1..20}; do
    [[ -S "/tmp/.cc-ide/iterm_daemon.sock" ]] && break
    sleep 0.05
  done
  log_ok "Started ccide_daemon.py (/tmp/.cc-ide/iterm_daemon.sock + OSC 1337 SSH listener)"
else
  log_ok "Linux host detected: OSC 1337 emitter & tmux backend ready"
fi

# ------------------------------------------------------------------------------
# 4. Package & Register Official Claude Code Plugin (`cc-ide@cc-ide`)
# ------------------------------------------------------------------------------
log_step "4/7 Registering unified official Claude Code plugin (cc-ide@cc-ide v$PLUGIN_VERSION)..."
rm -rf "$PLUGIN_CACHE_DIR"
mkdir -p "$PLUGIN_CACHE_DIR/.claude-plugin" \
         "$PLUGIN_CACHE_DIR/hooks" \
         "$PLUGIN_CACHE_DIR/skills" \
         "$PLUGIN_CACHE_DIR/scripts" \
         "$PLUGIN_CACHE_DIR/assets" \
         "$PLUGIN_CACHE_DIR/bin"

cp -f "$PROJECT_ROOT/.claude-plugin/plugin.json" "$PLUGIN_CACHE_DIR/.claude-plugin/plugin.json"
cp -f "$PROJECT_ROOT/.claude-plugin/marketplace.json" "$PLUGIN_CACHE_DIR/.claude-plugin/marketplace.json"
cp -f "$PROJECT_ROOT/hooks/hooks.json" "$PLUGIN_CACHE_DIR/hooks/hooks.json"
cp -rf "$PROJECT_ROOT/skills/"* "$PLUGIN_CACHE_DIR/skills/"
rm -rf "$PROJECT_ROOT/scripts/__pycache__"
cp -rf "$PROJECT_ROOT/scripts/"* "$PLUGIN_CACHE_DIR/scripts/"
cp -rf "$PROJECT_ROOT/assets/"* "$PLUGIN_CACHE_DIR/assets/"
cp -f "$PROJECT_ROOT/bin/ccide-core" "$PLUGIN_CACHE_DIR/bin/ccide-core"
[[ "$OS_NAME" == "Darwin" ]] && codesign -f -s - "$PLUGIN_CACHE_DIR/bin/ccide-core" >/dev/null 2>&1 || true
cp -f "$PROJECT_ROOT/AGENT.md" "$PLUGIN_CACHE_DIR/AGENT.md"

# Keep any prior cached plugin version directories (`0.4.0`, etc.) in sync so already-running
# Claude Code sessions whose `${CLAUDE_PLUGIN_ROOT}` still references an older version immediately
# execute the updated `ccide-core yazi-hook` and scripts without requiring a session restart.
for prev_cache in "$CLAUDE_DIR/plugins/cache/cc-ide/cc-ide"/*; do
  if [[ -d "$prev_cache" && "$prev_cache" != "$PLUGIN_CACHE_DIR" ]]; then
    mkdir -p "$prev_cache/bin" "$prev_cache/hooks" "$prev_cache/scripts" "$prev_cache/assets"
    cp -f "$PROJECT_ROOT/hooks/hooks.json" "$prev_cache/hooks/hooks.json"
    cp -rf "$PROJECT_ROOT/scripts/"* "$prev_cache/scripts/"
    cp -rf "$PROJECT_ROOT/assets/"* "$prev_cache/assets/"
    cp -f "$PROJECT_ROOT/bin/ccide-core" "$prev_cache/bin/ccide-core"
    [[ "$OS_NAME" == "Darwin" ]] && codesign -f -s - "$prev_cache/bin/ccide-core" >/dev/null 2>&1 || true
  fi
done

"$PYTHON_BIN" -I - "$PROJECT_ROOT" "$PLUGIN_CACHE_DIR" "$PLUGIN_VERSION" <<'PYEOF'
import datetime, json, os, sys

project_root, plugin_cache_dir, version = sys.argv[1], sys.argv[2], sys.argv[3]
claude_dir = os.path.expanduser("~/.claude")
plugins_dir = os.path.join(claude_dir, "plugins")
os.makedirs(plugins_dir, exist_ok=True)
now_iso = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")

km_path = os.path.join(plugins_dir, "known_marketplaces.json")
km = {}
if os.path.exists(km_path):
    try:
        with open(km_path, "r", encoding="utf-8") as f:
            km = json.load(f)
    except Exception:
        km = {}
km["cc-ide"] = {
    "source": {"source": "directory", "path": project_root},
    "installLocation": project_root,
    "lastUpdated": now_iso,
}
with open(km_path, "w", encoding="utf-8") as f:
    json.dump(km, f, indent=2, ensure_ascii=False)

ip_path = os.path.join(plugins_dir, "installed_plugins.json")
ip = {"version": 2, "plugins": {}}
if os.path.exists(ip_path):
    try:
        with open(ip_path, "r", encoding="utf-8") as f:
            ip = json.load(f)
    except Exception:
        pass
ip.setdefault("plugins", {})["cc-ide@cc-ide"] = [
    {
        "scope": "user",
        "installPath": plugin_cache_dir,
        "version": version,
        "installedAt": now_iso,
        "lastUpdated": now_iso,
    }
]
with open(ip_path, "w", encoding="utf-8") as f:
    json.dump(ip, f, indent=2, ensure_ascii=False)

st_path = os.path.join(claude_dir, "settings.json")
st = {}
if os.path.exists(st_path):
    with open(st_path, "r", encoding="utf-8") as f:
        st = json.load(f)

st.setdefault("enabledPlugins", {})["cc-ide@cc-ide"] = True
st.setdefault("extraKnownMarketplaces", {})["cc-ide"] = {
    "source": {"source": "directory", "path": project_root}
}
st["statusLine"] = {
    "type": "command",
    "command": f"{os.path.join(claude_dir, 'bin', 'ccide-core')} statusline",
}
if "hooks" in st and isinstance(st["hooks"], dict):
    for hk in ("PostToolUse", "SubagentStart", "SubagentStop"):
        if hk in st["hooks"]:
            filtered = [
                entry for entry in st["hooks"][hk]
                if "ccide-core" not in json.dumps(entry) and "yazi-reveal-hook" not in json.dumps(entry)
            ]
            if filtered:
                st["hooks"][hk] = filtered
            else:
                del st["hooks"][hk]
    if not st["hooks"]:
        del st["hooks"]

with open(st_path, "w", encoding="utf-8") as f:
    json.dump(st, f, indent=2, ensure_ascii=False)
PYEOF
log_ok "Registered cc-ide@cc-ide v$PLUGIN_VERSION in ~/.claude/plugins & ~/.claude/settings.json"

# ------------------------------------------------------------------------------
# 5. Patch `claude.exe` Binary (Subagent Retention + SIGUSR2 Hook + Clean Bottom)
# ------------------------------------------------------------------------------
log_step "5/7 Applying native claude.exe patches..."
CLAUDE_BIN_FOUND="$(discover_claude_exe)"
if [[ -n "$CLAUDE_BIN_FOUND" ]]; then
  "$PYTHON_BIN" -I "$PROJECT_ROOT/scripts/patch_claude_binary.py" || true
  STAMP_VAL="$(stat -f '%i:%z:%m' "$CLAUDE_BIN_FOUND" 2>/dev/null || stat -c '%i:%s:%Y' "$CLAUDE_BIN_FOUND" 2>/dev/null || echo "")"
  [[ -n "$STAMP_VAL" ]] && printf '%s' "$STAMP_VAL" > "$CACHE_DIR/claude_bin.stamp"
fi

# ------------------------------------------------------------------------------
# 6. Validate Official Plugin Manifest
# ------------------------------------------------------------------------------
log_step "6/7 Validating official plugin structure..."
CLAUDE_CLI="$(command -v claude || true)"
if [[ -n "$CLAUDE_CLI" && -x "$CLAUDE_CLI" ]]; then
  "$CLAUDE_CLI" plugin validate "$PROJECT_ROOT" || true
fi

# ------------------------------------------------------------------------------
# 7. Rebalance Open Tabs & Hot-Reload Active Sidebars
# ------------------------------------------------------------------------------
log_step "7/7 Rebalancing open tabs & hot-reloading sidebars..."
"$PROJECT_ROOT/bin/ccide-core" workspace rebalance >/dev/null 2>&1 || true
pkill -f "ccide-core sidebar" >/dev/null 2>&1 || true
pkill -f "cc-right-sidebar.py" >/dev/null 2>&1 || true
log_ok "cc-ide v$PLUGIN_VERSION installation & hot-reload complete!"
