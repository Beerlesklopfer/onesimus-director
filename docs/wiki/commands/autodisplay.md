# autodisplay — Nachrichten automatisch anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L311) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Autodisplay console messages

**Originalsyntax:**

```text
autodisplay on | off
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 16 von 16 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-limited.

**Paritätsfälle (16):** `c005-autodisplay-noarg`, `c005-autodisplay-foo`, `c005-autodisplay-upper`, `c005-autodisplay-later-kw`, `c005-autodisplay-kv`, `c005-autodisplay-off-on`, `c005-autodisplay-foo-api1`, `c005-autodisplay-off-api1`, `c005-autodisplay-foo-api2`, `c005-autodisplay-off-api2`, `c036-autodisplay-on-messages`, `c036-autodisplay-on-api1`, `c036-getcmd-dot-messages-autodisplay`, `c036-messages-limited`, `autodisplay-off`, `autodisplay-on`.

**Lokaler Handler:** [crates/odir/src/commands/extra.rs:50](../../../crates/odir/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

Der Director sammelt Nachrichten für die Konsole, etwa Jobberichte oder Aufforderungen zum Einlegen eines Volumes. Normalerweise (`autodisplay off`) weist er nach einem Befehl nur mit `You have messages.` darauf hin, und man holt sie mit [`messages`](messages.md) ab. Mit `autodisplay on` gibt der Director wartende Nachrichten nach jedem Befehl selbst aus. Die Einstellung gilt nur für die eigene Sitzung. Voraussetzung ist, dass die Konsole den Befehl `messages` benutzen darf.

## Syntax

```text
autodisplay on | off
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `on` | — | Wartende Nachrichten nach jedem Befehl automatisch ausgeben |
| `off` | — | Nur den Hinweis `You have messages.` zeigen (Standard) |

Groß- und Kleinschreibung ist egal (`autodisplay ON`). Der Director nimmt das erste Wort hinter `autodisplay`, das `on` oder `off` heißt (`autodisplay foo off` schaltet aus); ein Wert wie `on=1` zählt als `on`.

## API-Modi

Erfolgreich kommt in keinem Modus Text zurück; unter `.api 2` ist es ein leeres Ergebnis (`"result": {}`). Fehlt `on`/`off`, kommt unter `.api 0` und `.api 1` die Zeile `ON or OFF keyword missing.`, unter `.api 2` ein JSON-Fehler mit dieser Meldung.

## Beispiele

### Automatische Anzeige einschalten

Der Befehl antwortet ohne Text. Liegen keine Nachrichten vor, meldet `messages` danach wie gewohnt `You have no messages.` (Paritätsfall `c036-autodisplay-on-messages`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
You have no messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Eine automatisch ausgegebene Nachricht selbst: Beispielausgabe nicht verfügbar, weil dafür ein Job laufen müsste.

### Automatische Anzeige ausschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Schlüsselwort fehlt

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay foo
ON or OFF keyword missing.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Schlüsselwort fehlt, JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay
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
          "ON or OFF keyword missing.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Berechtigung

Die Konsole `parity-limited` darf weder `messages` noch `autodisplay` benutzen (Paritätsfall `c036-messages-limited`).

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay on
autodisplay: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `ON or OFF keyword missing.` | Hinter `autodisplay` steht weder `on` noch `off`. Befehl mit `on` oder `off` wiederholen. |
| `autodisplay: is an invalid command.` | Die `CommandACL` der Konsole erlaubt den Befehl nicht. |

## Siehe auch

- [messages](messages.md) — wartende Nachrichten abholen
- [.messages](dot-messages.md) — Nachrichten im Maschinenformat
- [gui](gui.md) — GUI-Modus, unterdrückt den Hinweis `You have messages.`
