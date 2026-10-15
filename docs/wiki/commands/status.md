# status — Show Status

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L446) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Report status

**Originalsyntax:**

```text
status all | dir=<dir-name> | director | scheduler | schedule=<schedule-name> | client=<client-name> | storage=<storage-name> slots | days=<nr_days> | job=<job-name> | subscriptions [clients] [plugins] [all] [anonymize] [client=<client-name>] | configuration
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler behandelt dir/director/client/storage/all. Originalziele wie scheduler, subscriptions und configuration besitzen hier keinen eigenen Dispatch. Geplante und beendete Jobs sind im Director-JSON noch nicht vollständig abgebildet.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/status.rs:7](../../../crates/onesimus-dird/src/commands/status.rs).

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
