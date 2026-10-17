# relabel — Vorhandenes Volume neu beschriften

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L416) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Relabel a tape

**Originalsyntax:**

```text
relabel storage=<storage-name> oldvolume=<old-volume-name>  volume=<new-volume-name> pool=<pool-name> [ encrypt ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 8 von 8 Fällen byte-identisch; Modi: Text.
- Verglichen sind Dialog- und Abbruchpfade; Relabel über den SD ist nur durch Einheitentests belegt (Antwortauswertung siehe C-053).

**Offene Logeinträge:** `C-053` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (8):** `c011-relabel-cancel`, `c011-relabel-pool-cancel`, `c011-relabel-status`, `c011-relabel-unknown-cancel`, `c012-relabel-unknown-old`, `relabel-noargs`, `t001-relabel-not-purged`, `t001-relabel-old-unknown-cancel`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:649](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`relabel` schreibt ein neues Label auf ein Volume, das schon im Katalog steht, und gibt ihm dabei einen neuen Namen. Das geht nur bei Volumes mit dem Status `Purged` oder `Recycle`, also wenn keine Jobs mehr auf das Volume verweisen. Der Storage Daemon überschreibt das Medium; der alte Media-Eintrag wird aus dem Katalog gelöscht und ein neuer angelegt. Alle Daten auf dem Volume gehen dabei verloren.

## Syntax

```text
relabel [ storage=<storage> ] [ oldvolume=<alter-name> ] [ volume=<neuer-name> ] [ pool=<pool> ] [ slot=<slot> ] [ drive=<laufwerk> ] [ encrypt ] [ yes ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Name einer Storage-Ressource | Storage Daemon und Gerät. Auch `sd=` oder der Name allein. Fehlt die Angabe, erscheint eine Auswahlliste. |
| `oldvolume=<alter-name>` | Volume-Name aus dem Katalog | Das Volume, das neu beschriftet wird. Fehlt die Angabe oder ist der Name unbekannt, wählt man das Volume im Dialog aus. |
| `volume=<neuer-name>` | Volume-Name | Neuer Name. Es gelten dieselben Regeln wie bei [label](label.md). Fehlt die Angabe, fragt der Director danach. |
| `pool=<pool>` | Pool-Name | Pool für die Auswahl des alten Volumes. |
| `slot=<slot>` | Ganzzahl ≥ 0 | Slot im Autochanger. |
| `drive=<laufwerk>` | Ganzzahl ab 0 | Laufwerk des Storage-Geräts. |
| `encrypt` | — | Erzeugt einen neuen Schlüssel für Hardware-Verschlüsselung. |
| `yes` | — | Wird akzeptiert; `relabel` stellt aber keine Ja/Nein-Frage. |

## Dialoge

1. **Storage** — `Select Storage resource (1-<n>):` mit der Nummer aus der Liste.
2. **Altes Volume** — wenn `oldvolume=` fehlt oder unbekannt ist: zuerst die Pool-Auswahl `Select the Pool (1-<n>):`, dann eine Tabelle der Volumes dieses Pools und die Frage `Enter the volume name or MediaId of the volume prefixed with an asterisk (*).` Man gibt den Namen ein (`Full-0001`) oder die MediaId mit Stern (`*47318`).
3. **Neuer Name** — `Enter new Volume name:`, wenn `volume=` fehlt.

Ein `.` bricht jede Frage ab. Nach dem Abbruch ist nichts verändert.

## API-Modi

Unter `.api 0` und `.api 1` erscheinen die Meldungen als Text. Unter `.api 2` kommen Fehler als JSON-Fehlerobjekt mit der Meldung unter `error.data.messages.error`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> relabel storage=File oldvolume=Full-0001 volume=x
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
          "Volume \"Full-0001\" has VolStatus Used. It must be Purged or Recycled before relabeling.\n"
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> relabel
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

### Altes Volume aus einem Pool auswählen

Mit Storage, aber ohne `oldvolume=` zeigt `relabel` erst die Pools und dann die Volumes des gewählten Pools.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> relabel storage=File
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
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume ist noch nicht geleert

Ein Volume mit dem Status `Append`, `Full`, `Used` oder `Error` lässt sich nicht neu beschriften.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> relabel storage=File oldvolume=Full-47321
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Volume "Full-47321" has VolStatus Append. It must be Purged or Recycled before relabeling.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> relabel storage=File oldvolume=Full-0001 volume=t001new
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Volume "Full-0001" has VolStatus Error. It must be Purged or Recycled before relabeling.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekanntes altes Volume

Findet der Director `oldvolume=` nicht im Katalog, lässt er das Volume auswählen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> relabel storage=File oldvolume=nosuchvol
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
cats/sql_get.cc:1177 Media record for Volume "nosuchvol" not found.
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
> .
Selection aborted, nothing done.
cats/sql_get.cc:1177 Media record for Volume "nosuchvol" not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Steht zusätzlich `volume=` auf der Zeile, sucht der Director das alte Volume unter diesem Namen. Gibt es auch das nicht, endet der Befehl:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> relabel storage=File oldvolume=c012nosuch volume=c012new
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
cats/sql_get.cc:1177 Media record for Volume "c012nosuch" not found.
cats/sql_get.cc:1177 Media record for Volume "c012new" not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume erfolgreich neu beschriften

```text
Beispielausgabe nicht verfügbar: das Neubeschriften überschreibt ein Medium über den Storage Daemon und darf im Testbetrieb nicht ausgeführt werden.
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Volume "<name>" has VolStatus <status>. It must be Purged or Recycled before relabeling.` | Auf dem Volume stehen noch Jobs. Erst mit [purge](purge.md) leeren oder warten, bis es recycelt wird. |
| `cats/sql_get.cc:1177 Media record for Volume "<name>" not found.` | Ein Volume dieses Namens gibt es im Katalog nicht. Namen mit `list volumes` prüfen. |
| `Storage resource "<name>": not found` | Unbekannter oder nicht erlaubter Storage; die Auswahlliste folgt. |
| `Expected a positive integer, got: <eingabe>` | In einer Auswahlliste wurde keine Nummer eingegeben. |
| `Selection aborted, nothing done.` | Der Dialog wurde mit `.` abgebrochen. |

Für den neuen Namen gelten dieselben Meldungen wie bei [label](label.md#fehlermeldungen).

## Siehe auch

- [label](label.md) — neues Volume beschriften
- [purge](purge.md) — Jobs eines Volumes aus dem Katalog entfernen, damit es neu beschriftet werden kann
- [truncate](truncate.md) — geleerte Volumes kürzen
- [list](list.md) — Volumes und ihren Status anzeigen
