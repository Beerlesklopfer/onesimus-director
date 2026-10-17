# .api — Ausgabemodus der Sitzung umschalten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L222) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Switch between different api modes

**Originalsyntax:**

```text
.api [ 0 | 1 | 2 | off | on | json ] [compact=<yes|no>]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 28 von 28 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (28):** `c018-api2`, `c018-api2-client`, `c020-api-noarg`, `c020-api-off`, `c020-api-on`, `c020-api-json`, `c020-api-upper`, `c020-api-invalid`, `c020-api-invalid-api2`, `c020-api-toomany`, `c020-api-compact-yes`, `c020-api-compact-no`, `c020-api-compact-upper`, `c020-api-compact-novalue`, `c020-api-1-compact`, `c020-compact-persists`, `c020-compact-reset`, `c028-api-toggle`, `c042-status-storage-api1`, `c042-status-storage-api2`, `c042-dot-status-storage-api2`, `c042-status-storage-slots-api1`, `c042-status-storage-slots-api2`, `c042-status-storage-api1-twice`, `dot-api-0`, `dot-api-1`, `dot-api-2`, `dot-api-json`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:414](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.api` schaltet um, in welcher Form der Director in dieser Konsolensitzung antwortet: als Text für Menschen (Modus 0), als knapper Text für Skripte (Modus 1) oder als JSON (Modus 2). Der Modus gilt bis zum nächsten `.api` oder bis zum Ende der Sitzung; andere Sitzungen sind nicht betroffen. Der Befehl verändert weder Katalog noch Konfiguration.

Wer ihn braucht: Programme, die den Director fernsteuern. Die Bareos-WebUI und die Python-Bibliothek `python-bareos` arbeiten mit `.api 2` (JSON), bconsole und oconsole nutzen für die Tab-Vervollständigung die Namenslisten der Punktbefehle. Am Terminal braucht man `.api` kaum; die Modi sind aber nützlich, um zu sehen, was ein Programm vom Director bekommt.

## Syntax

