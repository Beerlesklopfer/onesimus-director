# .jobstatus — Bedeutung der Job-Statuscodes anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L249) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List jobstatus information

**Originalsyntax:**

```text
.jobstatus [ =<jobstatus> ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 21 von 21 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (21):** `c029-jobstatus`, `c029-jobstatus-T`, `c029-jobstatus-e`, `c029-jobstatus-unknown-char`, `c029-jobstatus-long`, `c029-jobstatus-empty`, `c029-jobstatus-arg-ignored`, `c029-jobstatus-kv-ignored`, `c029-jobstatus-twice`, `c029-jobstatus-catalog`, `c029-jobstatus-catalog-bogus`, `c029-jobstatus-api1`, `c029-jobstatus-api1-T`, `c029-jobstatus-api1-bad`, `c029-jobstatus-api2`, `c029-jobstatus-api2-T`, `c029-jobstatus-api2-Z`, `c029-jobstatus-api2-bad`, `c029-jobstatus-api1-Z`, `c029-jobstatus-quote`, `dot-jobstatus`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:150](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.jobstatus` zeigt die Tabelle der Job-Statuscodes aus dem Katalog: Kennbuchstabe (`T`, `E`, `f`, …), Klartext, Schweregrad sowie Exit-Level und Exit-Status für die abgeschlossenen Zustände. Programme nutzen sie, um den einbuchstabigen Status aus `list jobs` verständlich anzuzeigen. Der Befehl liest die Tabelle `Status` des Katalogs und verändert nichts.

## Syntax

```text
.jobstatus
.jobstatus=<code>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `=<code>` | ein einzelnes Zeichen, z. B. `T` | direkt an den Befehlsnamen gehängt: nur die Zeile dieses Codes |
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

Der Filter muss am Befehlswort selbst hängen (`.jobstatus=T`). Ein getrenntes Wort wie `.jobstatus T` oder `.jobstatus status=T` wird ignoriert und liefert die ganze Tabelle. Ein unbekannter Katalogname wird ebenfalls ignoriert; dann wird wie ohne Argument der erste Katalog gewählt.

## API-Modi

- `.api 0`: Tabelle mit Rahmen, davor die Katalog-Hinweiszeilen beim ersten Katalogzugriff der Sitzung.
- `.api 1`: nur die Tabellenzeilen ohne Rahmenlinien, Kopfzeile und Hinweise.
- `.api 2`: `result.jobstatus`, ein Array von Objekten mit `jobstatus`, `jobstatuslong`, `severity`, `exitlevel` und `exitstatus` (alle als Zeichenketten, fehlende Werte leer).

## Beispiele

### Alle Statuscodes

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobstatus
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------+---------------------------------+----------+-----------+------------+
| jobstatus | jobstatuslong                   | severity | exitlevel | exitstatus |
+-----------+---------------------------------+----------+-----------+------------+
| A         | Canceled by user                |       90 | 2         | Error      |
| B         | Blocked                         |       15 |           |            |
| C         | Created, not yet running        |       15 |           |            |
| D         | Verify found differences        |       15 |           |            |
| E         | Terminated with errors          |       25 | 2         | Error      |
| F         | Waiting for Client              |       15 |           |            |
| I         | Incomplete job                  |       15 |           |            |
| L         | Committing data                 |       15 |           |            |
| M         | Waiting for media mount         |       15 |           |            |
| R         | Running                         |       15 |           |            |
| S         | Waiting for Storage daemon      |       15 |           |            |
| T         | Completed successfully          |       10 | 0         | Ok         |
| W         | Terminated with warnings        |       20 | 1         | Warning    |
| a         | SD despooling attributes        |       15 |           |            |
| c         | Waiting for client resource     |       15 |           |            |
| d         | Waiting on maximum jobs         |       15 |           |            |
| e         | Non-fatal error                 |       20 | 2         | Error      |
| f         | Fatal error                     |      100 | 2         | Error      |
| i         | Doing batch insert file records |       15 |           |            |
| j         | Waiting for job resource        |       15 |           |            |
| l         | Doing data despooling           |       15 |           |            |
| m         | Waiting for new media           |       15 |           |            |
| p         | Waiting on higher priority jobs |       15 |           |            |
| q         | Queued waiting for device       |       15 |           |            |
| s         | Waiting for storage resource    |       15 |           |            |
| t         | Waiting on start time           |       15 |           |            |
+-----------+---------------------------------+----------+-----------+------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ein einzelner Code

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobstatus=T
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------+------------------------+----------+-----------+------------+
| jobstatus | jobstatuslong          | severity | exitlevel | exitstatus |
+-----------+------------------------+----------+-----------+------------+
| T         | Completed successfully |       10 | 0         | Ok         |
+-----------+------------------------+----------+-----------+------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobstatus=T
 T         | Completed successfully |       10 | 0         | Ok         |
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobstatus=T
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobstatus": [
      {
        "jobstatus": "T",
        "jobstatuslong": "Completed successfully",
        "severity": "10",
        "exitlevel": "0",
        "exitstatus": "Ok"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Getrenntes Argument wird ignoriert

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobstatus T
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------+---------------------------------+----------+-----------+------------+
| jobstatus | jobstatuslong                   | severity | exitlevel | exitstatus |
+-----------+---------------------------------+----------+-----------+------------+
| A         | Canceled by user                |       90 | 2         | Error      |
| B         | Blocked                         |       15 |           |            |
| C         | Created, not yet running        |       15 |           |            |
[…]
+-----------+---------------------------------+----------+-----------+------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Unknown JobStatus 'TT'. JobStatus must be a single character.` | Hinter `=` steht mehr als ein Zeichen. |

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobstatus=TT
Unknown JobStatus 'TT'. JobStatus must be a single character.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Siehe auch

- [list](list.md) — `list jobs jobstatus=<code>`
- [status](status.md) — laufende und beendete Jobs
- [.jobs](dot-jobs.md) — Namen der Jobs
