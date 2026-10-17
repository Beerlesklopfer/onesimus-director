#!/usr/bin/env python3
"""Verify jobs (S-04, C-058): original bareos-dir vs onesimus-dird against
PRIVATE bareos-fd/bareos-sd daemons (own config, ports, storage, working
directories), never the live ones and never bareos_cppref.

  VERIFY_RIG_DIR=/tmp/v39191 tests/verify/rig.py run SIDE [SCENARIO]
  VERIFY_RIG_DIR=/tmp/v39191 tests/verify/compare.py

SIDE = cpp | rust; SCENARIO = all (default) | data. VERIFY_RIG_DIR must be a
short path (SD Archive Device names are limited to 127 characters). Ports:
Director VERIFY_RIG_PORT (default 39191), FD +1, SD +2. The catalog is a
throw-away database verify_<side>_<port> created from the Bareos DDL.
STORAGE_EXTRA="" keeps the Storage Maximum Concurrent Jobs default (1): a
Level=Data job then waits for its storage forever on both Directors.
"""
import json, os, shutil, signal, socket, subprocess, sys, time
from pathlib import Path

HERE = Path(os.environ.get('VERIFY_RIG_DIR', '/tmp/verify-rig'))
REPO = Path(__file__).resolve().parents[2]
CAPTURE = REPO / 'target/debug/examples/bnet_capture'
RUST = REPO / 'target/debug/onesimus-dird'
CPP = Path('/usr/sbin/bareos-dir')
DIRPORT = int(os.environ.get('VERIFY_RIG_PORT', '39191'))
FDPORT, SDPORT = DIRPORT + 1, DIRPORT + 2
ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C', 'TZ': 'UTC', 'NO_COLOR': '1'}
DIRPW = 'verify-dir-pw'
DATA = HERE / 'data'
WORK = HERE / 'dirwork'
STORAGE_EXTRA = os.environ.get('STORAGE_EXTRA', '  Maximum Concurrent Jobs = 10\n')


def wait_port(port, proc, seconds=20):
    end = time.time() + seconds
    while time.time() < end:
        if proc is not None and proc.poll() is not None:
            return False
        try:
            socket.create_connection(('127.0.0.1', port), 0.5).close()
            return True
        except OSError:
            time.sleep(0.2)
    return False


def stop(proc):
    if proc and proc.poll() is None:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()


def fresh(path):
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    return path


def write_daemons():
    fd = fresh(HERE / 'fd')
    fresh(HERE / 'fd-work')
    (fd / 'bareos-fd.conf').write_text(f'''
Client {{ Name = vfd; FD Port = {FDPORT}; FD Address = 127.0.0.1; Working Directory = {HERE}/fd-work; }}
Director {{ Name = bareos-dir; Password = "vfd-pw" }}
Messages {{ Name = Standard; Director = bareos-dir = all, !skipped, !restored }}
''')
    sd = fresh(HERE / 'sd')
    fresh(HERE / 'sd-work')
    fresh(HERE / 'storage')
    (sd / 'bareos-sd.conf').write_text(f'''
Storage {{ Name = vsd; SD Port = {SDPORT}; SD Address = 127.0.0.1; Working Directory = {HERE}/sd-work; }}
Director {{ Name = bareos-dir; Password = "vsd-pw" }}
Device {{ Name = VDev; Media Type = VFile; Archive Device = {HERE}/storage; LabelMedia = yes; Random Access = yes;
  AutomaticMount = yes; RemovableMedia = no; AlwaysOpen = no; }}
Messages {{ Name = Standard; Director = bareos-dir = all }}
''')


