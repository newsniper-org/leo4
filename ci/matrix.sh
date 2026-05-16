#!/usr/bin/env bash
# leo4 multi-version Lean matrix runner — host wrapper.
#
# Spawns *one* `docker run` whose entrypoint iterates the matrix
# internally (ci/entrypoint.sh). Per-version build artefacts survive
# between matrix runs via the single `leo4-matrix-cache` named volume.
#
# Usage:
#   ci/matrix.sh                  # full matrix, `just test`
#   ci/matrix.sh -- just plugin-build   # full matrix, custom command
#
# Single-version debug:
#   LEAN_VERSION=v4.29.1 ci/matrix.sh -- bash
# (drops you into the per-version work directory for poking.)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")"/.. && pwd)"
IMAGE=${LEO4_TEST_IMAGE:-leo4-test}
CACHE_VOLUME=${LEO4_CACHE_VOLUME:-leo4-matrix-cache}

# Pass through versions via env so the user can narrow the matrix
# without editing the entrypoint:
#   LEO4_MATRIX_VERSIONS="v4.29.1 v4.30.0-rc2" ci/matrix.sh

# Build the image once (no-op if cached).
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "── building ${IMAGE} ──"
  docker build -f "$ROOT/ci/Dockerfile.lean-test" -t "$IMAGE" "$ROOT"
fi

DOCKER_ENV=(
  -e "LEO4_MATRIX_VERSIONS=${LEO4_MATRIX_VERSIONS:-}"
)
if [[ -n "${LEAN_VERSION:-}" ]]; then
  DOCKER_ENV+=(-e "LEAN_VERSION=${LEAN_VERSION}")
fi

docker run --rm -t \
  "${DOCKER_ENV[@]}" \
  -v "$ROOT:/workspace:ro" \
  -v "${CACHE_VOLUME}:/cache" \
  "$IMAGE" \
  "$@"
