# update -- Update Volume, Pool, or Slot Attributes

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L501) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Update volume, pool, slots, job or statistics

**Originalsyntax:**

```text
update [volume=<volume-name> [volstatus=<status>] [volretention=<time-def>] [actiononpurge=<action>] [encrypt=<yes/no/rotate>] [pool=<pool-name>] [recycle=<yes/no>] [slot=<number>] [inchanger=<yes/no>]] | [pool=<pool-name> [maxvolbytes=<size>] [maxvolfiles=<nb>] [maxvoljobs=<nb>][enabled=<yes/no>] [recyclepool=<pool-name>] [actiononpurge=<action>] | slots [storage=<storage-name>] [scan]] | [jobid=<jobid> [jobname=<name>] [starttime=<time-def>] [client=<client-name>] [filesetid=<fileset-id>] [jobtype=<job-type>]] | [stats [days=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 47 von 47 Fällen byte-identisch; Modi: Text, API 1.
- `update slots` meldet „not supported“ (C-054). Volume-/Pool-Parameter sind verglichen, `update stats` und Autochanger-Pfade nicht vollständig.

**Offene Logeinträge:** `C-054` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (47):** `c011-update-cancel`, `c011-update-volume-cancel`, `c011-update-pool-cancel`, `c011-update-stats-cancel`, `c011-update-volume-done`, `c011-update-volume-status-cancel`, `c011-update-volume-badnumber-cancel`, `c011-update-pool-select-cancel`, `c011-update-volume-unknown`, `c011-update-slots`, `c011-update-jobid-missing`, `c011-update-api1-cancel`, `c012-update-bad-volstatus`, `update-dialog`, `t001-update-volstatus-bad`, `t001-update-missing-media-first`, `t001-update-maxvoljobs-word`, `t001-update-maxvolfiles-junk`, `t001-update-maxvolbytes-bad`, `t001-update-volretention-bad`, `t001-update-voluse-bad`, `t001-update-recycle-bad`, `t001-update-inchanger-bad`, `t001-update-slot-negative`, `t001-update-slot-maxvols`, `t001-update-enabled-bad`, `t001-update-enabled-word`, `t001-update-encrypt-bad`, `t001-update-encrypt-nokey`, `t001-update-pool-unknown`, `t001-update-recyclepool-unknown`, `t001-update-allfrompool-unknown`, `t001-update-pool-resource-unknown`, `t001-update-two-errors`, `t001-dialog-status-cancel`, `t001-dialog-retention-bad`, `t001-dialog-use-bad`, `t001-dialog-maxjobs-bad`, `t001-dialog-maxfiles-bad`, `t001-dialog-maxbytes-bad`, `t001-dialog-recycle-bad`, `t001-dialog-slot-bad`, `t001-dialog-inchanger-bad`, `t001-dialog-volfiles-decline`, `t001-dialog-pool-unknown`, `t001-dialog-enabled-bad`, `t001-dialog-recyclepool-cancel`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:199](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Modify volume or pool attributes in the catalog, or update slot information from an autochanger.

## Syntax

```
update [volume=<name>|pool=<name>|slots storage=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `volume=<name>` | Update attributes for this volume |
| `pool=<name>` | Update pool record from the resource definition |
| `slots` | Scan autochanger and update slot assignments |
| `storage=<name>` | Storage daemon for slot updates |

## Examples

### Update a volume
```
* update volume=Full-0001
Volume "Full-0001" updated in catalog
```

### Update a pool from resource definition
```
* update pool=Full
Pool "Full" updated from resource
```

### Update autochanger slots
```
* update slots storage=Tape
Updating slots for storage Tape...
Done.
```

## Behavior

- `update volume`: opens an interactive dialog (in C++ Bareos) to change volume attributes such as VolStatus, MaxVolBytes, MaxVolJobs, Recycle, Retention, etc. With the Director, the catalog record is updated directly.
- `update pool`: synchronizes the catalog pool record with the current Pool resource definition (e.g. after editing and reloading the config).
- `update slots`: queries the autochanger for current slot contents and updates the catalog accordingly.
- Without arguments, prints available sub-commands.
- This is an audited command.

## Related Commands

- [label](label.md) -- Label a new volume
- [list](list.md) -- List volumes to see current attributes
- [show](show.md) -- Show pool resource definitions
- [reload](reload.md) -- Reload config before updating pools
