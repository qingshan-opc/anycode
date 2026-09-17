#!/usr/bin/env python3
"""Run real harness checks. Missing tools are failures, never successful skips."""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

PACKAGES = (
    "anycode-harness-core",
    "anycode-harness-extensions",
    "anycode-harness-cloud818",
    "anycode-harness-host",
)


def run(command: list[str], cwd: Path) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--mode", choices=["node", "standalone-rust", "integrated-rust"], required=True
    )
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    repo = args.repo.resolve()
    try:
        if args.mode == "node":
            if not shutil.which("node"):
                raise RuntimeError("Node.js is not installed")
            tests = sorted(
                (repo / "crates/dashboard-ui/src/features/harness").glob("*.test.mjs")
            )
            if not tests:
                raise RuntimeError("Harness Node tests are missing")
            run(["node", "--test", *map(str, tests)], repo)
        else:
            if not shutil.which("cargo"):
                raise RuntimeError("Rust cargo is not installed; Rust tests NOT executed")
            if args.mode == "standalone-rust":
                # Diagnostic only: this resolves a new lock and excludes the native
                # host. It cannot substitute for the integrated --locked checks.
                with tempfile.TemporaryDirectory(prefix="anycode-harness-tests-") as temp:
                    root = Path(temp)
                    names = ["harness-core", "harness-extensions", "harness-cloud818"]
                    for name in names:
                        shutil.copytree(repo / "crates" / name, root / "crates" / name)
                    members = ", ".join('"crates/' + n + '"' for n in names)
                    (root / "Cargo.toml").write_text(
                        '[workspace]\nresolver = "2"\nmembers = [' + members + "]\n",
                        encoding="utf-8",
                    )
                    run(["cargo", "test", "--workspace"], root)
            else:
                packages = [arg for package in PACKAGES for arg in ("-p", package)]
                run(["cargo", "test", "--locked", *packages], repo)
                run(
                    ["cargo", "check", "--locked", "-p", "anycode-agent", "--features", "harness-v1"],
                    repo,
                )
                run(["cargo", "clippy", "--locked", *packages, "--all-targets", "--", "-D", "warnings"], repo)
        return 0
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"BLOCKED/FAILED: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
