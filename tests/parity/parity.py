#!/usr/bin/env python3
"""Director parity: original bareos-dir vs. onesimus-dird on identical fixtures.

Both Directors get the same configuration (derived from --source, default
/etc/bareos/bareos-dir.d, plus tests/parity/fixture overrides), the same
catalog database, LC_ALL=C and TZ=UTC. Every case runs in a fresh console
session on each side via the bnet_capture example; the per-step reply bytes
(all data frames, concatenated without trimming), the signal sequence and the
end reason are compared, and SHA-256/byte counts of both sides are recorded.

Configuration changes between commands (e.g. `reload` cases): a case may
carry `config`, a map of paths relative to bareos-dir.d to file content (or
null to remove the file). The files are written right before the case and
restored right after it; `restore_reload: true` then sends one extra `reload`
(outside the comparison) so later cases see the original configuration
again. `log: true` appends the Daemon messages log lines written during the
case as one extra compared step (date/time prefix normalized).

Normalization exists only per case (`normalize`: list of regexes whose
matches are replaced by "<N>") and is written into the report.

Cases with `"isolated": true` (they start jobs) run after all other cases in
a second Director start per side, so their jobs never show up in the other
cases (status dir, list jobs).

The baseline (tests/parity/baseline.json) is a ratchet: a case recorded as
"match" must keep matching, otherwise the run fails. Use --update-baseline
to record newly matching cases.

  tests/parity/parity.py run [--cases FILE...] [--work DIR] [--update-baseline]
  tests/parity/parity.py setup --work DIR     # only write configs
"""

import argparse
import base64
import difflib
import hashlib
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT / 'tests/parity'
CAPTURE = ROOT / 'target/debug/examples/bnet_capture'
RUST_DIR = ROOT / 'target/debug/onesimus-dird'
CPP_DIR = Path('/usr/sbin/bareos-dir')
PASSWORD = 'parity-dir-pw'
ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C', 'TZ': 'UTC', 'NO_COLOR': '1'}
SIDES = ('cpp', 'rust')
# Messages resource `append` destinations in the working directory
DAEMON_LOGS = ('bareos-audit.log', 'bareos.log')
AUDIT_TIMESTAMP = r'^\d\d-\w{3} \d\d:\d\d'
# Audit-log comparison only: after a reload that fails in CheckCatalog
# (C-075) the original formats the date with a freed buffer and writes
# garbage (random bytes, e.g. "P!\a0\xfd~", "sect571r1" or nothing) instead of
# "dd-Mon hh:mm" from then on; any space-free prefix before "<name>: " counts.
AUDIT_PREFIX = r'^(?:\d\d-\w{3} \d\d:\d\d|[^\n ]*(?= [^\s:]+: ))'


def write_config(source, target, port, args):
    """Copy the source config and apply the deterministic fixture overrides."""
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target, ignore=shutil.ignore_patterns('*.example'))
    # Fresh working directory per start: a leftover .conmsg/state file from an
    # earlier run would make one side announce "You have messages."
    work = target.parent / 'work'
    if work.exists():
        shutil.rmtree(work)
    work.mkdir()
    for path in (target / 'director').glob('*.conf'):
        path.unlink()
    (target / 'director/bareos-dir.conf').write_text(
        'Director {\n  Name = bareos-dir\n  QueryFile = "/usr/lib/bareos/scripts/query.sql"\n'
        f'  Maximum Concurrent Jobs = 10\n  Password = "{PASSWORD}"\n  Messages = Daemon\n'
        f'  Auditing = yes\n  Working Directory = {work}\n  DirPort = {port}\n}}\n')
    for path in (target / 'catalog').glob('*.conf'):
        path.unlink()
    (target / 'catalog/MyCatalog.conf').write_text(
        f'Catalog {{\n  Name = MyCatalog\n  dbname = "{args.db}"\n  dbuser = "{args.db_user}"\n'
        '  dbpassword = ""\n}\n')
    # Mail/operator delivery would fork bsmtp; console delivery of daemon
    # messages makes replies depend on timing. Neither is part of a case.
    for path in (target / 'messages').glob('*.conf'):
        lines = []
        for line in path.read_text().splitlines():
            key = line.strip().split('=')[0].strip().lower()
            if key in ('mailcommand', 'operatorcommand', 'mail', 'operator', 'mailonerror', 'mailonsuccess'):
                continue
            if 'Name = Daemon' in path.read_text() and key == 'console':
                continue
            line = re.sub(r'"/var/log/bareos/([^"]+)"', lambda m: f'"{work / m[1]}"', line)
            lines.append(line)
        path.write_text('\n'.join(lines) + '\n')
    # A scheduler firing during a run would start real jobs in the shared
    # catalog (seen at 21:00 UTC). Disable every schedule on both sides.
    for path in (target / 'schedule').glob('*.conf'):
        path.write_text(re.sub(r'(?im)^(\s*Schedule\s*\{)', r'\1\n  Enabled = no', path.read_text()))
    fixture = HERE / 'fixture'
    for path in fixture.rglob('*.conf'):
        destination = target / path.relative_to(fixture)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(path, destination)


