# leo4 — build helpers.
# Run `just` to list. Run `just <task>` to execute.
#
# Conventions:
#   • Lake builds first, Cargo builds second. Never the reverse (CLAUDE.md, D8).
#   • `just test` is the canonical "is everything green" command.

default:
    @just --list

# Repo paths.
leo4_pkg     := "lake/Leo4"
plugin_pkg   := "lake/Leo4Plugin"
sample_pkg   := "tests/sample-lean"
plugin_bin   := plugin_pkg + "/.lake/build/bin/leo4plugin"
leo4_oleans  := leo4_pkg   + "/.lake/build/lib/lean"
sample_oleans := sample_pkg + "/.lake/build/lib/lean"

# ─── Lake side ────────────────────────────────────────────────────────────

# Build the runtime library, the plugin library + exe, and the sample test pkg.
lake-build:
    cd {{plugin_pkg}} && lake build
    cd {{sample_pkg}} && lake build

# Build only the plugin (and its Leo4 dependency).
plugin-build:
    cd {{plugin_pkg}} && lake build

# Build the sample test package, including the per-module shared
# library (Sample:shared) the leo4 shim links against to resolve
# `initialize_<Sample>` and any other LEAN_EXPORT symbols the user
# module defines. Without `:shared`, lake only produces `.olean`s
# and the shim's transitive `initialize_*` call at load time fails
# with "undefined symbol".
sample-build:
    cd {{sample_pkg}} && lake build && lake build Sample:shared

# Lake tests (currently none beyond the smoke run).
lake-test: smoke-plugin

# Run the plugin against tests/sample-lean and emit handshake + mangling artifacts.
# OUTPUT_DIR defaults to {{sample_pkg}}/.lake/build/leo4/.
smoke-plugin: plugin-build sample-build
    #!/usr/bin/env bash
    set -euo pipefail
    LEAN_PATH="{{sample_oleans}}:{{leo4_oleans}}:/opt/lean4/lib/lean" \
      ./{{plugin_bin}} Sample {{sample_pkg}}/.lake/build/leo4

# Like `smoke-plugin`, but also shell out to leo4c to emit <pkg>.wit
# alongside the canonical artefacts (uses the `--with-lower` plugin flag).
# Requires leo4c built (PATH-discoverable); this recipe builds it first.
smoke-plugin-with-wit: plugin-build sample-build cargo-build
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="$(pwd)/target/debug:$PATH"
    LEAN_PATH="{{sample_oleans}}:{{leo4_oleans}}:/opt/lean4/lib/lean" \
      ./{{plugin_bin}} Sample {{sample_pkg}}/.lake/build/leo4 leo4-sample Sample --with-lower

# ─── Cargo side ───────────────────────────────────────────────────────────

cargo-build:
    cargo build --workspace

cargo-test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all -- --check

fmt:
    cargo fmt --all

# ─── Cross-cutting ────────────────────────────────────────────────────────

# Both sides, in order. Lake first per D8.
build: lake-build cargo-build

test: lake-test cargo-test mangling-test wit-test conformance-test

# Cross-impl mangling conformance — Lake plugin vs leo4c (Rust).
mangling-test:
    tests/mangling/run.sh

# WIT lowering golden + validation (wasm-tools / wit-bindgen).
wit-test:
    tests/wit/run.sh

# Lean/Rust encoder byte-for-byte conformance (Phase 4).
conformance-test:
    tests/conformance/run.sh

# Validate SPEC/*.md consistency (Phase 1+).
spec-lint:
    @echo "TODO(phase-1): SPEC consistency checker"

# Show the resolved schema hash for the sample package's handshake file.
schema-hash: smoke-plugin
    @jq -r '.schema_hash' {{sample_pkg}}/.lake/build/leo4/leo4_sample.leo4-handshake

# Type-check every `Leo4.MathlibBridge.*` module against actual
# Mathlib types. Heavy: Mathlib's cold build is 1-2 hours. NOT on the
# default `just test` ladder — invoke explicitly when bridge code
# changes. First run pulls Mathlib via Lake's reservoir; subsequent
# runs hit the local `.lake` cache.
mathlib-bridge-test:
    cd sibling/mathlib-bridge-test && lake build

# ─── Phase 9 reverse-direction (Rust → Lean) automation ─────────────────
#
# The full pipeline from user cdylib to executable Lean wrapper is a
# 4-step manual sequence today (`SPEC/reverse-direction.md` §7). These
# recipes collapse it into named, repeatable commands.

# Build all three Cargo artefacts the reverse direction needs:
#   * `libleo4_rust_bridge.a`  — the dispatcher static archive (9-4a/b).
#   * `leo4-rust-worker`       — the IPC worker binary (9-3).
#   * `leo4-rust-emit`         — the metadata + Lean wrapper emit CLI (9-2/9-5).
rust-bridge-build:
    cargo build --release -p leo4-rust-bridge
    cargo build --release -p leo4-rust-worker
    cargo build --release -p leo4-rust-emit

