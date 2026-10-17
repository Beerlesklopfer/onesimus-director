# create — Pool im Katalog anlegen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L321) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Create DB Pool from resource

**Originalsyntax:**

```text
create pool=<pool-name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 4 von 4 Fällen byte-identisch; Modi: Text.
- Verglichen sind Dialog und unbekannter Pool; das Anlegen eines Pools im Katalog ist nicht verglichen.

**Paritätsfälle (4):** `c011-create-cancel`, `c011-create-unknown-cancel`, `c012-create-unknown-pool`, `create-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:909](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`create` legt für eine Pool-Ressource aus der Director-Konfiguration den passenden Pool-Eintrag im Katalog an. Normalerweise macht der Director das beim Start selbst; `create` braucht man, wenn ein Pool neu konfiguriert wurde und vor dem nächsten Job im Katalog stehen soll. Der Befehl verändert nur den Katalog. Um einen schon vorhandenen Pool-Eintrag an eine geänderte Konfiguration anzupassen, dient `update pool=<pool>`.

## Syntax

```text
create [ pool=<pool> ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `pool=<pool>` | Name einer Pool-Ressource aus der Konfiguration | Pool, der angelegt wird. Fehlt die Angabe oder ist der Name unbekannt, erscheint eine Auswahlliste der konfigurierten Pools. |

## Dialoge

Ohne gültiges `pool=` fragt `create` mit `Select Pool resource (1-<n>):` nach dem Pool. Die Liste zeigt die Pool-Ressourcen aus der Konfiguration in alphabetischer Reihenfolge. Ein `.` bricht ab (`Selection aborted, nothing done.`).

## API-Modi

Unter `.api 0` und `.api 1` kommen die Meldungen als Text. Unter `.api 2` kommen Fehler als JSON-Fehlerobjekt; ein Beispiel steht unten bei „Pool steht schon im Katalog“.

## Beispiele

### Pool auswählen lassen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> create
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

### Unbekannter Pool

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> create pool=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Error: Pool resource "nosuch" does not exist.
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

### Pool steht schon im Katalog

Steht der Pool schon im Katalog, scheitert Onesimus derzeit an einer Katalogabfrage (Spalte `fileretention`) und meldet trotzdem `Pool Full created.`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> create pool=Full
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Query: error returned from database: Spalte >>fileretention<< von Relation >>pool<< existiert nicht
Pool Full created.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```
**Abweichung vom Original:** Das Original meldet `Error: Pool Full already exists.`, `Use update to change it.` und danach `Pool Full created.` (C-066)

Dasselbe über die Auswahlliste:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> create
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Pool resources are:
1: Differential
2: Full
3: Incremental
4: Scratch
Select Pool resource (1-4):
> 2
Query: error returned from database: Spalte >>fileretention<< von Relation >>pool<< existiert nicht
Pool Full created.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```
**Abweichung vom Original:** Das Original meldet `Error: Pool Full already exists.`, `Use update to change it.` und danach `Pool Full created.` (C-066)

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> create pool=Full
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
          "Query: error returned from database: Spalte >>fileretention<< von Relation >>pool<< existiert nicht\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```
**Abweichung vom Original:** Das Original liefert als Fehlermeldung `Error: Pool Full already exists.\nUse update to change it.\n` (C-066)

### Neuen Pool anlegen

```text
Beispielausgabe nicht verfügbar: alle Pool-Ressourcen der Testkonfiguration stehen schon im Katalog, ein neu angelegter Pool lässt sich ohne Konfigurationsänderung nicht zeigen.
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Error: Pool resource "<name>" does not exist.` | Es gibt keine Pool-Ressource dieses Namens in der Konfiguration. Pool aus der folgenden Liste wählen oder die Konfiguration prüfen (`show pools`). |
| `Error: Pool <name> already exists.` / `Use update to change it.` | Der Pool steht schon im Katalog (Bareos-Verhalten). Änderungen mit `update pool=<name>` übernehmen. |
| `Query: error returned from database: …` | Der Katalog hat die Abfrage abgewiesen. Bei Onesimus derzeit für vorhandene Pools, siehe oben. |
| `Selection aborted, nothing done.` | Die Auswahl wurde mit `.` abgebrochen. |

## Siehe auch

- [update](update.md) — Pool-Eintrag aus der Konfiguration aktualisieren (`update pool=<pool>`)
- [add](add.md) — Volumes in einem Pool anlegen
- [show](show.md) — Pool-Ressourcen der Konfiguration anzeigen
- [list](list.md) — Pools im Katalog anzeigen (`list pools`)
