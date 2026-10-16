# .bvfs_clear_cache -- Clear Entire BVFS Cache

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L307) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Clear BVFS cache

**Originalsyntax:**

```text
.bvfs_clear_cache yes
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Verglichen sind Ausgaben ohne `yes`; `.bvfs_clear_cache yes` ändert den Katalog und ist nur durch Einheitentests belegt.

**Paritätsfälle (3):** `c022-clear-cache-noyes`, `c035-bvfs-clear-cache-api1`, `c035-bvfs-clear-cache-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:839](../../../crates/onesimus-dird/src/commands/bvfs.rs).

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
