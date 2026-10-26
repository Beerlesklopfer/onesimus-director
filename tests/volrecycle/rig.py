#!/usr/bin/env python3
"""Volume recycling / autolabel on file volumes (C-085): original bareos-dir
vs odir against PRIVATE bareos-fd/bareos-sd daemons (own config,
ports, storage, working directories), never the live ones and never
bareos_cppref. Reproduces the PDC job 779 situation: expired volumes that
are pruned and recycled, a recycled volume whose file is missing, an Append
volume whose file is missing and an Append volume whose catalog VolBytes is
one byte larger than the file.

  VOLRIG_DIR=/tmp/vr39230 tests/volrecycle/rig.py run SIDE [SCENARIO]

SIDE = cpp | rust. VOLRIG_DIR must be a short path (SD Archive Device names
are limited to 127 characters). Ports: Director VOLRIG_PORT (default 39230),
FD +1, SD +2. The catalog is a throw-away database volrig_<side>_<port>
created from the Bareos DDL and dropped afterwards. Output in
VOLRIG_DIR/out-SIDE: console.txt, media-*.txt, job.log, sd.out (SD debug
level 200: every CatReq the SD sends and the Director's answer).
"""
import base64, json, os, shutil, signal, socket, subprocess, sys, time
from pathlib import Path

HERE = Path(os.environ.get('VOLRIG_DIR', '/tmp/vr39230'))
REPO = Path(__file__).resolve().parents[2]
CAPTURE = REPO / 'target/debug/examples/bnet_capture'
RUST = REPO / 'target/debug/odir'
CPP = Path('/usr/sbin/bareos-dir')
DIRPORT = int(os.environ.get('VOLRIG_PORT', '39230'))
FDPORT, SDPORT = DIRPORT + 1, DIRPORT + 2
ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C', 'TZ': 'UTC', 'NO_COLOR': '1'}
DIRPW = 'volrig-dir-pw'
DATA = HERE / 'data'
WORK = HERE / 'dirwork'
STORE = HERE / 'storage'
MEDIA_SQL = ('SELECT MediaId,VolumeName,PoolId,RecyclePoolId,ScratchPoolId,VolStatus,VolJobs,VolFiles,VolBlocks,VolBytes,VolMounts,VolWrites,'
             'RecycleCount,LabelDate IS NOT NULL,FirstWritten IS NOT NULL,LastWritten IS NOT NULL,Recycle,Enabled '
             'FROM Media ORDER BY MediaId')


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
    fresh(STORE)
    (sd / 'bareos-sd.conf').write_text(f'''
Storage {{ Name = vsd; SD Port = {SDPORT}; SD Address = 127.0.0.1; Working Directory = {HERE}/sd-work; }}
Director {{ Name = bareos-dir; Password = "vsd-pw" }}
Device {{ Name = FileStorage; Media Type = File; Archive Device = {STORE}; LabelMedia = yes; Random Access = yes;
  AutomaticMount = yes; RemovableMedia = no; AlwaysOpen = no; Volume Poll Interval = 15 s; }}
Messages {{ Name = Standard; Director = bareos-dir = all }}
''')


def write_director(db, pool_extra=''):
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
                            '  Device = FileStorage\n  Media Type = File\n}\n',
        'pool/BGLAGE.conf': 'Pool {\n  Name = BGLAGE\n  Pool Type = Backup\n  Label Format = "BGLAGE-"\n  Recycle = yes\n'
                            '  AutoPrune = yes\n  Volume Retention = 30 days\n  Maximum Volume Jobs = 1\n' + pool_extra + '}\n',
        'pool/Scratch.conf': 'Pool {\n  Name = Scratch\n  Pool Type = Scratch\n}\n',
        'fileset/VSet.conf': f'FileSet {{\n  Name = VSet\n  Include {{\n    Options {{\n      Signature = MD5\n    }}\n'
                             f'    File = {DATA}\n  }}\n}}\n',
        'job/Bk.conf': 'Job {\n  Name = Bk\n  Type = Backup\n  Level = Full\n  Client = vfd\n  FileSet = VSet\n  Storage = vsd\n'
                       '  Pool = BGLAGE\n  Messages = Standard\n}\n',
    }
    for rel, text in files.items():
        path = conf / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def make_data():
    fresh(DATA)
    for i in range(3):
        (DATA / f'file{i}.txt').write_text(f'content {i}\n' * (i + 1))


def psql(sql, db='postgres'):
    return subprocess.run(['psql', '-d', db, '-v', 'ON_ERROR_STOP=1', '-Atc', sql], capture_output=True, text=True)


def make_db(side):
    db = f'volrig_{side}_{DIRPORT}'
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
        text = b''.join(base64.b64decode(f['data']) for f in frames if 'data' in f).decode(errors='replace')
        out.append(f'>>> {step["send"]}\n{text}')
    return out


