# .quit — Verbindung beenden

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L265) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Close connection

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-limited.

**Paritätsfälle (3):** `c004-dot-quit-json`, `c004-dot-quit-limited`, `dot-quit`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/basic.rs:62](../../../crates/onesimus-dird/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.quit` beendet die Konsolensitzung: Der Director schließt die Verbindung, ohne einen Text zu senden. Es ist die Punktbefehl-Form von [quit](quit.md) und [exit](exit.md) für Programme. Der Befehl verändert nichts; laufende Jobs laufen weiter.

## Syntax

```text
.quit
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` schließt der Director die Verbindung ohne Antwort. Unter `.api 1` sendet er vorher die Signale `CMD_BEGIN` und `CMD_OK`, unter `.api 2` zusätzlich das Objekt `{"jsonrpc": "2.0", "id": null, "result": {}}`. Ein `MAIN_PROMPT` folgt in keinem Modus. oconsole beendet sich danach; die Shell zeigt wieder ihren eigenen Prompt.

## Beispiele

### Sitzung beenden

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .quit
```

### Im JSON-Modus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .quit
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
```

### Ohne Berechtigung

Fehlt `.quit` in der `Command ACL`, bleibt die Verbindung offen.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .quit
.quit: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.quit: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.quit` nicht. `quit` oder `exit` benutzen oder die Verbindung clientseitig schließen. |

## Siehe auch

- [quit](quit.md) — Sitzung beenden
- [exit](exit.md) — Sitzung beenden
- [.](dot-noop.md) — Leerbefehl
