# delete -- Delete Volume or Job Records

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L323) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Delete volume, pool or job

**Originalsyntax:**

```text
delete [volume=<vol-name> [yes] | volume pool=<pool-name> [yes] | storage storage=<storage-name> | pool=<pool-name> | jobid=<jobid> | jobid=<jobid1,jobid2,...> | jobid=<jobid1-jobid9>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 15 von 16 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Abweichende Fälle: `c035-delete-noarg-api2`.
- `.api 2`: Original liefert `"deleted": {}`, Onesimus ein leeres Ergebnis (C-046, Fall `c035-delete-noarg-api2`).
- Löschen von Volume/Pool/Job wird nur über Dialog- und Abbruchpfade verglichen; die Katalogwirkung ist nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-046` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (16):** `c011-delete-cancel`, `c011-delete-volume-cancel`, `c011-delete-menu-pool-cancel`, `c011-delete-menu-jobid-cancel`, `c011-delete-jobid-cancel`, `c011-delete-pool-cancel`, `c011-delete-volume-pool-cancel`, `c011-delete-volume-unknown`, `c011-delete-volume-star-unknown`, `c011-delete-volname-unknown`, `c011-delete-storage-empty`, `c011-delete-api1-cancel`, `c012-delete-illegal-jobid`, `c035-delete-noarg-api1`, `c035-delete-noarg-api2`, `delete-dialog`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:657](../../../crates/onesimus-dird/src/commands/extra.rs).

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