# Emit IDL + handshake + Lean wrapper for one user cdylib.
#
# Usage: just rust-emit /abs/path/lib.so out_dir/ MyApp.Rust
rust-emit CDYLIB OUT_DIR MODULE:
    mkdir -p {{OUT_DIR}}
    cargo run --release --quiet -p leo4-rust-emit -- \
      --cdylib {{CDYLIB}} \
      --out-dir {{OUT_DIR}} \
      --emit-lean \
      --lean-module {{MODULE}}

# Compile `shim/leo4_rust_bridge_lean.c` (the one Lean-aware shim
# that bridges lean_object* <-> the dispatcher's byte ABI) into a
# `.o` file at OUT_OBJ. Reruns leanc whenever the C source changes.
glue-shim-build OUT_OBJ:
    leanc -c -std=c2x shim/leo4_rust_bridge_lean.c -o {{OUT_OBJ}}

# End-to-end pipeline wrapper for examples/05-rust-export. With
# Phase 10-D2's `lake run Leo4Rust/regenerate` script, emit now
# goes through Lake rather than via a direct cargo-binary
# invocation. Steps collapse to 3.
rust-export-05-build: rust-bridge-build
    @echo "[1/3] Building example cdylib..."
    cargo build --release -p leo4-example-05-rust-export
    @echo "[2/3] lake run Leo4Rust/regenerate (Lake-driven emit, Phase 10-D2)..."
    rm -rf examples/05-rust-export/lean/Leo4ExampleMiniSolverRust \
           examples/05-rust-export/lean/.leo4-emit
    cd examples/05-rust-export/lean && \
      LEO4_RUST_EMIT_BIN=`realpath ../../../target/release/leo4-rust-emit` \
      LEO4_RUST_IFACE=Leo4ExampleMiniSolverRust \
        lake run Leo4Rust/regenerate
    @echo "[3/3] Lake build (auto-links bridge + glue-shim via Leo4Rust extern_libs)..."
    cd examples/05-rust-export/lean && lake build
    @echo "Done. Run via:"
    @echo "  LEO4_RUST_CDYLIB=\$$(realpath target/release/libleo4_example_05_rust_export.so) \\"
    @echo "  LEO4_RUST_WORKER_BIN=\$$(realpath target/release/leo4-rust-worker) \\"
    @echo "  LEO4_RUST_HANDSHAKE_PKG=leo4_example_05_rust_export \\"
    @echo "  LEO4_RUST_HANDSHAKE_IFACE=Leo4ExampleMiniSolverRust \\"
    @echo "    examples/05-rust-export/lean/.lake/build/bin/leo4Example05"

# Drop emitted reverse-direction artefacts for examples/05.
rust-export-05-clean:
    rm -rf examples/05-rust-export/lean/.leo4-emit \
           examples/05-rust-export/lean/Leo4ExampleMiniSolverRust \
           examples/05-rust-export/lean/.lake

# ─── Multi-version Lean matrix (Phase 5 prep) ───────────────────────────

# Run `just test` in a hermetic container across the Lean version matrix
# (default v4.27.0/v4.28.0/v4.29.1/v4.30.0-rc2). The first run pulls each
# toolchain + builds caches; subsequent runs reuse `/cache/work-<ver>/`.
ci-matrix:
    ci/matrix.sh

# Same harness, single Lean version. Drops into the per-version work dir.
ci-version VERSION:
    LEAN_VERSION={{VERSION}} ci/matrix.sh

# Build the container image only.
ci-image:
    docker build -f ci/Dockerfile.lean-test -t leo4-test .

# Drop the matrix cache volume (next ci-matrix rebuilds it from scratch).
ci-clean-cache:
    -docker volume rm leo4-matrix-cache

# Linux distro audit — launch a container of `<distro>`, install
# its toolchain, run the leo4 musl audit (`ci/linux-distro-audit/
# audit-payload.sh`). NO hard-coding: distro data lives in
# `ci/linux-distro-audit/distros.toml`; the runner picks up new
# `[distros.<id>]` tables on the next invocation. Pass `--list`
# to enumerate known ids without launching anything.
#
#   just linux-distro-audit archlinux
#   just linux-distro-audit debian-12
#   just linux-distro-audit --list
linux-distro-audit *DISTRO:
    python3 ci/linux-distro-audit/audit-runner.py {{DISTRO}}

# Nuke build outputs.
clean:
    cargo clean
    rm -rf {{plugin_pkg}}/.lake {{leo4_pkg}}/.lake {{sample_pkg}}/.lake
