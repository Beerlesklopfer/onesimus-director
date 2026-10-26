# .sql — SQL-Abfrage im Katalog ausführen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L266) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Send an arbitrary SQL command

**Originalsyntax:**

```text
.sql query=<sqlquery>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI.

**Paritätsfälle (30):** `c002-sql-then-list`, `c016-sql-count`, `c016-sql-multicol-null`, `c016-sql-empty`, `c016-sql-syntax-error`, `c016-sql-unknown-table`, `c016-sql-no-query`, `c016-sql-no-value`, `c016-sql-semicolons`, `c016-sql-trailing-semicolon`, `c016-sql-api1`, `c016-sql-api1-empty`, `c016-sql-api1-error`, `c016-sql-api2`, `c016-sql-api2-empty`, `c016-sql-api2-error`, `c016-sql-api2-noquery`, `c016-sql-error-truncated`, `c016-sql-error-end-of-input`, `c016-sql-error-runtime`, `c016-sql-error-hint`, `c016-sql-jobid-bytes`, `c016-sql-api1-semicolons`, `c016-sql-api2-error-hint`, `c020-compact-sql`, `c020-json-sql`, `c027-gui-sql`, `c028-sql`, `c028-sql-bad`, `dot-sql`.

**Lokaler Handler:** [crates/odir/src/commands/sql.rs:27](../../../crates/odir/src/commands/sql.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.sql` schickt eine beliebige SQL-Anweisung an die Katalogdatenbank und gibt das Ergebnis aus. Die WebUI und Skripte nutzen den Befehl für Auswertungen, die kein anderer Befehl liefert. **Vorsicht:** Die Anweisung wird ohne Prüfung ausgeführt; `UPDATE`, `DELETE` oder `DROP` verändern den Katalog dauerhaft. Für reines Lesen genügt ein `SELECT`.

## Syntax

```text
.sql query="<sql-anweisung>"
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `query="<sql>"` | SQL-Text in Anführungszeichen | die auszuführende Anweisung |
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

Enthält der Text mehrere durch `;` getrennte Anweisungen, werden alle ausgeführt, angezeigt wird nur das Ergebnis der letzten. Ein `;` am Ende schadet nicht. Andere Schlüsselwörter wie `tables` oder `schema` gibt es nicht; für Tabellen und Spalten siehe [.sqltables](dot-sqltables.md) und [.sqlschema](dot-sqlschema.md).

## API-Modi

- `.api 0`: Tabelle mit Rahmen; Zahlen rechtsbündig mit Tausenderpunkten (`1,234,567`), leere Werte als leere Zelle. Beim ersten Katalogzugriff der Sitzung stehen die Hinweiszeilen `Automatically selected Catalog: …` und `Using Catalog "…"` davor. Ohne Treffer: `No results to list.`
- `.api 1`: alle Zellen tabulatorgetrennt; nach jeder Zeile folgt ein zusätzlicher Tabulator statt eines Zeilenumbruchs, die Antwort ist also eine einzige Zeile. Ohne Treffer kommt nichts.
- `.api 2`: `result.query`, ein Array mit einem Objekt pro Zeile; Schlüssel sind die Spaltennamen, alle Werte Zeichenketten ohne Tausenderpunkte. Steuerzeichen werden JSON-gerecht maskiert.
- Unter `gui on` (siehe [gui](gui.md)) stehen Zahlen ohne Tausenderpunkte und linksbündig.

## Beispiele

### Einfache Abfrage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select count(*) from job"
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+
| count |
+-------+
|   160 |
+-------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Spalten und leere Werte

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select poolid, name, NULL as nothing, 1234567 as big, 'x'::text as t, 1.5 as f, now() is null as b from pool order by poolid"
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+--------+--------------+---------+-----------+------+------+------+
| poolid | name         | nothing | big       | t    | f    | b    |
+--------+--------------+---------+-----------+------+------+------+
|      1 | Scratch      |         | 1,234,567 | x    |  1.5 | f    |
|      2 | Incremental  |         | 1,234,567 | x    |  1.5 | f    |
|      3 | Full         |         | 1,234,567 | x    |  1.5 | f    |
|      4 | Differential |         | 1,234,567 | x    |  1.5 | f    |
+--------+--------------+---------+-----------+------+------+------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kein Treffer

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select poolid, name from pool where poolid < 0"
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
No results to list.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Anweisungen

Nur das Ergebnis der letzten Anweisung wird gezeigt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select 1 as a; select 2 as b"
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+------+
| b    |
+------+
|    2 |
+------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="SELECT poolid, name FROM pool ORDER BY poolid"
1	Scratch	2	Incremental	3	Full	4	Differential
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select poolid, name, NULL as nothing, 1234567 as big from pool order by poolid"
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "query": [
      {
        "poolid": "1",
        "name": "Scratch",
        "nothing": "",
        "big": "1234567"
      },
      {
        "poolid": "2",
        "name": "Incremental",
        "nothing": "",
        "big": "1234567"
      },
[…]
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Sonderzeichen im kompakten JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select 'a/b' || chr(34) || chr(92) || chr(1) || chr(9) || chr(10) || chr(31) || chr(127) || 'äö€' as s"
{"jsonrpc":"2.0","id":null,"result":{"query":[{"s":"a/b\"\\\u0001\t\n\u001Fäö€"}]}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `gui on`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select count(*) as n, 1234567 as big from pool"
Using Catalog "MyCatalog"
+------+-----------+
| n    | big       |
+------+-----------+
| 4    | 1234567   |
+------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Syntaxfehler

Die Fehlermeldung der Datenbank wird durchgereicht, hier in der Sprache des Datenbankservers (Deutsch).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="selec nothing"
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Query failed: .sql query="selec nothing". ERR=Query failed: FEHLER:  Syntaxfehler bei >>selec<<
LINE 1: selec nothing
        ^

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql query="select nosuch from nosuch"
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
          "Query failed: .sql query=\"select nosuch from nosuch\". ERR=Query failed: FEHLER:  Relation >>nosuch<< existiert nicht\nLINE 1: select nosuch from nosuch\n                           ^\n\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne `query=`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sql
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
query keyword not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `query keyword not found.` | `query=` fehlt oder hat keinen Wert (auch bei `.sql tables`). |
| `Query failed: .sql query="…". ERR=Query failed: …` | Die Datenbank hat die Anweisung abgelehnt; dahinter folgen ihre Meldung, die Zeile mit der Fehlerstelle und gegebenenfalls ein `HINT:`. Unter `.api 1` steht nach `ERR=Query failed:` zusätzlich die Anweisung selbst und `: ERR=`. |
| `.sql: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.sql` nicht. |

## Siehe auch

- [sqlquery](sqlquery.md) — interaktive SQL-Eingabe
- [query](query.md) — vordefinierte Abfragen
- [.sqltables](dot-sqltables.md), [.sqlschema](dot-sqlschema.md) — Tabellen und Spalten des Katalogs
- [list](list.md) — fertige Auswertungen von Jobs, Volumes und Pools
