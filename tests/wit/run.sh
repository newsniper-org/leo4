#!/usr/bin/env bash
# WIT lowering golden + validation harness.
#
# For each `tests/wit/cases/*.leo4-schema`:
#   1. Run `leo4c lower` and diff against `tests/wit/expected/*.wit`.
#   2. Validate the expected file with `wasm-tools component wit`.
# Optionally — when `wit-bindgen` is on PATH — additionally generate the
# markdown bindings to sanity-check that the world is consumable.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")"/../.. && pwd)"
cd "$ROOT"

cargo build --quiet -p leo4c
BIN="$ROOT/target/debug/leo4c"

WIT_BINDGEN_OK=1
command -v wit-bindgen > /dev/null || WIT_BINDGEN_OK=0
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

fail=0
for case in "$ROOT"/tests/wit/cases/*.leo4-schema; do
  name=$(basename "$case" .leo4-schema)
  expected="$ROOT/tests/wit/expected/$name.wit"
  actual=$("$BIN" lower "$case")

  if ! diff -u "$expected" <(echo "$actual") > "$TMPDIR/$name.diff" 2>&1; then
    echo "✗ $name: lower output diverges from golden"
    sed 's/^/    /' "$TMPDIR/$name.diff"
    fail=1
    continue
  fi

  if ! wasm-tools component wit "$expected" > /dev/null 2>&1; then
    echo "✗ $name: wasm-tools rejects the WIT"
    wasm-tools component wit "$expected" 2>&1 | sed 's/^/    /'
    fail=1
    continue
  fi

  if [ "$WIT_BINDGEN_OK" -eq 1 ]; then
    if ! wit-bindgen markdown "$expected" --out-dir "$TMPDIR/$name" > /dev/null 2>&1; then
      echo "✗ $name: wit-bindgen rejects the world"
      fail=1
      continue
    fi
  fi

  echo "✓ $name"
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "all $(ls "$ROOT"/tests/wit/cases | wc -l) WIT cases passed"