```text
.api
.api 0 | off
.api 1 | on
.api 2 | json [ compact=yes | compact=no ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| *(keins)* | | wie `.api 1` |
| `0`, `off` | | Textmodus (Standard nach dem Anmelden) |
| `1`, `on` | | Skriptmodus: Text ohne Tabellenrahmen und ohne Hinweiszeilen, jede Antwort mit Rahmensignalen |
| `2`, `json` | | JSON-Modus: jede Antwort ist ein JSON-RPC-2.0-Objekt |
| `compact=` | `yes` oder anderer Wert | nur als zweites Wort hinter `2`/`json`: `yes` liefert JSON in einer Zeile, jeder andere Wert schaltet auf eingerücktes JSON zurück. Ohne `compact=` bleibt die bisherige Einstellung erhalten. Hinter `0`/`1` wird `compact=` ignoriert. |

Groß- und Kleinschreibung spielt keine Rolle (`.api JSON`, `Compact=YES`). Ein unbekannter Modus (`.api 3`, `.api foo`) oder mehr als zwei Wörter (`.api 2 compact=yes extra`) lassen den Modus unverändert; der Director antwortet dann ohne Text.

## API-Modi

### Wie die drei Modi aussehen

- **Modus 0 (Text):** Tabellen mit Rahmen, Hinweise wie `Automatically selected Catalog: MyCatalog`, Fragen und Auswahllisten. So arbeitet man interaktiv.
- **Modus 1 (Skript):** derselbe Text, aber Tabellen ohne Rahmenlinien und Kopfzeile (bei `.sql` stehen die Zellen tabulatorgetrennt), keine Katalog-Hinweiszeilen. Namenslisten der Punktbefehle sind gleich wie in Modus 0.
- **Modus 2 (JSON):** ein Objekt `{"jsonrpc": "2.0", "id": null, "result": {…}}` pro Befehl. Schlägt ein Befehl fehl, steht statt `result` ein Objekt `error` mit `code: 1`, `message: "failed"` und unter `data.messages.error` den Fehlertexten. Mit `compact=yes` steht das Objekt in einer Zeile ohne Leerzeichen.

### Rahmensignale

Neben dem Text sendet der Director Steuersignale, die oconsole nicht anzeigt, die ein eigenes Programm aber auswerten muss, um das Ende einer Antwort zu erkennen:

| Modus | Signalfolge einer Antwort |
|---|---|
| 0 | Text, danach `EOD` (-1) |
| 1 und 2 | `CMD_BEGIN` (-16), Text, `CMD_OK` (-15) oder bei Fehlschlag `CMD_FAILED` (-14), zum Schluss `MAIN_PROMPT` (-18) |

Ein unbekannter Befehl unter Modus 1/2 liefert `ERROR_MSG` (-21) vor dem Text und dann `MAIN_PROMPT`, ohne `CMD_BEGIN`. Beim Umschalten selbst gilt der Modus, der beim Empfang aktiv war, für den Anfang der Antwort und der neue für ihr Ende: `.api 1` aus Modus 0 liefert `CMD_OK`, `MAIN_PROMPT`; `.api 0` aus Modus 1 liefert `CMD_BEGIN`, Text, `EOD`. Eine leere Zeile wird unter Modus 1/2 nur mit `MAIN_PROMPT` beantwortet, unter Modus 0 mit `EOD`. Wie der Leerbefehl `.` gerahmt wird, steht bei [`.`](dot-noop.md).

## Beispiele

### In den Skriptmodus wechseln

Ohne Argument und mit `1` oder `on` schaltet `.api` in Modus 1.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Zurück in den Textmodus

`list pools` zeigt im Skriptmodus nur die Zellen, nach `.api 0` wieder die Tabelle mit Rahmen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
      1 | Scratch      |       0 |       0 | Scratch  | *             |
      2 | Incremental  |       1 |     100 | Backup   | Incremental-  |
      3 | Full         |       3 |     100 | Backup   | Full-         |
      4 | Differential |       1 |     100 | Backup   | Differential- |
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 0
api: 0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
+--------+--------------+---------+---------+----------+---------------+
| poolid | name         | numvols | maxvols | pooltype | labelformat   |
+--------+--------------+---------+---------+----------+---------------+
|      1 | Scratch      |       0 |       0 | Scratch  | *             |
|      2 | Incremental  |       1 |     100 | Backup   | Incremental-  |
|      3 | Full         |       3 |     100 | Backup   | Full-         |
|      4 | Differential |       1 |     100 | Backup   | Differential- |
+--------+--------------+---------+---------+----------+---------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JSON-Modus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kompaktes JSON

`compact=yes` bleibt für alle folgenden Antworten aktiv, auch über einen Wechsel nach Modus 0 und zurück.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
{"jsonrpc":"2.0","id":null,"result":{"version":{"name":"bareos-dir","type":"bareos-director","version":"25.1.2+onesimus.0.2.0","bdate":"15 October 2026","operatingsystem":"Debian GNU/Linux 13 (trixie)","distname":"debian","distversion":"Debian GNU/Linux 13 (trixie)","customversionid":""}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 0
api: 0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
{"jsonrpc":"2.0","id":null,"result":{"version":{"name":"bareos-dir","type":"bareos-director","version":"25.1.2+onesimus.0.2.0","bdate":"15 October 2026","operatingsystem":"Debian GNU/Linux 13 (trixie)","distname":"debian","distversion":"Debian GNU/Linux 13 (trixie)","customversionid":""}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kompaktes JSON wieder abschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2 compact=no
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": {
      "name": "bareos-dir",
      "type": "bareos-director",
      "version": "25.1.2+onesimus.0.2.0",
      "bdate": "15 October 2026",
      "operatingsystem": "Debian GNU/Linux 13 (trixie)",
      "distname": "debian",
      "distversion": "Debian GNU/Linux 13 (trixie)",
      "customversionid": ""
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Modus

Im Textmodus kommt keine Antwort; der Modus bleibt, wie er war.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 3
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Unter `.api 2` meldet derselbe Fehler ein JSON-Fehlerobjekt ohne Text:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api foo
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {}
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`.api` gibt keine eigenen Fehlertexte aus. Ein ungültiger Aufruf ist im Textmodus an der leeren Antwort zu erkennen, im JSON-Modus am Objekt `error` mit `"messages": {}`.

## Siehe auch

- [.help](dot-help.md) — Befehle und Argumente in maschinenlesbarer Form
- [.](dot-noop.md) — Leerbefehl, z. B. als Lebenszeichen einer Skriptsitzung
- [gui](gui.md) — nicht-interaktiven Modus ein- und ausschalten
- [.grpc](dot-grpc.md) — gRPC-Modus der Sitzung
