# .sqltables — Tabellen des Katalogs auflisten

## Zweck

`.sqltables` listet alle Tabellen im Schema `public` der Katalogdatenbank, alphabetisch sortiert. Das hilft, bevor man mit [.sql](dot-sql.md) eine eigene Abfrage schreibt. Der Befehl liest nur und verändert nichts.

**Onesimus-Erweiterung, nicht im Original (C-074, Entscheidung in [#4](https://github.com/Beerlesklopfer/onesimus-director/issues/4): bleibt als dokumentierte Erweiterung).** Das Bareos-Original kennt den Befehl nicht und antwortet `.sqltables: is an invalid command.`.

## Berechtigungen

Neben `.sqltables` in der `Command ACL` braucht die Konsole wie für jede rohe Katalogabfrage `.sql` oder `sqlquery`. Der Katalog wird wie bei [.sql](dot-sql.md) gewählt (`catalog=`, `Catalog ACL`).

## Syntax

```text
.sqltables [catalog=<name>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

## API-Modi

Wie bei [.sql](dot-sql.md): unter `.api 0` eine Tabelle (beim ersten Katalogzugriff der Sitzung mit den Katalog-Hinweiszeilen davor), unter `.api 1` je Feld der Wert mit Tabulator dahinter, unter `.api 2` die Zeilen als Objekte unter `sqltables`.

## Beispiele

### Tabellen auflisten

Paritätsfall `c074-dot-sqltables` (erwartete Abweichung).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqltables
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------------------+
| tablename             |
+-----------------------+
| bareos_schema_version |
| basefiles             |
| client                |
| counters              |
| device                |
| devicestats           |
| file                  |
| fileset               |
| job                   |
| jobhisto              |
| jobmedia              |
| jobstats              |
| location              |
| locationlog           |
| log                   |
| media                 |
| mediatype             |
| ndmpjobenvironment    |
| ndmplevelmap          |
| path                  |
| pathhierarchy         |
| pathvisibility        |
| pool                  |
| quota                 |
| restoreobject         |
| status                |
| storage               |
| tapealerts            |
| version               |
+-----------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.sqltables: is an invalid command.` (C-074, Onesimus-Erweiterung).

### Als JSON

Paritätsfall `c074-dot-sqltables-api2`.

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
  "result": {
    "sqltables": [
      {
        "tablename": "bareos_schema_version"
      },
      {
        "tablename": "basefiles"
      },
[…]
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet mit einem Fehlerobjekt und der Meldung `.sqltables: is an invalid command.` (C-074).

### Konsole ohne `.sql`/`sqlquery`

Das Profil `operator` verbietet `.sql` und `sqlquery`:

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .sqltables
.sqltables: permission denied, reading raw catalog data requires the CommandACL .sql or sqlquery.
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.sqltables: is an invalid command.` | Die `Command ACL` erlaubt `.sqltables` nicht (im Original: den Befehl gibt es nicht). |
| `.sqltables: permission denied, reading raw catalog data requires the CommandACL .sql or sqlquery.` | Die Konsole darf weder `.sql` noch `sqlquery`. |
| `Query failed: …` | Die Abfrage ist in der Datenbank fehlgeschlagen. |

## Siehe auch

- [.sqlschema](dot-sqlschema.md) — Spalten der Tabellen
- [.sqlcount](dot-sqlcount.md) — Zeilen zählen
- [.sql](dot-sql.md) — beliebige SQL-Abfrage
- [sqlquery](sqlquery.md) — interaktive SQL-Abfrage
