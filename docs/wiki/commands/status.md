# status — Status von Director, Client und Storage anzeigen

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

## Zweck

`status` zeigt den aktuellen Zustand des Directors, eines Storage Daemons (SD) oder eines File Daemons (FD): geplante, laufende und beendete Jobs, Geräte, Vorschau des Schedulers, Subscriptions-Auswertung und veraltete Konfigurationseinstellungen. Für Client und Storage baut der Director eine Verbindung zum jeweiligen Daemon auf und gibt dessen Statusbericht weiter. Der Befehl liest nur; er verändert weder Katalog noch Konfiguration noch Daemons.

## Syntax

```text
status
status director [days=<n>]
status dir [days=<n>]
status client[=<client-name>]
status storage[=<storage-name>] [slots]
status sd=<storage-name> [slots]
status job=<job-name>
status jobid=<jobid>
status scheduler [schedule=<schedule-name>] [client=<client-name>] [job=<job-name>] [days=<n> | days=<von>,<bis>]
status subscriptions [clients] [plugins] [all] [anonymize] [client=<client-name>]
status configuration
status all
```

## Argumente

Das erste Wort nach `status` entscheidet, welcher Bericht kommt. Groß- und Kleinschreibung spielt keine Rolle, und mehrere Schlüsselwörter werden über ihren Anfang erkannt: alles, was mit `dir` beginnt, gilt als Director, `sched…` als Scheduler, `sub…` als Subscriptions und `conf…` als Configuration (`status ConfigurationX` liefert daher den Konfigurationsbericht). Jedes andere erste Wort führt zum Storage-Bericht.

| Argument | Werte | Bedeutung |
|---|---|---|
| `director`, `dir` | – | Status des Directors: Kopfzeile, geplante Jobs, laufende Jobs, zuletzt beendete Jobs, vom Client aufgebaute Verbindungen. |
| `days=<n>` (bei `director`) | 0 bis 500, Vorgabe 1 | Wie viele Tage im Voraus geplante Jobs gelistet werden. Ungültige Werte werden mit Hinweis ignoriert. |
| `client` | – | Statusbericht eines File Daemons. Ohne Wert erscheint eine Auswahlliste der Clients. |
| `client=<client-name>` | Name einer Client-Ressource | Statusbericht dieses File Daemons. Ein unbekannter Name führt zur Auswahlliste. |
| `storage`, `storage=<storage-name>` | Name einer Storage-Ressource | Statusbericht des Storage Daemons. Ohne Wert oder bei unbekanntem Namen erscheint eine Auswahlliste. |
| `sd=<storage-name>` | Name einer Storage-Ressource | Wie `storage=`. |
| `slots` | – | Zusammen mit `storage=`: Slot-Tabelle eines Autochangers statt des Statusberichts. |
| `job=<job-name>` | Name eines Jobs | Storage-Bericht für den Storage des laufenden Jobs; läuft er nicht, kommt eine Meldung. |
| `jobid=<jobid>` | JobId | Wie `job=`, aber über die JobId. |
| `scheduler` | – | Welche Schedules welche Jobs auslösen und eine Vorschau der nächsten Ausführungen. |
| `schedule=<schedule-name>` | Name einer Schedule-Ressource | Bei `scheduler`: nur diesen Schedule zeigen. |
| `client=<client-name>`, `job=<job-name>` (bei `scheduler`) | Ressourcenname | Bei `scheduler`: nur Jobs dieses Clients bzw. diesen Job zeigen. Ein unbekannter Jobname führt zur Auswahlliste der Jobs. |
| `days=<n>` (bei `scheduler`) | −365 bis 365, Vorgabe 7 | Vorschau für so viele Tage; negative Werte zeigen die zurückliegenden Tage. |
| `days=<von>,<bis>` (bei `scheduler`) | zwei Zahlen von −366 bis 366 | Zeitraum relativ zu jetzt. Im Textmodus zählt nur der größere Betrag der beiden Zahlen als Anzahl Tage ab jetzt; unter `.api 2` gilt der Zeitraum genau. |
| `subscriptions` | – | Auswertung der Backup-Einheiten für Subscriptions (aus dem Katalog). Braucht Zugriff auf alle Clients, Jobs und Filesets. |
| `clients` | – | Bei `subscriptions`: Zusammenfassung je Client. |
| `plugins` | – | Bei `subscriptions`: Zusammenfassung je Plugin. |
| `all` (bei `subscriptions`) | – | Bei `subscriptions`: alle Teilberichte. |
| `anonymize` | – | Bei `subscriptions`: Clientnamen durch `Client 1`, `Client 2`, … ersetzen. |
| `client=<client-name>` (bei `subscriptions`) | Name eines Clients | Detailbericht je Fileset für diesen Client. |
| `detail`, `unknown` | – | Veraltete Wörter bei `subscriptions`; sie werden mit Hinweis ignoriert. |
| `configuration` | – | Prüft die Konfiguration auf veraltete Einstellungen. Braucht das Recht für den Befehl `configure`. |
| `all` | – | Director-Status, danach Status jedes Storage Daemons und jedes File Daemons (jede Adresse nur einmal). |

