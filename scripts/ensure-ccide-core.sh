#!/bin/sh
# POSIX-compliant on-demand resolver & prebuilt GitHub Release downloader for `ccide-core`.
# Ensures Marketplace (`/plugin install cc-ide --marketplace Visio-Vanitas/ccide`) and
# zero-Rust hosts automatically fetch and SHA256-verify the matching native binary.
set -eu

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-}"
CLAUDE_BIN_DIR="${HOME}/.claude/bin"
TARGET_BIN="${CLAUDE_BIN_DIR}/ccide-core"
REPO_SLUG="${CCIDE_GITHUB_REPO:-Visio-Vanitas/ccide}"

if [ -n "$PLUGIN_ROOT" ] && [ -x "${PLUGIN_ROOT}/bin/ccide-core" ]; then
  exec "${PLUGIN_ROOT}/bin/ccide-core" "$@"
fi
if [ -x "$TARGET_BIN" ]; then
  exec "$TARGET_BIN" "$@"
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
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t ccide)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

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

if fetch_url "${BASE_URL}/SHA256SUMS.txt" "${TMP_DIR}/SHA256SUMS.txt"; then
  EXPECTED="$(grep " ${ARCHIVE}\$" "${TMP_DIR}/SHA256SUMS.txt" | awk '{print $1}' | head -n 1 || true)"
  if [ -n "$EXPECTED" ]; then
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
  fi
fi

mkdir -p "$CLAUDE_BIN_DIR"
tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"
if [ -f "${TMP_DIR}/ccide-core" ]; then
  chmod 0755 "${TMP_DIR}/ccide-core"
  if [ "$OS_S" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
    codesign -f -s - "${TMP_DIR}/ccide-core" >/dev/null 2>&1 || true
  fi
  mv -f "${TMP_DIR}/ccide-core" "$TARGET_BIN"
  if [ -n "${PLUGIN_ROOT:-}" ] && [ -d "$PLUGIN_ROOT" ]; then
    mkdir -p "${PLUGIN_ROOT}/bin" 2>/dev/null || true
    cp -f "$TARGET_BIN" "${PLUGIN_ROOT}/bin/ccide-core" 2>/dev/null || true
  fi
  exec "$TARGET_BIN" "$@"
fi
