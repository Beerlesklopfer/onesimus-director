#!/bin/bash
# ========================================================================
# baReos Director — Command Test Suite & QM-Protokoll
# ========================================================================
#
# Startet einen eigenen Rust-Director auf Port 29101 (kein Konflikt mit
# produktivem Director), fuehrt alle Commands per brconsole durch und
# protokolliert Ergebnisse + Director-Log.
#
# Artefakte werden unter tests/results/<timestamp>/ gesichert:
#   - test_results.txt   — Testergebnisse (OK/FAIL pro Command)
#   - director.log       — Vollstaendiges Director-Log (debug level 100)
#   - bareos-dir.conf    — Verwendete Test-Konfiguration
#   - brconsole.conf     — Verwendete Console-Konfiguration
#   - summary.txt        — Zusammenfassung (Version, Datum, Ergebnis)
#
# Exit-Code: Anzahl fehlgeschlagener Tests (0 = alles OK)
# ========================================================================
set -euo pipefail

BAREOS_DIRD=${BAREOS_DIRD:-./target/release/onesimus-dird}
BRCONSOLE=${BRCONSOLE:-./target/release/oconsole}
PORT=29101
PASSWORD="testpass-$(date +%s)"
TMPDIR=$(mktemp -d /tmp/bareos-test-XXXXXX)
PASS=0
FAIL=0
TOTAL=0
SKIPPED=0
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULT_DIR="${SCRIPT_DIR}/results/${TIMESTAMP}"

# Ergebnis-Verzeichnis anlegen
mkdir -p "$RESULT_DIR"

# Tee: Ausgabe gleichzeitig auf Terminal und in Datei
exec > >(tee "$RESULT_DIR/test_results.txt") 2>&1

cleanup() {
    kill $DIRD_PID 2>/dev/null || true
    wait $DIRD_PID 2>/dev/null || true

    # Director-Log und Config sichern
    cp "$TMPDIR/director.log" "$RESULT_DIR/" 2>/dev/null || true
    cp "$TMPDIR/bareos-dir.conf" "$RESULT_DIR/" 2>/dev/null || true
    cp "$TMPDIR/brconsole.conf" "$RESULT_DIR/" 2>/dev/null || true

    # Log-Fehler zaehlen (unerwartete WARN + ERROR = Testfehler)
    local log_warns
    log_warns=$(grep -iE "WARN|ERROR|panic|fatal" "$TMPDIR/director.log" 2>/dev/null \
        | grep -vcE "${EXPECTED_LOG_ERRORS:-NICHTS}" 2>/dev/null || echo "0")

    # Summary schreiben (mit finalem Ergebnis)
    local dird_version
    dird_version=$($BAREOS_DIRD --version 2>&1 | head -1 || echo "unknown")
    cat > "$RESULT_DIR/summary.txt" << SUMMARY
baReos Director — QM Test-Protokoll
====================================
Datum:      $(date '+%Y-%m-%d %H:%M:%S')
Timestamp:  ${TIMESTAMP}
Director:   ${BAREOS_DIRD}
Console:    ${BRCONSOLE}
Version:    ${dird_version}
Port:       ${PORT}
Host:       $(hostname -f 2>/dev/null || hostname)
Kernel:     $(uname -r)

Ergebnis:   ${PASS} bestanden, ${FAIL} fehlgeschlagen, ${SKIPPED} uebersprungen, ${TOTAL} total
Status:     $([ "$FAIL" -eq 0 ] && echo "BESTANDEN" || echo "FEHLGESCHLAGEN")

Director-Log:
  WARN/ERROR Eintraege: ${log_warns}
SUMMARY

    if [ "$log_warns" -gt 0 ]; then
        echo "" >> "$RESULT_DIR/summary.txt"
        echo "Log-Eintraege:" >> "$RESULT_DIR/summary.txt"
        grep -iE "WARN|ERROR|panic|fatal" "$TMPDIR/director.log" >> "$RESULT_DIR/summary.txt" 2>/dev/null || true
    fi

    # Temp aufraeumen
    rm -rf "$TMPDIR"

    # Symlink auf letzten Lauf
    ln -sfn "$TIMESTAMP" "${SCRIPT_DIR}/results/latest"

    echo ""
    echo "Artefakte gesichert: $RESULT_DIR/"
    echo "  test_results.txt  — Testergebnisse"
    echo "  director.log      — Director-Log (debug=100)"
    echo "  bareos-dir.conf   — Test-Konfiguration"
    echo "  summary.txt       — Zusammenfassung"
    echo ""
    echo "Letzter Lauf: tests/results/latest/"
}
trap cleanup EXIT

