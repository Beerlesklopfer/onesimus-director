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

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

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
