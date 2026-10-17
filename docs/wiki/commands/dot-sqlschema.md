# .sqlschema — Spalten der Katalogtabellen anzeigen

## Zweck

`.sqlschema` zeigt für die Tabellen im Schema `public` der Katalogdatenbank alle Spalten mit ihrem Datentyp, sortiert nach Tabelle und Spaltenreihenfolge; mit `table=<name>` nur für eine Tabelle. Das hilft beim Schreiben eigener Abfragen mit [.sql](dot-sql.md). Der Befehl liest nur und verändert nichts.

**Onesimus-Erweiterung, nicht im Original (C-074, Entscheidung in [#4](https://github.com/Beerlesklopfer/onesimus-director/issues/4): bleibt als dokumentierte Erweiterung).** Das Bareos-Original kennt den Befehl nicht und antwortet `.sqlschema: is an invalid command.`.

## Berechtigungen

Neben `.sqlschema` in der `Command ACL` braucht die Konsole wie für jede rohe Katalogabfrage `.sql` oder `sqlquery`. Der Katalog wird wie bei [.sql](dot-sql.md) gewählt (`catalog=`, `Catalog ACL`).

## Syntax

```text
.sqlschema [table=<name>] [catalog=<name>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `table=<name>` | Tabellenname (Buchstaben, Ziffern, `_`; Groß-/Kleinschreibung egal) | nur die Spalten dieser Tabelle; eine unbekannte Tabelle ergibt eine leere Liste |
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

## API-Modi

Wie bei [.sql](dot-sql.md): unter `.api 0` eine Tabelle (beim ersten Katalogzugriff der Sitzung mit den Katalog-Hinweiszeilen davor), unter `.api 1` je Feld der Wert mit Tabulator dahinter, unter `.api 2` die Zeilen als Objekte unter `sqlschema`.

## Beispiele

### Spalten einer Tabelle

Paritätsfall `c074-dot-sqlschema-table` (erwartete Abweichung).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlschema table=pool
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+------------+--------------------+-----------+
| table_name | column_name        | data_type |
+------------+--------------------+-----------+
| pool       | poolid             | integer   |
| pool       | name               | text      |
| pool       | numvols            | integer   |
| pool       | maxvols            | integer   |
| pool       | useonce            | smallint  |
| pool       | usecatalog         | smallint  |
| pool       | acceptanyvolume    | smallint  |
| pool       | volretention       | bigint    |
| pool       | voluseduration     | bigint    |
| pool       | maxvoljobs         | integer   |
| pool       | maxvolfiles        | integer   |
| pool       | maxvolbytes        | bigint    |
| pool       | autoprune          | smallint  |
| pool       | recycle            | smallint  |
| pool       | actiononpurge      | smallint  |
| pool       | pooltype           | text      |
| pool       | labeltype          | integer   |
| pool       | labelformat        | text      |
| pool       | enabled            | smallint  |
| pool       | scratchpoolid      | integer   |
| pool       | recyclepoolid      | integer   |
| pool       | nextpoolid         | integer   |
| pool       | minblocksize       | integer   |
| pool       | maxblocksize       | integer   |
| pool       | migrationhighbytes | bigint    |
| pool       | migrationlowbytes  | bigint    |
| pool       | migrationtime      | bigint    |
+------------+--------------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.sqlschema: is an invalid command.` (C-074, Onesimus-Erweiterung).

### Als JSON

Paritätsfall `c074-dot-sqlschema-table-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlschema table=pool
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "sqlschema": [
      {
        "table_name": "pool",
        "column_name": "poolid",
        "data_type": "integer"
      },
      {
        "table_name": "pool",
        "column_name": "name",
        "data_type": "text"
      },
[…]
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet mit einem Fehlerobjekt und der Meldung `.sqlschema: is an invalid command.` (C-074).

### Ungültiger Tabellenname

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlschema table="pool; x"
.sqlschema: invalid table name "pool; x".
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.sqlschema: is an invalid command.` | Die `Command ACL` erlaubt `.sqlschema` nicht (im Original: den Befehl gibt es nicht). |
| `.sqlschema: permission denied, reading raw catalog data requires the CommandACL .sql or sqlquery.` | Die Konsole darf weder `.sql` noch `sqlquery`. |
| `.sqlschema: invalid table name "…".` | `table=` ist kein einfacher Tabellenname. |
| `Query failed: …` | Die Abfrage ist in der Datenbank fehlgeschlagen. |

## Siehe auch

- [.sqltables](dot-sqltables.md) — nur die Tabellennamen
- [.sqlcount](dot-sqlcount.md) — Zeilen zählen
- [.sql](dot-sql.md) — beliebige SQL-Abfrage
