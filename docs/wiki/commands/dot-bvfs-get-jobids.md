# .bvfs_get_jobids — Job-Kette für einen Restore ermitteln (BVFS)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L292) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Get jobids required for a restore

**Originalsyntax:**

```text
.bvfs_get_jobids jobid=<jobid> | ujobid=<unique-jobid> [all]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 7 von 7 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-bvfs.

**Paritätsfälle (7):** `c022-get-jobids`, `c022-get-jobids-all`, `c022-get-jobids-ujobid`, `c022-get-jobids-errors`, `c022-get-jobids-api2`, `c022-acl-limited`, `bvfs-get-jobids`.

**Lokaler Handler:** [crates/odir/src/commands/bvfs.rs:998](../../../crates/odir/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_get_jobids` liefert zu einem Job die Kette der JobIds, die für einen vollständigen Restore seines Stands nötig ist: die Full-Sicherung, die letzte Differential und alle Incrementals bis zu diesem Job. Die Liste dient als `jobid=`-Argument für [.bvfs_update](dot-bvfs-update.md), [.bvfs_lsdirs](dot-bvfs-lsdirs.md) und [.bvfs_lsfiles](dot-bvfs-lsfiles.md). Der Befehl liest nur aus dem Katalog.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Der Onesimus-Weg für Restore-Bäume ist OVFS; das Gegenstück zu diesem Befehl ist [.ovfs_latest](dot-ovfs-latest.md).

## Syntax

```text
.bvfs_get_jobids jobid=<id> [all]
.bvfs_get_jobids ujobid=<jobname> [all]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<id>` | JobId | Job, dessen Stand wiederhergestellt werden soll. |
| `ujobid=<jobname>` | eindeutiger Jobname, z. B. `backup-bareos-fd.2026-07-10_21.00.00_04` | Statt `jobid=`; hat Vorrang. |
| `all` | – | Ketten aller FileSets des Clients zusammenfassen (je FileSet der neueste Stand bis zur Startzeit des Jobs). |

Für einen Job der Stufe Base kommt nur seine eigene JobId zurück. JobIds, die die Konsole nach Job-, Client- oder FileSet-ACL nicht sehen darf, fallen aus der Liste.

## API-Modi

`.api 0` und `.api 1` liefern die JobIds als eine Zeile, durch Kommas getrennt (unter `.api 1` ohne die Katalogzeilen). `.api 2` liefert `result.jobids` als Liste von Objekten `{"id": "<jobid>"}`; Fehler stehen in `error.data.messages.error`.

## Beispiele

### Job-Kette zu einer JobId

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
600,615,616,618,620,622,624,626,628,1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=628
600,615,616,618,620,622,624,626,628
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=629
629
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=5
600,615,616,618,620,622,624,626,628,1,3,5,9,10
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ketten aller FileSets

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=1 all
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
506,629,600,615,616,618,620,622,624,626,628,1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=629 all
506,629,600,615,616,618,620,622,624,626,628
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Über den eindeutigen Jobnamen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids ujobid=backup-bareos-fd.2026-07-10_21.00.00_04
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
600,615,616,618,620,622,624,626,628
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=5
600,615,616,618,620,622,624,626,628,1,3,5,9,10
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=5 all
506,629,600,615,616,618,620,622,624,626,628,1,3,5,9,10
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=1
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobids": [
      {
        "id": "600"
[…]
        "id": "1"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=99999
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
          "Unable to get Job record for JobId=99999: ERR=cats/sql_get.cc:275 No Job found for JobId 99999\n\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole mit eingeschränkten ACLs

Die Konsole `parity-bvfs` darf JobId 1 nicht sehen; die Liste bleibt leer.

```text
parity-bvfs@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

parity-bvfs@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlende oder unbekannte Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Can't find ujobid or jobid argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids jobid=99999
Unable to get Job record for JobId=99999: ERR=cats/sql_get.cc:275 No Job found for JobId 99999

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_get_jobids ujobid=nosuchjob
Unable to get Job record for JobId=nosuchjob: ERR=cats/sql_get.cc:273 No Job found for JobName 'nosuchjob'

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Can't find ujobid or jobid argument` | Weder `jobid=` noch `ujobid=` angegeben. |
| `Unable to get Job record for JobId=<id>: ERR=cats/sql_get.cc:275 No Job found for JobId <id>` | Die JobId gibt es nicht im Katalog. |
| `Unable to get Job record for JobId=<name>: ERR=cats/sql_get.cc:273 No Job found for JobName '<name>'` | Der Jobname aus `ujobid=` ist unbekannt. |

## Siehe auch

- [.bvfs_update](dot-bvfs-update.md) — BVFS-Cache für die Kette füllen
- [.bvfs_lsdirs](dot-bvfs-lsdirs.md) — Verzeichnisse der Kette auflisten
- [.ovfs_latest](dot-ovfs-latest.md) — Job-Kette des neuesten Stands (Onesimus)
- [restore](restore.md) — Restore im Dialog
