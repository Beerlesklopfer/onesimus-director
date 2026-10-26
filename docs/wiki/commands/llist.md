# llist — Katalogdatensätze ausführlich auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L369) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Full or long list like list command

**Originalsyntax:**

```text
llist basefiles ( jobid=<jobid> | ujobid=<complete_name> ) | backups client=<client-name> [fileset=<fileset-name>] [jobstatus=<status>] [level=<level>] [order=<asc|desc>] [limit=<number>] [days=<number>] [hours=<number>] | client=<client-name> | clients | copies jobid=<jobid> | files jobid=<jobid> | files ujobid=<complete_name> | filesets | fileset [ filesetid=<filesetid> ] | fileset [ jobid=<jobid> ] | fileset [ ujobid=<complete_name> ] | jobs [job=<job-name>] [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [pool=<pool>] [days=<number>] [hours=<number>] [last] [count] | job=<job-name> [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [days=<number>] [hours=<number>] | jobid=<jobid> | ujobid=<complete_name> | joblog jobid=<jobid> [count] | joblog ujobid=<complete_name> [count] | jobmedia jobid=<jobid> | jobmedia ujobid=<complete_name> | jobtotals | jobstatistics jobid=<jobid> | log [client=<client-name>] [reverse] | media [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] | media=<media-name> | nextvolume job=<job-name> [days=<number>] | pools | pool=<pool-name> | poolid=<poolid> | storages | volumes [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] [count] | volume=<volume-name> | volumeid=<volumeid> | mediaid=<volumeid> | [current] | [enabled | disabled] | [limit=<number> [offset=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 110 von 110 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-bvfs, parity-limited, parity-listacl, parity-operator.
- `llist nextvolume` teilt den Pfad von `list nextvolume` (C-046).

**Offene Logeinträge:** `C-046` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (110):** `c015-llist-no-keyword`, `c015-llist-jobs-jobstatus-f`, `c015-llist-jobs-last`, `c015-llist-jobs-count`, `c015-llist-jobs-limit-2-offset-1`, `c015-llist-jobs-reverse-limit-2`, `c015-llist-jobs-client-bareos-fd-limit-1`, `c015-llist-jobs-volume-Full-47321-limit-2`, `c015-llist-jobid-14`, `c015-llist-ujobid-nosuch`, `c015-llist-files-jobid-5`, `c015-llist-fileset-jobid-1`, `c015-llist-jobmedia-all`, `c015-llist-joblog-count`, `c015-llist-log`, `c015-llist-pool-nosuch`, `c015-llist-poolid-3`, `c015-llist-client`, `c015-llist-volumes-all`, `c015-llist-volumes-jobid-1`, `c015-llist-volumeid`, `c015-llist-backups`, `c015-llist-jobstats`, `c015-llist-pools-current`, `c015-llist-volumes-all-current`, `c015-llist-jobs-current-limit`, `c015-api2x-llist-jobs-disabled`, `c015-api2-llist-pool-Full`, `c015-api2-llist-jobs-jobid-14`, `c026-limited-llist-jobs`, `c026-limited-filesets`, `c026-limited-log`, `c026-limited-pools`, `c026-limited-clients`, `c026-limited-volumes`, `c026-limited-misc`, `c026-listacl-llist-jobs`, `c026-listacl-filesets`, `c026-listacl-log`, `c026-listacl-pools`, `c026-listacl-clients`, `c026-listacl-volumes`, `c026-listacl-misc`, `c026-operator-llist-jobs`, `c026-operator-filesets`, `c026-operator-log`, `c026-operator-pools`, `c026-operator-clients`, `c026-operator-volumes`, `c026-operator-misc`, `c026-listacl-api2-1`, `c026-listacl-api2-2`, `c026-listacl-api2-3`, `c026-bvfs-list-denied`, `c027-gui-llist-pools`, `c027-gui-llist-jobs`, `c027-gui-llist-media`, `c027-gui-list-clients`, `c027-gui-sqlquery-keyword`, `c027-gui-off-again`, `c028-llist-jobs`, `c028-llist-jobs-limit`, `c028-llist-jobid`, `c028-llist-jobtotals`, `c028-llist-files`, `c028-llist-fileset`, `c028-llist-filesets`, `c028-llist-jobmedia`, `c028-llist-joblog`, `c028-llist-log`, `c028-llist-pools`, `c028-llist-clients`, `c028-llist-storages`, `c028-llist-volumes`, `c028-llist-backups`, `c028-llist-jobs-last`, `c028-llist-pool-full`, `llist-jobs`, `llist-jobs-limit-5`, `llist-jobs-last`, `llist-jobs-jobstatus-T-limit-3`, `llist-jobs-client-bareos-fd-limit-3`, `llist-jobs-level-F-limit-3`, `llist-jobs-days-0`, `llist-jobid-1`, `llist-job-backup-bareos-fd-limit-2`, `llist-jobtotals`, `llist-clients`, `llist-pools`, `llist-pool-Full`, `llist-volumes`, `llist-volumes-pool-Full`, `llist-volume-Full-0001`, `llist-media`, `llist-filesets`, `llist-files-jobid-1-limit-5`, `llist-basefiles-jobid-1`, `llist-copies`, `llist-backups-client-bareos-fd-limit-3`, `llist-joblog-jobid-1`, `llist-jobmedia-jobid-1`, `llist-log-limit-5`, `llist-nextvolume-job-backup-bareos-fd`, `llist-storages`, `llist-mediatypes`, `llist-jobs-limit-3-api2`, `llist-pools-api2`, `llist-volumes-api2`, `llist-clients-api2`, `w011-llist-volumes-pool-cancel`.

**Lokaler Handler:** [crates/odir/src/commands/list.rs:17](../../../crates/odir/src/commands/list.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`llist` („long list“) zeigt dieselben Katalogdatensätze wie [list](list.md), aber mit allen Spalten der Tabelle und jedem Datensatz als Block `feld: wert` untereinander. Das ist nützlich, wenn man zu einem Job, Volume, Pool oder Client alle gespeicherten Werte sehen will (z.B. `jobtdate`, `volsessionid`, `firstwritten`, `uname`). Der Befehl liest nur und verändert nichts.

Schlüsselwörter, Filter, Dialoge und Fehlermeldungen sind dieselben wie bei `list`; diese Seite zeigt deshalb vor allem, wie die lange Ausgabe aussieht. Einige Schlüsselwörter haben keine eigene lange Form und geben genau dasselbe aus wie `list`: `jobtotals`, `files`, `basefiles` und `nextvolume`.

## Syntax

```text
llist <schlüsselwort> [<argumente> …]
```

Alle Schlüsselwörter und Argumente stehen bei [list](list.md#syntax), zum Beispiel:

```text
llist jobs [job=<name>] [client=<name>] [jobstatus=<status>] [joblevel=<level>] [last] [count] [reverse] [limit=<n> [offset=<n>]]
llist jobid=<jobid> | ujobid=<eindeutiger-jobname>
llist volumes [all | jobid=<jobid> | pool=<pool>] | volume=<name> | volumeid=<id>
llist pools | pool=<name> | poolid=<id>
llist clients | client=<name>
llist storages
llist joblog jobid=<jobid> [count]
llist log [client=<name>] [reverse]
llist jobmedia [jobid=<jobid>]
llist fileset jobid=<jobid> | filesetid=<id>
llist backups client=<name> [fileset=<name>]
```

## Argumente

Die Argumente sind dieselben wie bei [list](list.md#argumente). Unterschiede in der Ausgabe:

| Argument | Werte | Bedeutung bei `llist` |
|---|---|---|
| `jobs`, `jobid=`, `job=`, `backups` | wie `list` | Alle Spalten der Job-Tabelle: `job`, `purgedfiles`, `clientid`, `schedtime`, `endtime`, `realendtime`, `jobtdate`, `volsessionid`, `volsessiontime`, `joberrors`, `jobmissingfiles`, `poolid`, `poolname`, `priorjobid`, `filesetid`, `fileset`. |
| `count` | – | Eine Zeile ` count: <n>` statt einer Tabelle. |
| `volumes`, `volume=`, `volumeid=` | wie `list` | Alle Volume-Felder (Schreibzeiten, Blöcke, Fehler, Retention, Grenzen, Slot, …). |
| `volumes jobid=<jobid>` | Ganzzahl | `jobmediaid`, `jobid`, `mediaid` und `volumename` je beschriebenem Volume. |
| `pools`, `pool=`, `poolid=` | wie `list` | Alle Pool-Felder; bei `pools` und `pool=` zusätzlich `prunablevolumes`, `prunablejobs`, `prunablebytes`, und zwar ohne Ausrichtung und ohne Tausendertrennzeichen. |
| `clients`, `client=` | wie `list` | Zusätzlich `uname` (Version und Betriebssystem des Clients) und `autoprune`. |
| `jobmedia` | wie `list` | Zusätzlich `jobmediaid`, `mediaid`, `startfile`, `endfile`, `startblock`, `endblock`. |
| `joblog`, `log` | wie `list` | Jede Protokollzeile als Block mit `time` (nur `joblog`) und `logtext`. |
| `fileset` | wie `list` | Die FileSet-Felder untereinander, `filesettext` mit dem vollständigen FileSet. |

## Dialoge

Wie bei `list`: `llist volumes pool=<unbekannt>` fragt nach dem Pool, `llist nextvolume` ohne `job=` nach dem Job. `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist volumes pool=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Could not find Pool "nosuch": ERR=Pool record not found in Catalog.
Could not find Pool "nosuch": ERR=Pool record not found in Catalog.
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4): 
> .
Selection aborted, nothing done.
Pool nosuch doesn't exist.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## API-Modi

