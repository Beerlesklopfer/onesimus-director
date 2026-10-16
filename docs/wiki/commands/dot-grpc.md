# .grpc — gRPC-Modus der Sitzung (Onesimus)

## Zweck

`.grpc` ist eine Onesimus-Erweiterung für gRPC-Oberflächen; das Original kennt den Befehl nicht und antwortet `.grpc: is an invalid command.`. Ohne Argument meldet der Befehl, ob gRPC im Director eingeschaltet ist (`Enable gRPC = yes` in der Director-Ressource). `.grpc on` markiert die Sitzung als gRPC-Sitzung und schaltet sie dabei auf JSON-Antworten (wie `.api 2`) um. Ein eigener gRPC-Datenstrom wird über diese Konsolenverbindung derzeit nicht aufgebaut. Der Befehl ändert nur den Zustand der eigenen Sitzung.

## Syntax

```text
.grpc
.grpc on [format=<proto|json|text>] [compact=<yes|no>]
.grpc off
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| ohne | – | gRPC-Verfügbarkeit melden. |
| `on` | – | gRPC-Modus der Sitzung einschalten; alle folgenden Antworten kommen als JSON. |
| `off` | – | gRPC-Modus der Sitzung ausschalten. Der JSON-Modus bleibt bestehen; zurück zum Text geht es mit `.api 0`. |
| `format=<…>` | `proto`, `json`, `text` | Wird angenommen, wirkt sich derzeit nicht auf die Antworten aus. |
| `compact=<…>` | `yes`, `no` | Wird angenommen, wirkt sich derzeit nicht auf die Antworten aus. |

## API-Modi

Unter `.api 0` und `.api 1` antworten `.grpc` und `.grpc off` mit einer Textzeile. Unter `.api 2` und nach `.grpc on` ist die Antwort ein leeres Ergebnis `"result": {}`; der Text geht verloren.

## Beispiele

### Verfügbarkeit abfragen

In der Testkonfiguration ist gRPC nicht eingeschaltet (Paritätsfall `dcfg-grpc-grpc`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc
gRPC: not configured (set 'Enable gRPC = yes' in Director resource)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original kennt `.grpc` nicht und antwortet `.grpc: is an invalid command.` (Onesimus-Erweiterung, kein Kompatibilitätseintrag).

Beispielausgabe nicht verfügbar: für `Enable gRPC = yes` gibt es keinen Paritätsfall.

### gRPC-Modus einschalten

Schon die Antwort auf `.grpc on` kommt als JSON, ebenso jeder folgende Befehl (Paritätsfall `dcfg-grpc-on-then`). Mit `format=proto compact=yes` ist die Antwort dieselbe (`dcfg-grpc-grpc_on_format-proto_compact-yes`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc on
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
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
      "bdate": "16 October 2026",
      "operatingsystem": "Debian GNU/Linux 13 (trixie)",
      "distname": "debian",
      "distversion": "Debian GNU/Linux 13 (trixie)",
      "customversionid": ""
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### gRPC-Modus ausschalten

In einer Textsitzung antwortet `.grpc off` mit einer Zeile (Paritätsfall `dcfg-grpc-grpc_off`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc off
gRPC mode: off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Nach `.grpc on` bleibt die Sitzung auch nach `.grpc off` im JSON-Modus (Paritätsfall `dcfg2-grpc-on-off-version`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc on
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .grpc off
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": {
[…]
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`.grpc` gibt keine eigenen Fehlermeldungen aus. Unbekannte Argumente werden ignoriert.

## Siehe auch

- [.api](dot-api.md) — API-Modus der Sitzung (Text, JSON) umschalten
- [version](version.md) — Version des Directors anzeigen
