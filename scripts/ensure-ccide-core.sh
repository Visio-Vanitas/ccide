#!/bin/sh
# POSIX-compliant on-demand resolver & prebuilt GitHub Release downloader for `ccide-core`.
# Ensures Marketplace (`/plugin install cc-ide --marketplace Visio-Vanitas/ccide`) and
# zero-Rust hosts automatically fetch and SHA256-verify the matching native binary.
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd || true)"
DEFAULT_ROOT=""
[ -n "$SCRIPT_DIR" ] && DEFAULT_ROOT="$(dirname "$SCRIPT_DIR")"
PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$DEFAULT_ROOT}"
CLAUDE_BIN_DIR="${HOME}/.claude/bin"
TARGET_BIN="${CLAUDE_BIN_DIR}/ccide-core"
VER_CACHE="${CLAUDE_BIN_DIR}/.ccide-core.version"
REPO_SLUG="${CCIDE_GITHUB_REPO:-Visio-Vanitas/ccide}"

PLUGIN_VER=""
if [ -n "$PLUGIN_ROOT" ] && [ -f "${PLUGIN_ROOT}/.claude-plugin/plugin.json" ]; then
  PLUGIN_VER="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*[^"]*\)".*/\1/p' "${PLUGIN_ROOT}/.claude-plugin/plugin.json" | head -n 1 || true)"
fi
CUSTOM_TAG="${CCIDE_RELEASE_TAG:-}"
VER_SRC="${PLUGIN_VER:-${CUSTOM_TAG#v}}"
PLUG_MAJ=""
PLUG_MIN=""
case "$VER_SRC" in
  [0-9]*.[0-9]*)
    PLUG_MAJ="$(printf '%s' "$VER_SRC" | cut -d. -f1 | tr -cd '0-9')"
    PLUG_MIN="$(printf '%s' "$VER_SRC" | cut -d. -f2 | tr -cd '0-9')"
    ;;
esac
TAG="$CUSTOM_TAG"
if [ -z "$TAG" ] && [ -n "$PLUG_MAJ" ] && [ -n "$PLUG_MIN" ]; then
  TAG="v${PLUG_MAJ}.${PLUG_MIN}.0"
fi
UA="ccide-bootstrap/${PLUG_MAJ:-0}.${PLUG_MIN:-10}"

EXISTING_BIN=""
if [ -x "$TARGET_BIN" ]; then
  EXISTING_BIN="$TARGET_BIN"
elif [ -n "$PLUGIN_ROOT" ] && [ -x "${PLUGIN_ROOT}/bin/ccide-core" ]; then
  EXISTING_BIN="${PLUGIN_ROOT}/bin/ccide-core"
elif command -v ccide-core >/dev/null 2>&1; then
  EXISTING_BIN="$(command -v ccide-core)"
fi

stat_key() {
  stat -c '%s:%Y000' "$1" 2>/dev/null || stat -f '%z:%m000' "$1" 2>/dev/null || echo ""
}

bin_is_ok() {
  _b="$1"
  [ -n "$PLUG_MAJ" ] && [ -n "$PLUG_MIN" ] || return 0
  _sk="$(stat_key "$_b")"
  _ver=""
  if [ -n "$_sk" ] && [ -f "$VER_CACHE" ]; then
    _cached="$(cat "$VER_CACHE" 2>/dev/null | head -n 1 || true)"
    case "$_cached" in
      "${_sk}:"*) _ver="${_cached#"${_sk}:"}" ;;
    esac
  fi
  if [ -z "$_ver" ]; then
    _out="$("$_b" --version 2>/dev/null | head -n 1 || true)"
    _ver="$(printf '%s' "$_out" | sed -n 's/.*ccide-core[[:space:]]\{1,\}v\{0,1\}\([0-9][0-9]*\.[0-9][0-9]*\(\.[0-9][0-9]*\)*\).*/\1/p' | head -n 1 || true)"
    if [ -n "$_sk" ] && [ -n "$_ver" ]; then
      mkdir -p "$CLAUDE_BIN_DIR" 2>/dev/null || true
      printf '%s:%s\n' "$_sk" "$_ver" > "${VER_CACHE}.tmp.$$" 2>/dev/null && mv -f "${VER_CACHE}.tmp.$$" "$VER_CACHE" 2>/dev/null || rm -f "${VER_CACHE}.tmp.$$" 2>/dev/null || true
    fi
  fi
  [ -n "$_ver" ] || return 1
  _bmaj="$(printf '%s' "$_ver" | cut -d. -f1 | tr -cd '0-9')"
  _bmin="$(printf '%s' "$_ver" | cut -d. -f2 | tr -cd '0-9')"
  [ -n "$_bmaj" ] && [ -n "$_bmin" ] || return 1
  if [ "$_bmaj" -lt "$PLUG_MAJ" ] 2>/dev/null; then return 1; fi
  if [ "$_bmaj" -eq "$PLUG_MAJ" ] && [ "$_bmin" -lt "$PLUG_MIN" ] 2>/dev/null; then return 1; fi
  return 0
}

