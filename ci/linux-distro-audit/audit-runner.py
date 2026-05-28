#!/usr/bin/env python3
"""Generic runner for per-distro Linux audits.

Invoked by `just linux-distro-audit <distro-id>`. Reads
distro definitions from `distros.toml` *at run time* —
this script contains no hard-coded distro names, package
manager invocations, or image references. To add a new
distro, append a `[distros.<id>]` table to `distros.toml`;
the runner picks it up on the next invocation.

Workflow per invocation:
  1. Load + validate the distro entry by id.
  2. Locate a container runtime (`podman` preferred,
     `docker` fallback — same convention as `ci/matrix.sh`).
  3. Launch the distro's image with the leo4 source tree
     bind-mounted read-only at `/work`.
  4. Run the entry's `setup` commands one by one, then
     execute `audit-payload.sh` for each `audit_target`.
  5. Exit 0 on full success; non-zero on any step's
     failure, with the failing step printed verbatim.

Cache: each distro has its own anonymous Cargo target dir
inside the container (`/cache/<distro>-target`) volume-
mounted so repeated `just linux-distro-audit <distro>` runs
don't recompile from scratch.
"""

import argparse
import os
import pathlib
import shutil
import subprocess
import sys

# Python 3.11+ ships tomllib in stdlib. Fall back to `tomli`
# for older interpreters (apt's `python3-tomli` package).
try:
    import tomllib  # type: ignore[attr-defined]
except ModuleNotFoundError:  # pragma: no cover — older Pythons
    import tomli as tomllib  # type: ignore[no-redef]


HERE = pathlib.Path(__file__).resolve().parent
LEO4_ROOT = HERE.parent.parent

CACHE_VOLUME_PREFIX = "leo4-distro-audit-cache-"


def detect_container_runtime() -> str:
    for candidate in ("podman", "docker"):
        if shutil.which(candidate):
            return candidate
    sys.exit(
        "ERROR: no container runtime found. `just linux-distro-audit` "
        "needs `podman` or `docker` on PATH."
    )


def load_distros() -> dict:
    cfg_path = HERE / "distros.toml"
    if not cfg_path.is_file():
        sys.exit(f"ERROR: missing `{cfg_path}`")
    with cfg_path.open("rb") as f:
        cfg = tomllib.load(f)
    return cfg.get("distros", {})


def banner(s: str) -> None:
    bar = "─" * (len(s) + 4)
    print(f"\n{bar}", file=sys.stderr)
    print(f"  {s}", file=sys.stderr)
    print(f"{bar}\n", file=sys.stderr)


def shell_join(parts: list[str]) -> str:
    """Join shell snippets with `&&` so they short-circuit on
    failure inside the container. Each snippet is already a
    full shell command (possibly compound) per distros.toml."""
    return " && ".join(parts)


def run(cmd: list[str]) -> None:
    """Run a host-side command; abort on non-zero exit."""
    print(f"$ {' '.join(cmd)}", file=sys.stderr)
    res = subprocess.run(cmd, check=False)
    if res.returncode != 0:
        sys.exit(f"ERROR: command failed (exit {res.returncode}): {' '.join(cmd)}")


def audit_one(runtime: str, distro_id: str, entry: dict) -> None:
    image = entry.get("image")
    if not image:
        sys.exit(f"ERROR: distro `{distro_id}` missing `image` field")
    setup = entry.get("setup", [])
    if not isinstance(setup, list):
        sys.exit(f"ERROR: distro `{distro_id}` `setup` must be a list of strings")
    audit_targets = entry.get("audit_targets", ["x86_64-unknown-linux-musl"])
    if not isinstance(audit_targets, list) or not audit_targets:
        sys.exit(f"ERROR: distro `{distro_id}` needs ≥1 `audit_targets` entry")
    note = entry.get("note", "")

    cache_name = f"{CACHE_VOLUME_PREFIX}{distro_id}"

    banner(f"linux-distro-audit: {distro_id}  ({image})")
    if note:
        print(f"  note: {note}", file=sys.stderr)
    print(f"  audit targets: {', '.join(audit_targets)}", file=sys.stderr)
    print(f"  cache volume:  {cache_name}\n", file=sys.stderr)

    # Build the in-container command. Setup snippets run
    # under `bash -e -c "<joined>"` so any failure aborts
    # immediately. Then the payload script runs per target.
    container_cmd_parts: list[str] = []
    container_cmd_parts.extend(setup)
    for target in audit_targets:
        container_cmd_parts.append(
            f'AUDIT_TARGET="{target}" '
            f'AUDIT_DISTRO="{distro_id}" '
            f'bash /work/ci/linux-distro-audit/audit-payload.sh'
        )

    full_cmd = shell_join(container_cmd_parts)

    args = [
        runtime, "run", "--rm", "--init",
        "-v", f"{LEO4_ROOT}:/work:ro",
        "-v", f"{cache_name}:/cache",
        "-e", "CARGO_TARGET_DIR=/cache/target",
        "-e", "CARGO_HOME=/cache/cargo",
        "-e", "RUSTUP_HOME=/cache/rustup",
        "-w", "/work",
        image,
        "bash", "-e", "-o", "pipefail", "-c", full_cmd,
    ]
    run(args)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run a per-distro audit in a container."
    )
    parser.add_argument(
        "distro",
        nargs="?",
        default=None,
        help="distro identifier as listed in `distros.toml`. Pass `--list` "
        "to enumerate known ids.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List known distro ids + their notes and exit.",
    )
    args = parser.parse_args()

    distros = load_distros()
    if args.list or args.distro in (None, "list", "--list"):
        print("Available distros (defined in distros.toml):", file=sys.stderr)
        for did, entry in distros.items():
            note = entry.get("note", "")
            print(f"  - {did}: {note}", file=sys.stderr)
        return

    if args.distro not in distros:
        sys.stderr.write(
            f"ERROR: unknown distro `{args.distro}`. Known:\n"
        )
        for did in distros:
            sys.stderr.write(f"  - {did}\n")
        sys.stderr.write(
            "\nTip: `just linux-distro-audit --list` enumerates with notes.\n"
        )
        sys.exit(2)

    runtime = detect_container_runtime()
    audit_one(runtime, args.distro, distros[args.distro])
    banner(f"linux-distro-audit: {args.distro} — PASS")


if __name__ == "__main__":
    main()
