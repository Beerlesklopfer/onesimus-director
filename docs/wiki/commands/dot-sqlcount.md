# .sqlcount — Zeilen von Katalogtabellen zählen

## Zweck

`.sqlcount` zählt die Zeilen von Katalogtabellen. Ohne Argument kommen die Zahlen der wichtigsten Tabellen (`client`, `fileset`, `job`, `jobmedia`, `media`, `pool`, `storage`); `File` und `Path` fehlen absichtlich, weil das Zählen dort bei großen Katalogen lange dauert. Mit `table=<name>` wird genau eine Tabelle gezählt. Der Befehl liest nur und verändert nichts.

**Onesimus-Erweiterung, nicht im Original (C-074, Entscheidung in [#4](https://github.com/Beerlesklopfer/onesimus-director/issues/4): bleibt als dokumentierte Erweiterung).** Das Bareos-Original kennt den Befehl nicht und antwortet `.sqlcount: is an invalid command.`; Programme, die mit beiden Directors arbeiten sollen, dürfen sich nicht auf ihn verlassen.

## Berechtigungen

`.sqlcount` liest rohe Katalogdaten. Deshalb braucht die Konsole neben `.sqlcount` in ihrer `Command ACL` auch das Recht, das das Original für rohe Katalogabfragen verlangt: `.sql` oder `sqlquery`. Der Katalog wird wie bei [.sql](dot-sql.md) gewählt (`catalog=`, `Catalog ACL`).

## Syntax

```text
.sqlcount [table=<name>] [catalog=<name>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `table=<name>` | Tabellenname (Buchstaben, Ziffern, `_`; Groß-/Kleinschreibung egal) | nur diese Tabelle zählen |
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

## API-Modi

Wie bei [.sql](dot-sql.md): unter `.api 0` eine Tabelle (beim ersten Katalogzugriff der Sitzung mit den Katalog-Hinweiszeilen davor), unter `.api 1` je Feld der Wert mit Tabulator dahinter, unter `.api 2` die Zeilen als Objekte unter `sqlcount` (Werte als Zeichenketten, wie bei `.sql`).

## Beispiele

### Wichtige Tabellen zählen

Paritätsfall `c074-dot-sqlcount` (erwartete Abweichung, siehe unten).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlcount
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------+-------+
| tablename | count |
+-----------+-------+
| client    |     2 |
| fileset   |     4 |
| job       |   161 |
| jobmedia  |   264 |
| media     |     5 |
| pool      |     4 |
| storage   |     3 |
+-----------+-------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.sqlcount: is an invalid command.` (C-074, Onesimus-Erweiterung).

### Eine Tabelle als JSON

Paritätsfall `c074-dot-sqlcount-table-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlcount table=pool
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "sqlcount": [
      {
        "tablename": "pool",
        "count": "4"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet mit einem Fehlerobjekt und der Meldung `.sqlcount: is an invalid command.` (C-074).

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.sqlcount: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.sqlcount` nicht (im Original: der Befehl existiert nicht). |
| `.sqlcount: permission denied, reading raw catalog data requires the CommandACL .sql or sqlquery.` | Die Konsole darf `.sqlcount`, aber weder `.sql` noch `sqlquery`. |
| `.sqlcount: invalid table name "…".` | `table=` ist kein einfacher Tabellenname. |
| `Query failed: …` | Die Abfrage ist in der Datenbank fehlgeschlagen, z. B. weil es die Tabelle nicht gibt. |

## Siehe auch

- [.sql](dot-sql.md) — beliebige SQL-Abfrage
- [list](list.md) — `list jobs count`
- [.sqltables](dot-sqltables.md), [.sqlschema](dot-sqlschema.md) — weitere Onesimus-Erweiterungen
