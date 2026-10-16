# .status — Machine-Readable Status

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L270) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Report status

**Originalsyntax:**

```text
.status dir ( current | last | header | scheduled | running | terminated ) | storage=<storage> [ header | waitreservation | devices | volumes | spooling | running | terminated ] | client=<client> [ header | terminated | running ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler delegiert an den normalen status-Handler; die Original-Unterabschnitte current/last/header/scheduled/running/terminated usw. werden nicht separat umgesetzt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/status.rs:118](../../../crates/onesimus-dird/src/commands/status.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface version of the `status` command. Used by bconsole/oconsole and automation tools to query Director, Client, or Storage Daemon status in a parseable format.

## Syntax

```
.status [dir|director|client=<name>|storage=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `dir` / `director` | Director status (default) |
| `client=<name>` | File Daemon status (connects to FD via TLS-PSK) |
| `storage=<name>` | Storage Daemon status (connects to SD via TLS-PSK) |

## Examples

### Director status
```
* .status dir
bareos-dir Version: 25.1.2+onesimus.0.2.0
Daemon started 29-Mar-2026 08:00:00

Director: bareos-dir
Clients: 2
Jobs: 3
Storages: 1
Pools: 3
Schedules: 1
Catalogs: 1

Console Connections: 1
FD Connections: 0
SD Connections: 0

Scheduled Jobs:
  No scheduled jobs.

Running Jobs:
  No running jobs.
====
```

### Client status
```
* .status client=bareos-fd
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
* .status storage=File
bareos-sd Version: 25.0.3 (18 March 2026)
Daemon started 28-Mar-2026 16:25

Device "FileStorage" (/srv/BACKUP/bareos-storage) is not open.

Terminated Jobs:
JobId  Level  Files  Bytes       Status    Finished             Name
====================================================================
10     Full   458    71,683,878  OK        2026-03-28 21:20:42  backup-bareos-fd
```

## Notes

- `.status` delegates to the same handler as `status`; the output format is identical.
- Client and Storage queries connect to the remote daemon via TLS-PSK.
- Output is terminated by the EOD signal.

## Related Commands

- [status](status.md) -- Human-readable status display
- [.clients](dot-clients.md) -- List client names
- [.storage](dot-storage.md) -- List storage names