## Dialoge

Ohne Argument fragt `status`, wessen Status gezeigt werden soll. Gültig sind die Nummern 1 bis 5; `.` bricht ab. Eine Zahl außerhalb des Bereichs wird mit einem Hinweis abgelehnt und die Frage wiederholt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status
Status available for:
1: Director
2: Storage
3: Client
4: Scheduler
5: All
Select daemon type for status (1-5): 
> 9
Please enter a number between 1 and 5
Select daemon type for status (1-5): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Bei `status client`, `status storage` oder unbekannten Namen erscheint die Auswahlliste der passenden Ressourcen. Man antwortet mit der Nummer; `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status client
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## API-Modi

- **`.api 0`** (Standard): Textberichte wie in den Beispielen.
- **`.api 1`**: Bei `status director` kommen Kopfzeilen und geplante Jobs, die Tabelle aber tabulatorgetrennt und ohne Überschriften; laufende und beendete Jobs fehlen. Scheduler-, Client- und Storage-Berichte sind gleich wie im Textmodus, nur fehlt bei Client und Storage die Zeile `Connecting to …`. Die Auswahl ohne Argument kommt als Auswahlliste mit Einzeleinträgen, auf die oconsole keinen Eingabeprompt anbietet.
- **`.api 2`**: `status director` liefert `result.header`, `result.scheduled`, `result.running`, `result.terminated` und `result.client-connection`; `status scheduler` liefert `result.schedules` und `result.preview` (mit `runtime` als Unix-Zeit); `status subscriptions` liefert `result.unit-detail`, `result.unit-summary` und `result.checksum`. `status client=…`, `status storage=…`, `status configuration` und `status` ohne Argument liefern nur ein leeres `result`.

## Beispiele

### Status des Directors

Kopfzeile mit Version und Startzeit, geplante Jobs, laufende und beendete Jobs.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status dir
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 16-Oct-26 16:41. Jobs: run=0, running=0 db:postgresql, Onesimus binary

Scheduled Jobs:
Level          Type     Pri  Scheduled          Name               Volume
===================================================================================
Incremental    Backup    10  16-Oct-26 21:00    backup-bareos-fd   Incremental-47322
Full           Backup    11  16-Oct-26 21:10    BackupCatalog      Incremental-47322
Differential   Backup    10  17-Oct-26 21:00    backup-bareos-fd   Incremental-47322
====

Running Jobs:
Console connected at 16-Oct-26 16:43
No Jobs running.
====
No Terminated Jobs.

Client Initiated Connections (waiting for jobs):
Connect time        Protocol            Authenticated       Name                                    
====================================================================================================
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Geplante Jobs weiter im Voraus

Mit `days=` wird der Vorschauzeitraum für geplante Jobs verlängert; Werte über 500 werden mit Hinweis ignoriert.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status dir days=600
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 16-Oct-26 16:41. Jobs: run=0, running=0 db:postgresql, Onesimus binary
Ignoring invalid value for days. Max is 500.

Scheduled Jobs:
Level          Type     Pri  Scheduled          Name               Volume
===================================================================================
Incremental    Backup    10  16-Oct-26 21:00    backup-bareos-fd   Incremental-47322
Full           Backup    11  16-Oct-26 21:10    BackupCatalog      Incremental-47322
Differential   Backup    10  17-Oct-26 21:00    backup-bareos-fd   Incremental-47322
====

Running Jobs:
Console connected at 16-Oct-26 16:43
No Jobs running.
====
No Terminated Jobs.

Client Initiated Connections (waiting for jobs):
Connect time        Protocol            Authenticated       Name                                    
====================================================================================================
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Director-Status unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status dir
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 16-Oct-26 16:41. Jobs: run=0, running=0 db:postgresql, Onesimus binary
Incremental   	Backup  	 10	16-Oct-26 21:00   	backup-bareos-fd  	Incremental-47322
Full          	Backup  	 11	16-Oct-26 21:10   	BackupCatalog     	Incremental-47322
Differential  	Backup  	 10	17-Oct-26 21:00   	backup-bareos-fd  	Incremental-47322
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Director-Status als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status dir
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "header": {
      "director": "bareos-dir",
      "version": "25.1.2+onesimus.0.2.0",
      "release_date": "15 October 2026",
      "os": "Debian GNU/Linux 13 (trixie)",
      "binary_info": "Onesimus",
      "daemon_started": "16-Oct-26 16:41",
      "jobs_run": 0,
      "jobs_running": 0,
      "config_warnings": false
    },
    "scheduled": [
      {
        "name": "backup-bareos-fd",
        "type": "Backup",
        "level": "Incremental",
        "priority": 10,
        "scheduled": "16-Oct-26 21:00",
        "volume": "Incremental-47322",
        "storage": "File"
      },
      {
        "name": "BackupCatalog",
        "type": "Backup",
        "level": "Full",
        "priority": 11,
        "scheduled": "16-Oct-26 21:10",
        "volume": "Incremental-47322",
        "storage": "File"
      },
      {
        "name": "backup-bareos-fd",
        "type": "Backup",
        "level": "Differential",
        "priority": 10,
        "scheduled": "17-Oct-26 21:00",
        "volume": "Incremental-47322",
        "storage": "File"
      }
    ],
    "running": [],
    "terminated": [],
    "client-connection": []
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Status eines File Daemons

Der Director verbindet sich mit dem Client und zeigt dessen Bericht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status client=bareos-fd
Connecting to Client bareos-fd at localhost:9102
 Handshake: Immediate TLS, Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3

WS-JOERG.intranet.bernau.family-fd Version: 25.1.2~pre43.18be77e68 (09 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 12-Okt-26 13:01. Jobs: run=9 running=0, Bareos community binary
 Sizeof: boffset_t=8 size_t=8 debug=0 trace=0 bwlimit=0kB/s

Running Jobs:
bareos-dir (director) connected at: 16-Okt-26 18:54
No Jobs running.
====

Terminated Jobs:
 JobId  Level    Files      Bytes   Status   Finished        Name 
======================================================================
   629  Full         67    14.97 K  OK       10-Jul-26 21:10 BackupCatalog
     1  Incr        180    126.3 M  OK       13-Okt-26 21:00 backup-bareos-fd
     3  Incr          0         0   OK       14-Okt-26 21:00 backup-bareos-fd
     5  Incr         12    965.8 K  OK       15-Okt-26 21:00 backup-bareos-fd
     9  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    10  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    11  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    12  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    13  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    15  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Client

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status client=nosuch
Error: Client resource nosuch does not exist.
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Status eines Storage Daemons

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status storage=File
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

Jobs waiting to reserve a drive:
====

Terminated Jobs:
 JobId  Level    Files      Bytes   Status   Finished        Name 
===================================================================
     3  Incr          0         0   OK       14-Okt-26 21:00 backup-bareos-fd
     5  Incr         12    967.0 K  OK       15-Okt-26 21:00 backup-bareos-fd
     9  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    10  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    11  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    12  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    13  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    15  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
     7  Incr          0         0   Error    15-Okt-26 23:27 backup-bareos-fd
     8  Incr          0         0   Error    15-Okt-26 23:27 backup-bareos-fd
====

Device status:

Device "FileStorage" (/srv/BACKUP/bareos-storage) is not open.
==
====

Used Volume status:
====

====

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status storage=nosuch
Storage resource "nosuch": not found
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3): 
> 
```