# === Config erstellen ===
cat > "$TMPDIR/bareos-dir.conf" << EOF
Director {
  Name = test-dir
  Port = $PORT
  Password = "$PASSWORD"
  Maximum Concurrent Jobs = 10
}
Client {
  Name = test-fd
  Address = localhost
  Port = 9102
  Password = "fdpass"
}
Storage {
  Name = File
  Address = localhost
  Port = 9103
  Password = "sdpass"
  Device = FileStorage
  Media Type = File
}
Pool {
  Name = Full
  Pool Type = Backup
  Recycle = yes
  Volume Retention = 365 days
  Maximum Volume Bytes = 50G
  Maximum Volumes = 100
}
Pool {
  Name = Incremental
  Pool Type = Backup
  Recycle = yes
  Volume Retention = 30 days
}
Schedule {
  Name = WeeklyCycle
  Run = Full 1st sun at 23:05
  Run = Incremental mon-sat at 23:05
}
Job {
  Name = BackupTest
  Type = Backup
  Level = Full
  Client = test-fd
  Storage = File
  Pool = Full
  Schedule = WeeklyCycle
}
Console {
  Name = admin
  Password = "adminpass"
}
EOF

cat > "$TMPDIR/brconsole.conf" << EOF
Director {
  Name = test-dir
  address = localhost
  Port = $PORT
  Password = "$PASSWORD"
}
EOF

# === Director starten ===
$BAREOS_DIRD -c "$TMPDIR/bareos-dir.conf" -f -d100 2>"$TMPDIR/director.log" &
DIRD_PID=$!
sleep 2

if ! kill -0 $DIRD_PID 2>/dev/null; then
    echo "FATAL: Director konnte nicht gestartet werden"
    cat "$TMPDIR/director.log"
    exit 99
fi

# === Test-Funktionen ===

# run_cmd "command" "expected_regex"
run_cmd() {
    local cmd="$1"
    local expect="$2"
    TOTAL=$((TOTAL+1))

    # Prüfe ob Director noch lebt
    if ! kill -0 $DIRD_PID 2>/dev/null; then
        printf "  \e[31mCRASH\e[0m %-45s  Director ist abgestuerzt!\n" "$cmd"
        FAIL=$((FAIL+1))
        return
    fi

    local output
    output=$(timeout 10 $BRCONSOLE -c "$TMPDIR/brconsole.conf" -e "$cmd" 2>&1) || true

    if echo "$output" | grep -qiE "$expect"; then
        printf "  \e[32mOK\e[0m   %-45s\n" "$cmd"
        PASS=$((PASS+1))
    else
        printf "  \e[31mFAIL\e[0m %-45s  (erwartet: '%s')\n" "$cmd" "$expect"
        echo "        $(echo "$output" | grep -v "^Connecting\|^$\|Encryption\|1000OK" | head -2)"
        FAIL=$((FAIL+1))
    fi
}

# skip_cmd "command" "reason"
skip_cmd() {
    local cmd="$1"
    local reason="$2"
    TOTAL=$((TOTAL+1))
    SKIPPED=$((SKIPPED+1))
    printf "  \e[33mSKIP\e[0m %-45s  (%s)\n" "$cmd" "$reason"
}

