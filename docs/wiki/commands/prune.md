# prune — Abgelaufene Einträge aus dem Katalog entfernen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L383) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Prune records from catalog

**Originalsyntax:**

```text
prune files [client=<client>] [pool=<pool>] [yes] | jobs [client=<client>] [pool=<pool>] [yes] | volume [all] [=volume] [pool=<pool>] [yes] | stats [yes] | directory [=directory] [client=<client>] [recursive] [yes]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 35 von 35 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-c030-client, parity-c030-job, parity-c030-pool, parity-c030-storage.
- Verglichen sind Dialoge, ACL-Prüfungen (C-030) und Bestätigungstexte auf der Katalogkopie; der Katalogzustand nach dem Prunen wird nicht verglichen.

**Paritätsfälle (35):** `c011-prune-cancel`, `c011-prune-api1-cancel`, `c011-prune-files-cancel`, `c011-prune-volume-cancel`, `c011-prune-jobs-cancel`, `c011-prune-jobs-mod-no`, `c011-prune-stats-cancel`, `c011-prune-volume-select-cancel`, `c011-prune-volume-unknown`, `c011-prune-directory-cancel`, `c012-prune-jobs-confirm`, `c030-prune-job`, `c030-prune-storage`, `c030-prune-client`, `c030-prune-pool`, `c030-prune-volume-client`, `c030-prune-stats-client`, `c030-prune-stats-pool`, `c030-prune-directory-pool`, `c030-prune-directory-client-cancel`, `c030-prune-files-pool-cancel`, `c030-prune-jobs-pool-cancel`, `c030-prune-volume-pool-cancel`, `c030-prune-job-api1`, `c030-prune-storage-api2`, `c030-prune-volume-pool-report`, `c030-prune-volume-all-no`, `c030-prune-volume-all-pool-no`, `c030-prune-volume-wrong-pool-no`, `c030-prune-volume-invalid-name`, `c030-prune-files-no`, `c030-prune-files-pool-no`, `c030-prune-directory-client-no-path`, `c030-prune-volume-pool-api1`, `prune-dialog`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:948](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`prune` entfernt Katalogeinträge, deren Aufbewahrungszeit abgelaufen ist: Dateieinträge (File Retention), Jobs (Job Retention), Jobs eines Volumes (Volume Retention), Statistiken (Statistics Retention) oder einzelne Verzeichnisse. Anders als [purge](purge.md) beachtet `prune` die konfigurierten Aufbewahrungszeiten. Die Daten auf den Volumes bleiben unberührt. Der Director zeigt vor dem Löschen die gültige Aufbewahrungszeit und fragt nach; mit `yes` entfällt die Frage. Je nach Art braucht die Konsole vollen Zugriff (ACL ohne Einschränkung) auf Jobs, Clients, Storages bzw. Pools; sonst antwortet der Director mit `Permission denied`.

## Syntax

```text
prune
prune files [client=<client>] [pool=<pool>] [yes]
prune jobs [client=<client>] [pool=<pool>] [yes]
prune volume[=<volume>] [pool=<pool>] [yes]
prune volume all [pool=<pool>] [yes]
prune stats [yes]
prune directory[=<pfad>] [client=<client>] [recursive] [yes]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `files` | – | Dateieinträge nach File Retention des Clients entfernen. |
| `jobs` | – | Jobs nach Job Retention des Clients entfernen. |
| `volume[=<volume>]` | Volume-Name | Jobs des Volumes nach dessen Volume Retention entfernen; ohne Wert Auswahl. |
| `all` | – | Mit `volume`: alle Volumes (eines Pools). |
| `stats` | – | JobHisto, DeviceStats und JobStats nach Statistics Retention bereinigen. |
| `directory[=<pfad>]` | Pfad | Dateieinträge unterhalb eines Verzeichnisses entfernen. |
| `recursive` | – | Mit `directory`: auch Unterverzeichnisse. |
| `client=<client>` | Client | Client, dessen Einträge bereinigt werden; sonst Auswahl. |
| `pool=<pool>` | Pool | Pool, dessen Aufbewahrungszeit gilt bzw. dessen Volumes gemeint sind. |
| `yes` | – | Ohne Rückfrage ausführen. |

## Dialoge

