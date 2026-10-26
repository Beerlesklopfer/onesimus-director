# .mediatypes — Medientypen aus dem Katalog auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L257) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all media types

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c002-private-then-shared`, `c025-mediatypes-limited`, `c025-mediatypes-operator`, `c025-mediatypes-bvfs`, `c025-mediatypes-acl`, `c029-mediatypes`, `c029-mediatypes-api1`, `c029-mediatypes-api2`, `dot_mediatypes`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:335](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.mediatypes` gibt die Medientypen aus, die im Katalog eingetragen sind (Tabelle `MediaType`), alphabetisch sortiert und ohne Doppelte. Programme nutzen die Liste etwa beim Labeln oder Hinzufügen von Volumes. Der Befehl liest nur den Katalog und verändert nichts.

## Syntax

```text
.mediatypes
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

Andere Wörter werden ignoriert.

## API-Modi

- `.api 0`: beim ersten Katalogzugriff der Sitzung stehen die Hinweiszeilen `Automatically selected Catalog: …` und `Using Catalog "…"` vor der Liste.
- `.api 1`: nur die Namen.
- `.api 2`: `result.mediatypes`, ein Array von Objekten mit `name`.

## Beispiele

### Alle Medientypen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .mediatypes
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
File
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Zweiter Katalogzugriff in derselben Sitzung

Die Zeile `Automatically selected Catalog` erscheint nur beim ersten Katalogzugriff; danach steht nur noch `Using Catalog`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+--------+--------------+---------+---------+----------+---------------+
| poolid | name         | numvols | maxvols | pooltype | labelformat   |
+--------+--------------+---------+---------+----------+---------------+
|      1 | Scratch      |       0 |       0 | Scratch  | *             |
|      2 | Incremental  |       1 |     100 | Backup   | Incremental-  |
|      3 | Full         |       3 |     100 | Backup   | Full-         |
|      4 | Differential |       1 |     100 | Backup   | Differential- |
+--------+--------------+---------+---------+----------+---------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .mediatypes
Using Catalog "MyCatalog"
File
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .mediatypes
File
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .mediatypes
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "mediatypes": [
      {
        "name": "File"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.mediatypes: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.mediatypes` nicht. |
| `List MediaType failed: ERR=…` | Die Abfrage im Katalog ist fehlgeschlagen; der Text hinter `ERR=` kommt von der Datenbank. |

## Siehe auch

- [.storages](dot-storages.md) — Storages, deren Medientyp mit `.defaults storage=` sichtbar wird
- [.media](dot-media.md) — Volume-Namen
- [label](label.md) — Volume labeln
