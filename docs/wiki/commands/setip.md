# setip — Client-Adresse aktualisieren

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L466) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets new client address -- if authorized

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text, Konsolen parity-limited.
- Verglichen ist nur die Ablehnung für Konsolen ohne gleichnamigen Client; das Setzen der Client-Adresse ist nur durch Einheitentests belegt.

**Paritätsfälle (2):** `c012-setip-limited`, `setip`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:231](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`setip` ist für Clients mit wechselnder IP-Adresse gedacht (etwa Laptops). Ein Client meldet sich dazu über eine eigene Konsole beim Director an, deren Name dem Namen seiner Client-Ressource entspricht, und ruft `setip` auf. Der Director übernimmt dann die Absenderadresse dieser Konsolenverbindung als neue Adresse des Clients. Die Änderung gilt nur im laufenden Director; die Konfigurationsdateien bleiben unverändert.

## Syntax

```text
setip
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `setip` werden ignoriert.

## API-Modi

Unter `.api 1` kommt derselbe Meldungstext wie im Standardmodus. Unter `.api 2` steht die Fehlermeldung in `error.data.messages.error`.

## Beispiele

### Adresse eines Clients setzen

Beispielausgabe nicht verfügbar: die Testumgebung hat keine Konsole, deren Name einer Client-Ressource entspricht. Im Erfolgsfall meldet der Director `Client "<name>" address set to <adresse>`.

### Aufruf von der Standardkonsole

Die Standardkonsole gehört zu keinem Client.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setip
No corresponding client found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne gleichnamigen Client

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> setip
Client "parity-operator" not found.
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setip
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
          "No corresponding client found.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eingeschränkte Konsole

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> setip
setip: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `No corresponding client found.` | Die Konsole ist keinem Client zugeordnet (zum Beispiel die Standardkonsole). `setip` muss über die Konsole des Clients aufgerufen werden. |
| `Client "<name>" not found.` | Es gibt keine Client-Ressource mit dem Namen der Konsole. Konsole und Client gleich benennen. |
| `setip: is an invalid command.` | Die Konsole darf `setip` nicht ausführen (Command ACL). |

## Siehe auch

- [status](status.md) — Status eines Clients abfragen und so die neue Adresse prüfen
- [show](show.md) — Client-Ressourcen anzeigen
