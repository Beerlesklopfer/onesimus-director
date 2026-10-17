# delete — Einträge aus dem Katalog löschen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L323) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Delete volume, pool or job

**Originalsyntax:**

```text
delete [volume=<vol-name> [yes] | volume pool=<pool-name> [yes] | storage storage=<storage-name> | pool=<pool-name> | jobid=<jobid> | jobid=<jobid1,jobid2,...> | jobid=<jobid1-jobid9>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 15 von 16 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Abweichende Fälle: `c035-delete-noarg-api2`.
- `.api 2`: Original liefert `"deleted": {}`, Onesimus ein leeres Ergebnis (C-046, Fall `c035-delete-noarg-api2`).
- Löschen von Volume/Pool/Job wird nur über Dialog- und Abbruchpfade verglichen; die Katalogwirkung ist nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-046` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (16):** `c011-delete-cancel`, `c011-delete-volume-cancel`, `c011-delete-menu-pool-cancel`, `c011-delete-menu-jobid-cancel`, `c011-delete-jobid-cancel`, `c011-delete-pool-cancel`, `c011-delete-volume-pool-cancel`, `c011-delete-volume-unknown`, `c011-delete-volume-star-unknown`, `c011-delete-volname-unknown`, `c011-delete-storage-empty`, `c011-delete-api1-cancel`, `c012-delete-illegal-jobid`, `c035-delete-noarg-api1`, `c035-delete-noarg-api2`, `delete-dialog`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:602](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`delete` entfernt Volumes, Jobs, Pools oder verwaiste Storage-Einträge aus dem Katalog. Die Daten auf dem Volume selbst bleiben unberührt; nur die Katalogeinträge verschwinden, und damit kann man die Daten nicht mehr über [restore](restore.md) auswählen. Beim Löschen eines Volumes werden auch alle Jobs entfernt, die auf diesem Volume gespeichert sind. Der Befehl wirkt sofort und lässt sich nicht rückgängig machen.

## Syntax

```text
delete
delete volume[=<volume>] [pool=<pool>] [yes]
delete pool[=<pool>]
delete jobid[=<n>[,<n>|<von>-<bis>…]]
delete storage[=<storage>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `volume=<volume>` | Volume-Name | Dieses Volume und seine Jobs löschen. Ohne Wert: Pool- und Volume-Auswahl. |
| `pool=<pool>` | Pool-Name | Mit `volume`: Pool für die Volume-Auswahl. Allein: diesen Pool löschen. |
| `jobid=<liste>` | JobIds, Komma-Liste, Bereiche `von-bis` | Diese Jobs mit Dateien und JobMedia-Einträgen löschen. Ohne Wert: Abfrage der JobId. |
| `storage=<storage>` | Name | Storage-Eintrag löschen, der nicht mehr in der Konfiguration steht. |
| `catalog=<katalog>` | Name einer Catalog-Ressource | Katalog wählen. |
| `yes` | – | Volume ohne Rückfrage löschen. |

## Dialoge

- Ohne Argument warnt der Director und fragt `Choose catalog item to delete (1-4):` (volume, pool, jobid, storage). `.` beendet mit `Selection aborted, nothing done.` und `Nothing done.`
- **Volume**: Auswahl des Pools (`Select the Pool (1-n):`), dann die Volume-Liste des Pools und `Enter the volume name or MediaId of the volume prefixed with an asterisk (*).` Danach `Are you sure you want to delete Volume "<name>"? (yes/no):`; nur `yes` löscht.
- **Pool**: Pool-Auswahl, dann `Are you sure you want to delete Pool "<name>"? (yes/no):`.
- **JobId**: Ohne Wert fragt der Director `Enter JobId to delete:`. Ein Bereich mit mehr als 25 JobIds verlangt `Are you sure you want to delete <n> JobIds ? (yes/no):`.
- **Storage**: Ohne Wert fragt der Director `Enter storage to delete:`.

In jeder Auswahl bricht `.` ab; ein Wort statt einer Nummer führt zu `Expected a positive integer, got: …`.

## API-Modi

Unter `.api 1` kommen dieselben Texte; die Auswahllisten sind für grafische Oberflächen gedacht und in oconsole nicht bedienbar. Unter `.api 2` gibt es keine Rückfragen: `delete volume=<name> yes` und `delete jobid=…` löschen sofort, das Ergebnis steht in `result.deleted` mit den Listen `volumes` und `jobids`. `delete pool` ist unter `.api 2` nicht erlaubt.

## Beispiele

### Job löschen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete jobid=14
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Jobid 14 and associated records deleted from the catalog.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Jobs und Bereiche

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete jobid=507-509,511
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Jobid 507 and associated records deleted from the catalog.
Jobid 508 and associated records deleted from the catalog.
Jobid 509 and associated records deleted from the catalog.
Jobid 511 and associated records deleted from the catalog.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Großer Bereich mit Rückfrage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete jobid=520-560
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Are you sure you want to delete 40 JobIds ? (yes/no):
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume löschen, Rückfrage ablehnen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete volume=Full-0002
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

This command will delete volume Full-0002
and all Jobs saved on that volume from the Catalog
Are you sure you want to delete Volume "Full-0002"? (yes/no):
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume ohne Rückfrage löschen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete volume=Full-0002 yes
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

This command will delete volume Full-0002
and all Jobs saved on that volume from the Catalog
Deleted 1 jobs and associated records deleted from the catalog (jobids: 583).
Volume Full-0002 deleted.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume über die Auswahl

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete volume
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
> 2
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes       | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,318 | Full-0001  | Error     |       1 | 23,902,210,671 |        5 |            0 |       1 |    0 |         0 | File      | 2026-03-30 22:54:15 | File    |                0 |
|  47,320 | Full-0002  | Error     |       1 |    124,004,811 |        0 |            0 |       1 |    0 |         0 | File      |                     |         |                0 |
|  47,321 | Full-47321 | Append    |       1 |    158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
Enter the volume name or MediaId of the volume prefixed with an asterisk (*).
E.g. "full-0001" or "*42":
> nosuchvol
cats/sql_get.cc:1177 Media record for Volume "nosuchvol" not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Pool löschen, Rückfrage ablehnen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete pool=Scratch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Are you sure you want to delete Pool "Scratch"? (yes/no):
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beispielausgabe nicht verfügbar: nach `yes` antwortet Onesimus `delete pool: not supported`; das Löschen von Pools ist noch nicht umgesetzt.

### Auswahlmenü

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
In general it is not a good idea to delete either a
Pool or a Volume since they may contain data.

You have the following choices:
1: volume
2: pool
3: jobid
4: storage
Choose catalog item to delete (1-4):
> 3
Enter JobId to delete:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage-Eintrag

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete storage
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Enter storage to delete:
> .
Found 0 orphaned Storage records.
The given storage '' either does not exist at all, or still exists in the configuration. In order to remove a storage from the catalog, its configuration must be removed first.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige JobId

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete jobid=x
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Illegal JobId x ignored
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete volume=Incremental-47322 yes
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "deleted": {
      "jobids": [
        "607",
        "605",
        "1",
        "5",
        "602",
        "620",
        "624",
        "616",
        "622",
        "626"
      ],
      "volumes": [
        "Incremental-47322"
      ]
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete jobid=a-b,x
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {
        "deleted": {
          "jobids": []
        }
      },
      "messages": {
        "error": [
          "Illegal JobId range a - b, ignored\n",
          "Illegal JobId x ignored\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> delete pool=Full
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {
        "deleted": {}
      },
      "messages": {
        "error": [
          "Deleting pools from the catalog is not supported in batch mode.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Illegal JobId <wert> ignored` | Kein Zahlenwert in der JobId-Liste; die übrigen werden trotzdem gelöscht. |
| `Illegal JobId range <von> - <bis>, ignored` | Bereich mit nicht numerischen Grenzen. |
| `Illegal JobId range <von> - <bis> should define increasing JobIds, ignored` | Bereich absteigend. |
| `cats/sql_get.cc:1177 Media record for Volume "<name>" not found.` | Volume nicht im Katalog. |
| `cats/sql_get.cc:1174 Media record MediaId=<n> not found.` | `*<MediaId>` nicht im Katalog. |
| `The given storage '<name>' either does not exist at all, or still exists in the configuration. …` | Storage-Einträge lassen sich nur löschen, wenn die Storage-Ressource aus der Konfiguration entfernt ist. |
| `Deleting pools from the catalog is not supported in batch mode.` | `delete pool` unter `.api 2`. |
| `delete pool: not supported` / `delete storage: not supported` | Onesimus löscht Pools und verwaiste Storage-Einträge noch nicht. |
| `Nothing done.` | Menü ohne Auswahl verlassen. |

## Siehe auch

- [purge](purge.md) — Jobs und Dateien eines Volumes ohne Rücksicht auf Aufbewahrungszeiten entfernen
- [prune](prune.md) — abgelaufene Einträge nach Aufbewahrungszeit entfernen
- [list](list.md) — Volumes, Pools und Jobs anzeigen
- [truncate](truncate.md) — gelöschte (Purged) Volumes leeren