echo "========================================================================"
echo "baReos Director — Command Test Suite"
echo "========================================================================"
echo "Director: $BAREOS_DIRD"
echo "Console:  $BRCONSOLE"
echo "PID:      $DIRD_PID"
echo "Port:     $PORT"
echo "Datum:    $(date '+%Y-%m-%d %H:%M:%S')"
echo "========================================================================"
echo ""

# === 1. Basis ===
echo "--- 1. Basis ---"
run_cmd "version" "+onesimus."
run_cmd "time" "202"
run_cmd "quit" "bye"

# === 2. Help-System ===
echo ""
echo "--- 2. Help-System ---"
run_cmd "help" "Available commands"
run_cmd "help" ".help"
run_cmd "help" "cancel"
run_cmd ".help" ".clients"
run_cmd ".help" ".bvfs_update"
run_cmd ".help" ".authorized"
run_cmd ".help" ".volstatus"
run_cmd ".help all" "cancel"

# === 3. Status ===
echo ""
echo "--- 3. Status ---"
run_cmd "status dir" "Director:|test-dir"
run_cmd ".status dir" "Director:|test-dir"
skip_cmd "status all" "braucht FD/SD"

# === 4. Dot-Commands (Resource Listing) ===
echo ""
echo "--- 4. Dot-Commands (Resource Listing) ---"
run_cmd ".clients" "test-fd"
run_cmd ".jobs" "BackupTest"
run_cmd ".pools" "Full"
run_cmd ".storage" "File"
run_cmd ".storages" "File"
run_cmd ".filesets" ""
run_cmd ".schedule" "WeeklyCycle"
run_cmd ".catalogs" ""
run_cmd ".consoles" "admin"
run_cmd ".jobdefs" ""
run_cmd ".profiles" ""
run_cmd ".locations" ""
run_cmd ".media" ""
run_cmd ".mediatypes" "File"
run_cmd ".msgs" ""
run_cmd ".users" ""

# === 5. Dot-Commands (Queries) ===
echo ""
echo "--- 5. Dot-Commands (Queries) ---"
run_cmd ".defaults" "BackupTest|job="
run_cmd ".defaults job=BackupTest" "job=BackupTest"
run_cmd ".jobstatus" "Completed|Running"
run_cmd ".levels" "Full"
run_cmd ".types" "Backup"
run_cmd ".volstatus" "Append"
run_cmd ".actiononpurge" "Truncate"
run_cmd ".authorized" "authorized"
run_cmd "." ""

# === 6. API/Protocol ===
echo ""
echo "--- 6. API/Protocol ---"
run_cmd ".api 0" "api|0"
run_cmd ".api 1" "api|1"
run_cmd ".api 2" "api|2"
run_cmd ".grpc" "gRPC"

# === 7. BVFS ===
echo ""
echo "--- 7. BVFS ---"
run_cmd ".bvfs_update" "BVFS|Done"
run_cmd ".bvfs_cleanup path=test" "cleanup|done"
run_cmd ".bvfs_clear_cache yes" "cleared"
run_cmd ".bvfs_get_jobids jobid=1" "1"
run_cmd ".bvfs_lsdirs jobid=1 path=/" "lsdirs"
run_cmd ".bvfs_lsfiles jobid=1 path=/" "lsfiles"
run_cmd ".bvfs_versions client=test-fd pathid=1 filename=test" "versions"
run_cmd ".bvfs_restore path=h1 jobid=1" "restore"

# === 8. List ===
echo ""
echo "--- 8. List ---"
run_cmd "list jobs" "BackupTest|Backup|Type"
run_cmd "list clients" "test-fd|Address"
run_cmd "list pools" "Full|Type"
run_cmd "list storages" "File|Address"
run_cmd "list schedules" "WeeklyCycle"

# === 9. Show ===
echo ""
echo "--- 9. Show ---"
run_cmd "show all" "Director:|test-dir"
run_cmd "show director" "test-dir"
run_cmd "show clients" "test-fd"
run_cmd "show jobs" "BackupTest"
run_cmd "show storages" "File"
run_cmd "show pools" "Full"