if [ -n "$EXISTING_BIN" ] && bin_is_ok "$EXISTING_BIN"; then
  exec "$EXISTING_BIN" "$@"
fi

OS_S="$(uname -s 2>/dev/null || echo unknown)"
ARCH_M="$(uname -m 2>/dev/null || echo unknown)"
TRIPLE=""
case "${OS_S}:${ARCH_M}" in
  Darwin:arm64|Darwin:aarch64) TRIPLE="aarch64-apple-darwin" ;;
  Darwin:x86_64)               TRIPLE="x86_64-apple-darwin" ;;
  Linux:x86_64|Linux:amd64)    TRIPLE="x86_64-unknown-linux-gnu" ;;
  Linux:aarch64|Linux:arm64)   TRIPLE="aarch64-unknown-linux-gnu" ;;
esac

LOCK_DIR="${CLAUDE_BIN_DIR}/.ensure-ccide-core.lock"
LOCK_HELD=0
TMP_DIR=""
STAGED_BIN=""
cleanup() {
  rm -rf "${TMP_DIR:-}" "${STAGED_BIN:-}"
  TMP_DIR=""
  STAGED_BIN=""
  if [ "${LOCK_HELD:-0}" -eq 1 ]; then
    rm -rf "$LOCK_DIR" 2>/dev/null || true
    LOCK_HELD=0
  fi
}
trap cleanup EXIT INT TERM

fetch_url() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -A "$UA" --connect-timeout 5 --max-time 25 "$1" -o "$2" 2>/dev/null
  elif command -v wget >/dev/null 2>&1; then
    wget -q -U "$UA" -T 20 -O "$2" "$1" 2>/dev/null
  else
    return 1
  fi
}

try_download() {
  [ -n "$TRIPLE" ] && [ -n "$TAG" ] || return 1
  mkdir -p "$CLAUDE_BIN_DIR"
  _tries=0
  while [ "$_tries" -lt 300 ]; do
    if mkdir "$LOCK_DIR" 2>/dev/null; then
      LOCK_HELD=1
      break
    fi
    if [ -n "$(find "$LOCK_DIR" -maxdepth 0 -mmin +1 2>/dev/null || true)" ]; then
      rm -rf "$LOCK_DIR" 2>/dev/null || true
      continue
    fi
    _tries=$((_tries + 1))
    sleep 0.1 2>/dev/null || sleep 1
  done
  [ "$LOCK_HELD" -eq 1 ] || return 1

  if [ -x "$TARGET_BIN" ] && bin_is_ok "$TARGET_BIN"; then
    return 0
  fi

  ARCHIVE="ccide-core-${TRIPLE}.tar.gz"
  BASE_URL="https://github.com/${REPO_SLUG}/releases/download/${TAG}"
  TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t ccide)"

  fetch_url "${BASE_URL}/${ARCHIVE}" "${TMP_DIR}/${ARCHIVE}" || return 1
  fetch_url "${BASE_URL}/SHA256SUMS.txt" "${TMP_DIR}/SHA256SUMS.txt" || return 1

  EXPECTED="$(grep " ${ARCHIVE}\$" "${TMP_DIR}/SHA256SUMS.txt" | awk '{print $1}' | head -n 1 || true)"
  [ -n "$EXPECTED" ] || return 1
  if command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')"
  elif command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')"
  elif command -v openssl >/dev/null 2>&1; then
    ACTUAL="$(openssl dgst -sha256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $NF}')"
  else
    return 1
  fi
  [ "$ACTUAL" = "$EXPECTED" ] || return 1

  tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR" || return 1
  [ -f "${TMP_DIR}/ccide-core" ] || return 1

  STAGED_BIN="${TARGET_BIN}.tmp.$$"
  cp -f "${TMP_DIR}/ccide-core" "$STAGED_BIN"
  chmod 0755 "$STAGED_BIN"
  if [ "$OS_S" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
    codesign -f -s - "$STAGED_BIN" >/dev/null 2>&1 || true
  fi
  mv -f "$STAGED_BIN" "$TARGET_BIN"
  STAGED_BIN=""
  bin_is_ok "$TARGET_BIN" >/dev/null 2>&1 || true
  if [ -n "${PLUGIN_ROOT:-}" ] && [ -d "$PLUGIN_ROOT" ]; then
    mkdir -p "${PLUGIN_ROOT}/bin" 2>/dev/null || true
    PLUG_STAGED="${PLUGIN_ROOT}/bin/ccide-core.tmp.$$"
    if cp -f "$TARGET_BIN" "$PLUG_STAGED" 2>/dev/null; then
      chmod 0755 "$PLUG_STAGED" 2>/dev/null || true
      mv -f "$PLUG_STAGED" "${PLUGIN_ROOT}/bin/ccide-core" 2>/dev/null || rm -f "$PLUG_STAGED" 2>/dev/null || true
    fi
  fi
  return 0
}

if try_download; then
  cleanup
  exec "$TARGET_BIN" "$@"
fi
cleanup
if [ -n "$EXISTING_BIN" ] && [ -x "$EXISTING_BIN" ]; then
  exec "$EXISTING_BIN" "$@"
fi
exit 0
