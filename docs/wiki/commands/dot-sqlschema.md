# .sqlschema — Spalten aller Katalogtabellen anzeigen

## Zweck

`.sqlschema` zeigt für jede Tabelle im Schema `public` der Katalogdatenbank alle Spalten mit ihrem Datentyp, sortiert nach Tabelle und Spaltenreihenfolge. Das hilft beim Schreiben eigener Abfragen mit [.sql](dot-sql.md). Der Befehl liest nur und verändert nichts.

**Onesimus-Erweiterung (nicht im Original, C-074).** Das Bareos-Original kennt den Befehl nicht und antwortet `.sqlschema: is an invalid command.`. Ob er bleibt, ist noch nicht entschieden. Die Ausgabe ist eine rohe Datenbanktabelle mit festen Spaltenbreiten von 20 Zeichen; längere Namen laufen ohne Abstand in die nächste Spalte (`bareos_schema_versionversion`).

## Syntax

```text
.sqlschema
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

## API-Modi

Unter `.api 0` stehen beim ersten Katalogzugriff der Sitzung die Katalog-Hinweiszeilen davor, unter `.api 1` nicht; sonst ist der Text gleich. Unter `.api 2` enthält das JSON-Objekt kein Ergebnis (`"result": {}`).

## Beispiele

### Schema anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlschema
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
table_name          column_name         data_type
------------------------------------------------------------
bareos_schema_versionversion             integer
bareos_schema_versionapplied_at          timestamp without time zone
basefiles           baseid              bigint
basefiles           jobid               integer
basefiles           fileid              bigint
basefiles           fileindex           integer
basefiles           basejobid           integer
client              clientid            integer
client              name                text
client              uname               text
[…]
vw_volume_status    slot                bigint
vw_volume_status    poolname            text
vw_volume_status    pct_full            numeric
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.sqlschema: is an invalid command.` (C-074)

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlschema
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet mit einem Fehlerobjekt und der Meldung `.sqlschema: is an invalid command.` (C-074)

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.sqlschema: is an invalid command.` | Die `Command ACL` der Konsole erlaubt den Befehl nicht. |
| `SQL error: …` | Die Abfrage ist in der Datenbank fehlgeschlagen. |

## Siehe auch

- [.sqltables](dot-sqltables.md) — nur die Tabellennamen
- [.sql](dot-sql.md) — beliebige SQL-Abfrage
- [.catalog](dot-catalog.md) — Hinweis zur Schemaverwaltung
