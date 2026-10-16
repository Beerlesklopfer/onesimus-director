# .sqltables — Tabellen des Katalogs auflisten

## Zweck

`.sqltables` listet alle Tabellen im Schema `public` der Katalogdatenbank, alphabetisch sortiert. Das hilft, bevor man mit [.sql](dot-sql.md) eine eigene Abfrage schreibt. Der Befehl liest nur und verändert nichts.

**Onesimus-Erweiterung (nicht im Original, C-074).** Das Bareos-Original kennt den Befehl nicht und antwortet `.sqltables: is an invalid command.`. Ob er bleibt, ist noch nicht entschieden. Die Ausgabe ist eine rohe Datenbanktabelle mit Spaltenkopf `tablename` und Trennlinie, kein gerahmtes Format wie bei `.sql`.

## Syntax

```text
.sqltables
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

## API-Modi

Unter `.api 0` stehen beim ersten Katalogzugriff der Sitzung die Katalog-Hinweiszeilen davor, unter `.api 1` nicht; sonst ist der Text gleich. Unter `.api 2` enthält das JSON-Objekt kein Ergebnis (`"result": {}`).

## Beispiele

### Tabellen auflisten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqltables
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
tablename
--------------------
bareos_schema_version
basefiles
client
counters
device
devicestats
file
fileset
[…]
status
storage
tapealerts
version
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.sqltables: is an invalid command.` (C-074)

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqltables
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet mit einem Fehlerobjekt und der Meldung `.sqltables: is an invalid command.` (C-074)

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.sqltables: is an invalid command.` | Die `Command ACL` der Konsole erlaubt den Befehl nicht. |
| `SQL error: …` | Die Abfrage ist in der Datenbank fehlgeschlagen. |

## Siehe auch

- [.sqlschema](dot-sqlschema.md) — Spalten aller Tabellen
- [.sql](dot-sql.md) — beliebige SQL-Abfrage
- [sqlquery](sqlquery.md) — interaktive SQL-Abfrage
