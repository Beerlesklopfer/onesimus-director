# .bvfs_update -- Update BVFS Cache

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L290) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Update BVFS cache

**Originalsyntax:**

```text
.bvfs_update [jobid=<jobid>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (3):** `c022-update-jobid`, `c022-update-unknown-job`, `c022-update-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:817](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Populates or refreshes the BVFS (Bareos Virtual File System) cache for one or all jobs. The BVFS cache materializes the file/directory tree from the catalog so that restore browsing (via `.bvfs_lsdirs`, `.bvfs_lsfiles`) can work efficiently. Must be called before browsing files from a specific job.

## Syntax

```
.bvfs_update [jobid=<N>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `jobid=<N>` | Job ID to update the cache for. If omitted, all jobs are updated. |

## Examples

### Update cache for a specific job
```
* .bvfs_update jobid=42
Updating BVFS cache for JobId 42...
Done.
```

### Update cache for all jobs
```
* .bvfs_update
Updating BVFS cache for all jobs...
Done.
```

### Typical restore workflow: update then browse
```
* .bvfs_update jobid=10
Updating BVFS cache for JobId 10...
Done.
* .bvfs_lsdirs jobid=10 path=/
```

## Notes

- This is typically the first step in the BVFS restore workflow: update -> browse dirs/files -> select -> restore.
- Updating all jobs (no `jobid=` argument) can be slow on large catalogs.
- The cache is stored in temporary tables in the catalog database.
- The WebUI calls this automatically before presenting the restore file browser.
- If the cache is already up to date for a job, this is a no-op.

## Related Commands

- [.bvfs_lsdirs](dot-bvfs-lsdirs.md) -- Browse directories after cache update
- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) -- Browse files after cache update
- [.bvfs_get_jobids](dot-bvfs-get-jobids.md) -- Get the job chain before updating
- [.bvfs_clear_cache](dot-bvfs-clear-cache.md) -- Clear the entire BVFS cache
- [.bvfs_cleanup](dot-bvfs-cleanup.md) -- Cleanup temporary restore tables
