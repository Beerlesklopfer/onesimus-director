"""Normalized diff of the rig.py outputs (out-cpp vs out-rust)."""
import re, sys, difflib
from pathlib import Path

import os
HERE = Path(os.environ.get('VERIFY_RIG_DIR', '/tmp/verify-rig'))


def norm(t):
    t = re.sub(r'^\d\d-\w{3} \d\d:\d\d ', '', t, flags=re.M)
    t = re.sub(r'\d{4}-\d\d-\d\d_\d\d\.\d\d\.\d\d_\d+', '<JOB>', t)
    t = re.sub(r'25\.1\.2\S*', '<VER>', t)
    t = re.sub(r'\(\d\d \w+ \d{4}\)|\(\d\d\w{3}\d\d\)', '(<DATE>)', t)
    t = re.sub(r'(time:\s+)\d\d-\w{3}-\d{4} \d\d:\d\d:\d\d', r'\1<T>', t)
    t = re.sub(r'(Bareos binary info:\s+).*', r'\1<BIN>', t)
    t = re.sub(r'Volume Session Time:\s+\d+', 'Volume Session Time: <N>', t)
    return t


def show(name, a, b):
    d = list(difflib.unified_diff(a.splitlines(), b.splitlines(), 'cpp', 'rust', lineterm='', n=1))
    print(f'== {name}: {"SAME" if not d else "DIFF"}')
    print('\n'.join(d))


show('job.log', norm((HERE / 'out-cpp/job.log').read_text()), norm((HERE / 'out-rust/job.log').read_text()))
cut = lambda t: '\n'.join('|'.join(l.split('|')[:1] + l.split('|')[2:]) for l in t.splitlines())
show('jobs', cut((HERE / 'out-cpp/jobs.txt').read_text()), cut((HERE / 'out-rust/jobs.txt').read_text()))
show('files', (HERE / 'out-cpp/files.txt').read_text(), (HERE / 'out-rust/files.txt').read_text())
