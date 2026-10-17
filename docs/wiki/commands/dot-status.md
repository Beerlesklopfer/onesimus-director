# .status — einzelne Teile des Status abfragen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L270) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Report status

**Originalsyntax:**

```text
.status dir ( current | last | header | scheduled | running | terminated ) | storage=<storage> [ header | waitreservation | devices | volumes | spooling | running | terminated ] | client=<client> [ header | terminated | running ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 22 von 22 Fällen byte-identisch; Modi: Text, API 2.
- Warte-Zustände laufender Jobs (FD/SD/Ressourcen/Priorität) und beendete Jobs in der JCR-Liste fehlen (C-048); im Fixture läuft kein Job.

**Offene Logeinträge:** `C-048` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (22):** `c009-status-dir-header`, `c009-status-dir-header-json`, `c014-dot-status-dir-header`, `c014-dot-status-dir-current`, `c014-dot-status-dir-last`, `c014-dot-status-dir-running-json`, `c014-dot-status-dir-scheduled-json`, `c014-dot-status-dir-terminated-json`, `c014-dot-status-dir-bogus`, `c014-dot-status-dir-noarg`, `c014-dot-status-noarg`, `c014-dot-status-bogus`, `c014-dot-status-dir-running`, `c042-dot-status-storage`, `c042-dot-status-storage-header`, `c042-dot-status-storage-running`, `c042-dot-status-storage-api2`, `c042-dot-status-client-header`, `dot-status-dir-header`, `dot-status-dir-running`, `dot-status-dir-scheduled`, `dot-status-dir-terminated`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/status.rs:153](../../../crates/onesimus-dird/src/commands/status.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.status` liefert gezielt einen Abschnitt der Statusanzeige von Director, Storage Daemon oder Client, zum Beispiel nur die Kopfzeilen oder nur die laufenden Jobs. Die WebUI und Überwachungsskripte nutzen den Befehl, weil sie so nicht die ganze Ausgabe von [status](status.md) zerlegen müssen und unter `.api 2` strukturiertes JSON bekommen. Der Befehl liest nur; bei `storage=` und `client=` baut der Director dazu eine Verbindung zum jeweiligen Daemon auf.

## Syntax

```text
.status dir ( current | last | header | scheduled | running | terminated )
.status storage=<storage> [ header | waitreservation | devices | volumes | spooling | running | terminated ]
.status client=<client> [ header | terminated | running ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `dir` | | Director; ein Abschnitt muss folgen |
| `header` | | Version, Startzeit und Jobzähler |
| `scheduled` | | geplante Jobs |
| `running` | | laufende Jobs |
| `terminated` | | zuletzt beendete Jobs |
| `current`, `last` | | nur unter `dir`: Kurzantwort `1000 OK .status` |
| `storage=<storage>` | Name eines Storages | Storage Daemon; ohne Abschnitt die vollständige Statusausgabe |
| `waitreservation`, `devices`, `volumes`, `spooling` | | nur unter `storage=`: Jobs, die auf ein Laufwerk warten, Geräte, benutzte Volumes, Spool-Status |
| `client=<client>` | Name eines Clients | File Daemon; mit `header`, `running` oder `terminated` |

## API-Modi

Unter `.api 0` und `.api 1` kommt derselbe Text. Unter `.api 2` liefert `.status dir` die Abschnitte als JSON: `result.header` (Felder `director`, `version`, `release_date`, `os`, `binary_info`, `daemon_started`, `jobs_run`, `jobs_running`, `config_warnings`), `result.scheduled`, `result.running` und `result.terminated` (Arrays). Für `storage=` und `client=` gibt es unter `.api 2` kein JSON; das Ergebnis ist `"result": {}`.

## Beispiele

### Kopf des Director-Status

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir header
bareos-dir Version: 25.1.2+onesimus.0.2.0 (16 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 16-Oct-26 23:05. Jobs: run=0, running=0 db:postgresql, Onesimus binary
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kopf als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir header
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "header": {
      "director": "bareos-dir",
      "version": "25.1.2+onesimus.0.2.0",
      "release_date": "16 October 2026",
      "os": "Debian GNU/Linux 13 (trixie)",
      "binary_info": "Onesimus",
      "daemon_started": "16-Oct-26 23:05",
      "jobs_run": 0,
      "jobs_running": 0,
      "config_warnings": false
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Geplante Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir scheduled

Scheduled Jobs:
Level          Type     Pri  Scheduled          Name               Volume
===================================================================================
Differential   Backup    10  17-Oct-26 21:00    backup-bareos-fd   Incremental-47322
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Geplante Jobs als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir scheduled
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "scheduled": [
      {
        "name": "backup-bareos-fd",
        "type": "Backup",
        "level": "Differential",
        "priority": 10,
        "scheduled": "17-Oct-26 21:00",
        "volume": "Incremental-47322",
        "storage": "File"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Laufende Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir running

Running Jobs:
Console connected at 16-Oct-26 23:06
No Jobs running.
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Beendete Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir terminated
No Terminated Jobs.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kurzantwort `current`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir current
1000 OK .status
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kopf eines Storage Daemons

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status storage=File header
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103
 Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
bareos-sd Version: 25.1.2~pre43.18be77e68 (09 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 12-Okt-26 13:01. Jobs: run=11, running=0, Bareos community binary
 Sizes: boffset_t=8 size_t=8 int32_t=4 int64_t=8 bwlimit=0kB/s

There are WARNINGS for this storagedaemon's configuration!
See output of 'bareos-sd -t' for details.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vollständiger Status eines Storage Daemons

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status storage=File
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103
 Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3

bareos-sd Version: 25.1.2~pre43.18be77e68 (09 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 12-Okt-26 13:01. Jobs: run=11, running=0, Bareos community binary
 Sizes: boffset_t=8 size_t=8 int32_t=4 int64_t=8 bwlimit=0kB/s

There are WARNINGS for this storagedaemon's configuration!
See output of 'bareos-sd -t' for details.

Job inventory:

No Jobs running.
====
[…]

Used Volume status:
====

====

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage-Status als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status storage=File header
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kopf eines Clients

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status client=bareos-fd header
Connecting to Client bareos-fd at localhost:9102
Probing client protocol... (result will be saved until config reload)
 Handshake: Immediate TLS, Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
WS-JOERG.intranet.bernau.family-fd Version: 25.1.2~pre43.18be77e68 (09 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 12-Okt-26 13:01. Jobs: run=9 running=0, Bareos community binary
 Sizeof: boffset_t=8 size_t=8 debug=0 trace=0 bwlimit=0kB/s
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlende oder falsche Argumente

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status
1900 Bad .status command, missing arguments.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .status dir
1900 Bad .status command, wrong argument.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `1900 Bad .status command, missing arguments.` | Es fehlt `dir`, `storage=` oder `client=`. |
| `1900 Bad .status command, wrong argument.` | Unbekanntes Ziel oder unter `dir` fehlender oder unbekannter Abschnitt. |
| `.status: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.status` nicht. |

Kann der Director einen Daemon nicht erreichen, erscheinen dieselben Verbindungsmeldungen wie bei [status](status.md).

## Siehe auch

- [status](status.md) — vollständige Statusanzeige
- [.clients](dot-clients.md), [.storages](dot-storages.md) — gültige Namen
- [.jobstatus](dot-jobstatus.md) — Bedeutung der Job-Statuscodes
