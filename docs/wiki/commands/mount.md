# mount — Gerät im Storage Daemon einbinden

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L375) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Mount storage

**Originalsyntax:**

```text
mount storage=<storage-name> slot=<num> drive=<drivenum> jobid=<jobid> | job=<job-name> | ujobid=<complete_name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 16 von 16 Fällen byte-identisch; Modi: Text.
- Autochanger-Slots und NDMP fehlen bzw. sind ungeprüft (C-045/C-050 Umfeld); verglichen sind Auswahl und Fehlerpfade am File-Storage.

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (16):** `c013-mount-auto`, `c013-mount-positional`, `c013-mount-storage`, `c013-mount-sd-drive`, `c013-mount-slot`, `c013-mount-twice`, `c013-mount-question`, `c013-mount-notfound`, `c013-mount-bare-storage`, `c013-mount-jobid-bad`, `c013-mount-jobid-notrunning`, `c013-mount-job-notrunning`, `c013-mount-job-novalue`, `c013-mount-ujobid-notrunning`, `c013-release-then-mount`, `mount-dialog`.

**Lokaler Handler:** [crates/odir/src/commands/admin.rs:192](../../../crates/odir/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`mount` bittet den Storage Daemon, ein Gerät wieder zu benutzen: bei Bandlaufwerken das eingelegte Band zu lesen und für Jobs freizugeben, bei Autochangern zuvor ein Volume aus einem Slot zu laden. Man braucht den Befehl nach einem `unmount`, nach dem Einlegen eines Bandes oder wenn ein Job auf ein Volume wartet. Der Befehl ändert den Zustand des Geräts im Storage Daemon, nicht den Katalog. Datei-Geräte sind immer eingebunden.

Wartet ein Job auf ein Volume und hat seine Job-Ressource keine `Max Wait Time`, bricht Onesimus ihn nach 10 Minuten Wartezeit ab (Onesimus-Standard, im Original unbegrenzt; siehe [run](run.md#zeitgrenzen-und-warten-auf-ein-volume)). Bis dahin genügt `mount` oder ein neu gelabeltes Volume, damit der Job weiterläuft.

## Syntax

```text
mount [ storage=<storage> | <storage> ] [ drive=<laufwerk> ] [ slot=<slot> ]
mount jobid=<jobid> | job=<job-name> | ujobid=<eindeutiger-job-name>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Name einer Storage-Ressource | Storage, dessen Gerät eingebunden wird. Gleichwertig: `sd=<storage>` oder der Name allein als Wort. Fehlt die Angabe, erscheint eine Auswahlliste. |
| `drive=<laufwerk>` | Ganzzahl ab 0 | Laufwerk bei Geräten mit mehreren Laufwerken. Bei einem Laufwerk wird 0 genommen. |
| `slot=<slot>` | Ganzzahl | Autochanger-Slot, aus dem vor dem Mounten geladen wird. |
| `jobid=<jobid>` | Job-ID eines laufenden Jobs | Storage des laufenden Jobs benutzen. |
| `job=<job-name>` | Name eines laufenden Jobs | Wie `jobid=`, über den Job-Namen. |
| `ujobid=<eindeutiger-job-name>` | eindeutiger Job-Name, z. B. `Backup.2026-10-17_10.00.00_05` | Wie `jobid=`, über den eindeutigen Namen. |

Ein Schlüsselwort ohne Wert (etwa `mount storage` oder `mount job`) wird als Storage-Name gelesen.

## Dialoge

Ohne Storage-Angabe fragt `mount` mit `Select Storage resource (1-<n>):` nach dem Storage. Erwartet wird die Nummer aus der Liste; jede andere Eingabe, auch ein Befehl wie `mount`, ergibt `Expected a positive integer, got: …` und die Frage erneut. Ein `.` bricht ab.

## API-Modi

Unter `.api 0` und `.api 1` kommt die Antwort als Text. Unter `.api 2` stehen Fehler im JSON-Fehlerobjekt:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount jobid=999999
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
          "JobId 999999 is not running.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Beispiele

### Storage auswählen lassen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Gerät eines Storages einbinden

Der Director verbindet sich mit dem Storage Daemon und gibt dessen Antwort aus. Ein Datei-Gerät ist immer eingebunden.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount storage=File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3906 File device "FileStorage" (/srv/BACKUP/bareos-storage) is always mounted.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Dasselbe mit dem Namen allein, mit `sd=` und Laufwerk oder mit Slot:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3906 File device "FileStorage" (/srv/BACKUP/bareos-storage) is always mounted.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount sd=File drive=0
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3906 File device "FileStorage" (/srv/BACKUP/bareos-storage) is always mounted.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount storage=File slot=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3906 File device "FileStorage" (/srv/BACKUP/bareos-storage) is always mounted.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Band aus einem Autochanger-Slot laden

```text
Beispielausgabe nicht verfügbar: die Testumgebung hat keinen Autochanger mit nativem Storage Daemon, und Mount-Befehle an echte Geräte sind im Testbetrieb nicht erlaubt.
```

### NDMP-Storage

Bei einem NDMP-Storage führt der Director die Operation selbst über NDMP aus und meldet Laufwerk und Slot:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount storage=ParityNdmp slot=3 drive=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
NdmpAutochangerVolumeOperation: operation mount, drive 1, slot 3
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage über einen laufenden Job bestimmen

Läuft kein passender Job, meldet der Director das:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount jobid=999999
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
JobId 999999 is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount job=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Job "nosuch" is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount ujobid=nosuch.2026
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Job "nosuch.2026" is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehleingaben

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount File File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Storage name given twice.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount jobid=abc
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Expecting jobid=nn command, got: jobid
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> mount storage=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Storage resource "nosuch": not found
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
>
```

Im Auswahldialog wird auch ein neuer Befehl als Antwort gelesen:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> mount
Expected a positive integer, got: mount
Select Storage resource (1-3):
>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Storage resource "<name>": not found` | Unbekannter oder für die Konsole nicht erlaubter Storage; die Auswahlliste folgt. |
| `Storage name given twice.` | Zwei Storage-Namen in einer Zeile. Nur einen angeben. |
| `Expecting jobid=nn command, got: <wort>` | `jobid=` ohne Zahl. |
| `JobId <n> is not running.` / `Job "<name>" is not running.` | Zu `jobid=`, `job=` oder `ujobid=` läuft kein Job. |
| `Expected a positive integer, got: <eingabe>` | In der Auswahlliste wurde keine Nummer eingegeben. |
| `3906 File device "<gerät>" (<pfad>) is always mounted.` | Antwort des Storage Daemons: Datei-Geräte sind immer eingebunden, `mount` ist nicht nötig. |
| `Selection aborted, nothing done.` | Die Auswahl wurde mit `.` abgebrochen. |

## Siehe auch

- [unmount](unmount.md) — Gerät freigeben, um ein Band zu entnehmen
- [release](release.md) — Gerät freigeben, ohne es auszuhängen
- [label](label.md) — neues Volume beschriften
- [status](status.md) — Gerätezustand anzeigen (`status storage=<storage>`)
