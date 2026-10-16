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

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 53 von 53 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI.

**Paritätsfälle (53):** `c007-empty-then-command`, `c007-leading-spaces`, `c007-leading-tab`, `c007-trailing-spaces`, `c007-trailing-newline`, `c007-uppercase`, `c007-mixed-case-prefix`, `c007-command-with-value`, `c019-add-console`, `c019-add-profile`, `c019-add-schedule`, `c019-add-user`, `c019-add-fileset`, `c020-compact-show-client`, `c020-json-show-client`, `c027-gui-show`, `c028-show-pools`, `c028-show-job`, `c028-show-schedules`, `c028-show-disabled`, `c028-show-all`, `c028-show-disabled-jobs`, `c035-show-help`, `c035-show-help-api1`, `c035-show-help-api2`, `c041-show-job-backupcatalog`, `c041-show-job-backupcatalog-verbose`, `c041-show-job-backupfd-verbose`, `c041-show-jobdefs-defaultjob`, `c041-show-jobdefs-verbose`, `c041-show-job-backupcatalog-api2`, `show-director`, `show-clients`, `show-client-bareos-fd`, `show-jobs`, `show-job-backup-bareos-fd`, `show-jobdefs`, `show-storages`, `show-pools`, `show-pool-Full`, `show-filesets`, `show-fileset-LinuxAll`, `show-schedules`, `show-messages`, `show-catalogs`, `show-consoles`, `show-profiles`, `show-all`, `show-disabled`, `show-nosuch-foo`, `show-job-nosuch`, `show-verbose-client`, `show-jobs-api2`.

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
