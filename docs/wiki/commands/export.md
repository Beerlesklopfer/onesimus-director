# export — Volumes in die Import/Export-Slots eines Autochangers legen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L341) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Export volumes from normal slots to import/export slots

**Originalsyntax:**

```text
export storage=<storage-name> srcslots=<slot-selection> [ dstslots=<slot-selection> volume=<volume-name> scan ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text.
- Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen sind nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-045` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (2):** `c012-export-bad-jobid`, `export-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:988](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`export` lässt einen Autochanger (Bandbibliothek) Volumes aus normalen Slots in seine Import/Export-Slots (Mailslots) bringen, damit man die Bänder entnehmen kann, etwa zur Auslagerung. Der Director beauftragt dazu den Storage Daemon, der den Roboter bewegt, und trägt die neuen Slots danach im Katalog ein. Der Befehl verändert also den Zustand der Bibliothek und des Katalogs. Er funktioniert nur mit Storages, die als Autochanger konfiguriert sind.

## Syntax

```text
export storage=<storage> srcslots=<slots> [dstslots=<slots>] [volume=<name>|<name>…] [scan]
export storage=<storage> volume=<name>|<name>… [dstslots=<slots>] [scan]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Autochanger-Storage | Die Bibliothek. Fehlt die Angabe, fragt der Director nach (siehe Dialoge). |
| `srcslots=<slots>` (auch `srcslot=`) | Slotliste, z.B. `3`, `1-4`, `1,5,7-9` | Normale, belegte Slots, deren Volumes exportiert werden. |
| `dstslots=<slots>` (auch `dstslot=`) | Slotliste | Leere Import/Export-Slots als Ziel. Ohne Angabe nimmt der Director freie Import/Export-Slots der Reihe nach. |
| `volume=<name>\|<name>…` | Volume-Namen, durch `\|` getrennt | Volumes über ihren Namen statt über den Slot auswählen. |
| `scan` | — | Volume-Namen nicht aus den Barcodes nehmen, sondern die Bänder im Laufwerk lesen. Dafür fragt der Director nach dem Laufwerk. |

Slotlisten: Zahlen ab 1, Bereiche mit `-`, mehrere Einträge mit `,`. Leere Quellslots werden stillschweigend übergangen. Es muss eine Quelle (`srcslots` oder `volume`) angegeben sein.

## Dialoge

Ohne `storage=` listet der Director die Autochanger-Storages (`The defined Autochanger Storage resources are:`) und fragt nach der Nummer; mit nur einem wählt er ihn automatisch. Gibt es keinen, endet der Befehl mit `Selection list for "Storage" is empty!`. Mit `scan` fragt er bei mehreren Laufwerken nach dem Laufwerk. Beispielausgabe für die Auswahl nicht verfügbar: Der Testaufbau hat keinen Autochanger.

## API-Modi

Unter `.api 0` und `.api 1` kommen die Meldungen als Text. Unter `.api 2` endet ein Fehlschlag als JSON-Fehler; die Meldungen stehen dabei nicht im JSON (siehe Beispiel).

## Beispiele

Ein erfolgreicher Export: Beispielausgabe nicht verfügbar, weil der Testaufbau keinen Autochanger hat und der Befehl die Bibliothek bewegen würde.

### Kein Autochanger konfiguriert

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> export
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> export storage=Nope srcslots=1
Storage resource "Nope": not found
The defined Autochanger Storage resources are:
Selection list for "Storage" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage ohne Autochanger

`File` ist ein Datei-Storage ohne Slots (Paritätsfall `ds-export-file`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> export storage=File
Cannot perform an export operation without source slot selection
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original fragt zuerst den Storage Daemon nach den Slots und antwortet `Connecting to Storage daemon File at …:9103 ...`, `3998 Device ""FileStorage" (/srv/BACKUP/bareos-storage)" is not an autochanger.`, `Device "FileStorage" has 0 slots.`, `No slots in changer.` (C-071)

### Falsches Argument

`jobid=` gehört nicht zu `export`; der Director liest es bei der Storage-Auswahl und meldet (Paritätsfall `c012-export-bad-jobid`):

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> export jobid=x
Expecting jobid=nn command, got: jobid
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 2`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> export
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

## Fehlermeldungen

Die Meldungen ab der dritten Zeile stammen aus dem Originalquelltext; Beispielausgabe nicht verfügbar, weil sie einen Autochanger voraussetzen.

| Meldung | Ursache und Abhilfe |
|---|---|
| `Selection list for "Storage" is empty!` | Es gibt keinen Autochanger-Storage (oder die Konsole darf keinen benutzen). |
| `Storage resource "<name>": not found` | Unbekannter Storage-Name; danach folgt die Auswahl. |
| `Cannot perform an export operation without source slot selection` | Weder `srcslots=` noch `volume=` angegeben oder alle gewählten Slots leer. |
| `No slots in changer.` | Der Storage Daemon meldet keine Slots, das Gerät ist kein Autochanger. |
| `No Volumes found, or no barcodes.` | Die Bibliothek meldet keinen Inhalt. Ggf. mit `scan` arbeiten. |
| `Not all slots in source selection are normal slots and filled.` | Ein Quellslot ist ein Import/Export-Slot oder leer. |
| `Not all slots in destination selection are export slots and empty.` | Ein Zielslot ist kein freier Import/Export-Slot. |
| `Not enough free export slots available to export <n> volume(s)` | Zu wenige freie Mailslots. Erst Bänder entnehmen. |
| `Source slot selection doesn't fit into destination slot selection.` | Mehr Quell- als Zielslots angegeben. |
| `Source slot selection and destination slot selection overlap.` | Ein Slot steht in beiden Listen. |
| `No volume named <name> in changer or in selected source slots.` | Ein Volume aus `volume=` steckt nicht in der Bibliothek bzw. nicht in den gewählten Slots. |
| `Skipping illegal volumename <name>.` | Ungültiger Volume-Name in `volume=`. |
| `Negative numbers not permitted`, `Range end is not integer.`, `Range start is not an integer.`, `Range end not bigger than start.`, `Input value is not an integer.`, `Values must be be greater than zero.`, `Slot too large.` | Fehler in einer Slotliste. |
| `Failed to move volume from source slot <n> to …` | Der Roboter konnte ein Band nicht bewegen. Status der Bibliothek prüfen. |

## Siehe auch

- [import](import.md) — Volumes aus den Mailslots in normale Slots holen
- [move](move.md) — Volumes zwischen Slots bewegen
- [update](update.md) — `update slots` gleicht den Katalog mit der Bibliothek ab
- [status](status.md) — `status slots storage=<storage>` zeigt die Belegung
