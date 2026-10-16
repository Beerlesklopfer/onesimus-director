# unmount — Gerät im Storage Daemon aushängen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L492) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Unmount storage

**Originalsyntax:**

```text
unmount storage=<storage-name> [ drive=<drivenum> ] jobid=<jobid> | job=<job-name> | ujobid=<complete_name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Paritätsfall nachgewiesen.

- Kein Paritätsfall: Unmount am echten SD ist laut Testregeln nicht live erlaubt; Verhalten nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:175](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`unmount` bittet den Storage Daemon, ein Gerät nicht mehr zu benutzen und das Volume freizugeben, damit man ein Band entnehmen oder das Gerät warten kann. Solange ein Gerät ausgehängt ist, schreibt kein Job darauf; [mount](mount.md) bindet es wieder ein. Der Befehl ändert den Zustand des Geräts im Storage Daemon, nicht den Katalog. [umount](umount.md) ist ein anderer Name für denselben Befehl.

## Syntax

```text
unmount [ storage=<storage> | <storage> ] [ drive=<laufwerk> ] [ alldrives ]
unmount jobid=<jobid> | job=<job-name> | ujobid=<eindeutiger-job-name>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Name einer Storage-Ressource | Storage, dessen Gerät ausgehängt wird. Gleichwertig: `sd=<storage>` oder der Name allein. Fehlt die Angabe, erscheint eine Auswahlliste. |
| `drive=<laufwerk>` | Ganzzahl ab 0 | Laufwerk bei Geräten mit mehreren Laufwerken. |
| `alldrives` | — | Alle Laufwerke des Storages nacheinander aushängen. |
| `jobid=<jobid>`, `job=<job-name>`, `ujobid=<name>` | laufender Job | Storage des laufenden Jobs benutzen. |

## Dialoge

Ohne Storage-Angabe fragt `unmount` mit `Select Storage resource (1-<n>):` nach dem Storage. Erwartet wird die Nummer; ein `.` bricht ab.

## API-Modi

Unter `.api 0` kommt die Antwort als Text. Unter `.api 1` schickt der Director die Auswahlliste ohne Nummern als einzelne Einträge (`The defined Storage resources are:` gefolgt von `File`, `ParityNdmp`, `ParityNdmpNative`) und wartet auf eine Auswahl; diese Form ist für Programme gedacht. Unter `.api 2` stehen Fehler wie bei [mount](mount.md#api-modi) im JSON-Fehlerobjekt.

## Beispiele

### Storage auswählen lassen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> unmount
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

### Alle Laufwerke aushängen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> unmount alldrives
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

### Gerät eines Storages aushängen

```text
Beispielausgabe nicht verfügbar: unmount verändert den Gerätezustand im Storage Daemon und darf im Testbetrieb nicht an ein echtes Gerät gesendet werden.
```

### NDMP-Storage

Bei einem NDMP-Storage führt der Director die Operation selbst über NDMP aus:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> unmount storage=ParityNdmp
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
NdmpAutochangerVolumeOperation: operation unmount, drive 0, slot -1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehleingaben

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> unmount storage=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Storage resource "nosuch": not found
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> unmount File File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Storage name given twice.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> unmount jobid=abc
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Expecting jobid=nn command, got: jobid
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> unmount job=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Job "nosuch" is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Storage resource "<name>": not found` | Unbekannter oder für die Konsole nicht erlaubter Storage; die Auswahlliste folgt. |
| `Storage name given twice.` | Zwei Storage-Namen in einer Zeile. |
| `Expecting jobid=nn command, got: <wort>` | `jobid=` ohne Zahl. |
| `JobId <n> is not running.` / `Job "<name>" is not running.` | Zu `jobid=`, `job=` oder `ujobid=` läuft kein Job. |
| `Expected a positive integer, got: <eingabe>` | In der Auswahlliste wurde keine Nummer eingegeben. |
| `Selection aborted, nothing done.` | Die Auswahl wurde mit `.` abgebrochen. |

## Siehe auch

- [umount](umount.md) — anderer Name für `unmount`
- [mount](mount.md) — Gerät wieder einbinden
- [release](release.md) — Gerät freigeben, ohne es auszuhängen
- [status](status.md) — Gerätezustand anzeigen
