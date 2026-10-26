# show — Konfigurierte Ressourcen anzeigen

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

**Lokaler Handler:** [crates/odir/src/commands/show.rs:173](../../../crates/odir/src/commands/show.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`show` gibt Ressourcen aus der geladenen Konfiguration des Directors aus: Director, Clients, Jobs, JobDefs, Storages, Kataloge, Schedules, FileSets, Pools, Messages, Counter, Profile, Konsolen und Benutzer. Die Ausgabe hat die Form der Konfigurationsdateien und zeigt den Stand im Speicher, also nach dem letzten Start, [reload](reload.md) oder [configure add](configure.md). Der Befehl liest nur und verändert nichts. Für die Datensätze im Katalog gibt es [list](list.md).

Ohne `verbose` stehen nur die Direktiven da, die in der Konfiguration gesetzt sind. Mit `verbose` kommen alle anderen Direktiven mit ihrem Standardwert dazu, auskommentiert mit `#`.

In den Beispielen dieser Seite sind die Werte von `Password` durch `"[…]"` ersetzt. Der Director gibt dort den gespeicherten Wert aus, bei Passwörtern in MD5-Form `"[md5]…"`.

## Syntax

```text
show
show all [verbose]
show directors | clients | jobs | jobdefs | storages | catalogs | schedules | filesets | pools | messages | counters | profiles | consoles | users [verbose]
show director=<name> | client=<name> | job=<name> | jobdefs=<name> | storage=<name> | catalog=<name> | schedule=<name> | fileset=<name> | pool=<name> | message=<name> | counter=<name> | profile=<name> | console=<name> | user=<name> [verbose]
show <ressource>[=<name>] <ressource>[=<name>] … [verbose]
show disabled [jobs | clients | schedules]
show help
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| (keins), `all` | – | Alle Ressourcen aller Typen, in der Reihenfolge Director, Clients, JobDefs, Jobs, Storages, Kataloge, Schedules, FileSets, Pools, Messages, Counter, Profile, Konsolen, Benutzer. |
| `directors`, `clients`, `jobs`, `jobdefs`, `storages`, `catalogs`, `schedules`, `filesets`, `pools`, `messages`, `counters`, `profiles`, `consoles`, `users` | – | Alle Ressourcen dieses Typs. Ist keine definiert, kommt `No <Typ> resource defined`. |
| `<typ>=<name>` | Ressourcenname | Nur diese eine Ressource. Der Typ darf in Einzahl stehen (`client=`, `job=`, `pool=`, `message=` …). |
| Abkürzungen | Anfang eines Schlüsselworts, beliebige Groß-/Kleinschreibung | `show POO` wirkt wie `show pools`, `show Fi=LinuxAll` wie `show fileset=LinuxAll`. Passen mehrere Schlüsselwörter, gilt das erste in der Reihenfolge der Tabelle oben. |
| mehrere Angaben | – | `show pool=Full client=bareos-fd` gibt beide Ressourcen nacheinander aus. |
| `verbose` | – | Zusätzlich alle nicht gesetzten Direktiven mit Standardwert, mit `#` auskommentiert. Kann an jeder Stelle stehen. |
| `disabled` | optional `jobs`, `clients`, `schedules` | Welche Jobs, Clients und Schedules deaktiviert sind. Ohne Zusatz alle drei Listen; ein anderes Wort danach wirkt wie kein Zusatz. |
| `help` | – | Liste der Schlüsselwörter. |

## API-Modi

- **`.api 0`** (Standard): Ressourcenblöcke wie in den Konfigurationsdateien, nach jedem Block eine Leerzeile.
- **`.api 1`**: dieselben Blöcke wie unter `.api 0`. `show disabled` gibt nur die Namen aus, ohne die Überschriften `Disabled …:` und `No disabled …`.
- **`.api 2`**: JSON-Objekt unter `result`, gegliedert nach Typ (`jobs`, `clients`, `pools`, `directors`, …) und darin nach Ressourcenname. Die Direktiven heißen klein geschrieben (`labelformat`, `maximumvolumes`), Zahlen und Ja/Nein-Werte haben JSON-Typen, mehrfache Werte sind Listen. `show disabled` liefert `result.disabled` mit den Listen `jobs`, `clients` und `schedules`. `show help` liefert ein leeres `result`. Fehler stehen in `error.data.messages.error`.
- **`.api json compact=yes`**: dasselbe JSON in einer Zeile.

## Beispiele

### Alle Ressourcen

`show` ohne Argument ist gleichbedeutend mit `show all` (gekürzt):

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show
Director {
  Name = "bareos-dir"
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show all
Director {
  Name = "bareos-dir"
  Messages = "Daemon"
  Addresses = {
    ipv4 = {
      addr = 0.0.0.0
      port = 39101
    }
    ipv6 = {
      addr = ::
      port = 39101
    }
  }
  WorkingDirectory = "/home/INTRANET.BERNAU.FAMILY/Joerg.Bernau/Dokumente/baReos-dird/target/parity/conf/work"
  MaximumConcurrentJobs = 10
  Password = "[…]"
  Auditing = Yes
}

Client {
  Name = "parity-ndmp-fd"
  Protocol = NDMPV4
  Address = "localhost"
  Password = "[…]"
}

Client {
  Name = "bareos-fd"
  Description = "Client resource of the Director itself."
  Address = "localhost"
  Password = "[…]"
}

JobDefs {
  Name = "DefaultJob"
  Type = Backup
  Level = Incremental
  Messages = "Standard"
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alle Ressourcen eines Typs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show pools
Pool {
  Name = "Scratch"
  PoolType = Scratch
  Storage = "File"
  RecyclePool = "Scratch"
}

Pool {
  Name = "Incremental"
  LabelFormat = "Incremental-"
  MaximumVolumes = 100
  MaximumVolumeBytes = 1 g
  VolumeRetention = 1 months 
}

Pool {
  Name = "Full"
  LabelFormat = "Full-"
  MaximumVolumes = 100
  MaximumVolumeBytes = 50 g
}

Pool {
  Name = "Differential"
  LabelFormat = "Differential-"
  MaximumVolumes = 100
  MaximumVolumeBytes = 10 g
  VolumeRetention = 3 months 
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show clients
Client {
  Name = "parity-ndmp-fd"
  Protocol = NDMPV4
  Address = "localhost"
  Password = "[…]"
}

Client {
  Name = "bareos-fd"
  Description = "Client resource of the Director itself."
  Address = "localhost"
  Password = "[…]"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show schedules
Schedule {
  Name = "WeeklyCycleAfterBackup"
  Description = "This schedule does the catalog. It starts after the WeeklyCycle."
  Run = Full Mon-Fri at 21:10
  Enabled = No
}

Schedule {
  Name = "WeeklyCycle"
  Run = Full 1st Sat at 21:00
  Run = Differential 2nd-5th Sat at 21:00
  Run = Incremental Mon-Fri at 21:00
  Enabled = No
}

Schedule {
  Name = "ParityPreview"
  Run = Full 1st Sat at 21:00
  Run = pool=\"Full\" Differential 2nd-5th Sat at 03:30
  Run = Incremental Mon-Fri at 21:00
  Run = Full last Sun at 01:15
  Run = Incremental storage=\"File\" messages=\"Standard\" priority=5 at 12:45
  Run = Full Tue w42-w44 at 18:00
  Run = Full 1,15,31 at 00:05
  Run = Differential 3,13,23 Jan-Mar,Oct-Dec at 00:00
}

Schedule {
  Name = "c019h"
  Run = Full Mon at 03:00
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show jobs
Job {
  Name = "parity-admin"
  Type = Admin
  Messages = "Standard"
  Storage = "File"
  Pool = "Incremental"
  Client = "bareos-fd"
  FileSet = "SelfTest"
}

Job {
  Name = "parity-verify"
  Type = Verify
  Level = VolumeToCatalog
  Messages = "Standard"
  Storage = "File"
  Pool = "Incremental"
  Client = "bareos-fd"
  FileSet = "SelfTest"
  JobToVerify = "backup-bareos-fd"
}

[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show directors
Director {
  Name = "bareos-dir"
  Messages = "Daemon"
  Addresses = {
    ipv4 = {
      addr = 0.0.0.0
      port = 39173
    }
    ipv6 = {
      addr = ::
      port = 39173
    }
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eine einzelne Ressource

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show job=backup-bareos-fd
Job {
  Name = "backup-bareos-fd"
  Client = "bareos-fd"
  JobDefs = "DefaultJob"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show job=BackupCatalog
Job {
  Name = "BackupCatalog"
  Description = "Backup the catalog database (after the nightly save)"
  Level = Full
  FileSet = "Catalog"
  Schedule = "WeeklyCycleAfterBackup"
  JobDefs = "DefaultJob"
  WriteBootstrap = "|/usr/bin/bsmtp -h localhost -f \"\(Bareos\) \" -s \"Bootstrap for Job %j\" root"
  Priority = 11
  RunBeforeJob = "/usr/lib/bareos/scripts/make_catalog_backup MyCatalog"
  RunAfterJob = "/usr/lib/bareos/scripts/delete_catalog_backup MyCatalog"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show storage=File
Storage {
  Name = "File"
  Address = "WS-JOERG.intranet.bernau.family"
  Password = "[…]"
  Device = "FileStorage"
  MediaType = "File"
  MaximumConcurrentJobs = 10
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show fileset=LinuxAll
FileSet {
  Name = "LinuxAll"
  Description = "Backup all regular filesystems, determined by filesystem type."
  Include {
    Options {
      Signature = "XXH128"
      OneFS = No
      HardLinks = No
      AclSupport = Yes
      XattrSupport = Yes
      FsType = "btrfs"
      FsType = "ext2"
      FsType = "ext3"
      FsType = "ext4"
      FsType = "reiserfs"
      FsType = "jfs"
      FsType = "vfat"
      FsType = "xfs"
      FsType = "zfs"
    }
    File = "/"
  }
  Exclude {
    File = "/var/lib/bareos"
    File = "/var/lib/bareos/storage"
    File = "/proc"
    File = "/dev"
    File = "/sys"
    File = "/tmp"
    File = "/var/tmp"
    File = "/.journal"
    File = "/.fsck"
  }
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show schedule=WeeklyCycle
Schedule {
  Name = "WeeklyCycle"
  Run = Full 1st Sat at 21:00
  Run = Differential 2nd-5th Sat at 21:00
  Run = Incremental Mon-Fri at 21:00
  Enabled = No
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show jobdefs
JobDefs {
  Name = "DefaultJob"
  Type = Backup
  Level = Incremental
  Messages = "Standard"
  Storage = "File"
  Pool = "Incremental"
  FullBackupPool = "Full"
  IncrementalBackupPool = "Incremental"
  DifferentialBackupPool = "Differential"
  Client = "bareos-fd"
  FileSet = "SelfTest"
  Schedule = "WeeklyCycle"
  WriteBootstrap = "/var/lib/bareos/%c.bsr"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show catalogs
Catalog {
  Name = "MyCatalog"
  DbPassword = ""
  DbUser = "joerg.bernau"
  DbName = "parity_1244604"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show profile=operator
Profile {
  Name = "operator"
  Description = "Profile allowing normal Bareos operations."
  JobAcl = "*all*"
  ClientAcl = "*all*"
  StorageAcl = "*all*"
  ScheduleAcl = "*all*"
  PoolAcl = "*all*"
  CommandAcl = "!.bvfs_clear_cache", "!.exit", "!.sql", "!configure", "!create", "!delete", "!purge", "!prune", "!sqlquery", "!umount", "!unmount", "*all*"
  FileSetAcl = "*all*"
  CatalogAcl = "*all*"
  WhereAcl = "*all*"
  PluginOptionsAcl = "*all*"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show console=parity-limited
Console {
  Name = "parity-limited"
  Password = "[…]"
  JobAcl = "backup-bareos-fd"
  ClientAcl = "bareos-fd"
  StorageAcl = "File"
  ScheduleAcl = "WeeklyCycle"
  PoolAcl = "Full", "Scratch"
  CommandAcl = "list", "llist", "show", "help", ".help", "status", "version", ".jobs", ".clients", ".pools", ".storages", ".filesets", ".catalogs", ".schedule", ".authorized", ".defaults", "whoami", "time"
  FileSetAcl = "LinuxAll"
  CatalogAcl = "MyCatalog"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Bei `show message=<name>` stehen die Ziele der Messages-Ressource:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show message=Standard
Messages {
  Name = "Standard"
  console = all,!saved,!skipped,!audit
  append = /tmp/claude-10001/-home-INTRANET-BERNAU-FAMILY-Joerg-Bernau-Dokumente-baReos-dird/08def236-83d2-48b8-887a-df7f3e834b5d/scratchpad/docs-list/run/conf/work/bareos.log = all,!saved,!skipped,!audit
  catalog = all,!saved,!skipped,!audit
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Ressourcen auf einmal

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show pool=Full client=bareos-fd
Pool {
  Name = "Full"
  LabelFormat = "Full-"
  MaximumVolumes = 100
  MaximumVolumeBytes = 50 g
}

Client {
  Name = "bareos-fd"
  Description = "Client resource of the Director itself."
  Address = "localhost"
  Password = "[…]"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Abkürzungen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show POO
Pool {
  Name = "Scratch"
  PoolType = Scratch
  Storage = "File"
  RecyclePool = "Scratch"
}
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show Fi=LinuxAll
FileSet {
  Name = "LinuxAll"
  Description = "Backup all regular filesystems, determined by filesystem type."
  Include {
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit Standardwerten: `verbose`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show pool=Full verbose
Pool {
  Name = "Full"
  # Description = 
  # PoolType = Backup
  LabelFormat = "Full-"
  # LabelType = "bareos"
  # CleaningPrefix = "CLN"
  # UseCatalog = Yes
  # PurgeOldestVolume = No
  # ActionOnPurge = None
  # RecycleOldestVolume = No
  # RecycleCurrentVolume = No
  MaximumVolumes = 100
  # MaximumVolumeJobs = 0
  # MaximumVolumeFiles = 0
  MaximumVolumeBytes = 50 g
  # CatalogFiles = Yes
  # VolumeRetention = 1 years 
  # VolumeUseDuration = 0
  # MigrationTime = 0
  # MigrationHighBytes = 0
  # MigrationLowBytes = 0
  # NextPool = ""
  # Storage = 
  # AutoPrune = Yes
  # Recycle = Yes
  # RecyclePool = ""
  # ScratchPool = ""
  # Catalog = ""
  # MinimumBlockSize = 0
  # MaximumBlockSize = 0
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show client=bareos-fd verbose
Client {
  Name = "bareos-fd"
  Description = "Client resource of the Director itself."
  # Protocol = Native
  # AuthType = None
  Address = "localhost"
  # LanAddress = 
  # Port = 9102
  # Username = 
  Password = "[…]"
  # Catalog = "MyCatalog"
  # Passive = No
  # ConnectionFromDirectorToClient = Yes
  # ConnectionFromClientToDirector = No
  # Enabled = Yes
  # HardQuota = 0
  # SoftQuota = 0
  # SoftQuotaGracePeriod = 0
  # StrictQuotas = No
  # QuotaIncludeFailedJobs = Yes
  # HeartbeatInterval = 0
  # MaximumConcurrentJobs = 1
  # MaximumBandwidthPerJob = 0
  # NdmpLogLevel = 4
  # NdmpBlockSize = 63 k
  # NdmpUseLmdb = Yes
  # TlsAuthenticate = No
  # TlsEnable = Yes
  # TlsRequire = Yes
  # TlsCipherList = ""
  # TlsCipherSuites = ""
  # TlsDhFile = ""
  # TlsProtocol = ""
  # TlsVerifyPeer = No
  # TlsCaCertificateFile = ""
  # TlsCaCertificateDir = ""
  # TlsCertificateRevocationList = ""
  # TlsCertificate = ""
  # TlsKey = ""
  # TlsAllowedCn = 
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein Job zeigt mit `verbose`, welche Werte er aus seinen JobDefs erbt (gekürzt):

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show job=backup-bareos-fd verbose
Job {
  Name = "backup-bareos-fd"
  # Description = 
  # Type = Backup
  # Protocol = Native
  # BackupFormat = "Native"
  # Level = Incremental
  # Messages = "Standard"
  # Storage = "File"
  # Pool = "Incremental"
  # FullBackupPool = "Full"
  # VirtualFullBackupPool = ""
  # IncrementalBackupPool = "Incremental"
  # DifferentialBackupPool = "Differential"
  # NextPool = ""
  Client = "bareos-fd"
  # FileSet = "SelfTest"
  # Schedule = "WeeklyCycle"
  # JobToVerify = ""
  # Catalog = ""
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Deaktivierte Ressourcen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show disabled
No disabled Jobs.
No disabled Clients.
Disabled Schedules:
   WeeklyCycleAfterBackup
   WeeklyCycle
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show disabled jobs
No disabled Jobs.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show disabled clients
No disabled Clients.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show disabled schedules
Disabled Schedules:
   WeeklyCycleAfterBackup
   WeeklyCycle
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein mit [disable](disable.md) zur Laufzeit deaktivierter Job erscheint unter Onesimus nicht in `show disabled`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=backup-bareos-fd
Job "backup-bareos-fd" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show disabled jobs
No disabled Jobs.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original zeigt hier den Laufzeitstatus: `Disabled Jobs:` und darunter `   backup-bareos-fd` (C-063).

### Typen ohne Ressourcen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show counters
No Counter resource defined
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show users
No User resource defined
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nach `configure add` und `reload`

`configure add` legt die Ressource an und lädt sie sofort. `reload` liest die Konfiguration neu, die Ressource bleibt erhalten. Die `Run`-Angabe steht danach in der vom Director normalisierten Form.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show schedule=dlreload
schedules resource dlreload not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add schedule name=dlreload run="Full mon at 3:00"
Created resource config file "/tmp/claude-10001/-home-INTRANET-BERNAU-FAMILY-Joerg-Bernau-Dokumente-baReos-dird/08def236-83d2-48b8-887a-df7f3e834b5d/scratchpad/docs-list/run/conf/bareos-dir.d/schedule/dlreload.conf":
Schedule {
  Name = "dlreload"
  Run = Full mon at 3:00
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show schedule=dlreload
Schedule {
  Name = "dlreload"
  Run = Full Mon at 03:00
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
reloaded
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show schedule=dlreload
Schedule {
  Name = "dlreload"
  Run = Full Mon at 03:00
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Schlüsselwörter anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show help
Keywords for the show command are:
directors
clients
jobs
jobdefs
storages
catalogs
schedules
filesets
pools
messages
counters
profiles
consoles
users
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ausgabe unter `.api 1`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show client=bareos-fd
Client {
  Name = "bareos-fd"
  Description = "Client resource of the Director itself."
  Address = "localhost"
  Password = "[…]"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show disabled
   WeeklyCycleAfterBackup
   WeeklyCycle
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JSON: `.api 2`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show pool=Full
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "pools": {
      "Full": {
        "name": "Full",
        "labelformat": "Full-",
        "maximumvolumes": 100,
        "maximumvolumebytes": "50 g"
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show job=BackupCatalog
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": {
      "BackupCatalog": {
        "name": "BackupCatalog",
        "description": "Backup the catalog database (after the nightly save)",
        "level": "Full",
        "fileset": "Catalog",
        "schedule": "WeeklyCycleAfterBackup",
        "jobdefs": "DefaultJob",
        "writebootstrap": "|/usr/bin/bsmtp -h localhost -f \\\"\\(Bareos\\) \\\" -s \\\"Bootstrap for Job %j\\\" root",
        "priority": 11,
        "runscript": [
          {
            "runbeforejob": "/usr/lib/bareos/scripts/make_catalog_backup MyCatalog"
          },
          {
            "runafterjob": "/usr/lib/bareos/scripts/delete_catalog_backup MyCatalog"
          }
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show disabled
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "disabled": {
      "jobs": [],
      "clients": [],
      "schedules": [
        "WeeklyCycleAfterBackup",
        "WeeklyCycle"
      ]
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Fehler unter `.api 2`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show job=nosuch
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "error": [
          "jobs resource nosuch not found.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein ungültiges Wort vor einer gültigen Angabe: Die gültige Ressource steht unter `error.data.result`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show nosuch client=bareos-fd
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {
        "clients": {
          "bareos-fd": {
            "name": "bareos-fd",
            "description": "Client resource of the Director itself.",
            "address": "localhost",
            "password": "[…]"
          }
        }
      },
      "messages": {
        "error": [
          "Resource nosuch not found\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kompaktes JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show pool=Full
{"jsonrpc":"2.0","id":null,"result":{"pools":{"Full":{"name":"Full","labelformat":"Full-","maximumvolumes":100,"maximumvolumebytes":"50 g"}}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole mit eingeschränkten Rechten

Unter Onesimus sieht eine Konsole mit ACLs bei `show` derzeit alle Ressourcen, auch solche außerhalb ihrer ACLs, und die gespeicherten Passwortwerte:

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> show clients
Client {
  Name = "parity-ndmp-fd"
  Protocol = NDMPV4
  Address = "localhost"
  Password = "[…]"
}

Client {
  Name = "bareos-fd"
  Description = "Client resource of the Director itself."
  Address = "localhost"
  Password = "[…]"
}

parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original zeigt nur die Ressourcen, die die ACLs der Konsole erlauben (hier nur `bareos-fd`), und maskiert Passwörter als `"****************"` (C-062).

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Resource <wort> not found` | Das Wort ist kein Schlüsselwort und keine Abkürzung eines Schlüsselworts. Folgen weitere gültige Angaben, werden sie trotzdem ausgegeben. |
| `<typ> resource <name> not found.` | Eine Ressource dieses Namens gibt es nicht, z.B. `jobs resource nosuch not found.` Der Befehl endet danach; weitere Angaben werden nicht mehr ausgegeben. |
| `No <Typ> resource defined` | Für diesen Typ ist keine Ressource konfiguriert, z.B. `No Counter resource defined`. |

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show nosuch=foo
Resource nosuch not found
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show job=nosuch
jobs resource nosuch not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Siehe auch

- [list](list.md) — Datensätze aus dem Katalog
- [configure](configure.md) — Ressourcen anlegen und exportieren
- [reload](reload.md) — Konfiguration neu laden
- [enable](enable.md) / [disable](disable.md) — Jobs, Clients und Schedules ein- und ausschalten
- [dot-jobs](dot-jobs.md), [dot-clients](dot-clients.md), [dot-pools](dot-pools.md) — nur die Namen der Ressourcen
- [dot-api](dot-api.md) — API-Modus umschalten
