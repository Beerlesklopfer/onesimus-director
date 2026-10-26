# .messages — wartende Meldungen abholen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L254) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Display pending messages

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 8 von 8 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-limited, parity-operator.
- Jobmeldungen selbst (Texte aus Jobs) sind Gegenstand von C-051, nicht dieses Befehls.

**Paritätsfälle (8):** `c036-dot-messages-twice`, `c036-autodisplay-on-messages`, `c036-gui-on-messages`, `c036-messages-api1`, `c036-messages-api2`, `c036-messages-limited`, `c036-messages-operator`, `dot-messages`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:360](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.messages` gibt die Meldungen aus, die seit dem letzten Abholen für die Konsole aufgelaufen sind (Jobberichte, Warnungen der Daemons), und leert damit den Speicher. Anders als [messages](messages.md) antwortet der Befehl ohne Text, wenn nichts wartet. Deshalb eignet er sich für Programme, die regelmäßig nachsehen: bconsole und die WebUI fragen damit zwischendurch nach neuen Meldungen. Der Befehl wirkt unabhängig von `autodisplay` und `gui`.

## Syntax

```text
.messages
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Liegen Meldungen vor, werden sie wie bei [messages](messages.md) ausgegeben. Liegt nichts vor, ist die Antwort in jedem Modus leer; unter `.api 2` kommt `{"jsonrpc": "2.0", "id": null, "result": {}}`.

## Beispiele

### Meldungen liegen vor

Beispielausgabe nicht verfügbar: Wartende Meldungen entstehen nur durch echte Jobs, die in Paritätsläufen nicht gestartet werden.

### Nichts wartet

`messages` meldet `You have no messages.`, `.messages` antwortet leer.

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> messages
You have no messages.
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .messages
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> whoami
parity-operator
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .messages
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Berechtigung

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> messages
messages: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .messages
.messages: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay on
autodisplay: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.messages: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.messages` nicht. |

## Siehe auch

- [messages](messages.md) — wartende Meldungen anzeigen, mit Hinweis wenn keine vorliegen
- [autodisplay](autodisplay.md) — Meldungen automatisch anzeigen
- [.msgs](dot-msgs.md) — Namen der Messages-Ressourcen