def wait_port(port, process, seconds=20):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if process.poll() is not None:
            return False
        try:
            socket.create_connection(('127.0.0.1', port), 0.5).close()
            return True
        except OSError:
            time.sleep(0.2)
    return False


def start(side, config, port, log):
    # bareos-dir -c takes the directory containing bareos-dir.d
    command = [str(CPP_DIR), '-f', '-c', str(config.parent)] if side == 'cpp' \
        else [str(RUST_DIR), '-f', '-c', str(config)]
    process = subprocess.Popen(command, env=ENV, stdout=log, stderr=subprocess.STDOUT,
                               start_new_session=True)
    if not wait_port(port, process):
        stop(process)
        raise RuntimeError(f'{side} Director did not open port {port}; see {log.name}')
    return process


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def capture(port, case):
    name = case.get('console', '*UserAgent*')
    password = case.get('password', PASSWORD)
    steps = list(case.get('setup', [])) + list(case['steps'])
    env = {**ENV, 'CAPTURE_TIMEOUT_MS': str(case.get('timeout_ms', 15000))}
    if 'hello_version' in case:  # console version in the Hello (default: capture tool's)
        env['CAPTURE_HELLO_VERSION'] = case['hello_version']
    result = subprocess.run([str(CAPTURE), f'127.0.0.1:{port}', name, password, *steps], env=env,
                            capture_output=True, timeout=120)
    if result.returncode != 0:
        return {'error': result.stderr.decode(errors='replace').strip() or f'exit {result.returncode}'}
    data = json.loads(result.stdout)
    data['steps'] = data['steps'][len(case.get('setup', [])):]
    if case.get('banner'):  # compare the connection banner instead of replies
        data['steps'] = [{'send': '', 'frames': data['banner'], 'end': 'banner'}]
    return data


def apply_config(config_dir, changes):
    """Write a case's `config` files; returns what restore_config needs."""
    saved = {}
    for rel, content in changes.items():
        path = config_dir / rel
        saved[path] = path.read_bytes() if path.exists() else None
        if content is None:
            path.unlink(missing_ok=True)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
    return saved


def restore_config(saved):
    for path, content in saved.items():
        if content is None:
            path.unlink(missing_ok=True)
        else:
            path.write_bytes(content)


def run_case(port, case, config_dir, work):
    """capture() plus the optional per-case config change and log step."""
    log = work / 'bareos.log'
    offset = log.stat().st_size if log.exists() else 0
    saved = apply_config(config_dir, case.get('config', {}))
    try:
        data = capture(port, case)
    finally:
        restore_config(saved)
        if saved and case.get('restore_reload'):
            capture(port, {'steps': ['reload']})
    if case.get('log') and 'error' not in data:
        time.sleep(0.5)  # daemon messages may be written by another thread
        text = log.read_bytes()[offset:] if log.exists() else b''
        text = re.sub(AUDIT_TIMESTAMP.encode(), b'<T>', text, flags=re.M)
        data['steps'].append({'send': '<log>', 'end': 'log',
                              'frames': [{'data': base64.b64encode(text).decode()}]})
    return data


def reply(step):
    """Concatenated data bytes, signal list and end reason of one step."""
    data = b''.join(base64.b64decode(f['data']) for f in step['frames'] if 'data' in f)
    signals = [f['signal'] for f in step['frames'] if 'signal' in f]
    return data, signals, step['end']


def normalize(data, patterns):
    for pattern in patterns:
        data = re.sub(pattern.encode(), b'<N>', data)
    return data