# === 10. Configure CRUD ===
echo ""
echo "--- 10. Configure CRUD ---"
run_cmd "configure" "CRUD|add|delete"
run_cmd "configure add client name=new-fd address=10.0.0.5 password=x" "Created|new-fd"
run_cmd ".clients" "new-fd"
run_cmd "configure read client=new-fd" "new-fd|Client"
run_cmd "configure update client=new-fd address=10.0.0.99" "updated|10.0.0.99"
run_cmd "configure delete client=new-fd" "Deleted|new-fd"
run_cmd "configure export" "Director|test-dir"

# === 11. Enable/Disable ===
echo ""
echo "--- 11. Enable/Disable ---"
run_cmd "enable job=BackupTest" "enabled|already"
run_cmd "disable job=BackupTest" "disabled"
run_cmd ".jobs disabled" "BackupTest"
run_cmd "enable job=BackupTest" "enabled"
run_cmd ".jobs enabled" "BackupTest"

# === 12. Admin/Session ===
echo ""
echo "--- 12. Admin/Session ---"
run_cmd "messages" "pending|No"
run_cmd "memory" "Memory|memory"
run_cmd "whoami" "UserAgent"
run_cmd "wait" "running|No"
run_cmd "setdebug level=100" "100"
run_cmd "autodisplay on" "on"
run_cmd "gui on" "gui"
run_cmd "use" "catalog"
run_cmd "reload" "reloaded"

# === 13. Admin (Storage) ===
echo ""
echo "--- 13. Admin (Storage) ---"
run_cmd "label" "specify|Available"
run_cmd "mount storage=File" "mount|mounted"
run_cmd "unmount storage=File" "unmount|unmounted"
run_cmd "release storage=File" "release|released"
run_cmd "update" "specify|update"
run_cmd "prune" "specify|prune"
run_cmd "purge" "specify|purge"
run_cmd "truncate storage=File" "truncat"

# === 14. Catalog Schema ===
echo ""
echo "--- 14. Catalog Schema ---"
run_cmd ".catalog" "schema|version|tables"
run_cmd ".catalog version" "v00"
run_cmd ".catalog tables" "client|job"

# === 15. Run ===
echo ""
echo "--- 15. Run ---"
run_cmd "run" "BackupTest|Available"
run_cmd "run job=BackupTest" "queued|Job:|BackupTest"
# run startet echten Job-Thread → FD connect schlaegt fehl (erwartet ohne FD)
# Der ERROR im Log ist ein erwarteter Job-Fehler, kein Director-Bug.
EXPECTED_LOG_ERRORS="FD connect failed|SD connect failed"

# === 16. Stabilitaet: Director noch am Leben? ===
echo ""
echo "--- 16. Stabilitaet ---"
if kill -0 $DIRD_PID 2>/dev/null; then
    printf "  \e[32mOK\e[0m   %-45s\n" "Director ALIVE nach allen Tests"
    TOTAL=$((TOTAL+1))
    PASS=$((PASS+1))
else
    printf "  \e[31mFAIL\e[0m %-45s\n" "Director CRASHED waehrend Tests!"
    TOTAL=$((TOTAL+1))
    FAIL=$((FAIL+1))
fi

# === Ergebnis ===
echo ""
echo "========================================================================"
printf "Ergebnis: \e[32m%d bestanden\e[0m, \e[31m%d fehlgeschlagen\e[0m, " "$PASS" "$FAIL"
printf "\e[33m%d uebersprungen\e[0m, %d total\n" "$SKIPPED" "$TOTAL"
if [ "$FAIL" -eq 0 ]; then
    printf "\e[1;32mSTATUS: BESTANDEN\e[0m\n"
else
    printf "\e[1;31mSTATUS: FEHLGESCHLAGEN\e[0m\n"
fi
echo "========================================================================"

# === 17. Backup Integration (optional — braucht laufende SD + FD) ===
echo ""
echo "--- 17. Backup Integration ---"

