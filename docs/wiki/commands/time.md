# time — Uhrzeit des Directors anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L484) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print current time

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 1 von 1 Fällen byte-identisch; Modi: Text.

**Paritätsfälle (1):** `time`.

**Lokaler Handler:** [crates/odir/src/commands/basic.rs:45](../../../crates/odir/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`time` zeigt Datum und Uhrzeit des Rechners, auf dem der Director läuft, in dessen lokaler Zeitzone. Der Befehl liest nur. Er hilft, wenn Zeitpläne (`Schedule`) oder Zeitangaben in Joblisten nicht zur eigenen Uhr passen, etwa bei abweichender Zeitzone.

## Syntax

```text
time
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `time` werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt eine Zeile im Format `<Wochentag> <TT>-<Monat>-<JJJJ> <hh>:<mm>:<ss>`, zum Beispiel `Fri 16-Oct-2026 16:53:39`. Wochentag und Monat sind englisch abgekürzt.

Unter `.api 2` steht die Zeit im Objekt `result.time`: `full` enthält dieselbe Zeile wie im Textmodus, `year`, `month`, `day`, `hour`, `minute` und `second` die einzelnen Teile als zweistellige Zeichenketten (Jahr vierstellig).

## Beispiele

### Uhrzeit anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> time
Fri 16-Oct-2026 16:53:39
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Uhrzeit als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> time
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "time": {
      "full": "Fri 16-Oct-2026 16:51:48",
      "year": "2026",
      "month": "10",
      "day": "16",
      "hour": "16",
      "minute": "51",
      "second": "48"
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Uhrzeit als kompaktes JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> time
{"jsonrpc":"2.0","id":null,"result":{"time":{"full":"Fri 16-Oct-2026 23:09:35","year":"2026","month":"10","day":"16","hour":"23","minute":"09","second":"35"}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`time` gibt keine eigenen Fehlermeldungen aus.

## Siehe auch

- [version](version.md) — Version des Directors
- [status](status.md) — Status des Directors mit Startzeit und geplanten Jobs
- [var](var.md) — Variablen wie `${Year}` oder `${Hour}` auflösen
