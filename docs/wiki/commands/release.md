# release — Gerät im Storage Daemon freigeben

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L420) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Release storage

**Originalsyntax:**

```text
release storage=<storage-name> [ drive=<drivenum> ] [ alldrives ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 7 von 7 Fällen byte-identisch; Modi: Text.
- Verglichen sind Auswahl und Fehlerpfade am File-Storage; NDMP (C-050) ungeprüft.

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (7):** `c013-release-auto`, `c013-release-positional`, `c013-release-drive`, `c013-release-alldrives`, `c013-release-notfound`, `c013-release-then-mount`, `release-dialog`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:179](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`release` bittet den Storage Daemon, das Volume in einem Gerät freizugeben, ohne das Gerät auszuhängen. Bei Bandlaufwerken wird das Band zurückgespult; der nächste Job liest das Label neu und kann ein anderes Volume anfordern. Der Befehl ändert den Zustand des Geräts im Storage Daemon, nicht den Katalog.

## Syntax

```text
release [ storage=<storage> | <storage> ] [ drive=<laufwerk> ] [ alldrives ]
release jobid=<jobid> | job=<job-name> | ujobid=<eindeutiger-job-name>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Name einer Storage-Ressource | Storage, dessen Gerät freigegeben wird. Gleichwertig: `sd=<storage>` oder der Name allein. Fehlt die Angabe, erscheint eine Auswahlliste. |
| `drive=<laufwerk>` | Ganzzahl ab 0 | Laufwerk bei Geräten mit mehreren Laufwerken. |
| `alldrives` | — | Alle Laufwerke des Storages freigeben. |
| `jobid=<jobid>`, `job=<job-name>`, `ujobid=<name>` | laufender Job | Storage des laufenden Jobs benutzen. |

## Dialoge

Ohne Storage-Angabe fragt `release` mit `Select Storage resource (1-<n>):` nach dem Storage. Erwartet wird die Nummer; jede andere Eingabe, auch ein Befehl, ergibt `Expected a positive integer, got: …`. Ein `.` bricht ab.

## API-Modi

Unter `.api 0` und `.api 1` kommt die Antwort als Text, unter `.api 2` stehen Fehler wie bei [mount](mount.md#api-modi) im JSON-Fehlerobjekt. Für `release` selbst gibt es keinen Paritätsfall unter `.api 2`.

## Beispiele

### Storage auswählen lassen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release
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

### Gerät eines Storages freigeben

Der Director verbindet sich mit dem Storage Daemon und gibt dessen Antwort aus. Hier war das Gerät schon frei.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3921 Device ""FileStorage" (/srv/BACKUP/bareos-storage)" already released.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release storage=File drive=0
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3921 Device ""FileStorage" (/srv/BACKUP/bareos-storage)" already released.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alle Laufwerke freigeben

Ohne Storage-Angabe fragt `release alldrives` zuerst nach dem Storage:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release alldrives
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
>
```

### NDMP-Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release storage=ParityNdmp
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
NdmpAutochangerVolumeOperation: operation release, drive 0, slot -1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Mit `alldrives` gibt es bei NDMP-Storages keine Laufwerksmeldung:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release storage=ParityNdmp alldrives
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehleingaben

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release nosuch
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

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> release job=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Job "nosuch" is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Storage resource "<name>": not found` | Unbekannter oder nicht erlaubter Storage; die Auswahlliste folgt. |
| `Storage name given twice.` | Zwei Storage-Namen in einer Zeile. |
| `JobId <n> is not running.` / `Job "<name>" is not running.` | Zu `jobid=`, `job=` oder `ujobid=` läuft kein Job. |
| `3921 Device ""<gerät>" (<pfad>)" already released.` | Antwort des Storage Daemons: das Gerät war schon frei. |
| `Expected a positive integer, got: <eingabe>` | In der Auswahlliste wurde keine Nummer eingegeben. |
| `Selection aborted, nothing done.` | Die Auswahl wurde mit `.` abgebrochen. |

## Siehe auch

- [mount](mount.md) — Gerät einbinden
- [unmount](unmount.md) — Gerät aushängen
- [status](status.md) — Gerätezustand anzeigen
