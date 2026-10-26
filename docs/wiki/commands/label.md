# label — Neues Volume beschriften

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L363) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Label a tape

**Originalsyntax:**

```text
label storage=<storage-name> volume=<volume-name> pool=<pool-name> slot=<slot> [ drive = <drivenum>] [ barcodes ] [ encrypt ] [ yes ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 13 von 13 Fällen byte-identisch; Modi: Text, API 1.
- Verglichen sind Dialog-, Namens- und Abbruchpfade; das Labeln selbst (SD-Wirkung, Katalogeintrag) ist laut Testregeln nicht live erlaubt und nur durch Einheitentests belegt.

**Paritätsfälle (13):** `c011-label-cancel`, `c011-label-name-cancel`, `c011-label-pool-cancel`, `c011-label-illegal-cancel`, `c011-label-used-cancel`, `c011-label-barcodes`, `c011-label-unknown-storage-cancel`, `c011-label-api1-cancel`, `c012-label-no-pool`, `label-nostorage`, `t001-label-existing-cancel`, `t001-label-illegal-arg-cancel`, `t001-label-illegal-char-cancel`.

**Lokaler Handler:** [crates/odir/src/commands/admin.rs:26](../../../crates/odir/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`label` schreibt über den Storage Daemon ein Bareos-Label auf ein neues Medium (Datei, Band) und legt dazu den Media-Eintrag im Katalog an. Erst ein beschriftetes Volume kann der Storage Daemon für Backups benutzen. Der Befehl verändert sowohl das Medium als auch den Katalog. Fehlen Angaben, fragt der Director nach Storage, Volume-Name, Slot und Pool.

## Syntax

```text
label [ storage=<storage> ] [ volume=<volume> ] [ pool=<pool> ] [ slot=<slot> ] [ drive=<laufwerk> ] [ encrypt ] [ yes ]
label barcodes [ storage=<storage> ] [ pool=<pool> ] [ slots=<slotliste> ] [ drive=<laufwerk> ] [ encrypt ] [ yes ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Name einer Storage-Ressource | Storage Daemon und Gerät, auf dem beschriftet wird. Statt `storage=` geht auch `sd=` oder der Name allein als erstes Wort. Fehlt die Angabe, erscheint eine Auswahlliste. |
| `volume=<volume>` | Volume-Name | Name des neuen Volumes. Erlaubt sind Buchstaben, Ziffern und `:.-_/`; am Anfang darf kein `/` stehen. Fehlt die Angabe, fragt der Director danach. |
| `pool=<pool>` | Name eines Pools aus dem Katalog | Pool, dem das Volume zugeordnet wird. Fehlt die Angabe, erscheint eine Pool-Auswahl. |
| `slot=<slot>` | Ganzzahl ≥ 0 | Slot im Autochanger. Ein Wert über 0 setzt zugleich das InChanger-Flag. Bei einem Autochanger ohne Angabe fragt der Director nach dem Slot. |
| `drive=<laufwerk>` | Ganzzahl ab 0 | Laufwerk des Storage-Geräts. Bei Geräten mit nur einem Laufwerk wird 0 genommen. |
| `barcodes` (auch `barcode`) | — | Beschriftet alle Volumes eines Autochangers mit dem Namen aus dem Barcode. Nur Autochanger-Storages kommen in Frage. |
| `slots=<slotliste>` | z. B. `1-5,8` | Nur zusammen mit `barcodes`: beschränkt die Beschriftung auf diese Slots. |
| `encrypt` | — | Erzeugt einen neuen Schlüssel für Hardware-Verschlüsselung (LTO) und legt ihn im Katalog ab. |
| `yes` | — | Beantwortet bei `barcodes` die Frage `Do you want to label these Volumes? (yes\|no):` automatisch mit „ja“. |

## Dialoge

`label` fragt nacheinander nur nach dem, was auf der Befehlszeile fehlt:

1. **Storage** — Liste „The defined Storage resources are:“ mit der Frage `Select Storage resource (1-<n>):`. Erwartet wird die Nummer; jede andere Eingabe ergibt `Expected a positive integer, got: …` und dieselbe Frage noch einmal.
2. **Volume-Name** — `Enter new Volume name:`. Ist der Name ungültig oder schon als beschriebenes Volume im Katalog, kommt die Meldung und die Frage erneut.
3. **Slot** — nur bei Autochangern: `Enter slot (0 or Enter for none):`.
4. **Pool** — Liste „Defined Pools:“ mit `Select the Pool (1-<n>):`.

Ein einzelner `.` bricht an jeder Stelle ab. Bei den Auswahllisten bestätigt der Director das mit `Selection aborted, nothing done.`; bei der Frage nach dem Namen endet der Befehl ohne Ausgabe. Danach ist nichts beschriftet und nichts im Katalog geändert.

## API-Modi

Unter `.api 0` laufen die Dialoge wie unten gezeigt. Unter `.api 1` schickt der Director die Auswahlliste ohne Nummern als einzelne Einträge (zum Beispiel `The defined Storage resources are:` gefolgt von `File`, `ParityNdmp`, `ParityNdmpNative`) und wartet dann auf eine Auswahl; diese Form ist für Programme gedacht, nicht für die Eingabe von Hand. Unter `.api 2` fragt `label` nicht nach, sondern antwortet sofort mit einem leeren JSON-Ergebnis:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Beispiele

### Storage auswählen lassen und abbrechen

Ohne Argumente beginnt `label` mit der Storage-Auswahl. Ein `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label
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

### Falsche Eingabe in der Auswahlliste

Die Auswahlliste nimmt nur Nummern an. Weder Pfade noch Volume-Namen werden als Antwort akzeptiert.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> /x
Expected a positive integer, got: /x
Select Storage resource (1-3):
> a b
Expected a positive integer, got: a b
Select Storage resource (1-3):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Storage

Ein unbekannter Name führt zur Auswahlliste.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label storage=Nope
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Storage resource "Nope": not found
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nach dem Volume-Namen fragen lassen

Mit Storage, aber ohne Volume-Namen fragt `label` nach dem Namen. Ein `.` beendet den Befehl ohne weitere Ausgabe.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label storage=File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Enter new Volume name:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Pool auswählen

Mit Storage und Volume-Namen, aber ohne Pool zeigt `label` die Pools aus dem Katalog.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label storage=File volume=c011probe
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
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

### Name schon vergeben

Gibt es schon ein beschriebenes Volume dieses Namens, fragt `label` nach einem anderen Namen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label storage=File volume=Full-0001
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Media record for new Volume "Full-0001" already exists.
Enter new Volume name:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige Volume-Namen

Ein Name darf nicht mit `/` beginnen und keine Sonderzeichen wie `!` enthalten.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label storage=File volume=/ab
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Volume name can not start with "/".
Enter new Volume name:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label storage=File volume=a!b
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Illegal character "!" in a volume name.
Enter new Volume name:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit Barcodes beschriften

`label barcodes` bietet nur Autochanger-Storages an. In der Testkonfiguration gibt es keinen, daher bleibt die Liste leer.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label barcodes
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume auf einem NDMP-Storage beschriften

Für einen NDMP-Storage ohne gekoppelten nativen Storage Daemon scheitert das Beschriften; der Director erinnert ans Mounten.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> label storage=ParityNdmpNative volume=c050vol pool=Scratch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Label command failed for Volume c050vol.
Do not forget to mount the drive!!!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume vollständig beschriften

```text
Beispielausgabe nicht verfügbar: das Beschriften schreibt über den Storage Daemon auf ein Medium und darf im Testbetrieb nicht ausgeführt werden.
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Storage resource "<name>": not found` | Den Storage gibt es nicht oder die Konsole darf ihn nicht benutzen. Der Director zeigt danach die Auswahlliste. |
| `Expected a positive integer, got: <eingabe>` | In einer Auswahlliste wurde keine Nummer eingegeben. Nummer aus der Liste eingeben oder mit `.` abbrechen. |
| `Media record for new Volume "<name>" already exists.` | Ein Volume dieses Namens ist schon beschrieben. Anderen Namen wählen oder das alte Volume mit [relabel](relabel.md) neu beschriften. |
| `Volume name can not start with "/".` | Der Name beginnt mit `/`. |
| `Illegal character "<z>" in a volume name.` | Der Name enthält ein nicht erlaubtes Zeichen. Erlaubt sind Buchstaben, Ziffern und `:.-_/`. |
| `Volume name too long.` / `Volume name must be at least one character long.` | Der Name ist zu lang (128 Zeichen und mehr) oder leer. |
| `The defined Autochanger Storage resources are:` / `Selection list for "Storage" is empty!` | `barcodes` braucht einen Autochanger-Storage; es ist keiner konfiguriert. |
| `Label command failed for Volume <name>.` / `Do not forget to mount the drive!!!` | Der Storage Daemon konnte nicht beschriften. Gerät prüfen, ein Medium einlegen und mit [mount](mount.md) einbinden. |
| `Selection aborted, nothing done.` | Der Dialog wurde mit `.` abgebrochen. |

## Siehe auch

- [relabel](relabel.md) — vorhandenes, geleertes Volume neu beschriften
- [add](add.md) — Volumes nur im Katalog anlegen, ohne Label
- [mount](mount.md) — Gerät nach dem Beschriften einbinden
- [automount](automount.md) — automatisches Mounten nach `label` ein- oder ausschalten
- [list](list.md) — Volumes im Katalog anzeigen (`list volumes`)
