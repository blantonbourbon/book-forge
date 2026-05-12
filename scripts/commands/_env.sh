#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export BOOK_FORGE_ROOT="${BOOK_FORGE_ROOT:-$(cd "${SCRIPT_DIR}/../.." && pwd)}"
export CARGO_BIN="${CARGO_BIN:-cargo}"

if [ -z "${BOOK_FORGE_MISSION_DIR:-}" ] && [ -d "${HOME:-}/.factory/missions" ]; then
  for candidate in "${HOME}/.factory/missions"/*; do
    if [ -x "${candidate}/tools/zig-cc" ]; then
      export BOOK_FORGE_MISSION_DIR="${candidate}"
      break
    fi
  done
fi

if [ -n "${BOOK_FORGE_MISSION_DIR:-}" ] && [ -x "${BOOK_FORGE_MISSION_DIR}/init.sh" ]; then
  "${BOOK_FORGE_MISSION_DIR}/init.sh"
fi

if [ -n "${BOOK_FORGE_MISSION_DIR:-}" ] && [ -x "${BOOK_FORGE_MISSION_DIR}/tools/zig-cc" ]; then
  export CC="${CC:-${BOOK_FORGE_MISSION_DIR}/tools/zig-cc}"
  export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-${BOOK_FORGE_MISSION_DIR}/tools/zig-cc}"
fi

cd "${BOOK_FORGE_ROOT}"
