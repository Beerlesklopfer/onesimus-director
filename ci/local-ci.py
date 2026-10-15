#!/usr/bin/env python3
"""Local CI for onesimus-dird: every check is a separate step.

A failing step is reported, never a reason to stop: all steps always run, so
each one can be monitored over time. Steps:

  build:*         cargo build of the workspace, Director and capture tool
  test:<crate>    cargo test per workspace crate
  py:<file>       Python unit tests (scripts/test_*.py, tests/parity/test_*.py)
  parity:<case>   one step per Director parity case (tests/parity/parity.py)

Results go to target/ci/<run>/ (summary.json, summary.md, one log per step),
target/ci/latest -> <run>, and one line per run in target/ci/history.jsonl.
Exit code is 0 unless --strict is given (then 1 if any step failed).

  ci/local-ci.py [--only REGEX...] [--skip-parity] [--strict]
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/ci'


class Run:
    def __init__(self, directory, only):
        self.directory, self.only, self.steps = directory, only, []

    def wanted(self, name):
        return not self.only or any(re.search(pattern, name) for pattern in self.only)

    def step(self, name, command, timeout=3600, env=None):
        """Run one command as a step; its status never aborts the run."""
        if not self.wanted(name):
            return None
        log = self.directory / 'logs' / (re.sub(r'[^\w.-]+', '_', name) + '.log')
        started = time.time()
        try:
            result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    timeout=timeout, env={**os.environ, **(env or {})})
            output, status = result.stdout, 'pass' if result.returncode == 0 else 'fail'
            detail = f'exit {result.returncode}'
        except subprocess.TimeoutExpired as error:
            output, status, detail = error.stdout or b'', 'fail', f'timeout after {timeout}s'
        log.write_bytes(output)
        return self.record(name, status, detail, time.time() - started, log)

    def record(self, name, status, detail, seconds, log=None, **extra):
        entry = {'name': name, 'status': status, 'detail': detail, 'seconds': round(seconds, 2),
                 'log': str(log.relative_to(self.directory)) if log else None, **extra}
        self.steps.append(entry)
        print(f'{status.upper():5} {name}  [{detail}, {entry["seconds"]}s]', flush=True)
        return entry


def workspace_crates():
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=ROOT))
    return [package['name'] for package in metadata['packages']]


def parity_steps(run, args):
    """Run the parity matrix once and turn every case into its own step."""
    if not run.wanted('parity:'):
        return
    work = run.directory / 'parity'
    started = time.time()
    command = [sys.executable, str(ROOT / 'tests/parity/parity.py'), 'run', '--work', str(work)]
    if args.only:
        command += ['--only'] + [re.sub(r'^parity:', '', pattern) for pattern in args.only if pattern.startswith('parity:')]
    log = run.directory / 'logs' / 'parity-run.log'
    result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log.write_bytes(result.stdout)
    report_path = work / 'report.json'
    if not report_path.exists():
        run.record('parity:run', 'fail', 'no report (harness failed, see log)', time.time() - started, log)
        return
    report = json.loads(report_path.read_text())
    run.record('parity:run', 'pass', f'{report["counts"]}', time.time() - started, log)
    regressions = set(report['regressions'])
    for case in report['cases']:
        status = 'pass' if case['status'] == 'match' else 'fail'
        detail = '; '.join(case['reasons'][:3]) or 'byte-identical'
        if case['id'] in regressions:
            detail = 'REGRESSION: ' + detail
        run.record(f'parity:{case["id"]}', status, detail, 0, None,
                   regression=case['id'] in regressions,
                   sha256={side: [s['sha256'] for s in steps] for side, steps in case['sides'].items()})


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--only', nargs='*', default=[], help='regexes selecting step names')
    parser.add_argument('--skip-parity', action='store_true')
    parser.add_argument('--strict', action='store_true', help='exit 1 if any step failed')
    args = parser.parse_args()

    stamp = datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
    directory = OUT / stamp
    (directory / 'logs').mkdir(parents=True)
    run = Run(directory, args.only)
    started = time.time()

    run.step('build:workspace', ['cargo', 'build', '--workspace'])
    run.step('build:capture-tool', ['cargo', 'build', '-p', 'bareos-bnet', '--example', 'bnet_capture'])
    for crate in workspace_crates():
        run.step(f'test:{crate}', ['cargo', 'test', '-p', crate])
    for test in sorted(ROOT.glob('scripts/test_*.py')) + sorted(ROOT.glob('tests/parity/test_*.py')):
        run.step(f'py:{test.relative_to(ROOT)}', [sys.executable, str(test)], timeout=900)
    if not args.skip_parity:
        parity_steps(run, args)

    counts = {status: sum(step['status'] == status for step in run.steps) for status in ('pass', 'fail')}
    commit = subprocess.run(['git', 'describe', '--always', '--dirty'], cwd=ROOT, capture_output=True,
                            text=True).stdout.strip()
    summary = {'run': stamp, 'commit': commit, 'seconds': round(time.time() - started, 1),
               'counts': counts, 'steps': run.steps}
    (directory / 'summary.json').write_text(json.dumps(summary, indent=1))
    lines = [f'# Local CI {stamp}', '', f'commit `{commit}` — pass {counts["pass"]}, fail {counts["fail"]}', '',
             '| step | status | detail | s |', '| --- | --- | --- | ---: |']
    lines += [f'| `{s["name"]}` | {s["status"]} | {s["detail"].replace("|", "/")} | {s["seconds"]} |' for s in run.steps]
    (directory / 'summary.md').write_text('\n'.join(lines) + '\n')
    with open(OUT / 'history.jsonl', 'a') as history:
        history.write(json.dumps({'run': stamp, 'commit': commit, 'counts': counts,
                                  'steps': {s['name']: s['status'] for s in run.steps}}) + '\n')
    latest = OUT / 'latest'
    if latest.is_symlink() or latest.exists():
        latest.unlink()
    latest.symlink_to(stamp)
    print(f'\nlocal CI: pass {counts["pass"]}, fail {counts["fail"]} — {directory / "summary.md"}')
    return 1 if args.strict and counts['fail'] else 0


if __name__ == '__main__':
    sys.exit(main())
