#!/usr/bin/env bash
# Cross-impl mangling conformance harness.
#
# 1. Builds the Lake plugin and runs it against tests/sample-lean,
#    producing .leo4-schema / .leo4-mangling / .leo4-handshake.
# 2. Builds the leo4c CLI.
# 3. Runs `leo4c mangle <schema>` on the same input and diffs the
#    resulting (schema_hash, mangled-name set) against the Lake-emitted
#    .leo4-mangling.
#
# Exit code 0 ⇔ both sides produce byte-identical schema_hash AND
# byte-identical set of mangled names. Any divergence is a hard fail.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")"/../.. && pwd)"

cd "$ROOT"

just smoke-plugin > /dev/null

cargo build --quiet -p leo4c
BIN="$ROOT/target/debug/leo4c"

SCHEMA="$ROOT/tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-schema"
LEAN_MANGLING="$ROOT/tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-mangling"
LEAN_HANDSHAKE="$ROOT/tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-handshake"

# Schema hash check.
RUST_HASH=$("$BIN" mangle "$SCHEMA" | jq -r .schema_hash)
LEAN_HASH=$(jq -r .schema_hash "$LEAN_HANDSHAKE")

if [ "$RUST_HASH" != "$LEAN_HASH" ]; then
  echo "✗ schema_hash divergence:"
  echo "  Rust: $RUST_HASH"
  echo "  Lean: $LEAN_HASH"
  exit 1
fi
echo "✓ schema_hash $RUST_HASH"

# Mangled-name set check.
RUST_MANGLED=$("$BIN" mangle "$SCHEMA" | jq -r '.entries[].instantiations[].mangled' | sort)
LEAN_MANGLED=$(jq -r '.entries[].instantiations[].mangled' "$LEAN_MANGLING" | sort)

if ! diff <(echo "$RUST_MANGLED") <(echo "$LEAN_MANGLED") > /tmp/leo4-mangling-diff; then
  echo "✗ mangled-name set divergence (see /tmp/leo4-mangling-diff)"
  exit 1
fi
echo "✓ $(echo "$RUST_MANGLED" | wc -l) mangled names byte-identical"
