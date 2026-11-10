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
PLUGIN_VERSION="0.3.0"
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
# 1. Build Native Rust Engine (`ccide-core`)
# ------------------------------------------------------------------------------
log_step "1/7 Compiling native Rust engine (ccide-core) for $OS_NAME..."
if [[ ! -x "$CARGO_BIN" ]]; then
  echo "Error: cargo not found in PATH or ~/.rustup" >&2
  exit 1
fi
(
  cd "$PROJECT_ROOT"
  RUSTFLAGS="-C target-cpu=native" "$CARGO_BIN" build --release --quiet
)
mkdir -p "$PROJECT_ROOT/bin" "$CLAUDE_DIR/bin" "/tmp/.cc-ide"
cp -f "$PROJECT_ROOT/target/release/ccide-core" "$PROJECT_ROOT/bin/ccide-core"
[[ "$OS_NAME" == "Darwin" ]] && codesign -f -s - "$PROJECT_ROOT/bin/ccide-core" >/dev/null 2>&1 || true
ln -sf "$PROJECT_ROOT/bin/ccide-core" "$CLAUDE_DIR/bin/ccide-core"
log_ok "Built $PROJECT_ROOT/bin/ccide-core ($(du -h "$PROJECT_ROOT/bin/ccide-core" | awk '{print $1}'))"

# ------------------------------------------------------------------------------
# 2. Install Assets, Scripts, CLI Launchers & Yazi Configuration
# ------------------------------------------------------------------------------
log_step "2/7 Installing vector logo assets, scripts & CLI launchers..."
mkdir -p "$CACHE_DIR/logos" "$CACHE_DIR/status" "$CLAUDE_DIR/scripts" "$HOME_DIR/.local/bin" "$HOME_DIR/.config/yazi"
cp -f "$PROJECT_ROOT/assets/adaptive_quadrant_logos.json" "$CACHE_DIR/logos/adaptive_quadrant_logos.json"

for script_name in ccide_daemon.py iterm2_workspace.py pane-runner.sh patch_claude_binary.py cc_tui_lib.py ccide; do
  chmod +x "$PROJECT_ROOT/scripts/$script_name"
  ln -sf "$PROJECT_ROOT/scripts/$script_name" "$CLAUDE_DIR/scripts/$script_name"
done
ln -sf "$PROJECT_ROOT/scripts/ccide" "$HOME_DIR/.local/bin/cc-ide"
ln -sf "$PROJECT_ROOT/scripts/ccide" "$HOME_DIR/.local/bin/ccide"

if [[ ! -f "$HOME_DIR/.config/yazi/yazi.toml" ]] || ! grep -q "mouse_events" "$HOME_DIR/.config/yazi/yazi.toml"; then
  cat <<'EOF' > "$HOME_DIR/.config/yazi/yazi.toml"
[mgr]
mouse_events = ["click", "scroll", "touch", "move", "drag"]
scrolloff = 4
EOF
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