- **`.api 0`**: Blöcke `feld: wert`, Feldnamen rechtsbündig, Zahlen mit Tausendertrennzeichen, Leerzeile zwischen den Datensätzen.
- **`gui on`**: wie `.api 0`, Zahlen ohne Trennzeichen.
- **`.api 1`**: `feld: wert` ohne Einrückung, ohne Trennzeichen und ohne Leerzeilen zwischen den Datensätzen.
- **`.api 2`**: JSON wie bei `list`, nur mit allen Feldern je Objekt. `.api json compact=yes` gibt dasselbe einzeilig aus.

## Beispiele

### Einen Job vollständig anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
           jobid: 1
             job: backup-bareos-fd.2026-10-13_21.00.00_09
            name: backup-bareos-fd
     purgedfiles: 0
            type: B
           level: I
        clientid: 1
          client: bareos-fd
       jobstatus: T
       schedtime: 2026-10-13 21:00:00
       starttime: 2026-10-13 21:00:02
         endtime: 2026-10-13 21:00:03
     realendtime: 2026-10-13 21:00:03
        duration: 00:00:01
        jobtdate: 1,791,918,003
    volsessionid: 1
  volsessiontime: 1,791,802,887
        jobfiles: 180
        jobbytes: 126,345,661
       joberrors: 0
 jobmissingfiles: 0
          poolid: 2
        poolname: Incremental
      priorjobid: 0
       filesetid: 1
         fileset: SelfTest

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist jobs limit=5
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
           jobid: 479
             job: benchmark-small.2026-03-30_22.26.51_04
            name: benchmark-small
     purgedfiles: 0
            type: B
           level: F
        clientid: 1
          client: bareos-fd
       jobstatus: T
       schedtime: 2026-03-30 22:26:51
       starttime: 2026-03-30 22:26:51
         endtime: 2026-03-30 22:26:53
     realendtime: 2026-03-30 22:26:53
        duration: 00:00:02
        jobtdate: 1,774,902,413
    volsessionid: 7
  volsessiontime: 1,774,902,218
        jobfiles: 2,605
        jobbytes: 824,021,543
       joberrors: 0
 jobmissingfiles: 0
          poolid: 3
        poolname: Full
      priorjobid: 0
       filesetid: 4
         fileset: BenchmarkSmall

           jobid: 480
