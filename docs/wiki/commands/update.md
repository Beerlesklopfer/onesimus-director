# update — Volumes, Pools, Slots und Statistik im Katalog ändern

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L501) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Update volume, pool, slots, job or statistics

**Originalsyntax:**

```text
update [volume=<volume-name> [volstatus=<status>] [volretention=<time-def>] [actiononpurge=<action>] [encrypt=<yes/no/rotate>] [pool=<pool-name>] [recycle=<yes/no>] [slot=<number>] [inchanger=<yes/no>]] | [pool=<pool-name> [maxvolbytes=<size>] [maxvolfiles=<nb>] [maxvoljobs=<nb>][enabled=<yes/no>] [recyclepool=<pool-name>] [actiononpurge=<action>] | slots [storage=<storage-name>] [scan]] | [jobid=<jobid> [jobname=<name>] [starttime=<time-def>] [client=<client-name>] [filesetid=<fileset-id>] [jobtype=<job-type>]] | [stats [days=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 47 von 47 Fällen byte-identisch; Modi: Text, API 1.
- `update slots` meldet „not supported“ (C-054). Volume-/Pool-Parameter sind verglichen, `update stats` und Autochanger-Pfade nicht vollständig.

**Offene Logeinträge:** `C-054` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (47):** `c011-update-cancel`, `c011-update-volume-cancel`, `c011-update-pool-cancel`, `c011-update-stats-cancel`, `c011-update-volume-done`, `c011-update-volume-status-cancel`, `c011-update-volume-badnumber-cancel`, `c011-update-pool-select-cancel`, `c011-update-volume-unknown`, `c011-update-slots`, `c011-update-jobid-missing`, `c011-update-api1-cancel`, `c012-update-bad-volstatus`, `update-dialog`, `t001-update-volstatus-bad`, `t001-update-missing-media-first`, `t001-update-maxvoljobs-word`, `t001-update-maxvolfiles-junk`, `t001-update-maxvolbytes-bad`, `t001-update-volretention-bad`, `t001-update-voluse-bad`, `t001-update-recycle-bad`, `t001-update-inchanger-bad`, `t001-update-slot-negative`, `t001-update-slot-maxvols`, `t001-update-enabled-bad`, `t001-update-enabled-word`, `t001-update-encrypt-bad`, `t001-update-encrypt-nokey`, `t001-update-pool-unknown`, `t001-update-recyclepool-unknown`, `t001-update-allfrompool-unknown`, `t001-update-pool-resource-unknown`, `t001-update-two-errors`, `t001-dialog-status-cancel`, `t001-dialog-retention-bad`, `t001-dialog-use-bad`, `t001-dialog-maxjobs-bad`, `t001-dialog-maxfiles-bad`, `t001-dialog-maxbytes-bad`, `t001-dialog-recycle-bad`, `t001-dialog-slot-bad`, `t001-dialog-inchanger-bad`, `t001-dialog-volfiles-decline`, `t001-dialog-pool-unknown`, `t001-dialog-enabled-bad`, `t001-dialog-recyclepool-cancel`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:220](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`update` ändert Einträge im Katalog: die Eigenschaften eines Volumes (Status, Aufbewahrung, Grenzen, Slot, Pool …), den Pool-Eintrag nach einer Konfigurationsänderung, die Slot-Belegung eines Autochangers, einzelne Felder eines Jobs und die Langzeitstatistik. Bis auf `update slots`, das den Storage Daemon nach dem Inhalt des Autochangers fragt, arbeitet der Befehl nur im Katalog. Ohne Argumente fragt er im Menü, was geändert werden soll.

## Syntax

```text
update
update volume [ volume=<volume> ] [ <parameter>=<wert> … ]
update volume=<volume> <parameter>=<wert> …
update volume allfrompool=<pool>
update volume fromallpools
update pool[=<pool>]
update slots[=<slotliste>] [ storage=<storage> ] [ drive=<laufwerk> ] [ scan ] [ enabled=<wert> ]
update jobid=<jobid> [ starttime=<zeit> ] [ client=<client> ] [ filesetid=<id> ] [ jobname=<name> ] [ jobtype=<typ> ]
update stats [ days=<tage> ]
```

`media` ist gleichbedeutend mit `volume`, `slot` mit `slots`.

## Argumente

### Was geändert wird

| Argument | Werte | Bedeutung |
|---|---|---|
| `volume`, `volume=<volume>` (auch `media`) | Volume-Name | Volume-Eigenschaften ändern. Ohne Namen wählt man das Volume im Dialog. |
| `pool`, `pool=<pool>` | Pool-Name | Pool-Eintrag im Katalog aus der Pool-Ressource der Konfiguration aktualisieren. |
| `slots`, `slots=<slotliste>` (auch `slot`) | z. B. `1-3,7` | Slot-Belegung aus dem Autochanger übernehmen. |
| `jobid=<jobid>` | Job-ID | Felder eines Job-Eintrags ändern. |
| `stats` | — | Langzeitstatistik der Jobs fortschreiben. |

### Parameter für `update volume`

Mehrere Parameter dürfen in einer Zeile stehen; jeder wird einzeln geprüft und gemeldet.

| Argument | Werte | Bedeutung |
|---|---|---|
| `volstatus=<status>` | `Append`, `Archive`, `Disabled`, `Full`, `Used`, `Cleaning`, `Recycle`, `Read-Only`, `Error` | Neuer Volume-Status. Groß-/Kleinschreibung egal. |
| `volretention=<dauer>` | Zeitangabe, z. B. `30days`, `6months` | Aufbewahrungszeit des Volumes. |
| `voluse=<dauer>` | Zeitangabe | Wie lange das Volume nach dem ersten Schreiben benutzt werden darf. |
| `maxvoljobs=<n>` | Ganzzahl | Höchstzahl der Jobs auf dem Volume (0 = unbegrenzt). |
| `maxvolfiles=<n>` | Ganzzahl | Höchstzahl der Dateien. |
| `maxvolbytes=<größe>` | Größe, z. B. `10G`, `500M` | Höchstgröße des Volumes. |
| `recycle=<yes\|no>` | `yes`, `no` | Ob das Volume wiederverwendet werden darf. |
| `inchanger=<yes\|no>` | `yes`, `no` | Ob das Volume im Autochanger liegt. |
| `slot=<n>` | 0 bis `Maximum Volumes` des Pools | Slot im Autochanger. |
| `enabled=<wert>` | `yes`, `no`, `archived`, `0`, `1`, `2` | Volume für neue Jobs freigeben, sperren oder als archiviert markieren. |
| `encrypt=<wert>` | `yes`, `no`, `rotate` | Schlüssel für Hardware-Verschlüsselung setzen, entfernen oder erneuern. Nur bei Volumes mit Schlüssel. |
| `pool=<pool>` | Pool-Name | Volume in einen anderen Pool verschieben. |
| `recyclepool=<pool>` | Pool-Name | Pool, in den das Volume beim Recyceln wandert. |
| `actiononpurge=<wert>` | `None`, `Truncate` | Was beim Leeren mit dem Volume geschieht. |
| `storage=<storage>` | Storage-Name | Storage-Zuordnung des Volumes. |
| `frompool=<beliebig>` | — | Volume-Standardwerte erneut aus seinem Pool übernehmen. Ohne `=` und Wert öffnet sich stattdessen das Menü. |
| `allfrompool=<pool>` | Pool-Name | Standardwerte aller Volumes dieses Pools neu setzen. |
| `fromallpools` | — | Standardwerte aller Volumes aller Pools neu setzen. |

### Parameter für `update slots`, `update jobid` und `update stats`

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Autochanger-Storage | Autochanger, dessen Slots gelesen werden. Auch der Name allein ist möglich (`update slots File`). |
| `drive=<laufwerk>` | Ganzzahl ab 0 | Laufwerk für `scan`. |
| `scan` | — | Volumes ohne Barcode-Leser einzeln ins Laufwerk laden und das Label lesen. |
| `enabled=<wert>` | `yes`, `no`, `archived`, `0`, `1`, `2` | Enabled-Wert für die gefundenen Volumes. |
| `starttime=<zeit>` | `JJJJ-MM-TT hh:mm:ss` | Neue Startzeit des Jobs. |
| `client=<client>` | Client-Name | Neuer Client des Jobs. |
| `filesetid=<id>` | Ganzzahl | Neues FileSet des Jobs. |
| `jobname=<name>` | Job-Name | Neuer Job-Name. |
| `jobtype=<typ>` | Job-Typ-Buchstabe | Neuer Job-Typ. |
| `days=<tage>` | Ganzzahl | Nur Jobs der letzten `<tage>` Tage in die Statistik übernehmen. |

## Dialoge

### Hauptmenü

Ohne Schlüsselwort zeigt `update` das Menü `Update choice:` mit `Volume parameters`, `Pool from resource`, `Slots from autochanger` und `Long term statistics` und fragt `Choose catalog item to update (1-4):`.

### Parametermenü für Volumes

Bei `update volume` ohne `<parameter>=<wert>` erscheint `Parameters to modify:` mit 19 Punkten und der Frage `Select parameter to modify (1-19):`. Nach jeder Änderung kommt das Menü erneut, bis man `19` (`Done`) wählt; der Director bestätigt das mit `Selection terminated.`. Ist noch kein Volume gewählt, fragt der Director nach dem gewählten Punkt zuerst den Pool (`Select the Pool (1-<n>):`) und dann das Volume (`Enter the volume name or MediaId of the volume prefixed with an asterisk (*).`) ab.

Die Fragen der einzelnen Punkte:

| Punkt | Frage | Gültige Antworten |
|---|---|---|
| 1 Volume Status | `Choose new Volume Status (1-7):` | Nummer aus der Liste `Append` … `Read-Only` |
| 2 Volume Retention Period | `Enter Volume Retention period:` | Zeitangabe, z. B. `60days` |
| 3 Volume Use Duration | `Enter Volume Use Duration:` | Zeitangabe |
| 4 Maximum Volume Jobs | `Enter new Maximum Jobs:` | Ganzzahl ≥ 0 |
| 5 Maximum Volume Files | `Enter new Maximum Files:` | Ganzzahl ≥ 0 |
| 6 Maximum Volume Bytes | `Enter new Maximum Bytes:` | Größe, z. B. `10G` |
| 7 Recycle Flag | `Enter new Recycle status:` | `yes` oder `no`; andere Eingaben führen zur selben Frage |
| 8 Slot | `Enter new Slot:` | 0 bis `Maximum Volumes` des Pools |
| 9 InChanger Flag | `Set InChanger flag for Volume "<name>": yes/no:` | `yes` oder `no` |
| 10 Volume Files | `Enter new number of Files for Volume:`, danach `Increase Volume Files? (yes/no):` | Ganzzahl, dann `yes`/`no` |
| 11 Pool | `Enter new Pool name:` | Pool-Name; ein unbekannter Name führt zur Pool-Auswahl |
| 15 Enabled | `Enter new Enabled:` | `yes`, `no`, `archived`, `0`, `1`, `2` |
| 16 RecyclePool | `Select the Pool (1-<n>):` | Nummer; `*None*` entfernt den Recycle-Pool |

Ein `.` bricht die gerade gestellte Frage ab. Im Parametermenü beendet `.` die Auswahl wie `19`; war noch kein Volume gewählt, folgt trotzdem die Pool-Auswahl, die man erneut mit `.` abbricht. Dieses Verhalten entspricht dem Original.

### Auswahl für `update pool`

`update pool` ohne Namen fragt mit `Select Pool resource (1-<n>):` nach der Pool-Ressource.

## API-Modi

Unter `.api 0` kommen Bestätigungen und Fehler als Text. Unter `.api 1` kommt dieselbe Textzeile ohne die Katalogzeilen `Automatically selected Catalog: …`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 volstatus=Used
New Volume status is: Used
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Unter `.api 2` gibt ein erfolgreiches `update` ein leeres Ergebnis zurück; Fehler stehen im JSON-Fehlerobjekt:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 volstatus=Used
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 volstatus=Bogus
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
          "Invalid VolStatus specified: Bogus\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Auch das Menü ohne Argumente beantwortet `.api 2` sofort mit einem leeren Ergebnis:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Unter `.api 1` schickt der Director das Hauptmenü ohne Nummern als einzelne Einträge (`Update choice:` gefolgt von `Volume parameters`, `Pool from resource`, `Slots from autochanger`, `Long term statistics`) und wartet auf eine Auswahl; diese Form ist für Programme gedacht.

## Beispiele

### Hauptmenü und Abbruch

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Update choice:
1: Volume parameters
2: Pool from resource
3: Slots from autochanger
4: Long term statistics
Choose catalog item to update (1-4):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume-Status direkt setzen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 volstatus=Used
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
New Volume status is: Used
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Werte in einer Zeile

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 volretention=30days voluse=1day maxvoljobs=5 maxvolfiles=10 maxvolbytes=10G recycle=no
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
New retention period is: 1 month
New use duration is: 1 day
New max jobs is: 5
New max files is: 10
New Max bytes is: 10737418240
New Recycle flag is: no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 inchanger=yes slot=3 enabled=no
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
New InChanger flag is: yes
New Slot is: 3
New Enabled is: 0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume im Menü ändern

`update volume=<name>` ohne Parameter öffnet das Parametermenü. Hier wird der Status auf `Used` gesetzt (Punkt 1, dann 5) und das Menü mit `19` verlassen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
 2: Volume Retention Period
 3: Volume Use Duration
 4: Maximum Volume Jobs
 5: Maximum Volume Files
 6: Maximum Volume Bytes
 7: Recycle Flag
 8: Slot
 9: InChanger Flag
10: Volume Files
11: Pool
12: Volume from Pool
13: All Volumes from Pool
14: All Volumes from all Pools
15: Enabled
16: RecyclePool
17: Action On Purge
18: Storage
19: Done
Select parameter to modify (1-19):
> 1
Updating Volume "Full-0001"
Current Volume status is: Used
Possible Values are:
1: Append
2: Archive
3: Disabled
4: Full
5: Used
6: Cleaning
7: Read-Only
Choose new Volume Status (1-7):
> 5
New Volume status is: Used
Parameters to modify:
 1: Volume Status
 2: Volume Retention Period
 3: Volume Use Duration
 4: Maximum Volume Jobs
 5: Maximum Volume Files
 6: Maximum Volume Bytes
 7: Recycle Flag
 8: Slot
 9: InChanger Flag
10: Volume Files
11: Pool
12: Volume from Pool
13: All Volumes from Pool
14: All Volumes from all Pools
15: Enabled
16: RecyclePool
17: Action On Purge
18: Storage
19: Done
Select parameter to modify (1-19):
> 19
Selection terminated.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Weitere Punkte des Menüs (Menü gekürzt):

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 2
Updating Volume "Full-0001"
Current retention period is: 1 month
Enter Volume Retention period:
> 60days
New retention period is: 2 months
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 19
Selection terminated.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 10
Updating Volume "Full-0001"
Warning changing Volume Files can result
in loss of data on your Volume

Current Volume Files is: 5
Enter new number of Files for Volume:
> 5
Normally, you should only increase Volume Files by one!
Increase Volume Files? (yes/no):
> no
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> .
Selection aborted, nothing done.
Updating Volume "Full-0001"
Selection terminated.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume erst auswählen

Ohne Volume-Namen fragt `update volume` nach dem Parameter, dann nach Pool und Volume. Ein `.` im Parametermenü führt hier zur Pool-Auswahl.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 1
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
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> .
Selection aborted, nothing done.
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
>
```

