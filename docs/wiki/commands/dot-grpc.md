# .grpc — gRPC-Modus der Sitzung (Onesimus)

## Zweck

`.grpc` schaltet eine Konsolensitzung in den maschinenlesbaren Modus, den gRPC-Oberflächen erwarten, und wieder zurück. Ohne Argument meldet der Befehl den Zustand der Sitzung und ob der gRPC-Dienst des Directors eingeschaltet ist (`Enable gRPC = yes` in der Director-Ressource). Ein eigener gRPC-Datenstrom wird über die Konsolenverbindung nicht aufgebaut; der gRPC-Dienst läuft auf seinem eigenen Port. Der Befehl ändert nur den Zustand der eigenen Sitzung.

**Onesimus-Erweiterung, nicht im Original (C-074, Entscheidung in [#4](https://github.com/Beerlesklopfer/onesimus-director/issues/4): bleibt als dokumentierte Erweiterung).** Das Bareos-Original kennt den Befehl nicht und antwortet `.grpc: is an invalid command.`.

## Syntax

```text
.grpc
.grpc on [format=<json|text>] [compact=<yes|no>]
.grpc off
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| ohne | – | Zustand der Sitzung und Verfügbarkeit des gRPC-Dienstes melden. |
| `on` | – | gRPC-Modus einschalten. Der vorherige API-Modus und die Compact-Einstellung werden gemerkt. |
| `off` | – | gRPC-Modus ausschalten und zum API-Modus von vor `.grpc on` zurückkehren (z. B. Text nach `.api 0`, API 1 nach `.api 1`). |
| `format=<…>` | `json` (Standard), `text` | `json`: Antworten wie unter `.api 2`; `text`: Antworten wie unter `.api 1`. `proto` gibt es nur am gRPC-Port und wird abgelehnt. |
| `compact=<…>` | `yes`, `no` (Standard) | Nur bei `format=json`: JSON ohne Einrückung wie `.api json compact=yes`. |

`format=` oder `compact=` ohne `on` schalten ebenfalls ein bzw. ändern die Einstellungen eines eingeschalteten gRPC-Modus. Wie bei `.api` gilt der neue Modus schon für die Antwort auf den Befehl selbst. Ungültige Argumente ändern nichts.

## API-Modi

Unter `.api 0` und `.api 1` antwortet der Befehl mit Textzeilen. Unter `.api 2` (und im gRPC-Modus mit `format=json`) steht das Ergebnis unter `grpc`: `mode` (`on`/`off`), bei eingeschaltetem Modus `format` und `compact`, `apimode` (0, 1 oder 2), `available` und `text` mit der Textantwort.

## Beispiele

Die Beispiele stammen aus einem Onesimus-Director mit der Paritätskonfiguration; dort ist gRPC nicht eingeschaltet.

### Zustand abfragen

Paritätsfall `c074-dot-grpc`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc
gRPC mode: off
gRPC service: not configured (set 'Enable gRPC = yes' in Director resource)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.grpc: is an invalid command.` (C-074, Onesimus-Erweiterung; Paritätsfälle `c074-dot-grpc*` als erwartete Abweichung).

### Einschalten mit kompaktem JSON und zurück

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc on compact=yes
{"jsonrpc":"2.0","id":null,"result":{"grpc":{"mode":"on","format":"json","compact":true,"apimode":2,"available":false,"text":"gRPC mode: on, format=json, compact=yes\n"}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc
{"jsonrpc":"2.0","id":null,"result":{"grpc":{"mode":"on","format":"json","compact":true,"apimode":2,"available":false,"text":"gRPC mode: on, format=json, compact=yes\ngRPC service: not configured (set 'Enable gRPC = yes' in Director resource)\n"}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc off
gRPC mode: off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Textformat aus API 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc on format=text
gRPC mode: on, format=text, compact=no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
bareos-dir Version: 25.1.2+onesimus.0.2.0 (17 October 2026) Debian GNU/Linux 13 (trixie) debian Debian GNU/Linux 13 (trixie) 
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc off
gRPC mode: off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 2`

Paritätsfall `c074-dot-grpc-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "grpc": {
      "mode": "off",
      "apimode": 2,
      "available": false,
      "text": "gRPC mode: off\ngRPC service: not configured (set 'Enable gRPC = yes' in Director resource)\n"
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.grpc: is an invalid command.` | `.grpc` ist in der `Command ACL` nicht erlaubt (im Original: den Befehl gibt es nicht). |
| `.grpc: format=proto is only available on the gRPC port, not on a console connection.` | Protobuf gibt es nur am gRPC-Port. |
| `.grpc: format must be json or text.` / `.grpc: compact must be yes or no.` | Ungültiger Wert. |
| `.grpc: specify either on or off.` | `on` und `off` zugleich. |
| `.grpc: format= and compact= only apply to .grpc on.` | `off` mit `format=`/`compact=`. |
| `.grpc: unknown argument "…".` | Unbekanntes Argument. |

## Siehe auch

- [.api](dot-api.md) — API-Modus der Sitzung (Text, JSON) umschalten
- [version](version.md) — Version des Directors anzeigen
