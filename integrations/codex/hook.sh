#!/bin/sh
CORE="${CCIDE_CORE_BIN:-${HOME}/.claude/bin/ccide-core}"
if [ -x "$CORE" ]; then
  exec "$CORE" "$@"
fi
exit 0
