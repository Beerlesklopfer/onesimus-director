#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only
# SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
# SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
# SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# baReos Stress-Test: Eskalation bis zum Anschlag
# Testet parallele C++ bconsole Verbindungen mit Load/Memory/Thread Metriken

set -euo pipefail

BAREOS_BUILD=${BAREOS_BUILD:-/home/INTRANET.BERNAU.FAMILY/Joerg.Bernau/Dokumente/Projekte/bareos/build/core/src}
DIRD=${DIRD:-./target/release/bareos-dird}
BC=${BC:-$BAREOS_BUILD/console/bconsole}
DIRD_CONFIG=${DIRD_CONFIG:-/tmp/bareos-systemtest/bareos-dir.conf}
BC_CONFIG=${BC_CONFIG:-/tmp/bareos-systemtest/bconsole.conf}

export LD_LIBRARY_PATH=$BAREOS_BUILD/lib:$BAREOS_BUILD/findlib:$BAREOS_BUILD/fastlz:$BAREOS_BUILD/lmdb:$BAREOS_BUILD/ndmp:$BAREOS_BUILD/cats:$BAREOS_BUILD/stored

# Cleanup
pkill -9 bareos-dird 2>/dev/null || true
sleep 2

# Start Director
$DIRD -c $DIRD_CONFIG 2>/dev/null &
DIRD_PID=$!
sleep 2

if ! kill -0 $DIRD_PID 2>/dev/null; then
    echo "ERROR: Director failed to start"
    exit 1
fi

get_mem() { ps -o rss= -p $DIRD_PID 2>/dev/null | tr -d ' '; }
get_load() { awk '{print $1}' /proc/loadavg; }
get_threads() { ls /proc/$DIRD_PID/task 2>/dev/null | wc -l; }

MEM_BASE=$(get_mem)

echo "# baReos Stress-Test Protokoll"
echo ""
echo "**Datum:** $(date '+%Y-%m-%d %H:%M:%S')"
echo "**Director:** baReos-dird (Rust) v0.1.0, PID=$DIRD_PID"
echo "**System:** $(uname -r), $(nproc) CPUs, $(hostname)"
echo "**bconsole:** $BC"
echo "**Config:** $DIRD_CONFIG"
echo ""
echo "| N | OK | FAIL | Dauer | Load | Mem(KB) | Threads | Director |"
echo "|---|-----|------|-------|------|---------|---------|----------|"
echo "| 0 | - | - | - | $(get_load) | $MEM_BASE | $(get_threads) | baseline |"

for N in 32 64 128 256 512 1024 2048 4096 8192 16384; do
    T=$(date +%s%3N)
    PIDS=""
    for i in $(seq 1 $N); do
        echo "version" | timeout 30 $BC -c $BC_CONFIG >/dev/null 2>&1 &
        PIDS="$PIDS $!"
    done

    OK=0; FAIL=0
    for p in $PIDS; do
        wait $p 2>/dev/null && OK=$((OK+1)) || FAIL=$((FAIL+1))
    done

    DUR=$(( $(date +%s%3N) - T ))
    LOAD=$(get_load)
    MEM=$(get_mem)
    THR=$(get_threads)
    ALIVE=$(kill -0 $DIRD_PID 2>/dev/null && echo ALIVE || echo CRASHED)

    echo "| $N | $OK | $FAIL | ${DUR}ms | $LOAD | $MEM | $THR | $ALIVE |"

    if [ "$ALIVE" = "CRASHED" ]; then
        echo ""
        echo "**DIRECTOR CRASHED at N=$N**"
        break
    fi
    if [ $FAIL -gt $((N/3)) ]; then
        echo ""
        echo "**STOPPED at N=$N: >33% failures ($FAIL/$N)**"
        break
    fi
    sleep 2
done

echo ""
MEM_FINAL=$(get_mem)
echo "**Final:** Director $(kill -0 $DIRD_PID 2>/dev/null && echo ALIVE || echo CRASHED), Mem=${MEM_FINAL}KB (Baseline: ${MEM_BASE}KB)"

kill $DIRD_PID 2>/dev/null
wait $DIRD_PID 2>/dev/null