[…]

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobs zählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist jobs count
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
 count: 161

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ein Volume vollständig anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist volumeid=47322
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
          mediaid: 47,322
       volumename: Incremental-47322
             slot: 0
           poolid: 2
             pool: Incremental
        mediatype: File
     firstwritten: 2026-06-12 21:00:02
      lastwritten: 2026-10-15 21:00:02
        labeldate: 2026-06-12 21:00:02
          voljobs: 10
         volfiles: 0
        volblocks: 477
        volmounts: 10
         volbytes: 493,607,891
        volerrors: 0
        volwrites: 477
 volcapacitybytes: 0
        volstatus: Append
          enabled: 1
          recycle: 1
     volretention: 2,592,000
   voluseduration: 0
       maxvoljobs: 0
      maxvolfiles: 0
      maxvolbytes: 1,073,741,824
        inchanger: 0
          endfile: 0
         endblock: 493,607,890
        labeltype: 0
        storageid: 1
         deviceid: 0
       locationid: 0
     recyclecount: 0
     initialwrite: 
    scratchpoolid: 0
      scratchpool: 
    recyclepoolid: 0
      recyclepool: 
 hasencryptionkey: 0
          comment: 
          storage: File

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volumes eines Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist volumes jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
 jobmediaid: 2
      jobid: 1
    mediaid: 47,322
 volumename: Incremental-47322

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Pool

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist pool=Full
poolid: 3
name: Full
numvols: 3
maxvols: 100
useonce: 0
usecatalog: 1
acceptanyvolume: 0
volretention: 31536000
voluseduration: 0
maxvoljobs: 0
maxvolbytes: 53687091200
autoprune: 1
recycle: 1
pooltype: Backup
labelformat: Full-
enabled: 1
scratchpoolid: 0
recyclepoolid: 0
labeltype: 0
prunablevolumes: 0
prunablejobs: 0
prunablebytes: 0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Clients

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist clients
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
      clientid: 1
          name: bareos-fd
         uname: 25.1.2~pre43.18be77e68 (09Oct26) Debian GNU/Linux 13 (trixie),debian
     autoprune: 0
 fileretention: 5,184,000
  jobretention: 15,552,000

      clientid: 2
          name: parity-ndmp-fd
         uname: 
     autoprune: 0
 fileretention: 0
  jobretention: 0

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storages

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> llist storages
   storageid: 1
        name: File
 autochanger: 0

parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Die Konsole `parity-limited` sieht hier nur den Storage `File`, den ihre Storage-ACL erlaubt.

### Jobmedien

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobmedia jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+-------------------+------------+-----------+
| jobid | volumename        | firstindex | lastindex |
+-------+-------------------+------------+-----------+
|     1 | Incremental-47322 |          1 |       180 |
+-------+-------------------+------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist jobmedia jobid=1
 jobmediaid: 2
      jobid: 1
    mediaid: 47,322
 volumename: Incremental-47322
 firstindex: 1
  lastindex: 180
  startfile: 0
    endfile: 0
 startblock: 366,260,149
   endblock: 492,640,083

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobprotokoll und Log

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list joblog jobid=1 limit=5
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
 2026-03-21 23:44:50 bareos-dir JobId 1: Version: 25.0.3~pre51.cd3792b91 (18 March 2026) Debian GNU/Linux 13 (trixie)
 2026-03-21 23:44:50 bareos-dir JobId 1: Start Backup JobId 1, Job=backup-bareos-fd.2026-03-21_23.44.48_04
 2026-03-21 23:44:50 bareos-dir JobId 1: Connected Storage daemon at WS-JOERG.intranet.bernau.family:9103, encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
 2026-03-21 23:44:50 bareos-dir JobId 1:  Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
 2026-03-21 23:44:50 bareos-dir JobId 1: Probing client protocol... (result will be saved until config reload)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist joblog jobid=1 limit=2
    time: 2026-03-21 23:44:50
 logtext: bareos-dir JobId 1: Version: 25.0.3~pre51.cd3792b91 (18 March 2026) Debian GNU/Linux 13 (trixie)


    time: 2026-03-21 23:44:50
 logtext: bareos-dir JobId 1: Start Backup JobId 1, Job=backup-bareos-fd.2026-03-21_23.44.48_04


*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist log limit=2
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
 logtext: bareos-dir JobId 629: Error: Error running program when updating bootstrap file: |/usr/bin/bsmtp -h localhost -f "(Bareos) " -s "Bootstrap for Job BackupCatalog.2026-07-10_21.10.00_05" root: ERR=Child exited with code 1


 logtext: bareos-dir JobId 629: Error: Bareos bareos-dir 25.0.4~pre179.6e6f5b19f (16Jun26):
  OS Information:         Debian GNU/Linux 13 (trixie)
  JobId:                  629
  Job:                    BackupCatalog.2026-07-10_21.10.00_05
  Backup Level:           Full
  Client:                 "bareos-fd" 25.0.4~pre179.6e6f5b19f (16Jun26) Debian GNU/Linux 13 (trixie),debian
  FileSet:                "Catalog" 2026-03-30 21:10:01
  Pool:                   "Full" (From Job FullPool override)
  Catalog:                "MyCatalog" (From Client resource)
  Storage:                "File" (From Job resource)
  Scheduled time:         10-Jul-2026 21:10:00
  Start time:             10-Jul-2026 21:10:04
  End time:               10-Jul-2026 21:10:05
  Elapsed time:           1 sec
  Priority:               11
  Allow Mixed Priority:   no
  FD Files Written:       67
  SD Files Written:       67
  FD Bytes Read:          14,972 (14.97 KB)
  FD Bytes Written:       14,972 (14.97 KB)
  SD Bytes Written:       22,738 (22.73 KB)
  Rate:                   15,0 KB/s
  Software Compression:   None
  VSS:                    no
  Encryption:             no
  Accurate:               no
  Volume name(s):         Full-47321
  Volume Session Id:      4
  Volume Session Time:    1783588680
  Last Volume Bytes:      158,201,165 (158.2 MB)
  Non-fatal FD errors:    2
  SD Errors:              0
  FD termination status:  OK
  SD termination status:  OK
  Bareos binary info:     Bareos community build (UNSUPPORTED): Get professional support from https://www.bareos.com
  Job triggered by:       Scheduler
  Termination:            *** Backup Error ***