- Ohne Argument: `Choose item to prune (1-5):` mit Files, Jobs, Volume, Stats, Directory. `.` bricht ab.
- Fehlt der Client, folgt die Client-Auswahl, bei Volume die Pool- und Volume-Auswahl.
- Vor dem Löschen: `The current <Art> retention period is: <zeit>` und `Continue? (yes/mod/no):`. `yes` führt aus, `no` und `.` beenden ohne Meldung. `mod` fragt `Enter new retention period:` und zeigt danach die neue Zeit; die geänderte Zeit gilt für den laufenden Director, bis die Konfiguration neu geladen wird.
- `prune directory` ohne Client fragt zuerst `No client restriction given really remove directory for all clients (yes/no):` und dann `Please enter the full path to remove:`.

## API-Modi

Unter `.api 0` und `.api 1` kommen dieselben Texte. Unter `.api 2` wird nicht nachgefragt; fehlende Angaben führen zu einem leeren `result`, fehlende Rechte zu einem JSON-RPC-Fehler mit dem Text in `messages.error`.

## Beispiele

### Dateieinträge eines Clients

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune files client=bareos-fd yes
Using Catalog "MyCatalog"
The current File retention period is: 2 months
Begin pruning Files.
Pruned Files from 92 Jobs for client bareos-fd from catalog.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit Rückfrage und geänderter Aufbewahrungszeit

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune files client=bareos-fd
Using Catalog "MyCatalog"
The current File retention period is: 2 months
Continue? (yes/mod/no):
> mod
Enter new retention period:
> 3 months
The current File retention period is: 3 months
Continue? (yes/mod/no):
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobs eines Clients

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune jobs client=bareos-fd yes
Using Catalog "MyCatalog"
The current Job retention period is: 6 months
Begin pruning Jobs older than 6 months .
Purging the following 43 JobIds: 479,480,481,482,483,484,485,486,487,488,489,490,491,492,493,494,495,496,497,498,499,500,501,502,503,504,505,506,510,512,513,514,515,516,517,518,519,520,521,522,523,524,525
Pruned 43 Jobs for client bareos-fd from catalog.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune volume=Full-0001 yes
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The current Volume (Full-0001) retention period is: 0 secs
Pruning volume Full-0001: cannot prune as Volstatus is Error but needs to be Full or Used.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune volume all pool=Full
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The current Volume (Full-0001) retention period is: 0 secs
Continue? (yes/mod/no):
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune volume pool=Incremental
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
No prunable volumes found in Pool "Incremental".
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Statistiken

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune stats yes
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The current Statistics retention period is: 5 years 1 month 5 days
Pruned Jobs from JobHisto in catalog.
Pruned Statistics from DeviceStats in catalog.
Pruned Statistics from JobStats in catalog.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auswahlmenü

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have the following choices:
1: Files
2: Jobs
3: Volume
4: Stats
5: Directory
Choose item to prune (1-5):
> 1
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Verzeichnis

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> prune directory
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
No client restriction given really remove directory for all clients (yes/no):
> no
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlende Rechte

```text
parity-c030-job@bareos-dir[25.1.2+onesimus.0.2.0]> prune
Permission denied: need full job permission.
parity-c030-job@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
parity-c030-storage@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
parity-c030-storage@bareos-dir[25.1.2+onesimus.0.2.0]> prune
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "error": [
          "Permission denied: need full storage permission.\n"
        ]
      }
    }
  }
}
parity-c030-storage@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Permission denied: need full <job\|client\|storage\|pool> permission.` | Der Konsole fehlt der volle Zugriff auf diese Ressourcenart. |
| `cats/sql_get.cc:1177 Media record for Volume "<name>" not found.` | Volume nicht im Katalog. |
| `Volume "<name>" is not in Pool "<pool>".` | Volume und Pool passen nicht zusammen. |
| `No prunable volumes found in Pool "<pool>".` | Kein Volume des Pools hat abgelaufene Jobs. |
| `Pruning volume <name>: cannot prune as Volstatus is <status> but needs to be Full or Used.` | Nur volle oder benutzte Volumes lassen sich prunen. |
| `Selection aborted, nothing done.` | Auswahl mit `.` abgebrochen. |

## Siehe auch

- [purge](purge.md) — Einträge ohne Rücksicht auf Aufbewahrungszeiten entfernen
- [delete](delete.md) — einzelne Volumes, Jobs oder Pools löschen
- [list](list.md) — Jobs und Volumes anzeigen
- [show](show.md) — Aufbewahrungszeiten in Client- und Pool-Ressourcen ansehen
