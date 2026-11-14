#!/bin/sh
# POSIX-compliant on-demand resolver & prebuilt GitHub Release downloader for `ccide-core`.
# Ensures Marketplace (`/plugin install cc-ide --marketplace Visio-Vanitas/ccide`) and
# zero-Rust hosts automatically fetch and SHA256-verify the matching native binary.
set -eu

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-}"
CLAUDE_BIN_DIR="${HOME}/.claude/bin"
TARGET_BIN="${CLAUDE_BIN_DIR}/ccide-core"
REPO_SLUG="${CCIDE_GITHUB_REPO:-Visio-Vanitas/ccide}"

if [ -x "$TARGET_BIN" ]; then
  exec "$TARGET_BIN" "$@"
fi
if [ -n "$PLUGIN_ROOT" ] && [ -x "${PLUGIN_ROOT}/bin/ccide-core" ]; then
  exec "${PLUGIN_ROOT}/bin/ccide-core" "$@"
fi
if command -v ccide-core >/dev/null 2>&1; then
  exec ccide-core "$@"
fi

OS_S="$(uname -s 2>/dev/null || echo unknown)"
ARCH_M="$(uname -m 2>/dev/null || echo unknown)"

case "${OS_S}:${ARCH_M}" in
  Darwin:arm64|Darwin:aarch64) TRIPLE="aarch64-apple-darwin" ;;
  Darwin:x86_64)               TRIPLE="x86_64-apple-darwin" ;;
  Linux:x86_64|Linux:amd64)    TRIPLE="x86_64-unknown-linux-gnu" ;;
  Linux:aarch64|Linux:arm64)   TRIPLE="aarch64-unknown-linux-gnu" ;;
  *)
    exit 0
    ;;
esac

ARCHIVE="ccide-core-${TRIPLE}.tar.gz"
BASE_URL="https://github.com/${REPO_SLUG}/releases/latest/download"
LOCK_DIR="${CLAUDE_BIN_DIR}/.ensure-ccide-core.lock"
LOCK_HELD=0
TMP_DIR=""
STAGED_BIN=""
cleanup() {
  rm -rf "${TMP_DIR:-}" "${STAGED_BIN:-}"
  if [ "${LOCK_HELD:-0}" -eq 1 ]; then
    rm -rf "$LOCK_DIR" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

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

if [ -x "$TARGET_BIN" ]; then
  if [ "$LOCK_HELD" -eq 1 ]; then
    rm -rf "$LOCK_DIR" 2>/dev/null || true
    LOCK_HELD=0
  fi
  exec "$TARGET_BIN" "$@"
fi

TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t ccide)"

fetch_url() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout 5 --max-time 25 "$1" -o "$2" 2>/dev/null
  elif command -v wget >/dev/null 2>&1; then
    wget -q -T 20 -O "$2" "$1" 2>/dev/null
  else
    return 1
  fi
}

if ! fetch_url "${BASE_URL}/${ARCHIVE}" "${TMP_DIR}/${ARCHIVE}"; then
  exit 0
fi

if ! fetch_url "${BASE_URL}/SHA256SUMS.txt" "${TMP_DIR}/SHA256SUMS.txt"; then
  exit 0
fi

EXPECTED="$(grep " ${ARCHIVE}\$" "${TMP_DIR}/SHA256SUMS.txt" | awk '{print $1}' | head -n 1 || true)"
if [ -z "$EXPECTED" ]; then
  exit 0
fi
if command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')"
elif command -v openssl >/dev/null 2>&1; then
  ACTUAL="$(openssl dgst -sha256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $NF}')"
else
  exit 0
fi
if [ "$ACTUAL" != "$EXPECTED" ]; then
  exit 0
fi

mkdir -p "$CLAUDE_BIN_DIR"
tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"
if [ -f "${TMP_DIR}/ccide-core" ]; then
  STAGED_BIN="${TARGET_BIN}.tmp.$$"
  cp -f "${TMP_DIR}/ccide-core" "$STAGED_BIN"
  chmod 0755 "$STAGED_BIN"
  if [ "$OS_S" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
    codesign -f -s - "$STAGED_BIN" >/dev/null 2>&1 || true
  fi
  mv -f "$STAGED_BIN" "$TARGET_BIN"
  STAGED_BIN=""
  if [ -n "${PLUGIN_ROOT:-}" ] && [ -d "$PLUGIN_ROOT" ]; then
    mkdir -p "${PLUGIN_ROOT}/bin" 2>/dev/null || true
    PLUG_STAGED="${PLUGIN_ROOT}/bin/ccide-core.tmp.$$"
    if cp -f "$TARGET_BIN" "$PLUG_STAGED" 2>/dev/null; then
      chmod 0755 "$PLUG_STAGED" 2>/dev/null || true
      mv -f "$PLUG_STAGED" "${PLUGIN_ROOT}/bin/ccide-core" 2>/dev/null || rm -f "$PLUG_STAGED" 2>/dev/null || true
    fi
  fi
  if [ "$LOCK_HELD" -eq 1 ]; then
    rm -rf "$LOCK_DIR" 2>/dev/null || true
    LOCK_HELD=0
  fi
  exec "$TARGET_BIN" "$@"
fi