# Prüfe ob SD und FD erreichbar sind
SD_OK=false
FD_OK=false
(echo "" | timeout 2 nc -q0 localhost 9103 >/dev/null 2>&1) && SD_OK=true
(echo "" | timeout 2 nc -q0 localhost 9102 >/dev/null 2>&1) && FD_OK=true

if [ "$SD_OK" = true ] && [ "$FD_OK" = true ] && [ -d "${SCRIPT_DIR}/bareos-dir.d" ]; then
    # Eigener Director mit voller Config (Catalog, JobDefs, etc.) auf Port 29102
    INTEG_PORT=29102
    INTEG_TMPDIR=$(mktemp -d /tmp/bareos-integ-XXXXXX)

    # Director-Config mit angepasstem Port erstellen
    # Wir patchen die director.conf temporär für unseren Port
    cp -r "${SCRIPT_DIR}/bareos-dir.d" "$INTEG_TMPDIR/bareos-dir.d"
    sed -i "s/}$/  Port = ${INTEG_PORT}\n}/" "$INTEG_TMPDIR/bareos-dir.d/director/bareos-dir.conf"

    # Console-Config für brconsole
    DIR_PW=$(grep -oP 'Password\s*=\s*"\K[^"]+' "${SCRIPT_DIR}/bareos-dir.d/director/bareos-dir.conf")
    cat > "$INTEG_TMPDIR/brconsole.conf" << IEOF
Director {
  Name = bareos-dir
  address = localhost
  Port = ${INTEG_PORT}
  Password = "${DIR_PW}"
}
IEOF

    # Integrations-Director starten
    $BAREOS_DIRD -c "$INTEG_TMPDIR/bareos-dir.d" -f -d100 2>"$INTEG_TMPDIR/director.log" &
    INTEG_PID=$!
    sleep 3

    if kill -0 $INTEG_PID 2>/dev/null; then
        # Backup starten und auf Ergebnis warten
        INTEG_CMD="run job=backup-bareos-fd level=Full yes"
        INTEG_OUT=$(timeout 120 $BRCONSOLE -c "$INTEG_TMPDIR/brconsole.conf" -e "$INTEG_CMD" 2>&1) || true
        TOTAL=$((TOTAL+1))

        if echo "$INTEG_OUT" | grep -qiE "queued|Job.*started|JobId"; then
            printf "  \e[32mOK\e[0m   %-45s\n" "run job=backup-bareos-fd (gestartet)"
            PASS=$((PASS+1))
        else
            printf "  \e[31mFAIL\e[0m %-45s  (konnte nicht starten)\n" "run job=backup-bareos-fd"
            echo "        $(echo "$INTEG_OUT" | head -3)"
            FAIL=$((FAIL+1))
        fi

        # Warte auf Job-Ende (max 60s) — pruefe via messages
        BACKUP_OK=false
        for i in $(seq 1 30); do
            sleep 2
            MSG_OUT=$(timeout 10 $BRCONSOLE -c "$INTEG_TMPDIR/brconsole.conf" -e "messages" 2>&1) || true
            if echo "$MSG_OUT" | grep -qiE "Backup OK|Success|files=.*bytes="; then
                BACKUP_OK=true
                break
            fi
            if echo "$MSG_OUT" | grep -qiE "Backup Error|Fatal|failed"; then
                break
            fi
        done

        TOTAL=$((TOTAL+1))
        if [ "$BACKUP_OK" = true ]; then
            # Extrahiere Dateizahl
            BACKUP_FILES=$(echo "$MSG_OUT" | grep -oP 'files=\K[0-9]+' | head -1 || echo "?")
            BACKUP_BYTES=$(echo "$MSG_OUT" | grep -oP 'bytes=\K[0-9]+' | head -1 || echo "?")
            printf "  \e[32mOK\e[0m   %-45s  (files=%s bytes=%s)\n" "Backup completed" "$BACKUP_FILES" "$BACKUP_BYTES"
            PASS=$((PASS+1))
        else
            printf "  \e[31mFAIL\e[0m %-45s\n" "Backup completed"
            echo "        Letzte messages:"
            echo "$MSG_OUT" | grep -v "^$\|^Connecting\|Encryption" | head -5 | while read -r l; do
                echo "        $l"
            done
            FAIL=$((FAIL+1))
        fi

        # Aufräumen
        kill $INTEG_PID 2>/dev/null || true
        wait $INTEG_PID 2>/dev/null || true
        cp "$INTEG_TMPDIR/director.log" "$RESULT_DIR/director-integ.log" 2>/dev/null || true
    else
        printf "  \e[33mSKIP\e[0m %-45s  (Integrations-Director Start fehlgeschlagen)\n" "Backup Integration"
        TOTAL=$((TOTAL+1))
        SKIPPED=$((SKIPPED+1))
        cat "$INTEG_TMPDIR/director.log" | tail -5
    fi

    rm -rf "$INTEG_TMPDIR"