def write_director(db):
    conf = fresh(HERE / 'dir' / 'bareos-dir.d')
    fresh(WORK)
    files = {
        'director/bareos-dir.conf': f'Director {{\n  Name = bareos-dir\n  Password = "{DIRPW}"\n  Messages = Daemon\n'
                                    f'  Working Directory = {WORK}\n  DirPort = {DIRPORT}\n  Maximum Concurrent Jobs = 10\n'
                                    '  QueryFile = "/usr/lib/bareos/scripts/query.sql"\n}\n',
        'catalog/MyCatalog.conf': f'Catalog {{\n  Name = MyCatalog\n  dbname = "{db}"\n  dbuser = "{os.environ["USER"]}"\n  dbpassword = ""\n}}\n',
        'messages/Daemon.conf': f'Messages {{\n  Name = Daemon\n  append = "{WORK}/daemon.log" = all, !skipped, !saved, !audit\n}}\n',
        'messages/Standard.conf': f'Messages {{\n  Name = Standard\n  append = "{WORK}/job.log" = all, !skipped, !saved, !audit\n'
                                  '  catalog = all, !skipped, !saved, !audit\n}\n',
        'client/vfd.conf': f'Client {{\n  Name = vfd\n  Address = 127.0.0.1\n  FD Port = {FDPORT}\n  Password = "vfd-pw"\n}}\n',
        'storage/vsd.conf': f'Storage {{\n  Name = vsd\n  Address = 127.0.0.1\n  SD Port = {SDPORT}\n  Password = "vsd-pw"\n'
                            '  Device = VDev\n  Media Type = VFile\n' + STORAGE_EXTRA + '}\n',
        'pool/VPool.conf': 'Pool {\n  Name = VPool\n  Pool Type = Backup\n  Label Format = "VVol-"\n  Maximum Volume Bytes = 1G\n}\n',
        'fileset/VSet.conf': f'FileSet {{\n  Name = VSet\n  Include {{\n    Options {{\n      Signature = MD5\n      Verify = pins5\n    }}\n'
                             f'    File = {DATA}\n  }}\n}}\n',
        'fileset/VSet2.conf': f'FileSet {{\n  Name = VSet2\n  Include {{\n    Options {{\n      Signature = MD5\n    }}\n'
                              f'    File = {DATA}\n  }}\n}}\n',
        'jobdefs/VDefs.conf': 'JobDefs {\n  Name = VDefs\n  Client = vfd\n  FileSet = VSet\n  Storage = vsd\n  Pool = VPool\n'
                              '  Messages = Standard\n  Priority = 10\n}\n',
    }
    jobs = {
        'VBackup': 'Type = Backup\n  Level = Full',
        'VInit': 'Type = Verify\n  Level = InitCatalog',
        'VCat': 'Type = Verify\n  Level = Catalog',
        'VCatNoInit': 'Type = Verify\n  Level = Catalog',
        'VVol': 'Type = Verify\n  Level = VolumeToCatalog\n  Verify Job = VBackup',
        'VVolAny': 'Type = Verify\n  Level = VolumeToCatalog',
        'VDisk': 'Type = Verify\n  Level = DiskToCatalog',
        'VData': 'Type = Verify\n  Level = Data',
    }
    for name, body in jobs.items():
        files[f'job/{name}.conf'] = f'Job {{\n  Name = {name}\n  JobDefs = VDefs\n  {body}\n}}\n'
    for rel, text in files.items():
        path = conf / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def make_data():
    fresh(DATA)
    (DATA / 'sub').mkdir()
    for i in range(3):
        (DATA / f'file{i}.txt').write_text(f'content {i}\n' * (i + 1))
    (DATA / 'sub' / 'inner.txt').write_text('inner\n')
    for p in sorted(DATA.rglob('*')):
        os.utime(p, (1700000000, 1700000000))
    os.utime(DATA, (1700000000, 1700000000))


def psql(sql, db='postgres'):
    return subprocess.run(['psql', '-d', db, '-v', 'ON_ERROR_STOP=1', '-Atc', sql], capture_output=True, text=True)


def make_db(side):
    db = f'verify_{side}_{DIRPORT}'
    psql(f'DROP DATABASE IF EXISTS "{db}" WITH (FORCE)')
    r = psql(f"CREATE DATABASE \"{db}\" ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0")
    if r.returncode:
        sys.exit(r.stderr)
    r = subprocess.run(['psql', '-q', '-d', db, '-v', 'ON_ERROR_STOP=1', '-f', '/usr/lib/bareos/scripts/ddl/creates/postgresql.sql'],
                       capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stderr)
    return db


def console(*steps, timeout=60000):
    env = {**ENV, 'CAPTURE_TIMEOUT_MS': str(timeout)}
    r = subprocess.run([str(CAPTURE), f'127.0.0.1:{DIRPORT}', '*UserAgent*', DIRPW, *steps], env=env,
                       capture_output=True, timeout=600)
    if r.returncode:
        return [f'ERROR {r.stderr.decode(errors="replace")}']
    data = json.loads(r.stdout)
    out = []
    for step in data['steps']:
        frames = step.get('frames', [])
        import base64
        text = b''.join(base64.b64decode(f['data']) for f in frames if 'data' in f).decode(errors='replace')
        out.append(f'>>> {step["send"]}\n{text}')
    return out


