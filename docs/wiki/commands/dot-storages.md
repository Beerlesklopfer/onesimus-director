# .storages — Namen der Storages auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L277) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all storage resources

**Originalsyntax:**

```text
.storages [enabled | disabled]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 18 von 18 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (18):** `c025-storages-limited`, `c025-storages-limited-api2`, `c025-storages-enabled-limited`, `c025-storages-enabled-limited-api2`, `c025-storages-operator`, `c025-storages-operator-api2`, `c025-storages-enabled-operator`, `c025-storages-enabled-operator-api2`, `c025-storages-bvfs`, `c025-storages-bvfs-api2`, `c025-storages-enabled-bvfs`, `c025-storages-enabled-bvfs-api2`, `c025-storages-acl`, `c025-storages-acl-api2`, `c025-storages-enabled-acl`, `c025-storages-enabled-acl-api2`, `dot_storages`, `dot_storages-limited`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:110](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.storages` gibt die Namen aller Storage-Ressourcen der Director-Konfiguration aus, einen pro Zeile. Programme nutzen ihn für die Vervollständigung von `storage=` und für Auswahllisten. Der Befehl liest nur die Konfiguration, nimmt keinen Kontakt zum Storage Daemon auf und verändert nichts.

Es erscheinen nur Storages, die die Konsole laut `Storage ACL` sehen darf. Die Kurzform [`.storage`](dot-storage.md) führt denselben Befehl aus.

## Syntax

```text
.storages [ enabled | disabled ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `enabled` | | nur aktivierte Storages |
| `disabled` | | nur deaktivierte Storages |

Die Schlüsselwörter sind unabhängig von Groß- und Kleinschreibung; ein Wert dahinter wird ignoriert (`enabled=x` wirkt wie `enabled`). Beide zusammen ergeben eine leere Liste.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.storages`, ein Array von Objekten mit `name` und `enabled`.

## Beispiele

### Alle Storages

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .storages
ParityNdmpNative
ParityNdmp
File
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur deaktivierte Storages

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .storages disabled
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .storages
ParityNdmpNative
ParityNdmp
File
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .storages enabled
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "storages": [
      {
        "name": "ParityNdmpNative",
        "enabled": true
      },
      {
        "name": "ParityNdmp",
        "enabled": true
      },
      {
        "name": "File",
        "enabled": true
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit eingeschränkter Konsole

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .storages
File
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.storages: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.storages` nicht. |

## Siehe auch

- [.storage](dot-storage.md) — Kurzform desselben Befehls
- [.defaults](dot-defaults.md) — Adresse, Port, Medientyp und Device eines Storages
- [.mediatypes](dot-mediatypes.md) — Medientypen im Katalog
- [status](status.md) — Status eines Storage Daemons