def snapshot(db, out_dir, name):
    media = psql(MEDIA_SQL, db)
    sizes = '\n'.join(f'{p.name} {p.stat().st_size}' for p in sorted(STORE.iterdir()))
    (out_dir / f'media-{name}.txt').write_text(media.stdout + media.stderr + '--- files\n' + sizes + '\n')


def vol(n):
    return f'BGLAGE-{n:04d}'


def manipulate_pdc(db):
    """0001 Used, file deleted (recycled, file missing: PDC 0010/0015);
    0002 Used, file kept (recycled normally); 0004 Append with catalog
    VolBytes one byte larger than the file (PDC 0013 'Catalog=207');
    0005 Append, file deleted (PDC 0018 'not on device')."""
    time.sleep(2)
    psql('UPDATE Media SET VolRetention=1', db)
    (STORE / vol(1)).unlink()
    psql(f"UPDATE Media SET VolStatus='Append',VolJobs=0 WHERE VolumeName='{vol(4)}'", db)
    psql(f"UPDATE Media SET VolBytes=VolBytes+1 WHERE VolumeName='{vol(4)}'", db)
    psql(f"UPDATE Media SET VolStatus='Append',VolJobs=0 WHERE VolumeName='{vol(5)}'", db)
    (STORE / vol(5)).unlink()


def manipulate_recycle_missing(db):
    """PDC 0015: every volume Used and expired, every file missing."""
    time.sleep(2)
    psql('UPDATE Media SET VolRetention=1', db)
    for p in STORE.iterdir():
        p.unlink()


# name: (setup jobs, manipulation, extra BGLAGE pool directives)
SCENARIOS = {
    'pdc': (5, manipulate_pdc, ''),
    'missing': (2, manipulate_recycle_missing, ''),
    # six failing mounts: the SD gives up autolabelling (mount.cc retry > 4)
    'six': (6, manipulate_recycle_missing, '  Maximum Volumes = 6\n'),
    # the same with a RecyclePool: purged volumes move to Scratch
    # seven: the 6th failing mount is PDC's BGLAGE-0015 (recycled, file
    # missing, no autolabel any more, polled every Volume Poll Interval)
    'seven': (7, manipulate_recycle_missing, '  Maximum Volumes = 7\n'),
    'rpool': (6, manipulate_recycle_missing, '  Maximum Volumes = 6\n  Recycle Pool = Scratch\n  Scratch Pool = Scratch\n'),
}


def run(side, scenario):
    HERE.mkdir(parents=True, exist_ok=True)
    out_dir = fresh(HERE / f'out-{side}-{scenario}')
    make_data()
    write_daemons()
    db = make_db(side)
    setup_jobs, manipulate, pool_extra = SCENARIOS[scenario]
    write_director(db, pool_extra)
    procs = []
    transcript = []
    try:
        fd = subprocess.Popen(['/usr/sbin/bareos-fd', '-f', '-c', str(HERE / 'fd/bareos-fd.conf')], env=ENV,
                              stdout=open(out_dir / 'fd.out', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
        procs.append(fd)
        sd = subprocess.Popen(['/usr/sbin/bareos-sd', '-f', '-d', '200', '-c', str(HERE / 'sd/bareos-sd.conf')], env=ENV,
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
        for _ in range(setup_jobs):
            transcript += console('run job=Bk yes', 'wait')
        snapshot(db, out_dir, '1-setup')
        manipulate(db)
        snapshot(db, out_dir, '2-manipulated')
        for i in range(2):
            transcript += console('run job=Bk yes')
            end = time.time() + int(os.environ.get('VOLRIG_WAIT', '40'))
            while time.time() < end:
                time.sleep(2)
                running = psql("SELECT count(*) FROM Job WHERE JobStatus IN ('R','C','F','S','m','M','s','j','c','d','t','p','i','a','l','L')", db).stdout.strip()
                if running == '0':
                    break
            transcript += console('status dir', 'messages')
            if running != '0':
                transcript += console('cancel all yes', 'wait')
                time.sleep(3)
            snapshot(db, out_dir, f'3-after-job{i + 1}')
        transcript += console('list volumes', 'list volumes pool=BGLAGE')
        time.sleep(1)
        jobs = psql('SELECT JobId,Name,Level,JobStatus,JobFiles,PoolId FROM Job ORDER BY JobId', db)
        (out_dir / 'jobs.txt').write_text(jobs.stdout + jobs.stderr)
        jm = psql('SELECT JobId,MediaId,FirstIndex,LastIndex,StartBlock,EndBlock FROM JobMedia ORDER BY JobMediaId', db)
        (out_dir / 'jobmedia.txt').write_text(jm.stdout + jm.stderr)
    finally:
        (out_dir / 'console.txt').write_text('\n'.join(transcript))
        for p in reversed(procs):
            stop(p)
        for name in ('job.log', 'daemon.log'):
            if (WORK / name).exists():
                shutil.copy(WORK / name, out_dir / name)
        psql(f'DROP DATABASE IF EXISTS "{db}" WITH (FORCE)')


if __name__ == '__main__':
    run(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else 'pdc')
