# import — Volumes aus den Import/Export-Slots eines Autochangers holen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L358) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Import volumes from import/export slots to normal slots

**Originalsyntax:**

```text
import storage=<storage-name> [ srcslots=<slot-selection> dstslots=<slot-selection> volume=<volume-name> scan ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text.
- Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen und Katalogupdates sind nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-045` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (2):** `c012-import-unknown-storage`, `import-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:984](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`import` lässt einen Autochanger (Bandbibliothek) Volumes, die in seine Import/Export-Slots (Mailslots) eingelegt wurden, in freie normale Slots bringen. Der Director beauftragt dazu den Storage Daemon, der den Roboter bewegt, und trägt die Slots danach im Katalog ein. Der Befehl verändert also den Zustand der Bibliothek und des Katalogs. Er funktioniert nur mit Storages, die als Autochanger konfiguriert sind. Das Gegenstück ist [`export`](export.md).

## Syntax

```text
import storage=<storage> [srcslots=<slots>] [dstslots=<slots>] [volume=<name>|<name>…] [scan]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Autochanger-Storage | Die Bibliothek. Fehlt die Angabe, fragt der Director nach (siehe Dialoge). |
| `srcslots=<slots>` (auch `srcslot=`) | Slotliste, z.B. `3`, `1-4`, `1,5,7-9` | Belegte Import/Export-Slots als Quelle. Ohne Angabe nimmt der Director alle belegten Import/Export-Slots. |
| `dstslots=<slots>` (auch `dstslot=`) | Slotliste | Leere normale Slots als Ziel. Ohne Angabe nimmt der Director freie Slots der Reihe nach. |
| `volume=<name>\|<name>…` | Volume-Namen, durch `\|` getrennt | Nur diese Volumes importieren. |
| `scan` | — | Volume-Namen nicht aus den Barcodes nehmen, sondern die Bänder im Laufwerk lesen. Dafür fragt der Director nach dem Laufwerk. |

Slotlisten: Zahlen ab 1, Bereiche mit `-`, mehrere Einträge mit `,`.

## Dialoge

Wie bei [`export`](export.md): Ohne `storage=` listet der Director die Autochanger-Storages und fragt nach der Nummer; gibt es keinen, endet der Befehl mit `Selection list for "Storage" is empty!`. Mit `scan` fragt er bei mehreren Laufwerken nach dem Laufwerk. Beispielausgabe für die Auswahl nicht verfügbar: Der Testaufbau hat keinen Autochanger.

## API-Modi

Unter `.api 0` und `.api 1` kommen die Meldungen als Text. Unter `.api 2` endet ein Fehlschlag als JSON-Fehler ohne die Meldungen:

```text
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
```

## Beispiele

Ein erfolgreicher Import: Beispielausgabe nicht verfügbar, weil der Testaufbau keinen Autochanger hat und der Befehl die Bibliothek bewegen würde.

### Kein Autochanger konfiguriert

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> import
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> import storage=Nope
Storage resource "Nope": not found
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage ohne Autochanger

`File` ist ein Datei-Storage ohne Slots (Paritätsfall `ds-export-file`). Onesimus schickt hier den Befehl an den Storage Daemon, der ihn ablehnt; an der Bibliothek ändert sich nichts.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> import storage=File
import on storage File:
3900 Invalid command: import

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original fragt nur die Slots ab und antwortet `Connecting to Storage daemon File at …:9103 ...`, `3998 Device ""FileStorage" (/srv/BACKUP/bareos-storage)" is not an autochanger.`, `Device "FileStorage" has 0 slots.`, `No slots in changer.` (C-071)

## Fehlermeldungen

Die Meldungen ab der dritten Zeile stammen aus dem Originalquelltext; Beispielausgabe nicht verfügbar, weil sie einen Autochanger voraussetzen.

| Meldung | Ursache und Abhilfe |
|---|---|
| `Selection list for "Storage" is empty!` | Es gibt keinen Autochanger-Storage (oder die Konsole darf keinen benutzen). |
| `Storage resource "<name>": not found` | Unbekannter Storage-Name; danach folgt die Auswahl. |
| `No slots in changer.` | Der Storage Daemon meldet keine Slots, das Gerät ist kein Autochanger. |
| `No Volumes found, or no barcodes.` | Die Bibliothek meldet keinen Inhalt. Ggf. mit `scan` arbeiten. |
| `Not all slots in source selection are import slots and filled.` | Ein Quellslot ist kein belegter Import/Export-Slot. |
| `Not all slots in destination selection are normal slots and empty.` | Ein Zielslot ist kein freier normaler Slot. |
| `Not enough free slots available to import <n> volumes` | Zu wenige freie Slots in der Bibliothek. |
| `Source slot selection doesn't fit into destination slot selection.` | Mehr Quell- als Zielslots angegeben. |
| `Source slot selection and destination slot selection overlap.` | Ein Slot steht in beiden Listen. |
| `No volume named <name> in changer or in selected source slots.` | Ein Volume aus `volume=` steckt nicht in den gewählten Slots. |
| `Negative numbers not permitted`, `Range end is not integer.`, `Range start is not an integer.`, `Range end not bigger than start.`, `Input value is not an integer.`, `Values must be be greater than zero.`, `Slot too large.` | Fehler in einer Slotliste. |

## Siehe auch

- [export](export.md) — Volumes in die Mailslots legen
- [move](move.md) — Volumes zwischen Slots bewegen
- [update](update.md) — `update slots` gleicht den Katalog mit der Bibliothek ab
- [label](label.md) — neue Bänder nach dem Import beschriften
