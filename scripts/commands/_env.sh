#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export BOOK_FORGE_ROOT="${BOOK_FORGE_ROOT:-$(cd "${SCRIPT_DIR}/../.." && pwd)}"

cd "${BOOK_FORGE_ROOT}"