def compare(case, results, out):
    record = {'id': case['id'], 'steps': case['steps'], 'normalize': case.get('normalize', []),
              'sides': {}, 'status': 'match', 'reasons': []}
    if case.get('note'):
        record['note'] = case['note']
    for side in SIDES:
        if 'error' in results[side]:
            record['status'] = 'error'
            record['reasons'].append(f'{side}: {results[side]["error"]}')
    if record['status'] == 'error':
        return record
    count = max(len(results[side]['steps']) for side in SIDES)
    for index in range(count):
        parts = {}
        for side in SIDES:
            steps = results[side]['steps']
            if index >= len(steps):
                record['status'] = 'diff'
                record['reasons'].append(f'step {index}: {side} has no reply')
                continue
            data, signals, end = reply(steps[index])
            path = out / f'{case["id"]}.{index}.{side}.raw'
            path.write_bytes(data)
            record['sides'].setdefault(side, []).append({
                'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
                'signals': signals, 'end': end, 'raw': str(path.relative_to(out.parent))})
            parts[side] = (normalize(data, record['normalize']), signals, end)
        if len(parts) < 2:
            continue
        (left, lsig, lend), (right, rsig, rend) = parts['cpp'], parts['rust']
        if left != right:
            record['status'] = 'diff'
            record['reasons'].append(f'step {index}: reply bytes differ')
            diff = difflib.unified_diff(left.decode('latin-1').splitlines(True),
                                        right.decode('latin-1').splitlines(True),
                                        f'cpp:{case["id"]}#{index}', f'rust:{case["id"]}#{index}')
            (out / f'{case["id"]}.{index}.diff').write_text(''.join(diff))
        if lsig != rsig:
            record['status'] = 'diff'
            record['reasons'].append(f'step {index}: signals {lsig} != {rsig}')
        if lend != rend:
            record['status'] = 'diff'
            record['reasons'].append(f'step {index}: end {lend} != {rend}')
    return record


# Jobs that may be started for real: they touch neither the live FD nor SD.
SAFE_JOBS = {'parity-admin', 'parity-migrate', 'parity-consolidate',
             # S-22 (cases/s22.jsonl defines them as Type = Admin)
             'parity-s22-nodup', 'parity-s22-cq', 'parity-s22-cr', 'parity-s22-delay', 'parity-s22-rst',
             # Admin jobs with Director-side RunScripts (fixture/job/parity-runscript.conf)
             'parity-admin-rs', 'parity-admin-rs-fail', 'parity-admin-rs-noerr',
             # Copy control job selecting nothing, with Director-side RunScripts (S-24)
             'parity-copy-rs'}


def unsafe_job_start(case):
    """Refuse cases that can start a real job against the live FD/SD.

    Under `.api 2`/`.api json` the original starts `run job=…` and a finished
    restore selection without the yes/mod/no question (incident 17.10.2026:
    a real restore ran). In every mode `yes` on a run/restore does the same.
    Cases that are known to fail before the start set "api_job_safe": true.
    """
    if case.get('api_job_safe'):
        return None
    api2 = False
    pending = None  # (command, job) of the last run/restore dialog
    for step in list(case.get('setup', [])) + list(case['steps']):
        if not isinstance(step, str):
            continue
        words = step.split()
        first = words[0] if words else ''
        mode = re.match(r'\s*\.api\s+(\S+)', step)
        if mode:
            api2 = mode.group(1) in ('2', 'json')
            continue
        if first in ('run', 'restore'):
            job = next((w.split('=', 1)[1] for w in words
                        if w.startswith(('job=', 'restorejob='))), None)
            pending = (first, job)
            if job in SAFE_JOBS:
                continue
            if 'yes' in words or (api2 and ('done' in words or 'all' in words
                                            or (first == 'run' and job))):
                return f'step {step!r} can start a real job'
        elif pending and pending[1] not in SAFE_JOBS:
            if first == 'yes' or (api2 and first == 'done' and pending[0] == 'restore'):
                return f'step {step!r} starts the {pending[0]} job'
    return None


def load_cases(files, selected):
    cases = []
    for file in files:
        for number, line in enumerate(Path(file).read_text().splitlines(), 1):
            if line.strip() and not line.lstrip().startswith('#'):
                try:
                    case = json.loads(line)
                except json.JSONDecodeError as error:
                    sys.exit(f'{file}:{number}: {error}')
                if not selected or any(re.search(s, case['id']) for s in selected):
                    problem = unsafe_job_start(case)
                    if problem:
                        sys.exit(f'{file}:{number}: {case["id"]}: {problem}')
                    cases.append(case)
    ids = [case['id'] for case in cases]
    duplicates = {i for i in ids if ids.count(i) > 1}
    if duplicates:
        sys.exit(f'duplicate case ids: {sorted(duplicates)}')
    return cases


