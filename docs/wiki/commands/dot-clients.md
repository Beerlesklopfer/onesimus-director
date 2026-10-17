# .clients — Namen der Clients auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L231) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all client resources

**Originalsyntax:**

```text
.clients [enabled | disabled]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 20 von 20 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (20):** `c025-clients-limited`, `c025-clients-limited-api2`, `c025-clients-disabled-limited`, `c025-clients-disabled-limited-api2`, `c025-clients-operator`, `c025-clients-operator-api2`, `c025-clients-disabled-operator`, `c025-clients-disabled-operator-api2`, `c025-clients-bvfs`, `c025-clients-bvfs-api2`, `c025-clients-disabled-bvfs`, `c025-clients-disabled-bvfs-api2`, `c025-clients-acl`, `c025-clients-acl-api2`, `c025-clients-disabled-acl`, `c025-clients-disabled-acl-api2`, `c028-dot-clients`, `dot_clients`, `dot_clients-limited`, `dot_clients-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:73](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.clients` gibt die Namen aller Client-Ressourcen der Director-Konfiguration aus, einen pro Zeile, ohne Kopfzeile. Der Befehl ist für Programme gedacht: bconsole und oconsole füllen damit die Vervollständigung für `client=`, die WebUI ihre Auswahllisten. Er liest nur die Konfiguration, nicht den Katalog, und verändert nichts.

Die Liste enthält nur Clients, die die Konsole laut `Client ACL` sehen darf. Die Reihenfolge ist die der Konfiguration.

## Syntax

```text
.clients [ enabled | disabled ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `enabled` | | nur aktivierte Clients |
| `disabled` | | nur deaktivierte Clients |

Die Schlüsselwörter sind unabhängig von Groß- und Kleinschreibung; ein angehängter Wert wird ignoriert (`disabled=yes` wirkt wie `disabled`). Werden `enabled` und `disabled` zusammen angegeben, ist die Liste leer. Andere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.clients`, ein Array von Objekten mit `name` und `enabled`.

## Beispiele

### Alle Clients

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .clients
parity-ndmp-fd
bareos-fd
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur deaktivierte Clients

Im Testsystem ist kein Client deaktiviert; die Antwort ist leer.

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .clients disabled
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .clients
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "clients": [
      {
        "name": "parity-ndmp-fd",
        "enabled": true
      },
      {
        "name": "bareos-fd",
        "enabled": true
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit eingeschränkter Konsole

Die Konsole `parity-limited` darf nur den Client `bareos-fd` sehen.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .clients
bareos-fd
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.clients: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.clients` nicht. Der Befehl muss in der Konsolen- oder Profilkonfiguration freigegeben werden. |

## Siehe auch

- [.defaults](dot-defaults.md) — Vorgabewerte eines Clients
- [.jobs](dot-jobs.md) — Namen der Jobs
- [status](status.md) — Status eines Clients abfragen
- [enable](enable.md) / [disable](disable.md) — Client aktivieren oder deaktivieren