*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### FileSet eines Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist fileset jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
   filesetid: 1
     fileset: SelfTest
         md5: 35wO25lkbw+icQACU80RdB
  createtime: 2026-03-21 23:44:48
 filesettext: FileSet {
  Name = "SelfTest"
  Description = "fileset just to backup some files for selftest"
  Include {
    Options {
      Signature = "XXH128"
      HardLinks = No
      AclSupport = Yes
      XattrSupport = Yes
    }
    File = "/usr/sbin"
  }
}



*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Summen: gleiche Ausgabe wie `list`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist jobtotals
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+------+--------+----------------+------------------+
| jobs | files  | bytes          | job              |
+------+--------+----------------+------------------+
|   28 | 72,940 | 23,072,603,204 | benchmark-small  |
|   57 |    938 |        209,608 | BackupCatalog    |
|   75 |    685 |    697,375,315 | backup-bareos-fd |
+------+--------+----------------+------------------+
+------+--------+----------------+
| jobs | files  | bytes          |
+------+--------+----------------+
|  160 | 74,563 | 23,770,188,127 |
+------+--------+----------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ausgabe für Programme: `.api 1`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist pools
poolid: 1
name: Scratch
numvols: 0
maxvols: 0
useonce: 0
usecatalog: 1
acceptanyvolume: 0
volretention: 31536000
voluseduration: 0
maxvoljobs: 0
maxvolbytes: 0
autoprune: 1
recycle: 1
pooltype: Scratch
labelformat: *
enabled: 1
scratchpoolid: 0
recyclepoolid: 1
labeltype: 0
prunablevolumes: 0
prunablejobs: 0
prunablebytes: 0
poolid: 2
name: Incremental
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JSON: `.api 2`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist jobs limit=1
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {
        "jobid": "479",
        "job": "benchmark-small.2026-03-30_22.26.51_04",
        "name": "benchmark-small",
        "purgedfiles": "0",
        "type": "B",
        "level": "F",
        "clientid": "1",
        "client": "bareos-fd",
        "jobstatus": "T",
        "schedtime": "2026-03-30 22:26:51",
        "starttime": "2026-03-30 22:26:51",
        "endtime": "2026-03-30 22:26:53",
        "realendtime": "2026-03-30 22:26:53",
        "duration": "00:00:02",
        "jobtdate": "1774902413",
        "volsessionid": "7",
        "volsessiontime": "1774902218",
        "jobfiles": "2605",
        "jobbytes": "824021543",
        "joberrors": "0",
        "jobmissingfiles": "0",
        "poolid": "3",
        "poolname": "Full",
        "priorjobid": "0",
        "filesetid": "4",
        "fileset": "BenchmarkSmall"
      }
    ],
    "meta": {
      "range": {
        "limit": 1,
        "filtered": 0
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### GUI-Modus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist clients
      clientid: 1
          name: bareos-fd
         uname: 25.1.2~pre43.18be77e68 (09Oct26) Debian GNU/Linux 13 (trixie),debian
     autoprune: 0
 fileretention: 5184000
  jobretention: 15552000

      clientid: 2
          name: parity-ndmp-fd
         uname: 
     autoprune: 0
 fileretention: 0
  jobretention: 0

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

Die Meldungen sind dieselben wie bei [list](list.md#fehlermeldungen), mit dem Befehlsnamen `llist`:

| Meldung | Ursache und Abhilfe |
|---|---|
| `llist command requires a keyword` | `llist` ohne Schlüsselwort. |
| `llist: is an invalid command.` | Die Konsole darf `llist` nicht ausführen (Command-ACL). |

Wo `list` `No results to list.` meldet, bleibt die Antwort von `llist` bei manchen Schlüsselwörtern leer, etwa bei `llist pool=<unbekannt>` oder `llist ujobid=<unbekannt>`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> llist pool=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Siehe auch

- [list](list.md) — dieselben Daten als kompakte Tabelle, mit allen Argumenten
- [show](show.md) — Ressourcen aus der Konfiguration
- [sqlquery](sqlquery.md) — eigene SQL-Abfragen
- [dot-api](dot-api.md) — API-Modus umschalten
- [gui](gui.md) — GUI-Modus ein- und ausschalten
