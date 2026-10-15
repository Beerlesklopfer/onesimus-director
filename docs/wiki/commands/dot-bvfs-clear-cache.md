# .bvfs_clear_cache -- Clear Entire BVFS Cache

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L307) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Clear BVFS cache

**Originalsyntax:**

```text
.bvfs_clear_cache yes
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `stub`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler bestätigt die Cache-Operation lediglich per Text; keine BVFS-Katalogoperation wird ausgeführt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:485](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Drops all BVFS cache tables from the catalog database. This forces a full rebuild on the next `.bvfs_update` call. Requires explicit confirmation.

## Syntax

```
.bvfs_clear_cache yes
```

## Arguments

| Argument | Description |
|----------|-------------|
| `yes` | Required confirmation flag. The command refuses to run without it. |

## Examples

### Clear the cache
```
* .bvfs_clear_cache yes
BVFS cache cleared
```

### Without confirmation (rejected)
```
* .bvfs_clear_cache
Use: .bvfs_clear_cache yes
```

### Full rebuild workflow
```
* .bvfs_clear_cache yes
BVFS cache cleared
* .bvfs_update
Updating BVFS cache for all jobs...
Done.
```

## Notes

- This is a destructive operation: all cached BVFS data is removed.
- After clearing, `.bvfs_lsdirs` and `.bvfs_lsfiles` will return no results until `.bvfs_update` is run.
- Use this when the BVFS cache is suspected to be corrupt or inconsistent.
- The `yes` argument is a positional flag, not a key=value pair.
- On large catalogs, the subsequent `.bvfs_update` to rebuild the cache can take significant time.

## Related Commands

- [.bvfs_update](dot-bvfs-update.md) -- Rebuild the cache after clearing
- [.bvfs_cleanup](dot-bvfs-cleanup.md) -- Remove individual restore session tables
- [.bvfs_lsdirs](dot-bvfs-lsdirs.md) -- Browse directories (requires cache)
- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) -- Browse files (requires cache)
