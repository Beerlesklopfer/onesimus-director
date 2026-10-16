# .bvfs_get_jobids -- Get Job Chain for Restore

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L292) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Get jobids required for a restore

**Originalsyntax:**

```text
.bvfs_get_jobids jobid=<jobid> | ujobid=<unique-jobid> [all]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 7 von 7 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-bvfs.

**Paritätsfälle (7):** `c022-get-jobids`, `c022-get-jobids-all`, `c022-get-jobids-ujobid`, `c022-get-jobids-errors`, `c022-get-jobids-api2`, `c022-acl-limited`, `bvfs-get-jobids`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:978](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Returns the list of Job IDs needed to reconstruct a full restore for a given job. For incremental/differential backups, this walks the job chain back to the last full backup and returns all job IDs in the chain (full + differentials + incrementals).

## Syntax

```
.bvfs_get_jobids jobid=<N>
.bvfs_get_jobids ujobid=<unique-id>
.bvfs_get_jobids jobid=<N> all
```

## Arguments

| Argument | Description |
|----------|-------------|
| `jobid=<N>` | Job ID to resolve the backup chain for. |
| `ujobid=<unique-id>` | Unique Job ID (alternative to numeric jobid). |
| `all` | Optional flag: include copy and migration jobs in the chain. |

## Examples

### Get job chain for a specific job
```
* .bvfs_get_jobids jobid=42
1,15,28,42
```

### Using unique job ID
```
* .bvfs_get_jobids ujobid=backup-bareos-fd.2026-03-29_01.00.00_05
ujobid=backup-bareos-fd.2026-03-29_01.00.00_05: 1,15,28,42
```

### Include copy jobs in the chain
```
* .bvfs_get_jobids jobid=42 all
1,15,28,42,50
```

## Notes

- The returned comma-separated list of job IDs should be passed to `.bvfs_update` and the browsing commands (`.bvfs_lsdirs`, `.bvfs_lsfiles`).
- For a full backup, only the single job ID is returned.
- For incremental backups, the chain includes: last Full + all Differentials + all Incrementals up to the specified job.
- The `all` flag adds copy and migration jobs that may hold data on different volumes.
- Either `jobid` or `ujobid` must be specified; if neither is given, an error is returned.

## Related Commands

- [.bvfs_update](dot-bvfs-update.md) -- Update cache for the returned job IDs
- [.bvfs_lsdirs](dot-bvfs-lsdirs.md) -- Browse directories using the job chain
- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) -- Browse files using the job chain
- [.bvfs_restore](dot-bvfs-restore.md) -- Restore selected files
- [restore](restore.md) -- Interactive restore command