def psql_admin(sql):
    return subprocess.run(['psql', '-d', 'postgres', '-v', 'ON_ERROR_STOP=1', '-Atc', sql],
                          capture_output=True, text=True)


def create_catalog_copy(name, template):
    """CREATE DATABASE … TEMPLATE; retried while the template is briefly in use."""
    for attempt in range(30):
        result = psql_admin(f'CREATE DATABASE "{name}" TEMPLATE "{template}"')
        if result.returncode == 0:
            return
        if 'permission denied' in result.stderr:
            sys.exit(f'cannot create catalog copy: {result.stderr.strip()}\n'
                     'Grant once: sudo -u postgres psql -c \'ALTER ROLE "<user>" CREATEDB\' '
                     'or run with --shared-db (live cases then write into the template).')
        if 'being accessed by other users' not in result.stderr:
            sys.exit(f'cannot create catalog copy {name}: {result.stderr.strip()}')
        time.sleep(1 + attempt % 3)
    sys.exit(f'catalog template {template} stayed in use; stop sessions connected to it')


def drop_catalog_copy(name):
    psql_admin(f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE)')


def setup(args):
    """One configuration for both sides: same paths, port and bytes."""
    write_config(args.source, args.work / 'conf' / 'bareos-dir.d', args.port, args)


def run(args):
    for binary in (CAPTURE, RUST_DIR, CPP_DIR):
        if not binary.exists():
            sys.exit(f'missing {binary} (cargo build -p onesimus-dird && cargo build -p bareos-bnet --example bnet_capture)')
    cases = load_cases(args.cases, args.only)
    out = args.work / 'out'
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    # The sides run one after the other on the identical configuration, so
    # ports, working directory and file names in replies are the same.
    # The Messages destinations live in the working directory, which the
    # next setup() recreates: each side's logs are moved to
    # <work>/<side>-<name> right after its Director stopped.
    work = args.work / 'conf' / 'work'
    results = {}
    template = args.db
    for side in SIDES:
        # Each side works on its own throw-away copy of the catalog template,
        # so neither live cases nor one side's effects can change shared data.
        if not args.shared_db:
            args.db = f'parity_{os.getpid()}'  # same name on both sides: show/use print it
            create_catalog_copy(args.db, template)
        try:
            setup(args)
            # Cases with "isolated": true start jobs: they run after all other
            # cases in a Director of their own (fresh RecentJobResultsList,
            # nothing they leave in the catalog reaches the other cases).
            shared = [case for case in cases if not case.get('isolated')]
            isolated = [case for case in cases if case.get('isolated')]
            results[side] = {}
            with open(args.work / f'{side}.log', 'w') as log:
                config_dir = args.work / 'conf' / 'bareos-dir.d'
                for group in (shared, isolated):
                    if not group:
                        continue
                    if group is isolated:
                        for path in work.iterdir():  # state files, console message queue
                            if path.is_file() and path.name not in DAEMON_LOGS:
                                path.unlink()
                    process = start(side, config_dir, args.port, log)
                    try:
                        results[side].update({case['id']: run_case(args.port, case, config_dir, work)
                                              for case in group})
                    finally:
                        stop(process)
            for name in DAEMON_LOGS:
                target = args.work / f'{side}-{name.removeprefix("bareos-")}'
                if (work / name).exists():
                    (work / name).replace(target)
                else:
                    target.write_bytes(b'')
        finally:
            if not args.shared_db:
                drop_catalog_copy(args.db)
                args.db = template
    records = []
    for case in cases:
        record = compare(case, {side: results[side][case['id']] for side in SIDES}, out)
        records.append(record)
    records.append(compare_audit_log(args.work, out))
    for record in records:
        if args.verbose or record['status'] != 'match':
            print(f'{record["status"]:6} {record["id"]}' + (f'  ({"; ".join(record["reasons"][:2])})' if record['reasons'] else ''))
    return gate(args, records)


