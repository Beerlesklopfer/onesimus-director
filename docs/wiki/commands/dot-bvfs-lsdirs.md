# .bvfs_lsdirs -- List Directories in BVFS

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L282) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List directories using BVFS

**Originalsyntax:**

```text
.bvfs_lsdirs jobid=<jobid> path=<path> | pathid=<pathid> [limit=<limit>] [offset=<offset>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `stub`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler gibt die übergebenen Suchparameter als Text zurück; er liest keine Verzeichnis-, Datei- oder Versionsdaten aus dem Katalog.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:446](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Lists directories at a given path in the BVFS virtual file tree for one or more jobs. Used by the WebUI and restore tools to let the user browse the backed-up directory structure.

## Syntax

```
.bvfs_lsdirs jobid=<N> path=<path> [limit=<n>] [offset=<n>]
.bvfs_lsdirs jobid=<N> pathid=<id> [limit=<n>] [offset=<n>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `jobid=<N>` | Job ID (or comma-separated list of job IDs from `.bvfs_get_jobids`). |
| `path=<path>` | Directory path to list (e.g., `/`, `/etc/`). |
| `pathid=<id>` | Numeric path ID (alternative to `path`). |
| `limit=<n>` | Maximum number of entries to return (for pagination). |
| `offset=<n>` | Number of entries to skip (for pagination). |

## Examples

### List root directory
```
* .bvfs_lsdirs jobid=1,15,28,42 path=/
lsdirs jobid=1,15,28,42 path=/
```

### List with pagination
```
* .bvfs_lsdirs jobid=42 path=/etc/ limit=50 offset=0
lsdirs jobid=42 path=/etc/
```

### Using pathid instead of path
```
* .bvfs_lsdirs jobid=42 pathid=1
lsdirs jobid=42 path=1
```

## Notes

- Requires `.bvfs_update` to have been run for the specified job IDs first.
- Output format is tab-separated: `pathid \t fileid \t jobid \t lstat \t path`.
- The special entries `.` and `..` are included in the listing.
- Use `path` for human-readable browsing or `pathid` for programmatic access.
- When using multiple job IDs (comma-separated), the merged directory tree across all jobs is returned.
- Pagination via `limit` and `offset` is recommended for directories with many entries.

## Related Commands

- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) -- List files in a directory
- [.bvfs_update](dot-bvfs-update.md) -- Update cache (prerequisite)
- [.bvfs_get_jobids](dot-bvfs-get-jobids.md) -- Get job IDs for the jobid argument
- [.bvfs_versions](dot-bvfs-versions.md) -- List versions of a specific file
- [.bvfs_restore](dot-bvfs-restore.md) -- Restore selected directories
