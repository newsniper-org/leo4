#!/usr/bin/env bash
# leo4 CI entrypoint — single-container dispatcher with hardlink-shared
# source mirror.
#
# Mounts expected at run time:
#   /workspace  RO  — repository root (host mount)
#   /cache      RW  — named volume; persistent across runs.
#
# Layout inside /cache:
#   /cache/src/                  — single master mirror of the source,
#                                  rsync'd from /workspace on each run.
#   /cache/work-<ver>/           — per-version work tree, populated with
#                                  `rsync --link-dest=/cache/src/`, so
#                                  every unchanged source file is a
#                                  hardlink to /cache/src/. The
#                                  `lean-toolchain` files are then
#                                  re-written (new inode) so the stamp
#                                  does not leak across versions.
#   /cache/work-<ver>/target/    — cargo cache (per-version).
#   /cache/work-<ver>/**/.lake/  — Lake cache  (per-version).
#
# Build tools never write into the *source* portion of the tree, so the
# hardlink dedup is safe: lake/cargo only touch `target/` and `.lake/`,
# both excluded from rsync and resident only in each work-dir.
#
# Operating modes:
#   * Single-version  — `LEAN_VERSION=vX.Y.Z` is set; sync, cd, exec.
#   * Matrix (default) — iterate `LEO4_MATRIX_VERSIONS`
#                        (default: v4.27.0 v4.28.0 v4.29.1 v4.30.0-rc2).

set -euo pipefail

SRC=/workspace
CACHE=/cache
SRC_MIRROR=$CACHE/src

if [[ ! -d "$SRC" ]]; then
  echo "leo4-entrypoint: missing /workspace mount" >&2
  exit 2
fi
mkdir -p "$CACHE" "$SRC_MIRROR"

# Refresh the master source mirror exactly once per container start.
refresh_master() {
  rsync -a --delete \
    --exclude='/target/' \
    --exclude='/.lake/' \
    --exclude='**/.lake/' \
    --exclude='/result/' \
    --exclude='.git/objects/pack/*.idx' \
    "$SRC/" "$SRC_MIRROR/"
}

sync_workspace() {
  local v=$1
  local work="$CACHE/work-${v//./-}"
  mkdir -p "$work"
  # `--link-dest` hardlinks unchanged files from the master mirror into
  # the per-version work tree. target/ and .lake/ in the work tree are
  # left alone, so build caches persist across runs.
  rsync -a \
    --link-dest="$SRC_MIRROR/" \
    --exclude='/target/' \
    --exclude='/.lake/' \
    --exclude='**/.lake/' \
    --exclude='/result/' \
    "$SRC_MIRROR/" "$work/"
  # Stamp toolchain files. Remove the hardlink first so the new write
  # produces a fresh inode — otherwise we would smear the version
  # across every work tree.
  while IFS= read -r f; do
    rm -f "$f"
    echo "leanprover/lean4:${v}" > "$f"
  done < <(find "$work" -name lean-toolchain -not -path '*/.lake/*' -not -path '*/target/*')
  echo "$work"
}

refresh_master

DEFAULT_CMD=(just test)
CMD=("$@")
[[ ${#CMD[@]} -eq 0 ]] && CMD=("${DEFAULT_CMD[@]}")

# ── single-version mode ──────────────────────────────────────────────
if [[ -n "${LEAN_VERSION:-}" ]]; then
  work=$(sync_workspace "$LEAN_VERSION")
  cd "$work"
  exec "${CMD[@]}"
fi

# ── matrix mode ──────────────────────────────────────────────────────
# Default version list anchors on spike/SPIKE-0-FINDINGS.md Q4
# (v4.27/v4.28/v4.29.1 API-surface diffed there) plus the next RC.
default_versions="v4.27.0 v4.28.0 v4.29.1 v4.30.0-rc2"
read -ra VERSIONS <<< "${LEO4_MATRIX_VERSIONS:-$default_versions}"

declare -a results
fail=0
for v in "${VERSIONS[@]}"; do
  echo "── Lean ${v} ──"
  work=$(sync_workspace "$v")
  if (cd "$work" && "${CMD[@]}"); then
    echo "✓ ${v}"
    results+=("✓ ${v}")
  else
    echo "✗ ${v}"
    results+=("✗ ${v}")
    fail=1
  fi
done

echo
echo "── matrix summary ──"
printf '  %s\n' "${results[@]}"
if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "── all ${#VERSIONS[@]} versions green ──"