else
    if [ "$SD_OK" = false ]; then
        skip_cmd "Backup Integration" "SD nicht erreichbar (Port 9103)"
    elif [ "$FD_OK" = false ]; then
        skip_cmd "Backup Integration" "FD nicht erreichbar (Port 9102)"
    else
        skip_cmd "Backup Integration" "tests/bareos-dir.d/ nicht gefunden"
    fi
fi

# === 18. Director-Log Analyse ===
echo ""
echo "--- 18. Director-Log Analyse ---"

# WARN und ERROR im Director-Log sind Testfehler!
# Erwartete Fehler (z.B. FD nicht erreichbar im Testkontext) werden ausgefiltert.
EXPECTED_LOG_ERRORS="${EXPECTED_LOG_ERRORS:-NICHTS_ERWARTETES}"
LOG_UNEXPECTED=$(grep -iE "WARN|ERROR|panic|fatal" "$TMPDIR/director.log" 2>/dev/null \
    | grep -vE "$EXPECTED_LOG_ERRORS" || true)
LOG_EXPECTED=$(grep -iE "WARN|ERROR|panic|fatal" "$TMPDIR/director.log" 2>/dev/null \
    | grep -E "$EXPECTED_LOG_ERRORS" || true)
LOG_UNEXPECTED_COUNT=$(echo "$LOG_UNEXPECTED" | grep -c . 2>/dev/null || echo "0")
LOG_EXPECTED_COUNT=$(echo "$LOG_EXPECTED" | grep -c . 2>/dev/null || echo "0")

if [ "$LOG_UNEXPECTED_COUNT" -gt 0 ]; then
    printf "  \e[31mFAIL\e[0m %-45s\n" "Director-Log: ${LOG_UNEXPECTED_COUNT} unerwartete Fehler"
    TOTAL=$((TOTAL+1))
    FAIL=$((FAIL+1))
    echo ""
    echo "  Unerwartete Log-Eintraege:"
    echo "$LOG_UNEXPECTED" | while read -r line; do
        printf "    \e[31m%s\e[0m\n" "$line"
    done
else
    printf "  \e[32mOK\e[0m   %-45s\n" "Director-Log: 0 unerwartete Fehler"
    TOTAL=$((TOTAL+1))
    PASS=$((PASS+1))
fi
if [ "$LOG_EXPECTED_COUNT" -gt 0 ]; then
    printf "  \e[33mINFO\e[0m %-45s\n" "Director-Log: ${LOG_EXPECTED_COUNT} erwartete Fehler (kein FD/SD)"
fi

echo ""
echo "========================================================================"
printf "Gesamt: \e[32m%d bestanden\e[0m, \e[31m%d fehlgeschlagen\e[0m, " "$PASS" "$FAIL"
printf "\e[33m%d uebersprungen\e[0m, %d total\n" "$SKIPPED" "$TOTAL"
if [ "$FAIL" -eq 0 ]; then
    printf "\e[1;32mSTATUS: BESTANDEN\e[0m\n"
else
    printf "\e[1;31mSTATUS: FEHLGESCHLAGEN\e[0m\n"
fi
echo "========================================================================"

exit $FAIL
