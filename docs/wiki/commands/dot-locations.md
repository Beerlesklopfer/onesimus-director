# .locations — Lagerorte aus dem Katalog auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L253) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Keine Beschreibung in der Originaltabelle (NULL)

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 4 von 4 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (4):** `c002-shared-then-private`, `c029-locations`, `c029-locations-api2`, `dot_locations`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:315](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.locations` gibt die Lagerorte (Tabelle `Location` des Katalogs) aus, alphabetisch sortiert und ohne Doppelte. Lagerorte beschreiben, wo sich ein Band physisch befindet, etwa „Tresor“ oder „Außenlager“. Der Befehl liest nur den Katalog und verändert nichts. Gibt es keine Lagerorte, ist die Liste leer.

## Syntax

```text
.locations
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

Andere Wörter werden ignoriert.

## API-Modi

- `.api 0`: beim ersten Katalogzugriff der Sitzung stehen die Hinweiszeilen `Automatically selected Catalog: …` und `Using Catalog "…"` vor der Liste.
- `.api 1`: nur die Namen.
- `.api 2`: `result.locations`, ein Array von Objekten mit `name`.

## Beispiele

Im Testkatalog ist kein Lagerort eingetragen.

### Lagerorte auflisten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .locations
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .locations
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .locations
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "locations": []
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.locations: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.locations` nicht. |
| `List Location failed: ERR=…` | Die Abfrage im Katalog ist fehlgeschlagen; der Text hinter `ERR=` kommt von der Datenbank. |

## Siehe auch

- [.media](dot-media.md) — Volume-Namen
- [list](list.md) — `list volumes`
