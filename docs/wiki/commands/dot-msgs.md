# .msgs — Namen der Messages-Ressourcen auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L259) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all message resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_msgs-api2`, `c025-msgs-limited`, `c025-msgs-limited-api2`, `c025-msgs-operator`, `c025-msgs-operator-api2`, `c025-msgs-bvfs`, `c025-msgs-bvfs-api2`, `c025-msgs-acl`, `c025-msgs-acl-api2`, `dot_msgs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:369](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.msgs` gibt die Namen aller Messages-Ressourcen der Director-Konfiguration aus, einen pro Zeile. Eine Messages-Ressource legt fest, wohin Job- und Daemon-Meldungen gehen (Konsole, Datei, Mail, Katalog). Konfigurationswerkzeuge brauchen die Liste für die Auswahl von `messages=`. Der Befehl liest nur die Konfiguration und verändert nichts. Nicht zu verwechseln mit [.messages](dot-messages.md), das wartende Meldungen ausgibt.

## Syntax

```text
.msgs
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.messages`, ein Array von Objekten — anders als bei den übrigen Listen mit dem Feld `text` statt `name`.

## Beispiele

### Alle Messages-Ressourcen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .msgs
Standard
Daemon
c019j
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .msgs
Standard
Daemon
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .msgs
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "messages": [
      {
        "text": "Standard"
      },
      {
        "text": "Daemon"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.msgs: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.msgs` nicht. |

## Siehe auch

- [.messages](dot-messages.md) — wartende Meldungen abholen
- [messages](messages.md) — wartende Meldungen anzeigen
- [show](show.md) — `show messages` zeigt die Ressourcen vollständig
