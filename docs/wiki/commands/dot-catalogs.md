# .catalogs — Namen der Kataloge auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L229) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all catalog resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c025-catalogs-limited`, `c025-catalogs-limited-api2`, `c025-catalogs-operator`, `c025-catalogs-operator-api2`, `c025-catalogs-bvfs`, `c025-catalogs-bvfs-api2`, `c025-catalogs-acl`, `c025-catalogs-acl-api2`, `dot_catalogs`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:234](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.catalogs` gibt die Namen aller Catalog-Ressourcen der Director-Konfiguration aus, einen pro Zeile. Programme nutzen ihn für die Vervollständigung von `catalog=`. Der Befehl liest nur die Konfiguration, öffnet keine Datenbankverbindung und verändert nichts.

Es erscheinen nur Kataloge, die die Konsole laut `Catalog ACL` sehen darf.

## Syntax

```text
.catalogs
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.catalogs`, ein Array von Objekten mit `name`.

## Beispiele

### Alle Kataloge

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .catalogs
MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .catalogs
MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .catalogs
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "catalogs": [
      {
        "name": "MyCatalog"
      }
    ]
  }
}
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.catalogs: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.catalogs` nicht. |

## Siehe auch

- [use](use.md) — Katalog für die Sitzung wählen
- [.catalog](dot-catalog.md) — Abkürzung, die ebenfalls `.catalogs` ausführt
- [.sql](dot-sql.md) — SQL-Abfrage im Katalog
