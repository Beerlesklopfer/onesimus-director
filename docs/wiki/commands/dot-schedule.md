# .schedule — Namen der Zeitpläne auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L268) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all schedule resources

**Originalsyntax:**

```text
.schedule [enabled | disabled]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 18 von 18 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (18):** `c003-dot_schedule-api2`, `c025-schedule-limited`, `c025-schedule-limited-api2`, `c025-schedule-enabled-limited`, `c025-schedule-enabled-limited-api2`, `c025-schedule-operator`, `c025-schedule-operator-api2`, `c025-schedule-enabled-operator`, `c025-schedule-enabled-operator-api2`, `c025-schedule-bvfs`, `c025-schedule-bvfs-api2`, `c025-schedule-enabled-bvfs`, `c025-schedule-enabled-bvfs-api2`, `c025-schedule-acl`, `c025-schedule-acl-api2`, `c025-schedule-enabled-acl`, `c025-schedule-enabled-acl-api2`, `dot_schedule`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:130](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.schedule` gibt die Namen aller Schedule-Ressourcen der Director-Konfiguration aus, einen pro Zeile. Programme nutzen ihn für Auswahllisten, etwa beim Aktivieren oder Deaktivieren eines Zeitplans. Der Befehl liest nur die Konfiguration und verändert nichts.

Es erscheinen nur Zeitpläne, die die Konsole laut `Schedule ACL` sehen darf.

## Syntax

```text
.schedule [ enabled | disabled ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `enabled` | | nur aktivierte Zeitpläne |
| `disabled` | | nur deaktivierte Zeitpläne |

Die Schlüsselwörter sind unabhängig von Groß- und Kleinschreibung; ein Wert dahinter wird ignoriert (`enabled=1` wirkt wie `enabled`). Beide zusammen ergeben eine leere Liste.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.schedules`, ein Array von Objekten mit `name` und `enabled`.

## Beispiele

### Alle Zeitpläne

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .schedule
WeeklyCycleAfterBackup
WeeklyCycle
ParityPreview
c019h
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur deaktivierte Zeitpläne

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .schedule disabled
WeeklyCycleAfterBackup
WeeklyCycle
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .schedule
WeeklyCycleAfterBackup
WeeklyCycle
ParityPreview
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .schedule enabled
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "schedules": [
      {
        "name": "ParityPreview",
        "enabled": true
      },
      {
        "name": "c019h",
        "enabled": true
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit eingeschränkter Konsole

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .schedule
WeeklyCycle
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.schedule: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.schedule` nicht. |

## Siehe auch

- [status](status.md) — `status scheduler` zeigt, wann welcher Job geplant ist
- [enable](enable.md) / [disable](disable.md) — Zeitplan aktivieren oder deaktivieren
- [show](show.md) — `show schedule=<name>` zeigt die Ausführungszeiten