def compare_audit_log(work, out):
    """Pseudo case `audit-log`: both sides' audit logs, only the timestamp
    prefix of each message normalized."""
    record = {'id': 'audit-log', 'steps': [], 'normalize': [AUDIT_PREFIX], 'sides': {},
              'status': 'match', 'reasons': []}
    parts = {}
    for side in SIDES:
        path = work / f'{side}-audit.log'
        data = path.read_bytes()
        record['sides'][side] = [{'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
                                  'signals': [], 'end': 'file', 'raw': str(path)}]
        parts[side] = re.sub(AUDIT_PREFIX.encode(), b'<N>', data, flags=re.M)
    if parts['cpp'] != parts['rust']:
        record['status'] = 'diff'
        diff = list(difflib.unified_diff(parts['cpp'].decode('latin-1').splitlines(True),
                                         parts['rust'].decode('latin-1').splitlines(True),
                                         'cpp:audit-log', 'rust:audit-log'))
        changed = sum(line[0] in '+-' and not line.startswith(('+++', '---')) for line in diff)
        record['reasons'].append(f'audit log differs ({changed} lines)')
        (out / 'audit-log.diff').write_text(''.join(diff))
    return record


def gate(args, records):
    versions = {'cpp': subprocess.run([str(CPP_DIR), '--version'], capture_output=True, text=True).stdout.strip(),
                'rust': subprocess.run(['git', '-C', str(ROOT), 'describe', '--always', '--dirty'],
                                       capture_output=True, text=True).stdout.strip()}
    baseline_path = args.baseline
    baseline = json.loads(baseline_path.read_text()) if baseline_path.exists() else {}
    regressions = [r['id'] for r in records if baseline.get(r['id']) == 'match' and r['status'] != 'match']
    improved = [r['id'] for r in records if r['status'] == 'match' and baseline.get(r['id']) != 'match']
    counts = {status: sum(r['status'] == status for r in records) for status in ('match', 'diff', 'error')}
    report = {'versions': versions, 'env': {k: ENV[k] for k in ('LC_ALL', 'TZ', 'NO_COLOR')},
              'counts': counts, 'regressions': regressions, 'improved': improved, 'cases': records}
    (args.work / 'report.json').write_text(json.dumps(report, indent=1))
    lines = [f'# Parity report\n', f'cpp: {versions["cpp"]}  \nrust: {versions["rust"]}\n',
             f'match {counts["match"]} / diff {counts["diff"]} / error {counts["error"]} of {len(records)}\n',
             '| case | status | cpp bytes | rust bytes | reasons |', '| --- | --- | ---: | ---: | --- |']
    for r in records:
        size = lambda side: '+'.join(str(s['bytes']) for s in r['sides'].get(side, [])) or '-'
        lines.append(f'| `{r["id"]}` | {r["status"]} | {size("cpp")} | {size("rust")} | {"; ".join(r["reasons"])} |')
    (args.work / 'report.md').write_text('\n'.join(lines) + '\n')
    print(f'parity: match {counts["match"]}, diff {counts["diff"]}, error {counts["error"]} '
          f'of {len(records)} — report {args.work / "report.md"}')
    if improved:
        print(f'newly matching ({len(improved)}): {" ".join(improved)}')
    if args.update_baseline:
        for r in records:  # never downgrade a recorded match
            if baseline.get(r['id']) != 'match':
                baseline[r['id']] = r['status']
        baseline_path.write_text(json.dumps(dict(sorted(baseline.items())), indent=1) + '\n')
        print(f'baseline updated: {baseline_path}')
    if regressions:
        print(f'REGRESSIONS ({len(regressions)}): {" ".join(regressions)}')
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('action', choices=('run', 'setup'))
    parser.add_argument('--work', type=Path, default=ROOT / 'target/parity')
    parser.add_argument('--source', type=Path, default=Path('/etc/bareos/bareos-dir.d'))
    parser.add_argument('--port', type=int, default=39101)
    parser.add_argument('--db', default=os.environ.get('PARITY_DB', 'bareos_cppref'), help='catalog template')
    parser.add_argument('--shared-db', action='store_true',
                        help='use --db directly instead of a per-side copy of it (no isolation)')
    parser.add_argument('--db-user', default=os.environ.get('PARITY_DB_USER', os.environ.get('USER', 'bareos')))
    parser.add_argument('--cases', nargs='+', default=sorted((HERE / 'cases').glob('*.jsonl')))
    parser.add_argument('--only', nargs='*', default=[], help='regexes selecting case ids')
    parser.add_argument('--baseline', type=Path, default=HERE / 'baseline.json')
    parser.add_argument('--update-baseline', action='store_true')
    parser.add_argument('-v', '--verbose', action='store_true')
    args = parser.parse_args()
    args.work = args.work.resolve()
    if args.action == 'setup':
        setup(args)
        return 0
    return run(args)


if __name__ == '__main__':
    sys.exit(main())
