# purge -- Purge All Records from a Volume

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L390) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Purge records from catalog

**Originalsyntax:**

```text
purge [files [job=<job> | jobid=<jobid> | client=<client> | volume=<volume>]] | [jobs [client=<client> | volume=<volume>] | pool=<pool>] [jobstatus=<status>]] | [volume[=<volume>] [storage=<storage>] [pool=<pool> | allpools] [devicetype=<type>] [drive=<drivenum>] [action=<action>]] | [quota [client=<client>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:1109](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Forcefully remove all catalog records associated with a volume, job, or client, ignoring retention periods.

## Syntax

```
purge [files jobid=<id>|jobs client=<name>|volume=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `files` | Purge file records for a specific job |
| `jobs` | Purge job records for a specific client |
| `volume=<name>` | Purge all records associated with this volume |
| `jobid=<id>` | Job ID (used with `files`) |
| `client=<name>` | Client name (used with `jobs`) |

## Examples

### Purge a volume
```
* purge volume=Full-0001
Volume "Full-0001" purged from catalog
```

### Purge file records for a job
```
* purge files jobid=10
Purged files from JobId 10
```

### Purge all jobs for a client
```
* purge jobs client=bareos-fd
Purged jobs for client bareos-fd
```

## Behavior

- Unlike `prune`, purge ignores retention periods and immediately removes all records.
- Purging a volume removes all associated file and job records and sets the volume status to `Purged`.
- A purged volume can be relabeled, truncated, or recycled for new backups.
- The physical data on the volume is not affected -- use `truncate` to reclaim disk space.
- **Use with caution**: purged records cannot be recovered, and restore from the affected backups will no longer be possible via the catalog.
- Without arguments, prints available sub-commands.
- This is an audited command.

## Related Commands

- [prune](prune.md) -- Prune records respecting retention periods
- [truncate](truncate.md) -- Truncate a purged volume to reclaim space
- [relabel](relabel.md) -- Relabel a purged volume
- [delete](delete.md) -- Delete individual records
