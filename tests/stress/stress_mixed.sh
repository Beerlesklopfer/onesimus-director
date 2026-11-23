#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only
# SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
# SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
# SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# baReos Mixed Stress-Test: BNET + TLS-PSK + gRPC + DDoS
set -euo pipefail

BAREOS_BUILD=${BAREOS_BUILD:-/home/INTRANET.BERNAU.FAMILY/Joerg.Bernau/Dokumente/Projekte/bareos/build/core/src}
DIRD=${DIRD:-./target/release/bareos-dird}
BC=${BC:-$BAREOS_BUILD/console/bconsole}
RBC=${RBC:-./target/release/bconsole}
DIRD_CONFIG=${DIRD_CONFIG:-/tmp/bd-stress-mixed.conf}
BCONF=${BCONF:-/tmp/bareos-systemtest/bconsole.conf}
TCONF=${TCONF:-/tmp/bc-tls-test.conf}

export LD_LIBRARY_PATH=$BAREOS_BUILD/lib:$BAREOS_BUILD/findlib:$BAREOS_BUILD/fastlz:$BAREOS_BUILD/lmdb:$BAREOS_BUILD/ndmp:$BAREOS_BUILD/cats:$BAREOS_BUILD/stored

pkill -9 bareos-dird 2>/dev/null || true; sleep 2
$DIRD -c $DIRD_CONFIG 2>/dev/null &
DIRD_PID=$!; sleep 2

get_mem() { ps -o rss= -p $DIRD_PID 2>/dev/null | tr -d ' '; }
get_load() { awk '{print $1}' /proc/loadavg; }

echo "| Test | N | BNET-OK | TLS-OK | gRPC-OK | FAIL | Dauer | Load | Mem(KB) | Dir |"
echo "|------|---|---------|--------|---------|------|-------|------|---------|-----|"

# Pure BNET
for N in 64 256 1024; do
  T=$(date +%s%3N); PIDS=""
  for i in $(seq 1 $N); do echo "version" | timeout 15 $BC -c $BCONF >/dev/null 2>&1 & PIDS="$PIDS $!"; done
  OK=0; FAIL=0; for p in $PIDS; do wait $p 2>/dev/null && OK=$((OK+1)) || FAIL=$((FAIL+1)); done
  echo "| BNET | $N | $OK | - | - | $FAIL | $(($(date +%s%3N)-T))ms | $(get_load) | $(get_mem) | $(kill -0 $DIRD_PID 2>/dev/null && echo Y || echo N) |"
  sleep 1
done

# Pure TLS-PSK
for N in 64 256; do
  T=$(date +%s%3N); PIDS=""
  for i in $(seq 1 $N); do echo "version" | timeout 15 $BC -c $TCONF >/dev/null 2>&1 & PIDS="$PIDS $!"; done
  OK=0; FAIL=0; for p in $PIDS; do wait $p 2>/dev/null && OK=$((OK+1)) || FAIL=$((FAIL+1)); done
  echo "| TLS | $N | - | $OK | - | $FAIL | $(($(date +%s%3N)-T))ms | $(get_load) | $(get_mem) | $(kill -0 $DIRD_PID 2>/dev/null && echo Y || echo N) |"
  sleep 1
done

# Pure gRPC
for N in 64 256; do
  T=$(date +%s%3N); PIDS=""
  for i in $(seq 1 $N); do timeout 15 $RBC --grpc -a 127.0.0.1 -p 20101 -e "version" >/dev/null 2>&1 & PIDS="$PIDS $!"; done
  OK=0; FAIL=0; for p in $PIDS; do wait $p 2>/dev/null && OK=$((OK+1)) || FAIL=$((FAIL+1)); done
  echo "| gRPC | $N | - | - | $OK | $FAIL | $(($(date +%s%3N)-T))ms | $(get_load) | $(get_mem) | $(kill -0 $DIRD_PID 2>/dev/null && echo Y || echo N) |"
  sleep 1
done

# Mixed
for N in 64 256; do
  THIRD=$((N/3)); T=$(date +%s%3N); BP=""; TP=""; GP=""
  for i in $(seq 1 $THIRD); do
    echo "version" | timeout 15 $BC -c $BCONF >/dev/null 2>&1 & BP="$BP $!"
    echo "version" | timeout 15 $BC -c $TCONF >/dev/null 2>&1 & TP="$TP $!"
    timeout 15 $RBC --grpc -a 127.0.0.1 -p 20101 -e "version" >/dev/null 2>&1 & GP="$GP $!"
  done
  BOK=0; TOK=0; GOK=0; FAIL=0
  for p in $BP; do wait $p 2>/dev/null && BOK=$((BOK+1)) || FAIL=$((FAIL+1)); done
  for p in $TP; do wait $p 2>/dev/null && TOK=$((TOK+1)) || FAIL=$((FAIL+1)); done
  for p in $GP; do wait $p 2>/dev/null && GOK=$((GOK+1)) || FAIL=$((FAIL+1)); done
  echo "| MIX | $N | $BOK | $TOK | $GOK | $FAIL | $(($(date +%s%3N)-T))ms | $(get_load) | $(get_mem) | $(kill -0 $DIRD_PID 2>/dev/null && echo Y || echo N) |"
  sleep 1
done

# DDoS
for N in 1000 5000; do
  T=$(date +%s%3N); PIDS=""
  for i in $(seq 1 $N); do (echo "" | timeout 2 nc 127.0.0.1 19101 >/dev/null 2>&1) & PIDS="$PIDS $!"; done
  for p in $PIDS; do wait $p 2>/dev/null; done
  echo "| DDoS | $N | - | - | - | - | $(($(date +%s%3N)-T))ms | $(get_load) | $(get_mem) | $(kill -0 $DIRD_PID 2>/dev/null && echo Y || echo N) |"
  sleep 1
done

echo ""; echo "Final: $(kill -0 $DIRD_PID 2>/dev/null && echo ALIVE || echo CRASHED) Mem=$(get_mem)KB"
kill $DIRD_PID 2>/dev/null; wait $DIRD_PID 2>/dev/null
