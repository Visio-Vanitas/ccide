#!/usr/bin/env zsh
# Cross-platform (`macOS` + `Linux`) companion TUI pane runner for `cc-ide`.
NVM_NODE_BIN=""
for d in "$HOME"/.nvm/versions/node/*/bin(N); do
  NVM_NODE_BIN="$d"
done
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$HOME/.cargo/bin:$HOME/.local/bin:$HOME/.claude/bin${NVM_NODE_BIN:+:$NVM_NODE_BIN}:$PATH"
export LANG="en_US.UTF-8"
export LC_ALL="en_US.UTF-8"
export COLORTERM="truecolor"
export EDITOR="nvim"

ROLE="${1:-yazi}"
TARGET_DIR="${2:-$PWD}"
CLIENT_ID="${3:-1000}"
KEEP_PANES="${4:-0}"
INSTANCE_ID="${5:-}"
shift_count=$(( $# >= 5 ? 5 : $# ))
shift "$shift_count" || true

mkdir -p /tmp/.cc-ide "$HOME/.cache/cc-ide" 2>/dev/null || true
# Acknowledge client-side native split over SSH (`OSC 1337` Tier 2 handshake)
if [[ -n "$CLIENT_ID" ]]; then
  printf 'ok' > "/tmp/.cc-ide/osc_ack_${CLIENT_ID}" 2>/dev/null || true
fi

[[ -d "$TARGET_DIR" ]] && cd "$TARGET_DIR"

clear_terminal_buffers() {
  printf '\033[?1049l\033[2J\033[3J\033[H\033]1337;ClearScrollback\007\033[?1049h\033[2J\033[3J\033[H\033]1337;ClearScrollback\007'
}

SCRIPT_DIR="${0:A:h}"
REPO_ROOT="${SCRIPT_DIR:h}"
YAZI_BIN="$(command -v yazi || true)"
YA_BIN="$(command -v ya || true)"
LAZYGIT_BIN="$(command -v lazygit || true)"
MACMON_BIN="$(command -v macmon || command -v btop || command -v htop || true)"
CCIDE_CORE_BIN="$HOME/.claude/bin/ccide-core"
[[ -x "$CCIDE_CORE_BIN" ]] || CCIDE_CORE_BIN="$(command -v ccide-core || echo "$REPO_ROOT/bin/ccide-core")"

case "$ROLE" in
  yazi)
    clear_terminal_buffers
    printf '\033]1;[TREE] yazi\007'
    if [[ -z "$YAZI_BIN" || ! -x "$YAZI_BIN" ]]; then
      echo "yazi not found in PATH. Install yazi (https://github.com/sxyazi/yazi) to enable the file tree pane."
      exec "${SHELL:-/bin/sh}"
    fi
    {
      [[ -n "$YA_BIN" && -x "$YA_BIN" ]] && "$YA_BIN" cache --clear >/dev/null 2>&1 || true
      find "$HOME/.cache/cc-ide" -maxdepth 1 -type f -mmin +720 -delete 2>/dev/null || true
    } </dev/null >/dev/null 2>&1 &!
    while true; do
      clear_terminal_buffers
      "$YAZI_BIN" --client-id "$CLIENT_ID" "$TARGET_DIR" || true
      sleep 0.2
    done
    ;;
  lazygit)
    clear_terminal_buffers
    printf '\033]1;[GIT] lazygit\007'
    if [[ -z "$LAZYGIT_BIN" || ! -x "$LAZYGIT_BIN" ]]; then
      echo "lazygit not found in PATH. Install lazygit (https://github.com/jesseduffield/lazygit) to enable the Git pane."
      exec "${SHELL:-/bin/sh}"
    fi
    while true; do
      clear_terminal_buffers
      "$LAZYGIT_BIN" -p "$TARGET_DIR" || true
      sleep 0.2
    done
    ;;
  macmon)
    clear_terminal_buffers
    printf '\033]1;[SYS] monitor\007'
    if [[ -n "$MACMON_BIN" && -x "$MACMON_BIN" ]]; then
      exec "$MACMON_BIN"
    else
      exec "${SHELL:-/bin/sh}"
    fi
    ;;
  clawd|sidebar)
    clear_terminal_buffers
    printf '\033]1;[AI] Companion & Agents\007'
    while true; do
      clear_terminal_buffers
      "$CCIDE_CORE_BIN" sidebar "$TARGET_DIR" "$CLIENT_ID" "$KEEP_PANES" || true
      sleep 0.25
    done
    ;;
  claude)
    export CC_YAZI_ID="$CLIENT_ID"
    export CC_INSTANCE_ID="$INSTANCE_ID"
    claude --dangerously-skip-permissions "$@" || true
    if [[ "$KEEP_PANES" == "0" && -x "$HOME/.local/bin/cc-close" && -n "$INSTANCE_ID" ]]; then
      "$HOME/.local/bin/cc-close" "$INSTANCE_ID" >/dev/null 2>&1 || true
    fi
    exec "${SHELL:-/bin/zsh}" -l
    ;;
esac
