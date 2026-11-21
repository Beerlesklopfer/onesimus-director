#!/bin/bash
# Windows test host (docs/plans/PORTIERUNG-MACOS-WINDOWS.md, "Testen auf der VM"): no Wine, work only in D:\odir-test.
# pack the tree, copy it to the VM, rebuild ofd there (detached), wait for build.done
set -e
HOST=${ONESIMUS_WINDOWS_FD:?set ONESIMUS_WINDOWS_FD to the address of the Windows host}
HERE=$(cd "$(dirname "$0")" && pwd)
cd "$HERE/../.."
S=${TMPDIR:-/tmp}
tar czf $S/odir-src.tgz --exclude=./target --exclude=./target-bookworm --exclude=./.git .
scp -o BatchMode=yes "$S/odir-src.tgz" "$HOST:D:/odir-test/odir-src.tgz" >/dev/null
timeout 120 ssh -o BatchMode=yes $HOST 'powershell -NoProfile -Command -' < $HERE/rebuild.ps1 | tail -1
for i in $(seq 1 60); do
  r=$(timeout 60 ssh -o BatchMode=yes -o ConnectTimeout=20 $HOST 'powershell -NoProfile -Command -' < $HERE/poll.ps1 2>&1 | tail -1)
  case "$r" in DONE*) echo "$r"; break;; esac
  sleep 20
done
timeout 60 ssh -o BatchMode=yes $HOST 'powershell -NoProfile -Command -' <<'P' 2>&1 | grep -E "^(error|warning: unused)|error\[" -A7 | head -60
Get-Content D:\odir-test\build.log | Select-String -Pattern '^(error|warning: unused)' -Context 0,7 | ForEach-Object { $_.ToString() }
P
