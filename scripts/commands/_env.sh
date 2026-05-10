#!/usr/bin/env bash
set -euo pipefail

export BOOK_FORGE_ROOT="${BOOK_FORGE_ROOT:-/home/kratos/projects/book-forge}"
export BOOK_FORGE_MISSION_DIR="${BOOK_FORGE_MISSION_DIR:-/home/kratos/.factory/missions/bdb6fb92-4fc3-47e7-9eac-a10d4c47fb83}"
export CARGO_BIN="${CARGO_BIN:-/home/kratos/.cargo/bin/cargo}"

if [ -x "${BOOK_FORGE_MISSION_DIR}/init.sh" ]; then
  "${BOOK_FORGE_MISSION_DIR}/init.sh"
fi

export CC="${CC:-${BOOK_FORGE_MISSION_DIR}/tools/zig-cc}"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-${BOOK_FORGE_MISSION_DIR}/tools/zig-cc}"

cd "${BOOK_FORGE_ROOT}"
