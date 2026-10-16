# .pools — List Pool Names

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

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:100](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one pool resource name per line from the Director configuration.

## Syntax

```
.pools [type=<pooltype>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List all pools |
| `type=<pooltype>` | Filter by pool type (e.g., Backup, Scratch) |

## Examples

### List all pools
```
* .pools
Default
Full
Incremental
Scratch
```

### Filter by type
```
* .pools type=Backup
Default
Full
Incremental
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- The `type=` filter is case-insensitive and matches the PoolType directive.
- Used by console programs to populate the `pool=` argument completion list.

## Related Commands

- [.defaults](dot-defaults.md) -- Show default values for a pool
- [.media](dot-media.md) -- List volume names
- [.volstatus](dot-volstatus.md) -- List volume status values
