#!/usr/bin/env python3
"""One-time, hash-guarded integration on the review branch. Never pushes main."""
from __future__ import annotations
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EDITS = ROOT / 'tools/harness/merge-edits.json'
ORIGINALS = {'Cargo.toml', 'crates/agent/Cargo.toml', 'crates/agent/src/lib.rs', 'crates/agent/src/runtime/mod.rs'}
CRATES = ('harness-core', 'harness-extensions', 'harness-host', 'harness-cloud818')

def run(*args: str) -> None:
    subprocess.run(args, cwd=ROOT, check=True)

def main() -> None:
    edits = json.loads(EDITS.read_text(encoding='utf-8'))
    if {e['path'] for e in edits} != ORIGINALS or len(edits) != len(ORIGINALS):
        raise SystemExit('Unexpected integration file set')
    plans = []
    for edit in edits:
        path = ROOT / edit['path']
        data = path.read_bytes()
        text = data.decode('utf-8')
        if edit['after'] in text:
            continue
        blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
        if blob != edit['blob'] or text.count(edit['before']) != 1:
            raise SystemExit('Source baseline changed: ' + edit['path'])
        plans.append((path, text.replace(edit['before'], edit['after'], 1)))
    package = ROOT / 'crates/dashboard-ui/package.json'
    pkg = json.loads(package.read_text(encoding='utf-8'))
    scripts = pkg['scripts']
    expected = 'node --test src/features/harness/*.test.mjs'
    if scripts.get('test:harness') not in (None, expected):
        raise SystemExit('Unexpected test:harness script; review before replacing')
    if scripts['test'] not in ('vitest run', 'vitest run && npm run test:harness'):
        raise SystemExit('Unexpected existing test command')
    scripts['test:harness'] = expected
    scripts['test'] = 'vitest run && npm run test:harness'
    for path, text in plans:
        path.write_text(text, encoding='utf-8')
    package.write_text(json.dumps(pkg, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    files = sorted(str(p.relative_to(ROOT)) for name in CRATES for p in (ROOT/'crates'/name).rglob('*.rs'))
    files += ['crates/agent/src/lib.rs', 'crates/agent/src/runtime/mod.rs', 'crates/agent/src/runtime/harness_bridge.rs']
    run('rustfmt', '--edition', '2021', '--config', 'skip_children=true', *files)
    # Resolve against the existing lock rather than deleting/regenerating everything.
    subprocess.run(['cargo', 'metadata', '--format-version', '1'], cwd=ROOT, stdout=subprocess.DEVNULL, check=True)
    changed = subprocess.check_output(['git', 'diff', '--name-only'], cwd=ROOT, text=True).splitlines()
    allowed = ORIGINALS | {'Cargo.lock', 'crates/dashboard-ui/package.json', 'crates/agent/src/runtime/harness_bridge.rs'} | set(files)
    if any(path not in allowed for path in changed):
        raise SystemExit('Normalization touched an unexpected path: ' + repr(changed))
    run('git', 'diff', '--check')
    print('Prepared files:', ', '.join(changed))

if __name__ == '__main__':
    main()
