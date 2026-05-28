#!/usr/bin/env bash
# Per-distro audit payload — runs inside each container.
#
# Driven by `audit-runner.py` with two env vars:
#   AUDIT_TARGET = the Rust target triple to drive
#   AUDIT_DISTRO = the distro identifier (purely for log
#                  banner; the script's logic is target-
#                  driven, distro-agnostic)
#
# Mirrors the `linux-musl` job in
# `.github/workflows/ci.yml` — same crate set, same
# build / test split. Keeping the two in sync means a
# regression caught by GitHub Actions also reproduces
# locally with `just linux-distro-audit <id>`.
#
# Exit codes:
#   0  full pass
#   1+ any cargo invocation failed; the failing line is
#      printed before exit

set -euo pipefail

: "${AUDIT_TARGET:?AUDIT_TARGET env var not set (audit-runner.py bug)}"
: "${AUDIT_DISTRO:?AUDIT_DISTRO env var not set (audit-runner.py bug)}"

echo
echo "──────────────────────────────────────────────────"
echo "  audit-payload  distro=${AUDIT_DISTRO}  target=${AUDIT_TARGET}"
echo "──────────────────────────────────────────────────"

# Some distros mount the leo4 tree read-only; cargo
# needs CARGO_TARGET_DIR somewhere writable. The runner
# sets it via env (defaults to /cache/target).
: "${CARGO_TARGET_DIR:=/tmp/leo4-target}"
mkdir -p "${CARGO_TARGET_DIR}"

# musl-clang / musl-gcc selection. cc-rs reads
# CC_<target-with-hyphens>. We accept whichever the
# host distro packages — `which` resolves the first
# available wrapper.
if command -v musl-clang >/dev/null 2>&1; then
    export "CC_$(echo "${AUDIT_TARGET}" | tr - _)"="musl-clang"
elif command -v musl-gcc >/dev/null 2>&1; then
    export "CC_$(echo "${AUDIT_TARGET}" | tr - _)"="musl-gcc"
fi

echo "[1/3] cargo build --workspace --target ${AUDIT_TARGET} (excluding leo4-wasm)"
cargo build \
    --workspace \
    --target "${AUDIT_TARGET}" \
    --exclude leo4-wasm

echo "[2/3] cargo test --target ${AUDIT_TARGET} on the audit-verified musl-clean subset"
# The subset matches `.github/workflows/ci.yml`'s
# `linux-musl` job. leo4-mslean4 is compile-only (Lean
# runtime needs glibc), leo4-wasm needs wasmtime
# build.rs toolchain not pinned in CI images.
cargo test \
    --target "${AUDIT_TARGET}" \
    -p schema-idl \
    -p leo4-idl \
    -p leo4-abi \
    -p leo4-build \
    -p leo4-macros \
    -p leo4-macros-backend \
    -p leo4c \
    -p leo4-rust-emit \
    -p leo4-cli \
    -p leo4-rust-worker

echo "[3/3] PASS — distro=${AUDIT_DISTRO} target=${AUDIT_TARGET}"
