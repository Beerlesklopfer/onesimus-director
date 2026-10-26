# .pools — Namen der Pools auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L261) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all pool resources

**Originalsyntax:**

```text
.pools type=<pooltype>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 21 von 21 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.
- `type=` vergleicht exakt statt als Präfix ohne Groß-/Kleinschreibung (C-044); die Fälle prüfen nur vollständige Typnamen.

**Offene Logeinträge:** `C-044` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (21):** `c025-pools-limited`, `c025-pools-limited-api2`, `c025-pools-type-Backup-limited`, `c025-pools-type-Backup-limited-api2`, `c025-pools-operator`, `c025-pools-operator-api2`, `c025-pools-type-Backup-operator`, `c025-pools-type-Backup-operator-api2`, `c025-pools-bvfs`, `c025-pools-bvfs-api2`, `c025-pools-type-Backup-bvfs`, `c025-pools-type-Backup-bvfs-api2`, `c025-pools-acl`, `c025-pools-acl-api2`, `c025-pools-type-Backup-acl`, `c025-pools-type-Backup-acl-api2`, `c028-dot-pools`, `dot_pools`, `dot_pools-type-Backup`, `dot_pools-limited`, `dot_pools-api2`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:100](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.pools` gibt die Namen aller Pool-Ressourcen der Director-Konfiguration aus, einen pro Zeile. Programme nutzen ihn für die Vervollständigung von `pool=` und für Auswahllisten. Der Befehl liest nur die Konfiguration, nicht die Pool-Tabelle des Katalogs, und verändert nichts.

Es erscheinen nur Pools, die die Konsole laut `Pool ACL` sehen darf.

## Syntax

```text
.pools [ type=<pooltyp> ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `type=<pooltyp>` | `Backup`, `Scratch`, `Copy`, `Archive`, … | nur Pools dieses Typs |

Der Vergleich bei `type=` ist unabhängig von Groß- und Kleinschreibung und prüft nur, ob der Pooltyp mit dem angegebenen Text **beginnt**: `type=b`, `type=back` und `type=BACKUP` liefern dasselbe, `type=Backupx` nichts. Ohne Wert (`type`, `type=`) wird nicht gefiltert; es zählt nur das erste `type`-Wort.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.pools`, ein Array von Objekten mit `name` (ohne `enabled`).

## Beispiele

### Alle Pools

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .pools
Scratch
Incremental
Full
Differential
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur Backup-Pools

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .pools type=Backup
Incremental
Full
Differential
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Typ als Anfang des Namens

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .pools type=sCr
Scratch
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .pools type=Backup
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "pools": [
      {
        "name": "Incremental"
      },
      {
        "name": "Full"
      },
      {
        "name": "Differential"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit eingeschränkter Konsole

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .pools
Scratch
Full
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.pools: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.pools` nicht. |

## Siehe auch

- [.defaults](dot-defaults.md) — Vorgabewerte eines Pools
- [.media](dot-media.md) — Volumes im Katalog
- [list](list.md) — `list pools` zeigt die Pools des Katalogs
