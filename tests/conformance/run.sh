#!/usr/bin/env bash
# Cross-impl encoder conformance harness (Phase 4).
#
# 1. Builds the Leo4 runtime library.
# 2. Runs `tests/conformance/gen.lean` via `lean --run` to dump a hex
#    fixture file at $LEO4_CONFORMANCE_FILE (default /tmp/leo4-conformance.txt).
# 3. Runs `cargo test -p leo4-abi --test cross_impl`, which re-encodes
#    each named value on the Rust side and diffs against the fixture.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")"/../.. && pwd)"
cd "$ROOT"

# 1. Lean side.
(cd lake/Leo4Plugin && lake build) > /dev/null

# 2. Generator.
FIXTURE="${LEO4_CONFORMANCE_FILE:-/tmp/leo4-conformance.txt}"
LEO4_PATH="$ROOT/lake/Leo4/.lake/build/lib/lean"
LEAN_PATH="$LEO4_PATH:/opt/lean4/lib/lean" \
  lean --run "$ROOT/tests/conformance/gen.lean" > "$FIXTURE"
echo "wrote fixture $(wc -l < "$FIXTURE") lines → $FIXTURE"

# 3. Rust side.
LEO4_CONFORMANCE_FILE="$FIXTURE" cargo test -q -p leo4-abi --test cross_impl
echo "✓ cross-impl conformance"
