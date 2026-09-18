#!/usr/bin/env python3
"""Run real checks. Missing tools are BLOCKED, never a successful skipped build."""
from __future__ import annotations
import argparse, shutil, subprocess, sys, tempfile
from pathlib import Path

def run(command:list[str],cwd:Path)->None:
    print('+',' '.join(command),flush=True)
    subprocess.run(command,cwd=cwd,check=True)

def main()->int:
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--mode',choices=['node','standalone-rust','integrated-rust'],required=True)
    p.add_argument('--repo',type=Path,default=Path(__file__).resolve().parents[2]);args=p.parse_args();repo=args.repo.resolve()
    try:
        if args.mode=='node':
            if not shutil.which('node'):raise RuntimeError('Node.js is not installed')
            tests = sorted((repo/'crates/dashboard-ui/src/features/harness').glob('*.test.mjs'))
            if not tests: raise RuntimeError('Harness Node tests are missing')
            run(['node','--test',*[str(test) for test in tests]],repo)
        else:
            if not shutil.which('cargo'):raise RuntimeError('Rust cargo is not installed; Rust tests NOT executed')
            if args.mode=='standalone-rust':
                with tempfile.TemporaryDirectory(prefix='anycode-harness-tests-') as temp:
                    root=Path(temp);names=['harness-core','harness-extensions','harness-cloud818']
                    for name in names:shutil.copytree(repo/'crates'/name,root/'crates'/name)
                    members=', '.join('"crates/'+n+'"' for n in names)
                    (root/'Cargo.toml').write_text('[workspace]\nresolver = "2"\nmembers = ['+members+']\n',encoding='utf-8')
                    run(['cargo','test','--workspace'],root)
            else:
                run(['cargo','test','-p','anycode-harness-core','-p','anycode-harness-extensions','-p','anycode-harness-cloud818','-p','anycode-harness-host'],repo)
                run(['cargo','check','-p','anycode-agent','--features','harness-v1'],repo)
        return 0
    except (RuntimeError,OSError,subprocess.CalledProcessError) as e:
        print(f'BLOCKED/FAILED: {e}',file=sys.stderr);return 2
if __name__=='__main__':raise SystemExit(main())