SCENARIOS = {
    # (description, python action before, console steps)
    'all': [
        (None, ['run job=VCatNoInit yes', 'wait']),
        (None, ['run job=VVolAny yes', 'wait']),
        (None, ['run job=VBackup yes', 'wait']),
        (None, ['run job=VInit yes', 'wait']),
        (None, ['run job=VCat yes', 'wait']),
        (None, ['run job=VCat level=InitCatalog yes', 'wait']),
        (None, ['run job=VCat yes', 'wait']),
        ('modify', ['run job=VCat yes', 'wait']),
        (None, ['run job=VVol yes', 'wait']),
        (None, ['run job=VDisk yes', 'wait']),
        (None, ['run job=VData yes', 'wait']),
        (None, ['run job=VCat level=DiskToCatalog yes', 'wait']),
        (None, ['run job=VVolAny jobid=3 yes', 'wait']),
    ],
    'data': [
        (None, ['run job=VData yes', 'wait']),
        (None, ['run job=VBackup yes', 'wait']),
        (None, ['run job=VData yes', 'wait', 'status dir']),
    ],
}


def modify():
    (DATA / 'file1.txt').write_text('changed\n')
    (DATA / 'file2.txt').unlink()
    (DATA / 'new.txt').write_text('new\n')
    for p in sorted(DATA.rglob('*')):
        os.utime(p, (1700000000, 1700000000))
    os.utime(DATA, (1700000000, 1700000000))


def run(side, scenario):
    HERE.mkdir(parents=True, exist_ok=True)
    out_dir = fresh(HERE / f'out-{side}')
    make_data()
    write_daemons()
    db = make_db(side)
    write_director(db)
    procs = []
    try:
        fd = subprocess.Popen(['/usr/sbin/bareos-fd', '-f', '-c', str(HERE / 'fd/bareos-fd.conf')], env=ENV,
                              stdout=open(out_dir / 'fd.out', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
        procs.append(fd)
        sd = subprocess.Popen(['/usr/sbin/bareos-sd', '-f', '-c', str(HERE / 'sd/bareos-sd.conf')], env=ENV,
                              stdout=open(out_dir / 'sd.out', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
        procs.append(sd)
        if not (wait_port(FDPORT, fd) and wait_port(SDPORT, sd)):
            sys.exit('daemons did not start')
        cmd = [str(CPP), '-f', '-c', str(HERE / 'dir')] if side == 'cpp' else [str(RUST), '-f', '-c', str(HERE / 'dir/bareos-dir.d')]
        d = subprocess.Popen(cmd, env=ENV, stdout=open(out_dir / 'dir.out', 'w'), stderr=subprocess.STDOUT,
                             start_new_session=True)
        procs.append(d)
        if not wait_port(DIRPORT, d):
            sys.exit('director did not start')
        transcript = []
        for action, steps in SCENARIOS[scenario]:
            if action == 'modify':
                modify()
            transcript += console(*steps)
        time.sleep(1)
        (out_dir / 'console.txt').write_text('\n'.join(transcript))
        jobs = psql('SELECT JobId,Job,Name,Type,Level,JobStatus,JobFiles,JobBytes,ReadBytes,JobErrors,ClientId,PoolId,FileSetId,'
                    'PriorJobId,VolSessionId>0 FROM Job ORDER BY JobId', db)
        (out_dir / 'jobs.txt').write_text(jobs.stdout + jobs.stderr)
        files = psql('SELECT JobId,FileIndex,Path.Path,Name,MarkId,MD5 FROM File JOIN Path USING (PathId) ORDER BY JobId,FileIndex,Name', db)
        (out_dir / 'files.txt').write_text(files.stdout + files.stderr)
        log = psql('SELECT JobId,LogText FROM Log ORDER BY LogId', db)
        (out_dir / 'logtable.txt').write_text(log.stdout + log.stderr)
    finally:
        for p in reversed(procs):
            stop(p)
        for name in ('job.log', 'daemon.log'):
            if (WORK / name).exists():
                shutil.copy(WORK / name, out_dir / name)
        psql(f'DROP DATABASE IF EXISTS "{db}" WITH (FORCE)')


if __name__ == '__main__':
    run(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else 'all')
