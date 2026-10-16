# .types — Jobtypen auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L279) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all job types

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (2):** `c029-types-api2`, `dot_types`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:378](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.types` gibt alle Jobtypen aus, die der Director kennt (`Backup`, `Restore`, `Verify` und so weiter). Programme füllen damit Auswahllisten, etwa für `type=` bei [.jobs](dot-jobs.md) oder für `jobtype=` bei `list jobs`. Die Liste ist fest im Director hinterlegt; der Befehl liest weder Konfiguration noch Katalog und verändert nichts.

## Syntax

```text
.types
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Liste. Unter `.api 2` steht sie als `result.jobtypes`, ein Array von Objekten mit `name`.

## Beispiele

### Jobtypen auflisten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .types
Backup
Admin
Archive
Verify
Restore
Migrate
Copy
Consolidate
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .types
Backup
Admin
Archive
Verify
Restore
Migrate
Copy
Consolidate
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .types
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobtypes": [
      {
        "name": "Backup"
      },
      {
        "name": "Admin"
      },
      {
[…]
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.types: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.types` nicht. |

## Siehe auch

- [.jobs](dot-jobs.md) — Jobs eines Typs auflisten
- [.levels](dot-levels.md) — Level je Jobtyp
- [list](list.md) — `list jobs jobtype=<typ>`
