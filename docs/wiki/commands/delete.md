# delete -- Delete Volume or Job Records

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L323) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Delete volume, pool or job

**Originalsyntax:**

```text
delete [volume=<vol-name> [yes] | volume pool=<pool-name> [yes] | storage storage=<storage-name> | pool=<pool-name> | jobid=<jobid> | jobid=<jobid1,jobid2,...> | jobid=<jobid1-jobid9>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:352](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Remove volume or job records from the catalog database. This does not delete the physical volume data.

## Syntax

```
delete [volume=<name>|pool=<name>|jobid=<id>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `volume=<name>` | Delete this volume record from the catalog |
| `pool=<name>` | Delete all volumes in this pool from the catalog |
| `jobid=<id>` | Delete this job record and all associated file records |

## Examples

### Delete a volume from catalog
```
* delete volume=Full-0001
Volume Full-0001 deleted from catalog
```

### Delete a job record
```
* delete jobid=10
JobId 10 deleted from catalog
```

### Without arguments
```
* delete
delete: specify volume=<name> or jobid=<id>
```

## Behavior

- `delete volume`: removes the media record from the catalog. The physical volume file/tape is not affected.
- `delete jobid`: removes the job record and all associated file and jobmedia records from the catalog.
- Deleted records cannot be recovered -- use with caution.
- The volume must not be in use by a running job.
- For removing expired records based on retention periods, use `prune` instead.
- This is an audited command.

## Related Commands

- [prune](prune.md) -- Prune expired records based on retention
- [purge](purge.md) -- Purge all records from a volume
- [list](list.md) -- List volumes and jobs before deleting
