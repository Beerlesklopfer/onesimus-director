# show -- Show Resource Configuration

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L468) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Show resource records

**Originalsyntax:**

```text
show catalog=<catalog-name> | client=<client-name> | console=<console-name> | director=<director-name> | fileset=<fileset-name> | jobdefs=<job-defaults> | job=<job-name> | message=<message-resource-name> | pool=<pool-name> | profile=<profile-name> | schedule=<schedule-name> | storage=<storage-name> | catalog | clients | consoles | directors | filesets | jobdefs | jobs | messages | pools | profiles | schedules | storages | disabled [ clients | jobs | schedules ] | all [verbose]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/show.rs:155](../../../crates/onesimus-dird/src/commands/show.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display the Director's in-memory resource configuration for jobs, clients, storages, pools, and other resource types.

## Syntax

```
show [all|director|clients|jobs|jobdefs|storages|pools|schedules|consoles|filesets|catalogs|disabled]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `all` | Summary of all resource counts (default) |
| `director` | Show Director resource settings |
| `clients` | Show all Client resources |
| `jobs` | Show all Job resources with full configuration |
| `jobdefs` | Show all JobDefs resources |
| `storages` | Show all Storage resources |
| `pools` | Show all Pool resources |
| `schedules` | Show all Schedule resources |
| `consoles` | Show all Console resources |
| `filesets` | Show all FileSet resources |
| `catalogs` | Show all Catalog resources |
| `disabled` | Show only disabled resources |

## Examples

### Show all resource summary
```
* show all
Director: bareos-dir
Clients: 1
Jobs: 3
JobDefs: 1
Storages: 1
Pools: 4
FileSets: 2
Schedules: 1
Consoles: 1
```

### Show job configuration
```
* show jobs
Job {
  Name = "backup-bareos-fd"
  Type = Backup
  Level = Incremental
  Client = bareos-fd
  FileSet = "SelfTest"
  Storage = File
  Pool = Incremental
  Messages = Standard
  Priority = 10
  Schedule = "WeeklyCycle"
  Full Backup Pool = Full
  Differential Backup Pool = Differential
  Incremental Backup Pool = Incremental
  Enabled = yes
}
```

### Show Director resource
```
* show director
Director: name=bareos-dir port=9101 maxjobs=10
```

## Behavior

- Reads from the in-memory configuration, not from disk. After `reload`, shows the updated config.
- `show all` without arguments prints a summary count of each resource type.
- `show jobs` prints each Job in a Bareos-compatible block format with all directives.
- `show clients` and `show storages` print one-line summaries per resource.
- This is not an audited command.

## Related Commands

- [list](list.md) -- List records from the catalog database
- [reload](reload.md) -- Reload configuration from disk
- [enable](enable.md) / [disable](disable.md) -- Toggle resource enabled state
