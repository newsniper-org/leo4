# leo4 — build helpers.
# Run `just` to list. Run `just <task>` to execute.
#
# Conventions:
#   • Lake builds first, Cargo builds second. Never the reverse.
#   • `just test` is the canonical "is everything green" command.

default:
    @just --list

# Build both sides, in order.
build: lake-build cargo-build

# Lake side only.
lake-build:
    cd lake && lake build

# Cargo side only.
cargo-build:
    cargo build --workspace

# All tests, both sides.
test: lake-test cargo-test mangling-test

lake-test:
    cd lake && lake test

cargo-test:
    cargo test --workspace

# Cross-impl mangling conformance (Phase 2+).
mangling-test:
    @echo "TODO(phase-2): mangling conformance"

# Clippy + format checks.
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all -- --check

# Format everything.
fmt:
    cargo fmt --all

# Validate SPEC/*.md consistency (Phase 1+).
spec-lint:
    @echo "TODO(phase-1): SPEC consistency checker"

# Nuke build outputs.
clean:
    cargo clean
    rm -rf lake/.lake lake/build

# Show the resolved schema hash for a given IDL file (Phase 2+).
hash idl:
    @cargo run --bin leo4c -- mangle {{idl}} | jq -r '.schema_hash'