### Fehleingaben im Menü

Im Parametermenü gelten nur Nummern von 1 bis 19:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 0
Please enter a number between 1 and 19
Select parameter to modify (1-19):
> x
Expected a positive integer, got: x
Select parameter to modify (1-19):
> .
Selection aborted, nothing done.
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
>
```

Ungültige Werte melden einen Fehler und führen zurück ins Menü:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 2
Updating Volume "Full-0001"
Current retention period is: 0 secs
Enter Volume Retention period:
> bogus
Invalid retention period specified: bogus
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> .
Selection aborted, nothing done.
Updating Volume "Full-0001"
Selection terminated.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Bei Ja/Nein-Fragen wiederholt der Director die Frage ohne Meldung:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 9
Updating Volume "Full-0001"
Current InChanger flag is: 0
Set InChanger flag for Volume "Full-0001": yes/no:
> maybe
Set InChanger flag for Volume "Full-0001": yes/no:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein unbekannter Pool-Name führt zur Pool-Auswahl:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 11
Updating Volume "Full-0001"
Current Pool is: Full
Enter new Pool name:
> t001nosuch
Could not find Pool "t001nosuch": ERR=Pool record not found in Catalog.
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Recycle-Pool wählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Parameters to modify:
 1: Volume Status
[…]
19: Done
Select parameter to modify (1-19):
> 16
Updating Volume "Full-0001"
No current RecyclePool
Defined Pools:
1: *None*
2: Incremental
3: Full
4: Differential
5: Scratch
Select the Pool (1-5):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Pool-Eintrag aus der Konfiguration aktualisieren

`update pool=<pool>` übernimmt die Werte der Pool-Ressource in den Katalog und zeigt den Eintrag danach.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update pool=Full
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+--------+------+---------+---------+---------+------------+-----------------+--------------+----------------+------------+-------------+----------------+-----------+---------+---------------+----------+-----------+-------------+---------+---------------+---------------+------------+--------------+--------------+--------------------+-------------------+---------------+
| poolid | name | numvols | maxvols | useonce | usecatalog | acceptanyvolume | volretention | voluseduration | maxvoljobs | maxvolfiles | maxvolbytes    | autoprune | recycle | actiononpurge | pooltype | labeltype | labelformat | enabled | scratchpoolid | recyclepoolid | nextpoolid | minblocksize | maxblocksize | migrationhighbytes | migrationlowbytes | migrationtime |
+--------+------+---------+---------+---------+------------+-----------------+--------------+----------------+------------+-------------+----------------+-----------+---------+---------------+----------+-----------+-------------+---------+---------------+---------------+------------+--------------+--------------+--------------------+-------------------+---------------+
|      3 | Full |       5 |     100 |       0 |          1 |               0 |   31,536,000 |              0 |          0 |           0 | 53,687,091,200 |         1 |       1 |             0 | Backup   |         0 | Full-       |       1 |             0 |             0 |          0 |            0 |            0 |                  0 |                 0 |             0 |
+--------+------+---------+---------+---------+------------+-----------------+--------------+----------------+------------+-------------+----------------+-----------+---------+---------------+----------+-----------+-------------+---------+---------------+---------------+------------+--------------+--------------+--------------------+-------------------+---------------+
Pool DB record updated from resource.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ohne Namen erscheint die Auswahl der Pool-Ressourcen:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update pool
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Pool resources are:
1: Differential
2: Full
3: Incremental
4: Scratch
Select Pool resource (1-4):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Slots eines Autochangers aktualisieren

`update slots` fragt den Storage Daemon nach dem Inhalt des Autochangers. Das Datei-Gerät der Testkonfiguration ist kein Autochanger:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update slots storage=File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3998 Device ""FileStorage" (/srv/BACKUP/bareos-storage)" is not an autochanger.
Device "FileStorage" has 0 slots.
No slots in changer to scan.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ohne `storage=` bietet `update slots` nur Autochanger-Storages an; in der Testkonfiguration gibt es keinen:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update slots
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Unter `.api 2`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update slots storage=File
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume in einen anderen Pool verschieben

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 pool=Incremental
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
New Pool is: Incremental
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Recycle-Pool, Action On Purge und Storage setzen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 recyclepool=Scratch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
New RecyclePool is: Scratch
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 actiononpurge=Truncate
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
New ActionOnPurge is: Truncate
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein unbekannter Wert für `actiononpurge=` setzt `None`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 actiononpurge=bogus
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
New ActionOnPurge is: None
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

`storage=` ändert die Storage-Zuordnung ohne Bestätigung:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume=Full-0001 storage=File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Standardwerte aus dem Pool übernehmen

`allfrompool=<pool>` setzt Aufbewahrung, Grenzen und Recycle-Einstellungen aller Volumes eines Pools auf die Werte des Pool-Eintrags, `fromallpools` tut das für alle Pools:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume allfrompool=Full
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
All Volume defaults updated from "Full" Pool record.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update volume fromallpools
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
All Volume defaults updated from "Differential" Pool record.
All Volume defaults updated from "Scratch" Pool record.
All Volume defaults updated from "Full" Pool record.
All Volume defaults updated from "Incremental" Pool record.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Langzeitstatistik fortschreiben

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update stats days=30
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Updating 148 job(s).
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Über das Hauptmenü (Punkt 4) ohne `days=`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Update choice:
1: Volume parameters
2: Pool from resource
3: Slots from autochanger
4: Long term statistics
Choose catalog item to update (1-4):
> 4
Updating 13 job(s).
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job-Eintrag ändern

`update jobid=<id>` ändert die angegebenen Felder ohne Bestätigung. Werte mit Leerzeichen stehen in Anführungszeichen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update jobid=506 jobname=DocsRenamed
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update jobid=506 starttime="2026-10-01 10:00:00"
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ohne Feld meldet der Director, was fehlt:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update jobid=506
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Neither Client, StartTime, Filesetid, JobType nor Name specified.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Eine unbekannte Job-ID meldet der Katalog:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update jobid=999999 jobname=x
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
cats/sql_get.cc:275 No Job found for JobId 999999
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Fehlt bei `update jobid` die Job-ID, kommt die Meldung aus dem Original:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> update jobid
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Expect JobId keyword, not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `cats/sql_get.cc:1177 Media record for Volume "<name>" not found.` | Das Volume gibt es nicht. Namen mit `list volumes` prüfen. Weitere Parameter derselben Zeile werden dann nicht mehr geprüft. |
| `Invalid VolStatus specified: <wert>` | Unbekannter Status, siehe Tabelle oben. |
| `Invalid retention period specified: <wert>` | Keine gültige Zeitangabe (z. B. `30days`). |
| `Invalid use duration specified: <wert>` | Keine gültige Zeitangabe. |
| `Invalid max. bytes specification: <wert>` | Keine gültige Größe (z. B. `10G`). |
| `Query failed: UPDATE Media SET MaxVolJobs=<wert> … ERR=…` | `maxvoljobs=` oder `maxvolfiles=` mit einem Wert, der keine Zahl ist. Der Wert geht ungeprüft an die Datenbank, die ihn abweist (wie im Original). |
| `Invalid value. It must be yes or no.` | `recycle=` oder `inchanger=` mit anderem Wert als `yes`/`no`. |
| `Invalid slot, it must be between 0 and MaxVols=<n>` | Slot außerhalb des erlaubten Bereichs. |
| `Invalid Enabled value, it must be yes, no, archived, 0, 1, or 2` | Ungültiger `enabled=`-Wert (auch bei `update slots`). |
| `Valid values for Encrypt are yes/no/rotate` | Ungültiger `encrypt=`-Wert. |
| `No encryption key for volume '<name>' configured.` | Das Volume hat keinen Verschlüsselungsschlüssel. |
| `Could not find Pool "<name>": ERR=Pool record not found in Catalog.` | Unbekannter Pool bei `pool=`, `recyclepool=` oder `allfrompool=`; die Pool-Auswahl folgt. |
| `Error: Pool resource "<name>" does not exist.` | `update pool=` mit einem Namen, den die Konfiguration nicht kennt. |
| `Expected a positive integer, got: <eingabe>` / `Please enter a number between 1 and 19` | Im Menü wurde keine gültige Nummer eingegeben. |
| `Expect JobId keyword, not found.` | `update jobid` ohne `=<jobid>`. |
| `Neither Client, StartTime, Filesetid, JobType nor Name specified.` | `update jobid=<id>` ohne eines der Felder `client=`, `starttime=`, `filesetid=`, `jobtype=`, `jobname=`. |
| `The defined Autochanger Storage resources are:` / `Selection list for "Storage" is empty!` | `update slots` ohne Autochanger in der Konfiguration. |
| `3998 Device "…" is not an autochanger.` / `No slots in changer to scan.` | Antwort des Storage Daemons: das Gerät ist kein Autochanger. |
| `cats/sql_get.cc:275 No Job found for JobId <n>` | `update jobid=` mit einer Job-ID, die nicht im Katalog steht. |
| `Selection aborted, nothing done.` | Der Dialog wurde mit `.` abgebrochen. |

## Siehe auch

- [list](list.md) — Volumes, Pools und Jobs anzeigen
- [llist](llist.md) — alle Felder eines Volumes anzeigen
- [create](create.md) — Pool-Eintrag neu anlegen
- [label](label.md) — Volume beschriften
- [purge](purge.md) — Volume leeren
- [status](status.md) — Slots eines Autochangers anzeigen (`status storage=<storage> slots`)
