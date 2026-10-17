# query — vordefinierte Katalogabfragen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L401) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Query catalog

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 8 von 8 Fällen byte-identisch; Modi: Text, API 1.

**Paritätsfälle (8):** `c011-query-cancel`, `c011-query-badnumber`, `c011-query-prompt-cancel`, `c011-query-client`, `c011-query-api1-cancel`, `c016-query-menu`, `c028-query-1`, `query-list`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/sql.rs:129](../../../crates/onesimus-dird/src/commands/sql.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`query` bietet eine Auswahl fertiger SQL-Abfragen an den Katalog an, etwa „wo liegt eine Datei“, „letzte Vollsicherungen eines Clients“ oder „Volumes eines Jobs“. Man wählt eine Abfrage per Nummer, beantwortet ihre Fragen (Client, Volume, JobId, …) und erhält das Ergebnis als Tabelle. Die Abfragen stehen in der Datei, die die Director-Direktive `QueryFile` nennt (Standard `/usr/lib/bareos/scripts/query.sql`). Die mitgelieferten Abfragen lesen nur.

## Syntax

```text
query
```

## Argumente

Der Befehl hat keine Argumente; alles Weitere wird im Dialog abgefragt. Der Katalog wird wie bei [`use`](use.md) gewählt (beim ersten Mal mit `Automatically selected Catalog: …`).

## Dialoge

1. **Auswahl der Abfrage:** Der Director listet `Available queries:` mit Nummern und fragt `Choose a query (1-22): ` (die Zahl hängt von der Abfragedatei ab). Antwort ist die Nummer. Eine Zahl außerhalb des Bereichs ergibt `Please enter a number between 1 and 22`, und die Frage kommt erneut. `.` bricht ab: `Selection aborted, nothing done.`
2. **Parameter der Abfrage:** Viele Abfragen fragen nach Werten, zum Beispiel `Enter Client name: `, `Enter Volume name: ` oder `Enter JobId: `. Die Eingabe wird in die Abfrage eingesetzt (Sonderzeichen werden maskiert). `.` überspringt den Wert; die Abfrage läuft dann ohne ihn und findet in der Regel nichts (`No results to list.`).

Die mitgelieferten Abfragen:

```text
 1: List up to 20 places where a File is saved regardless of the directory and Client
 2: List where the most recent copies of a file with path of a Client are saved
 3: List last 20 Full Backups for a Client
 4: List all backups for a Client after a specified time
 5: List all backups for a Client and COUNT the Volumes which been used
 6: List Volume Attributes for a selected Volume
 7: List Volumes used by selected JobId
 8: List Volumes to Restore All Files
 9: List Pool Attributes for a selected Pool
10: List total files/bytes by Job
11: List total files/bytes by Volume
12: List Files for a selected JobId
13: List Jobs stored on a selected MediaId
14: List Jobs stored for a given Volume name
15: List Volumes Bareos thinks are in changer
16: List Volumes likely to need replacement from age or errors
17: List Volumes Bareos thinks are eligible for the changer
18: List Volumes by Volume:
19: List Volumes by Jobs:
20: List Volumes for a jobname:
21: List Jobs of a client that contain a file (regardless of the directory, with wildcards)
22: List the top largest files for a JobId
```

## API-Modi

Unter `.api 1` schickt der Director die Liste als Auswahlliste für Programme: `Available queries:`, danach die Einträge ohne Nummern als einzelne Meldungen, eingerahmt von den Signalen für Listenanfang und -ende, und am Ende das Signal für eine Auswahleingabe (Paritätsfälle `c011-query-api1-cancel`, `c028-query-1`). Diese Auswahleingabe zeigt oconsole nicht als Prompt an; es wartet weiter, bis man mit CTRL-C abbricht. Beispielausgabe in oconsole daher nicht verfügbar. Unter `.api 2` startet kein Dialog: Das Ergebnis ist sofort leer (`"result": {}`).

## Beispiele

### Gesicherte Dateien und Bytes je Job

Abfrage 10 braucht keine weiteren Angaben (Paritätsfall `ds-query-select`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> query
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Available queries:
 1: List up to 20 places where a File is saved regardless of the directory and Client
 2: List where the most recent copies of a file with path of a Client are saved
[…]
22: List the top largest files for a JobId
Choose a query (1-22):
> 10
+------+--------+---------+------------------+
| jobs | files  | gb      | job              |
+------+--------+---------+------------------+
|   57 |    938 |   0.000 | BackupCatalog    |
|   28 | 72,940 |  21.488 | benchmark-small  |
|   76 |    865 |   0.767 | backup-bareos-fd |
+------+--------+---------+------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Letzte Vollsicherungen eines Clients

Abfrage 3 fragt nach dem Client (Paritätsfall `c011-query-client`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> query
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Available queries:
[…]
Choose a query (1-22):
> 3
Enter Client name:
> bareos-fd
+-------+-----------+---------------------+----------+--------+
| jobid | client    | starttime           | jobfiles | gb     |
+-------+-----------+---------------------+----------+--------+
|   629 | bareos-fd | 2026-07-10 21:10:04 |       67 |  0.000 |
|   627 | bareos-fd | 2026-07-09 21:10:04 |       67 |  0.000 |
[…]
|   506 | bareos-fd | 2026-03-30 22:53:21 |    2,605 |  0.767 |
+-------+-----------+---------------------+----------+--------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eigenschaften eines Volumes

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> query
[…]
Choose a query (1-22):
> 6
Enter Volume name:
> Full-47321
+------+----------------+------------------+-----------+---------+--------------+----------------+------------+-------------+
| slot | maxvolbytes    | volcapacitybytes | volstatus | recycle | volretention | voluseduration | maxvoljobs | maxvolfiles |
+------+----------------+------------------+-----------+---------+--------------+----------------+------------+-------------+
|    0 | 53,687,091,200 |                0 | Append    |       1 |   31,536,000 |              0 |          0 |           0 |
+------+----------------+------------------+-----------+---------+--------------+----------------+------------+-------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Falsche Nummer und Abbruch

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> query
[…]
Choose a query (1-22):
> 999
Please enter a number between 1 and 22
Choose a query (1-22):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Parameter überspringen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> query
[…]
Choose a query (1-22):
> 3
Enter Client name:
> .
No results to list.
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> query
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Please enter a number between 1 and <n>` | Die Nummer liegt außerhalb der Liste. Erneut eingeben. |
| `Selection aborted, nothing done.` | Die Auswahl wurde mit `.` abgebrochen. |
| `No results to list.` | Die Abfrage hat keine Zeilen gefunden, etwa wegen eines falschen oder übersprungenen Werts. |
| `Could not open <datei>: ERR=<grund>` | Die `QueryFile` des Directors fehlt oder ist nicht lesbar. Beispielausgabe nicht verfügbar. |
| `Too many prompts in query, max is 9.` | Eine eigene Abfrage in der `QueryFile` hat mehr als neun Fragen. Beispielausgabe nicht verfügbar. |

## Siehe auch

- [sqlquery](sqlquery.md) — eigenes SQL eingeben
- [list](list.md) — Katalogeinträge ohne SQL auflisten
- [use](use.md) — Katalog wählen
