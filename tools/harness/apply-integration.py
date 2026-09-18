#!/usr/bin/env python3
"""One-time, exact-baseline wiring for the reviewed v1.1 source import.

Never resets, cleans, stashes, deploys, or writes a branch ref. Existing unrelated
changes cause a hard failure. Rerunning on exactly applied source is a no-op.
"""
from __future__ import annotations
import hashlib
import json
import subprocess
from pathlib import Path

BASE = "0411ea3a94fe6aa2d37326f9342cfc5d6a13f4aa"
ROOT = Path(__file__).resolve().parents[2]


def blob(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def main() -> None:
    edits = json.loads(Path(__file__).with_name("integration-edits.json").read_text())
    plan = []
    for edit in edits:
        path = ROOT / edit["path"]
        if path.is_symlink() or not path.is_file():
            raise SystemExit(f"Refusing non-regular source: {edit['path']}")
        original = subprocess.check_output(["git", "show", f"{BASE}:{edit['path']}"], cwd=ROOT)
        if blob(original) != edit["blob"]:
            raise SystemExit(f"Baseline hash mismatch: {edit['path']}")
        before, after = edit["before"].encode(), edit["after"].encode()
        if original.count(before) != 1:
            raise SystemExit(f"Ambiguous source anchor: {edit['path']}")
        expected = original.replace(before, after, 1)
        current = path.read_bytes()
        if current == expected:
            continue
        if current != original:
            raise SystemExit(f"Local source diverged; review instead of overwrite: {edit['path']}")
        plan.append((path, original, expected))
    # Validate all files again before the first write.
    if any(path.read_bytes() != original for path, original, _ in plan):
        raise SystemExit("Source changed during preflight")
    for path, _, expected in plan:
        path.write_bytes(expected)
        print(f"Applied exact integration: {path.relative_to(ROOT)}")
    print(f"Wiring ready: {len(plan)} file(s) changed; harness-v1 remains opt-in")


if __name__ == "__main__":
    main()
