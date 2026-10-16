# truncate -- Truncate a Purged Volume

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L487) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Truncate purged volumes

**Originalsyntax:**

```text
truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:1314](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Truncate one or more purged volumes to reclaim disk space. Only works on volumes with `Purged` status.

## Syntax

```
truncate [volume=<name>] [storage=<name>] [pool=<name>] [yes]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `volume=<name>` | Truncate this specific volume |
| `storage=<name>` | Storage daemon where the volume resides |
| `pool=<name>` | Truncate all purged volumes in this pool |
| `yes` | Skip confirmation prompt |

## Examples

### Truncate a specific volume
```
* truncate volume=Full-0001 storage=File
Sending truncate command for Volume "Full-0001" on Storage "File"...
Volume "Full-0001" truncated.
```

### Truncate all purged volumes in a pool
```
* truncate pool=Full storage=File
Truncating all purged volumes in pool "Full" on storage "File"...
3 volumes truncated.
```

### Truncate with automatic storage selection
```
* truncate volume=Full-0001
Sending truncate command for Volume "Full-0001" on Storage "File"...
Volume "Full-0001" truncated.
```

## Behavior

- The Director sends a truncate command to the Storage Daemon.
- The SD truncates the volume file to its label size (typically ~237 bytes), reclaiming all data space.
- Only volumes in `Purged` status can be truncated -- volumes with active data are rejected.
- After truncation, the volume status changes to `Recycle` and the volume can accept new data.
- For tape devices, truncation is not applicable.
- The `yes` flag suppresses the confirmation prompt.
- This is an audited command.

## Related Commands

- [purge](purge.md) -- Purge a volume before truncating
- [list](list.md) -- List volumes to check status (`list volumes`)
- [label](label.md) -- Label a volume after truncation