### Storage-Status unter .api 2

Unter `.api 2` kommt für Storage und Client nur ein leeres Ergebnis.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status storage=File
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Slots eines Storage

Ist das Gerät kein Autochanger, meldet der Storage Daemon das.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status storage=File slots
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3998 Device ""FileStorage" (/srv/BACKUP/bareos-storage)" is not an autochanger.
Device "FileStorage" has 0 slots.
No slots in changer to scan.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Status über Job oder JobId

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status job=backup-bareos-fd
Job "backup-bareos-fd" is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status jobid=999999
JobId 999999 is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Scheduler-Vorschau

Ohne `days=` zeigt der Scheduler sieben Tage.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status scheduler
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for 7 days:

Date                   Schedule                Overrides
==============================================================
Fri 16-Oct-2026 21:00  ParityPreview           Level=Incremental
Sat 17-Oct-2026 03:30  ParityPreview           Level=Differential Pool=Full
Sat 17-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Sun 18-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Mon 19-Oct-2026 03:00  c019h                   Level=Full
Mon 19-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Mon 19-Oct-2026 21:00  ParityPreview           Level=Incremental
Tue 20-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Tue 20-Oct-2026 18:00  ParityPreview           Level=Full
Tue 20-Oct-2026 21:00  ParityPreview           Level=Incremental
Wed 21-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Wed 21-Oct-2026 21:00  ParityPreview           Level=Incremental
Thu 22-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Thu 22-Oct-2026 21:00  ParityPreview           Level=Incremental
Fri 23-Oct-2026 00:00  ParityPreview           Level=Differential
Fri 23-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vorschau eines Schedules

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status scheduler schedule=ParityPreview days=1
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for 1 days:

Date                   Schedule                Overrides
==============================================================
Sat 17-Oct-2026 03:30  ParityPreview           Level=Differential Pool=Full
Sat 17-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Rückblick in die Vergangenheit

Negative Werte für `days=` zeigen zurückliegende Ausführungen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status scheduler schedule=ParityPreview days=-20
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for -20 days:

Date                   Schedule                Overrides
==============================================================
Sun 27-Sep-2026 01:15  ParityPreview           Level=Full
Sun 27-Sep-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Mon 28-Sep-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Mon 28-Sep-2026 21:00  ParityPreview           Level=Incremental
[…]
Fri 16-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Zeitraum als Bereich

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status scheduler schedule=ParityPreview days=-3,5
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for 5 days:

Date                   Schedule                Overrides
==============================================================
Fri 16-Oct-2026 21:00  ParityPreview           Level=Incremental
Sat 17-Oct-2026 03:30  ParityPreview           Level=Differential Pool=Full
Sat 17-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Sun 18-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Mon 19-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Mon 19-Oct-2026 21:00  ParityPreview           Level=Incremental
Tue 20-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Tue 20-Oct-2026 18:00  ParityPreview           Level=Full
Tue 20-Oct-2026 21:00  ParityPreview           Level=Incremental
Wed 21-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige Anzahl Tage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status scheduler schedule=ParityPreview days=400
Ignoring invalid value for days. Allowed is -366 < days < 366.
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for 7 days:

Date                   Schedule                Overrides
==============================================================
Fri 16-Oct-2026 21:00  ParityPreview           Level=Incremental
Sat 17-Oct-2026 03:30  ParityPreview           Level=Differential Pool=Full
Sat 17-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
[…]
Fri 23-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Scheduler für einen Job oder Client

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status scheduler job=backup-bareos-fd days=3
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for 3 days:

Date  Schedule                Overrides
==============================================================
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Scheduler-Vorschau als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status scheduler schedule=ParityPreview days=3
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "schedules": [],
    "preview": [
      {
        "datetime": "Fri 16-Oct-2026 21:00",
        "runtime": 1792184400,
        "schedule": "ParityPreview",
        "level": "Incremental"
      },
      {
        "datetime": "Sat 17-Oct-2026 03:30",
        "runtime": 1792207800,
        "schedule": "ParityPreview",
        "level": "Differential",
        "pool": "Full"
      },
      {
        "datetime": "Sat 17-Oct-2026 12:45",
[…]
        "accurate": false
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Subscriptions

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status subscriptions
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Bareos version: 25.1.2+onesimus.0.2.0 (Debian GNU/Linux 13 (trixie))
Binary info: Onesimus
Report time: 2026-10-16 16:43:46

Detailed backup unit report:
+-----------+------------------+-------+---------+
| client    | plugin           | count | size_gb |
+-----------+------------------+-------+---------+
| bareos-fd | none (file data) |     1 |    0.98 |
| TOTAL     |                  |     1 |    0.98 |
+-----------+------------------+-------+---------+

Backup unit summary:
 accounting_mode: volume-based (1 volume-based units >= 1 count-based units)
            used: 1
      configured: 0
       remaining: -1

Checksum: {SHA256}84a7592071ce22f977f75ed569b5eeced120328f72d9c893e490701494f33ec
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Subscriptions vollständig und anonymisiert

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status sub all anonymize
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Bareos version: 25.1.2+onesimus.0.2.0 (Debian GNU/Linux 13 (trixie))
Binary info: Onesimus
Report time: 2026-10-16 16:43:47

Detailed backup unit report:
+----------+------------------+-------+---------+
| client   | plugin           | count | size_gb |
+----------+------------------+-------+---------+
| Client 1 | none (file data) |     1 |    0.98 |
| TOTAL    |                  |     1 |    0.98 |
+----------+------------------+-------+---------+

Backup unit report aggregated by client:
+----------+-------+---------+
| client   | count | size_gb |
+----------+-------+---------+
| Client 1 |     1 |    0.98 |
| TOTAL    |     1 |    0.98 |
+----------+-------+---------+
For a detailed per-client report run 'status subscriptions client=<client-name>'

Backup unit report aggregated by plugin:
+------------------+-------+---------+
| plugin           | count | size_gb |
+------------------+-------+---------+
| none (file data) |     1 |    0.98 |
| TOTAL            |     1 |    0.98 |
+------------------+-------+---------+

Backup unit summary:
 accounting_mode: volume-based (1 volume-based units >= 1 count-based units)
            used: 1
      configured: 0
       remaining: -1

Checksum: {SHA256}6172cc52de4240d34ef6eb4db37a4d879068c545bc54b24dbf70d270d22b1b71
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Subscriptions je Client

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status subscriptions clients
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

Backup unit report aggregated by client:
+-----------+-------+---------+
| client    | count | size_gb |
+-----------+-------+---------+
| bareos-fd |     1 |    0.98 |
| TOTAL     |     1 |    0.98 |
+-----------+-------+---------+
For a detailed per-client report run 'status subscriptions client=<client-name>'

Backup unit summary:
 accounting_mode: volume-based (1 volume-based units >= 1 count-based units)
            used: 1
      configured: 0
       remaining: -1

Checksum: {SHA256}d5c96b82e39956b70162ca9386ddd4a2b91da4acec84de56e3ae21971b325
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Subscriptions je Plugin

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status subscriptions plugins unknown
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameter 'unknown' ignored. There is no unknown data anymore.

Backup unit report aggregated by plugin:
+------------------+-------+---------+
| plugin           | count | size_gb |
+------------------+-------+---------+
| none (file data) |     1 |    0.98 |
| TOTAL            |     1 |    0.98 |
+------------------+-------+---------+

Backup unit summary:
 accounting_mode: volume-based (1 volume-based units >= 1 count-based units)
            used: 1
      configured: 0
       remaining: -1

Checksum: {SHA256}c0e527de53738ea1ba7f52fc7ec04cb9abd4483b8a24b6f5f0f8a346cb4790fb
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Subscriptions für einen Client

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status subscriptions client=bareos-fd detail
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameter 'detail' ignored. This is now the default.

Detailed backup unit report for client 'bareos-fd':
+----------------+------------------+---------+
| fileset        | plugin           | size_gb |
+----------------+------------------+---------+
| BenchmarkSmall | none (file data) |    0.82 |
| Catalog        | none (file data) |    0.00 |
| SelfTest       | none (file data) |    0.16 |
+----------------+------------------+---------+
All plugin jobs and the first non-plugin job are counted.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Subscriptions als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status subscriptions
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": "25.1.2+onesimus.0.2.0",
    "os": "Debian GNU/Linux 13 (trixie)",
    "binary-info": "Onesimus",
    "report-time": "2026-10-16 16:43:49",
    "unit-detail": [
      {
        "client": "bareos-fd",
        "plugin": "none (file data)",
        "count": "1",
        "size_gb": "0.98"
      },
      {
        "client": "TOTAL",
        "plugin": "",
        "count": "1",
        "size_gb": "0.98"
      }
    ],
    "unit-summary": {
      "accounting_mode": "volume-based (1 volume-based units >= 1 count-based units)",
      "used": "1",
      "configured": "0",
      "remaining": "-1"
    },
    "checksum": "{SHA256}d5c96b82e39956b70162ca9386ddd4a2b91da4acec84de56e3ae21971b325"
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konfiguration prüfen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status configuration
No deprecated configuration settings detected.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alles auf einmal

`status all` beginnt mit dem Director-Status und fragt danach jeden Storage und jeden Client ab. Im Beispiel endet der Bericht nach dem Director-Teil mit einer Fehlermeldung zum NDMP-Storage der Testumgebung, dessen Adresse sich nicht auflösen lässt; weitere Storage- und Client-Berichte folgen dort nicht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status all
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 16-Oct-26 16:41. Jobs: run=0, running=0 db:postgresql, Onesimus binary

Scheduled Jobs:
Level          Type     Pri  Scheduled          Name               Volume
===================================================================================
Incremental    Backup    10  16-Oct-26 21:00    backup-bareos-fd   Incremental-47322
Full           Backup    11  16-Oct-26 21:10    BackupCatalog      Incremental-47322
Differential   Backup    10  17-Oct-26 21:00    backup-bareos-fd   Incremental-47322
====

Running Jobs:
Console connected at 16-Oct-26 16:43
No Jobs running.
====
No Terminated Jobs.

Client Initiated Connections (waiting for jobs):
Connect time        Protocol            Authenticated       Name                                    
====================================================================================================
====
build_storage_job failed
err bad-host-name
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auswahl ohne Argument

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> status
Status available for:
1: Director
2: Storage
3: Client
4: Scheduler
5: All
Select daemon type for status (1-5): 
> 4
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for 7 days:

Date                   Schedule                Overrides
==============================================================
Fri 16-Oct-2026 21:00  ParityPreview           Level=Incremental
Sat 17-Oct-2026 03:30  ParityPreview           Level=Differential Pool=Full
[…]
Fri 23-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
====
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eingeschränkte Konsole

Eine Konsole ohne Zugriff auf alle Ressourcen bekommt bei `subscriptions` und `configuration` eine Ablehnung. Gibt es nur einen erlaubten Storage, wird er ohne Rückfrage gewählt.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> status subscriptions
status subscriptions: needs access to all client, job and fileset resources.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> status configuration
status configuration: is an invalid command or needs access right to the "configure" command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> status nosuch
Automatically selected Storage: File
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103
 Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3

bareos-sd Version: 25.1.2~pre43.18be77e68 (09 October 2026) Debian GNU/Linux 13 (trixie) 
Daemon started 12-Okt-26 13:01. Jobs: run=11, running=0, Bareos community binary
[…]

Device status:

Device "FileStorage" (/srv/BACKUP/bareos-storage) is not open.
==
====

Used Volume status:
====

====

parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Storage resource "<name>": not found` | Den Storage gibt es nicht oder die Konsole darf ihn nicht sehen. Es folgt die Auswahlliste. |
| `Error: Client resource <name> does not exist.` | Den Client gibt es nicht oder die Konsole darf ihn nicht sehen. Es folgt die Auswahlliste. |
| `Selection aborted, nothing done.` | Die Auswahl wurde mit `.` abgebrochen. |
| `Please enter a number between 1 and 5` | Bei der Auswahl ohne Argument wurde eine ungültige Nummer eingegeben. |
| `JobId <n> is not running.` / `Job "<name>" is not running.` | Bei `jobid=` bzw. `job=` läuft kein solcher Job. Für beendete Jobs `list jobs` verwenden. |
| `Ignoring invalid value for days. Max is 500.` | `days=` bei `status director` außerhalb 0–500; es gilt 1 Tag. |
| `Ignoring invalid value for days. Allowed is -366 < days < 366.` | `days=` bei `status scheduler` außerhalb des Bereichs; es gelten 7 Tage. |
| `Ignoring invalid days range. Allowed: -366..366.` | `days=<von>,<bis>` außerhalb des Bereichs; es gelten 7 Tage ab jetzt. |
| `Parameter 'detail' ignored. This is now the default.` / `Parameter 'unknown' ignored. There is no unknown data anymore.` | Veraltete Wörter bei `status subscriptions`; einfach weglassen. |
| `status subscriptions: needs access to all client, job and fileset resources.` | Die Konsole hat eingeschränkte ACLs. Eine Konsole mit vollem Zugriff verwenden. |
| `status configuration: is an invalid command or needs access right to the "configure" command.` | Die Konsole darf `configure` nicht ausführen. |
| `3998 Device "…" is not an autochanger.` | `slots` für ein Gerät ohne Autochanger. |
| `build_storage_job failed` / `err bad-host-name` | Der Director konnte den Storage (hier ein NDMP-Storage) nicht erreichen, weil die Adresse nicht auflösbar ist. Adresse in der Storage-Ressource prüfen. |

## Siehe auch

- [.status](dot-status.md) — einzelne Teile des Status für Programme
- [list](list.md) — beendete Jobs und Volumes aus dem Katalog
- [messages](messages.md) — aufgelaufene Meldungen anzeigen
- [setdebug](setdebug.md) — Debug-Level von Director, Client und Storage setzen
- [show](show.md) — Konfiguration der Ressourcen anzeigen
