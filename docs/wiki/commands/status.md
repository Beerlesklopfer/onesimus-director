# status — Show Status

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L446) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Report status

**Originalsyntax:**

```text
status all | dir=<dir-name> | director | scheduler | schedule=<schedule-name> | client=<client-name> | storage=<storage-name> slots | days=<nr_days> | job=<job-name> | subscriptions [clients] [plugins] [all] [anonymize] [client=<client-name>] | configuration
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 51 von 52 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-limited.
- Instabil (gemeinsamer echter FD bzw. nicht eindeutige Sortierung, allein grün): `c042-status-client-star`.
- `status storage=… slots` endet ohne Slottabelle (C-045; kein Autochanger im Fixture, der Fall `c042-status-storage-slots` deckt nur den Nicht-Autochanger ab).
- Warte-Zustände laufender Jobs fehlen (C-048); NDMP-Storages werden nicht gesondert behandelt (C-050).

**Offene Logeinträge:** `C-045`, `C-048`, `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (52):** `c013-status-bogus`, `c013-status-sd`, `c013-status-storage-novalue`, `c013-status-storage-notfound`, `c013-status-jobid-notrunning`, `c013-status-limited`, `c014-status-dir`, `c014-status-director-days`, `c014-status-dir-days-invalid`, `c014-status-dir-api1`, `c014-status-dir-json`, `c014-status-scheduler-preview`, `c014-status-scheduler-past`, `c014-status-scheduler-range`, `c014-status-scheduler-invalid`, `c014-status-sched-client`, `c014-status-scheduler-json`, `c014-status-subscriptions`, `c014-status-subscriptions-all`, `c014-status-subscriptions-client`, `c014-status-subscriptions-limited`, `c014-status-configuration`, `c014-status-configuration-limited`, `c014-status-subscriptions-json`, `c014-status-subscriptions-clients`, `c014-status-subscriptions-plugins`, `c014-status-all`, `c027-gui-status-dir`, `c028-status-scheduler`, `c028-status-dir`, `c028-status-scheduler-days`, `c042-status-storage`, `c042-status-sd`, `c042-status-bogus`, `c042-status-storage-star`, `c042-status-client-star`, `c042-status-storage-slots`, `c042-status-storage-api1`, `c042-status-storage-api2`, `c042-status-all`, `c042-status-storage-twice`, `c042-status-storage-slots-api1`, `c042-status-storage-slots-api2`, `c042-status-storage-api1-twice`, `status-client`, `status-storage`, `status-scheduler`, `status-scheduler-job`, `status-subscriptions`, `status-configuration`, `status-dir`, `status-bogus`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/status.rs:91](../../../crates/onesimus-dird/src/commands/status.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display status of the Director, a Client (FD), or a Storage Daemon (SD).

## Syntax

```
status [dir|director|client=<name>|storage=<name>]
```

## Sub-Commands

| Argument | Description |
|----------|-------------|
| `dir` / `director` | Director status (default) |
| `client=<name>` | File Daemon status (connects to FD) |
| `storage=<name>` | Storage Daemon status (connects to SD) |

## Examples

### Director status
```
* status dir
Onesimus Director Version 25.1.2+onesimus.0.2.0 (Debian GNU/Linux 13 (trixie))

Scheduled Jobs:
Level  Type  Priority  Scheduled           Name
==============================================
Full   B     10        2026-03-29 01:00:00  backup-bareos-fd

Running Jobs:
JobId  Level  Name              Status
==========================================
No jobs running.

Terminated Jobs:
JobId  Level  Files  Bytes       Status    Finished             Name
====================================================================
10     Full   458    71,683,878  OK        2026-03-28 21:20:42  backup-bareos-fd
```

### Client status
```
* status client=bareos-fd
bareos-fd Version: 25.0.3 (18 March 2026)
Daemon started 28-Mar-2026 16:25
Jobs: run=0 running=0

Running Jobs:
No Jobs running.

Terminated Jobs:
JobId  Level  Files  Bytes       Status    Finished             Name
====================================================================
10     Full   458    71,683,878  OK        2026-03-28 21:20:42  backup-bareos-fd
```

### Storage status
```
* status storage=File
bareos-sd Version: 25.0.3 (18 March 2026)
Daemon started 28-Mar-2026 16:25

Device "FileStorage" (/srv/BACKUP/bareos-storage) is not open.

Terminated Jobs:
JobId  Level  Files  Bytes       Status    Finished             Name
====================================================================
10     Full   458    71,683,878  OK        2026-03-28 21:20:42  backup-bareos-fd
```

## Behavior

- `status dir`: reads from local Director state (threads, job queue, scheduler)
- `status client`: connects to FD via TLS-PSK, sends `status`, reads response
- `status storage`: connects to SD via TLS-PSK, sends `status`, reads response

## Related Commands

- [.status](dot-status.md) — Machine-readable status
- [list](list.md) — List jobs from catalog
