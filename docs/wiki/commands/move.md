# move — Volumes im Autochanger umsetzen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L379) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Move slots in an autochanger

**Originalsyntax:**

```text
move storage=<storage-name> srcslots=<slot-selection> dstslots=<slot-selection>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text.
- Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen sind nur durch Einheitentests belegt. Die Originalsyntax srcslots/dstslots wird inzwischen ausgewertet.

**Offene Logeinträge:** `C-045` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (2):** `c012-move-job-not-running`, `move-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:1020](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`move` lässt den Autochanger Volumes von einem Slot in einen anderen umsetzen. Quell- und Zielslots gibt man als Slotlisten an; der Director prüft vorher am Storage Daemon, welche Slots belegt sind, und setzt nur belegte Quellslots in freie Zielslots um. Der Befehl verändert die Belegung des Autochangers und die Slot-Angaben im Katalog. Er funktioniert nur mit Autochanger-Storages.

## Syntax

```text
move [ storage=<storage> ] srcslots=<slotliste> dstslots=<slotliste> [ scan [ drive=<laufwerk> ] ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Autochanger-Storage | Autochanger, in dem umgesetzt wird. Fehlt die Angabe, erscheint eine Auswahl der Autochanger-Storages. |
| `srcslots=<slotliste>` (auch `srcslot=`) | z. B. `1-3,7` | Slots, aus denen Volumes genommen werden. Leere Slots in der Liste werden übergangen. |
| `dstslots=<slotliste>` (auch `dstslot=`) | z. B. `10-12` | Slots, in die die Volumes kommen. |
| `scan` | — | Die Volume-Namen nicht aus den Barcodes nehmen, sondern jedes Volume ins Laufwerk laden und das Label lesen. |
| `drive=<laufwerk>` | Ganzzahl ab 0 | Laufwerk für `scan`. |

Ohne Quell- oder Zielauswahl führt `move` nichts aus.

## Dialoge

Ohne `storage=` fragt `move` nach einem Autochanger-Storage. Gibt es keinen, endet der Befehl mit `Selection list for "Storage" is empty!`.

## API-Modi

Unter `.api 0` und `.api 1` kommt die Antwort als Text. Unter `.api 2` endet ein fehlgeschlagenes `move` mit einem JSON-Fehlerobjekt ohne Meldungstext:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> move
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {}
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Beispiele

### Ohne Autochanger in der Konfiguration

Die Testkonfiguration hat keinen Autochanger; die Auswahl bleibt leer.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> move
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> move srcslots=1 dstslots=2
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> move storage=Nope srcslots=1 dstslots=2
Storage resource "Nope": not found
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage ohne Autochanger

Mit `storage=` fragt `move` zuerst den Storage Daemon nach der Zahl der Slots. Das Datei-Gerät der Testkonfiguration ist kein Autochanger:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> move storage=File
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
3998 Device ""FileStorage" (/srv/BACKUP/bareos-storage)" is not an autochanger.
Device "FileStorage" has 0 slots.
No slots in changer.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage über einen Job bestimmen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> move job=nosuch
Job "nosuch" is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volumes umsetzen

```text
Beispielausgabe nicht verfügbar: die Testumgebung hat keinen Autochanger, und Bewegungen in einem echten Autochanger sind im Testbetrieb nicht erlaubt.
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `The defined Autochanger Storage resources are:` / `Selection list for "Storage" is empty!` | Kein Autochanger-Storage konfiguriert oder für die Konsole erlaubt. |
| `Storage resource "<name>": not found` | Unbekannter Storage. |
| `Cannot perform a move operation without source and/or destination selection` | `srcslots=` oder `dstslots=` fehlt. |
| `No slots in changer.` | Der Storage Daemon meldet keine Slots; das Gerät ist kein Autochanger. |
| `No Volumes found, or no barcodes.` | Der Autochanger meldet keine Volumes. Mit `scan` die Labels lesen lassen. |
| `Job "<name>" is not running.` | Zu `job=` läuft kein Job. |

## Siehe auch

- [export](export.md) — Volumes in Ausgabe-Slots (Import/Export) bringen
- [import](import.md) — Volumes aus Ausgabe-Slots holen
- [update](update.md) — Slot-Belegung im Katalog aktualisieren (`update slots`)
- [status](status.md) — Slots anzeigen (`status storage=<storage> slots`)
