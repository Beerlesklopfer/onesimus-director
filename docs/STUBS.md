# Stubs-Backlog

Platzhalter, fest verdrahtete Werte und „not available yet“-Zweige im Code
(Suche vom 17.10.2026 nach `TODO`, `not supported`, `not available yet`,
`placeholder`, `allow(dead_code)`). Jeder Punkt wird gegen das Original
(`/usr/sbin/bareos-dir`, Quelle 18be77e6) umgesetzt und mit Paritätsfällen bzw.
Einheitentests belegt.

Status: **offen** · **in Arbeit** (Agent) · **erledigt** (Commit) · **blockiert** (Grund)

Testregel: Pfade mit FD/SD-Kontakt nie gegen die laufenden Daemons testen.
Erlaubt sind eigene Test-Daemons (installierte `bareos-fd`/`bareos-sd` mit
eigener Konfiguration, eigenen Ports und Speicherverzeichnis im Scratchpad)
oder simulierte Daemons in Einheitentests. `tests/parity/parity.py` weist
Fälle ab, die echte Jobs starten können (`unsafe_job_start`).

| ID | Ort | Stub | Bezug | Status |
|---|---|---|---|---|
| S-01 | `onesimus-dird/src/catalog_worker.rs` (GenerateBootstrap) | `Storage="File"`, `Device="FileStorage"` fest verdrahtet; Rückfallweg von `restore` (`commands/restore.rs`), wenn keine Bootstrap-Datei geschrieben wurde | — | offen |
| S-02 | `onesimus-dird/src/scheduler.rs` | `already_queued = false; // TODO: proper dedup` – keine Prüfung auf bereits eingereihte Jobs | — | offen |
| S-03 | `onesimus-dird/src/internal_jobs.rs` | Selection Type PoolOccupancy „not available yet“ | C-058, #1 | offen |
| S-04 | `onesimus-dird/src/commands/job.rs` | Verify-Jobs (alle Level) „not available yet“; Menüpunkt „Verify Volume Data (not yet implemented)“ | C-058, #1 | offen |
| S-05 | `onesimus-dird/src/commands/job.rs`, `internal_jobs.rs` | Copy/Migrate-Arbeitsjobs: `run job=<Copy> jobid=N` abgewiesen, Steuerjob meldet „Could not start migration job.“ | C-058, #1 | offen |
| S-06 | `onesimus-dird/src/commands/job.rs` | Consolidate mit Always-Incremental-Jobs „not available yet“ | C-058, #1 | offen |
| S-07 | Job-Ablauf (alle Jobtypen) | RunScripts (RunBeforeJob/RunAfterJob, Client-/Console-Skripte) laufen nicht | C-058, #1 | offen |
| S-08 | `onesimus-dird/src/commands/admin.rs` (`UpdateAction::Unsupported`) | `update volume encrypt=` „not supported“ | C-067, #2 | erledigt (9fa0aad, ba5396f; neues Modul `volume_key.rs`, Fälle `s08-encrypt-*`; offen außerhalb S-08: `label`/`relabel … encrypt` erzeugt noch keinen Schlüssel) |
| S-09 | `onesimus-dird/src/commands/restore.rs` | API-2-Restore-Start liefert `{"result":{}}` statt `run.jobid` | C-072, #3 | erledigt (238b6b4: seit C-058 korrekt, belegt durch Einheitentests `api2_restore_*`) |
| S-10 | `onesimus-dird/src/commands/acl.rs` | `.acl` nur Platzhalter („(ACL configuration pending)“) | C-074, #4 | blockiert (Entscheidung des Nutzers zu #4) |
| S-11 | `onesimus-dird/src/commands/extra.rs` (`cmd_show`), `show_items.rs`, `version_strings.rs` | Totes `extra::cmd_show`; dateiweites `allow(dead_code)` verdeckt ungenutzten Code | — | offen |
| S-12 | `bareos-filed/src/restore.rs`, `accept.rs` | Eigener FD: Restore ist `_placeholder()`; FD-zu-FD-Verbindungen „not supported“ | — | offen |
