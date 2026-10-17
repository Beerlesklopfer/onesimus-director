# sqlquery — SQL direkt im Katalog ausführen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L482) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Use SQL to query catalog

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 13 von 13 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-limited.

**Paritätsfälle (13):** `c006-unknown-limited`, `c011-sqlquery-cancel`, `c011-sqlquery-empty`, `c011-sqlquery-multiline`, `c011-sqlquery-args-ignored`, `c016-sqlquery-select`, `c016-sqlquery-multiline`, `c016-sqlquery-error`, `c016-sqlquery-empty`, `c016-sqlquery-args-ignored`, `c016-sqlquery-api2`, `c028-sqlquery`, `sqlquery-select`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/sql.rs:89](../../../crates/onesimus-dird/src/commands/sql.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`sqlquery` öffnet einen SQL-Eingabemodus für den Katalog der Sitzung. Jede Anweisung, die mit `;` endet, wird sofort ausgeführt, und das Ergebnis erscheint als Tabelle. Der Befehl ist für Auswertungen gedacht, die `list` und `query` nicht bieten.

**Vorsicht:** Der Director prüft die Anweisungen nicht. Auch `UPDATE`, `DELETE` oder `DROP` werden ausgeführt und verändern den Katalog dauerhaft. Wer den Befehl benutzen darf, legt die `CommandACL` der Konsole fest.

## Syntax

```text
sqlquery [sharedsession]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `sharedsession` | — | Die gemeinsame Katalogverbindung des Directors verwenden statt einer eigenen für diese Sitzung. Muss nach `sqlquery` stehen. |

Andere Wörter hinter `sqlquery` werden ignoriert; insbesondere wird `sqlquery select 1;` nicht ausgeführt, sondern öffnet nur den Eingabemodus.

## Dialoge

Nach dem Start erscheinen drei Hinweiszeilen und die Frage `Enter SQL query: `.

- Eine Zeile, die mit `;` endet, schließt die Anweisung ab; sie wird ausgeführt, dann folgt wieder `Enter SQL query: `.
- Eine Zeile ohne `;` am Ende wird gesammelt; der Director fragt `Add to SQL query: ` nach der Fortsetzung. Die Zeilen werden mit einem Leerzeichen verbunden.
- Eine leere Zeile oder `.` beendet den Modus mit `End query mode.`

## API-Modi

Unter `.api 1` kommen die Fragen als Texteingabe (oconsole zeigt `input> `), der Fragetext erst mit der nächsten Antwort, und Ergebnisse ohne Tabellenrahmen. Unter `.api 2` startet kein Eingabemodus: `sqlquery` liefert sofort ein leeres Ergebnis (`"result": {}`), und eine folgende SQL-Zeile wird als normaler Befehl gelesen und abgelehnt (`select: is an invalid command.`). Für SQL mit JSON-Ausgabe gibt es [`.sql`](dot-sql.md).

## Beispiele

### Eine Abfrage ausführen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> sqlquery
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Entering SQL query mode.
Terminate each query with a semicolon.
Terminate query mode with a blank line.
Enter SQL query:
> select poolid, name, NULL as n from pool order by poolid;
+--------+--------------+------+
| poolid | name         | n    |
+--------+--------------+------+
|      1 | Scratch      |      |
|      2 | Incremental  |      |
|      3 | Full         |      |
|      4 | Differential |      |
+--------+--------------+------+
Enter SQL query:
>
End query mode.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Anweisung über mehrere Zeilen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> sqlquery
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Entering SQL query mode.
Terminate each query with a semicolon.
Terminate query mode with a blank line.
Enter SQL query:
> select count(*)
Add to SQL query:
> from pool;
+-------+
| count |
+-------+
|     4 |
+-------+
Enter SQL query:
>
End query mode.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Leeres Ergebnis

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> sqlquery
[…]
Enter SQL query:
> select poolid from pool where poolid < 0;
No results to list.
Enter SQL query:
>
End query mode.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler in der Anweisung

Die Meldung kommt von PostgreSQL in der Sprache des Datenbankservers, hier Deutsch (Paritätsfall `c016-sqlquery-error`). oconsole zeigt die Zeile wegen `failed` rot.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> sqlquery
[…]
Enter SQL query:
> selec nothing;
Query failed: FEHLER:  Syntaxfehler bei >>selec<<
LINE 1: selec nothing;
        ^

Enter SQL query:
>
End query mode.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit `.` beenden

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> sqlquery
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Entering SQL query mode.
Terminate each query with a semicolon.
Terminate query mode with a blank line.
Enter SQL query:
> .
End query mode.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 1`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> sqlquery
Entering SQL query mode.
Terminate each query with a semicolon.
Terminate query mode with a blank line.
input> SELECT poolid, name FROM pool ORDER BY poolid;
Enter SQL query:
      1 | Scratch      |
      2 | Incremental  |
      3 | Full         |
      4 | Differential |
input> .
Enter SQL query:
End query mode.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 2`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> sqlquery
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> select count(*) from pool;
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
          "select: is an invalid command.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Query failed: <Fehler der Datenbank>` | Die Anweisung ist fehlerhaft. Der Modus bleibt offen; Anweisung korrigiert neu eingeben. |
| `No results to list.` | Die Anweisung lieferte keine Zeilen. |
| `sqlquery: is an invalid command.` | Die `CommandACL` der Konsole erlaubt den Befehl nicht. |

## Siehe auch

- [query](query.md) — vordefinierte Abfragen
- [.sql](dot-sql.md) — SQL in einer Zeile, auch mit JSON-Ausgabe
- [.sqltables](dot-sqltables.md) — Tabellen des Katalogs
- [list](list.md) — Katalogeinträge ohne SQL
- [use](use.md) — Katalog wählen
