# .sqlcount — Anzahl der Jobs im Katalog

## Zweck

`.sqlcount` zählt die Einträge der Tabelle `job` im Katalog und gibt das Ergebnis aus. Der Befehl liest nur und verändert nichts.

**Onesimus-Erweiterung (nicht im Original, C-074).** Das Bareos-Original kennt den Befehl nicht und antwortet `.sqlcount: is an invalid command.`; Programme, die mit beiden Directors arbeiten sollen, dürfen sich nicht auf ihn verlassen. Ob er bleibt, ist noch nicht entschieden. Die Ausgabe ist eine rohe Datenbanktabelle hinter dem Präfix `Jobs: `, kein gerahmtes Format wie bei [.sql](dot-sql.md).

## Syntax

```text
.sqlcount
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt derselbe Text. Unter `.api 2` enthält das JSON-Objekt kein Ergebnis (`"result": {}`); der Befehl ist für den JSON-Modus nicht ausgelegt.

## Beispiele

### Jobs zählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlcount
Jobs: count
--------------------
161
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.sqlcount: is an invalid command.` (C-074)

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlcount
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet mit einem Fehlerobjekt und der Meldung `.sqlcount: is an invalid command.` (C-074)

### Konsole mit Command ACL

Konsolen mit eigener `Command ACL` dürfen den Befehl nur, wenn er dort ausdrücklich steht.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .sqlcount
.sqlcount: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.sqlcount: is an invalid command.` | Die `Command ACL` der Konsole erlaubt den Befehl nicht. |
| `SQL error: …` | Die Abfrage ist in der Datenbank fehlgeschlagen. |
| `No catalog` | Es ist keine Katalogverbindung vorhanden. |

## Siehe auch

- [.sql](dot-sql.md) — beliebige SQL-Abfrage, z. B. `select count(*) from job`
- [list](list.md) — `list jobs count`
- [.sqltables](dot-sqltables.md), [.sqlschema](dot-sqlschema.md) — weitere Onesimus-Erweiterungen
