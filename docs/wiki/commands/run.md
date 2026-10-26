# run — Job starten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L431) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Run a job

**Originalsyntax:**

```text
run job=<job-name> client=<client-name> fileset=<fileset-name> level=<level> storage=<storage-name> where=<directory-prefix> when=<universal-time-specification> pool=<pool-name> pluginoptions=<plugin-options-string> accurate=<yes|no> comment=<text> spooldata=<yes|no> priority=<number> jobid=<jobid> catalog=<catalog> migrationjob=<job-name> backupclient=<client-name> backupformat=<format> nextpool=<pool-name> since=<universal-time-specification> verifyjob=<job-name> verifylist=<verify-list> migrationjob=<complete_name> [ yes ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.
- `run job=RestoreFiles` fragt im Original nach der JobId (C-043); Onesimus geht direkt zu „OK to run?“.
- Der Paritätslauf startet keine echten Jobs (nur Zusammenfassung, `mod`/`no`/Abbruch). Jobablauf, Jobmeldungen (C-051) und CatReq-Updates (C-055) sind nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-043`, `C-051`, `C-055` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (30):** `c011-run-cancel`, `c011-run-badnumber`, `c011-run-api1-cancel`, `c011-run-api2`, `c011-run-job-no`, `c011-run-job-cancel`, `c011-run-job-illegal-no`, `c011-run-job-mod-cancel`, `c011-run-job-mod-level-no`, `c011-run-job-mod-priority-no`, `c011-run-job-mod-pool-no`, `c011-run-job-dotmod-no`, `c011-run-select-no`, `c011-run-job-api1-no`, `c012-run-unknown-noyes`, `c012-run-confirm`, `c025-run-select-limited`, `c025-run-select-limited-api2`, `c025-run-select-operator`, `c025-run-select-operator-api2`, `c025-run-select-bvfs`, `c025-run-select-bvfs-api2`, `c025-run-select-acl`, `c025-run-select-acl-api2`, `c041-run-backupcatalog-no`, `c041-run-backupcatalog-api1-no`, `c041-run-backupcatalog-mod-cancel`, `c041-run-backupfd-no`, `run-noargs`, `run-unknownjob`.

**Lokaler Handler:** [crates/odir/src/commands/job.rs:13](../../../crates/odir/src/commands/job.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`run` startet einen Job, der in der Director-Konfiguration als Job-Ressource definiert ist: Backup, Restore, Verify, Admin, Copy, Migrate oder Consolidate. Vor dem Start zeigt der Director eine Zusammenfassung der Jobparameter und fragt `OK to run? (yes/mod/no):`. Mit `mod` lassen sich die Parameter vor dem Start ändern, mit `yes` als Argument entfällt die Frage. Ein gestarteter Job legt einen Job-Eintrag im Katalog an und erscheint in [status](status.md) und [list jobs](list.md); seine Meldungen holt man mit [messages](messages.md) ab.

Was Onesimus heute starten kann (Kompatibilitätslog C-058):

| Jobtyp | Start mit `run` | Hinweis |
|---|---|---|
| Backup | ja | Full, Incremental, Differential, Since, VirtualFull |
| Admin | ja | keine Verbindung zu FD oder SD |
| Restore | ja | über `run job=<Restore-Job> jobid=…` oder `bootstrap=…`; Bootstrap-Erzeugung und Ablauf sind nicht gegen das Original geprüft |
| Copy, Migrate | nur der Steuerjob | Auswahl der Jobs nach Selection Type (außer PoolOccupancy); die Arbeitsjobs, die die Daten kopieren, startet Onesimus noch nicht |
| Consolidate | ja, ohne Always-Incremental-Jobs | gibt es Jobs mit `Always Incremental = yes`, wird der Start abgelehnt |
| Verify | nein | alle Level brauchen FD oder SD |

Before- und AfterJob-RunScripts laufen für keinen Jobtyp.

### Zeitgrenzen und Warten auf ein Volume

Ein gestarteter Job unterliegt den Zeitgrenzen seiner Job-Ressource. Der Director prüft sie wie das Original alle 60 Sekunden und bricht einen Job, der eine Grenze überschreitet, ab wie mit [cancel](cancel.md):

| Direktive | Wirkung |
|---|---|
| `Max Wait Time` | Höchstdauer, die ein laufender Job insgesamt in Wartezuständen verbringt (auf File oder Storage Daemon, auf ein Volume oder einen Mount). Meldung `Fatal error: Max wait time exceeded. Job canceled.` |
| `Max Run Time`, `Full/Incremental/Differential Max Runtime` | Höchstdauer seit dem Start des Jobs, wahlweise je Level. |
| `Max Start Delay` | Wie lange ein Job nach seiner geplanten Startzeit noch starten darf. |
| `Max Run Sched Time` (in einer `Run`-Direktive) | Höchstdauer seit der geplanten Startzeit. |

**Onesimus-Standard (abweichend vom Original):** Hat der Job keine `Max Wait Time`, wartet er im Original unbegrenzt auf ein Volume (etwa bei `Please mount append Volume … or label a new one` des Storage Daemons). Onesimus bricht einen solchen Job ab, sobald er insgesamt 10 Minuten (600 Sekunden) blockiert auf ein Volume gewartet hat. Gezählt wird nur die Zeit, in der der Job oder sein Storage-Daemon-Job auf Medien wartet (Status „waiting for media“ / „waiting for mount“). Die Meldung lautet `Fatal error: Max wait time exceeded. Job canceled. (Onesimus default: no Max Wait Time configured, the job waited 600 seconds for a volume)`. Eine konfigurierte `Max Wait Time` hat immer Vorrang; wer unbegrenzt warten will, setzt einen großen Wert. (Kompatibilitätslog C-085)

## Syntax

```text
run
run job=<job> [yes]
run job=<job> [level=<level>] [client=<client>] [fileset=<fileset>] [pool=<pool>] [storage=<storage>] [when="<JJJJ-MM-TT hh:mm:ss>"] [priority=<n>] [comment=<text>] [yes]
run job=<job> [accurate=yes|no] [spooldata=yes|no] [ignoreduplicatecheck=yes|no] [pluginoptions=<text>] [backupformat=<format>] [since="<JJJJ-MM-TT hh:mm:ss>"] [yes]
run job=<backup-job> level=VirtualFull [nextpool=<pool>] [consolidatejob=<job>] [yes]
run job=<restore-job> jobid=<n> | bootstrap=<datei> [where=<pfad> | regexwhere=<ausdruck>] [replace=always|ifnewer|ifolder|never] [restoreclient=<client>] [backupclient=<client>] [catalog=<katalog>] [yes]
run job=<verify-job> [level=<level>] [verifyjob=<job>] [jobid=<n>] [verifylist=<liste>] [yes]
run job=<copy-oder-migrate-job> [pool=<pool>] [nextpool=<pool>] [migrationjob=<job>] [jobid=<n>] [yes]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `job=<job>` | Name einer Job-Ressource | Der zu startende Job. Fehlt er oder ist er unbekannt, zeigt der Director eine Auswahlliste. Zweimal angegeben: `Job name specified twice.` |
| `level=<level>` | Backup: `Full`, `Incremental`, `Differential`, `Since`, `VirtualFull`; Verify: `InitCatalog`, `Catalog`, `VolumeToCatalog`, `DiskToCatalog`, `Data` | Überschreibt das Level aus der Job-Ressource. |
| `client=<client>`, `fd=<client>` | Name einer Client-Ressource | Client für den Job. Unbekannter Name: Auswahlliste der Clients. |
| `fileset=<fileset>` | Name einer FileSet-Ressource | FileSet für den Job. |
| `pool=<pool>` | Name einer Pool-Ressource | Pool, in den geschrieben (Backup) bzw. aus dem gelesen wird (Copy, Migrate). In der Zusammenfassung steht dann `(From command line)`. |
| `nextpool=<pool>` | Name einer Pool-Ressource | Zielpool für VirtualFull, Copy und Migrate. |
| `storage=<storage>`, `sd=<storage>` | Name einer Storage-Ressource | Storage für den Job. Unbekannter Name: Auswahlliste. |
| `when="<zeit>"` | `JJJJ-MM-TT hh:mm:ss` | Startzeit. Ungültige Angaben ersetzt der Director durch die aktuelle Zeit (`Invalid time, using current time.`). Einzelne Ziffern ohne führende Null werden angenommen. |
| `priority=<n>` | Ganzzahl größer 0 | Jobpriorität (kleiner = früher). |
| `comment=<text>` | Text ohne `'` | Kommentar im Job-Eintrag. |
| `accurate=yes\|no` | `yes`, `no` | Accurate-Modus für diesen Lauf. |
| `spooldata=yes\|no` | `yes`, `no` | Daten-Spooling für diesen Lauf. |
| `ignoreduplicatecheck=yes\|no` | `yes`, `no` | Prüfung auf doppelt laufende Jobs abschalten. |
| `pluginoptions=<text>` | Text | Plugin-Optionen; erscheinen als Zeile `Plugin Options:` in der Zusammenfassung. |
| `backupformat=<format>` | z. B. `Native` | Backup-Format. |
| `since="<zeit>"` | `JJJJ-MM-TT hh:mm:ss` | Zeitpunkt für Level `Since`. Andere Formate: `Since option expects string in format "yyyy-mm-dd hh:mm:ss", but is: …` |
| `consolidatejob=<job>` | Name eines Consolidate-Jobs | Nur mit `level=VirtualFull`. |
| `jobid=<n>` | JobId | Restore: der wiederherzustellende Job; Verify, Copy, Migrate: der zu prüfende bzw. zu kopierende Job. |
| `bootstrap=<datei>` | Pfad | Restore: vorhandene Bootstrap-Datei statt JobId. |
| `where=<pfad>` | Pfad | Restore: Zielverzeichnis (`/` = am Originalort). |
| `regexwhere=<ausdruck>` | `!von!nach!` | Restore: Pfade per regulärem Ausdruck umschreiben. Nicht zusammen mit `where=`. |
| `replace=<wert>` | `always`, `ifnewer`, `ifolder`, `never` | Restore: was mit vorhandenen Dateien geschieht. |
| `restoreclient=<client>` | Client | Restore: Client, auf den zurückgeschrieben wird. |
| `backupclient=<client>` | Client | Restore: Client, dessen Daten wiederhergestellt werden. |
| `catalog=<katalog>` | Name einer Catalog-Ressource | Katalog für den Job. |
| `verifyjob=<job>` | Job | Verify: der zu prüfende Backup-Job. |
| `verifylist=<liste>` | Text | Verify: Liste für das Verify. |
| `migrationjob=<job>` | Job | Copy, Migrate: Job, dessen Daten verschoben werden. |
| `yes` | – | Ohne Rückfrage starten. |

Unbekannte Schlüsselwörter beendet der Director mit `Invalid keyword: <wort>`, ein Schlüsselwort ohne Wert mit `Value missing for keyword <wort>`.

## Dialoge

**Jobauswahl.** Ohne `job=` oder mit unbekanntem Jobnamen zeigt der Director alle Jobs, die die Konsole sehen darf, und fragt `Select Job resource (1-n):`. Gültig ist eine Nummer aus der Liste. `0` oder eine zu große Zahl führt zu `Please enter a number between 1 and n`, ein Wort zu `Expected a positive integer, got: …`; die Frage kommt dann erneut. `.` bricht ab (`Selection aborted, nothing done.`). Die Auswahl erscheint auch bei `run yes` ohne Jobnamen.

**OK to run? (yes/mod/no).** Nach der Zusammenfassung gelten:

- `yes` (auch abgekürzt, etwa `y`): Job starten, Antwort `Job queued. JobId=<n>`.
- `no` (auch abgekürzt, etwa `n`) und `.`: nichts starten, Antwort `Job not run.`
- `mod`: Parameter ändern (siehe unten), danach kommt die Zusammenfassung erneut.
- `.mod <schlüssel>=<wert> …`: Parameter direkt ändern, ohne Menü; es gelten dieselben Schlüsselwörter wie auf der Befehlszeile.
- Alles andere: `Illegal response <eingabe>`, die Frage kommt erneut.

**mod-Menü.** `Select parameter to modify (1-n):` bietet je nach Jobtyp:

| Jobtyp | Menüpunkte |
|---|---|
| Backup | Level, Storage, Job, FileSet, Client, Backup Format, When, Priority, Pool, Plugin Options; bei VirtualFull zusätzlich NextPool (11 Punkte) |
| Admin, Consolidate | Level, Storage, Job, FileSet, Client, Backup Format, When, Priority |
| Restore | Level, Storage, Job, FileSet, Restore Client, Backup Format, When, Priority, Bootstrap, Where, File Relocation, Replace, JobId, Plugin Options |
| Verify | Level, Storage, Job, FileSet, Client, Backup Format, When, Priority, Pool, Verify Job |
| Copy, Migrate | Level, Storage, Job, FileSet, Client, Backup Format, When, Priority, Pool, NextPool |

Die einzelnen Punkte:

- **Level**: Auswahlliste der Level (Backup: Full, Incremental, Differential, Since, VirtualFull; Verify: Initialize Catalog, Verify Catalog, Verify Volume to Catalog, Verify Disk to Catalog, Verify Volume Data). Bei Admin, Restore und Consolidate: `Level not appropriate for this Job. Cannot be changed.`
- **Storage**, **FileSet**, **Client**/**Restore Client**, **Pool**, **NextPool**: Auswahlliste der jeweiligen Ressourcen.
- **Job**: Auswahlliste aller Jobs. Ein Job desselben Typs ersetzt den aktuellen; bei einem Job anderen Typs endet der Dialog mit `Job not run.`
- **Backup Format**: `Please enter Backup Format:` (freier Text).
- **When**: `Please enter desired start time as YYYY-MM-DD HH:MM:SS (return for now):`; leere Eingabe bedeutet „jetzt“.
- **Priority**: `Enter new Priority:`; 0 oder negative Werte werden mit `Priority must be a positive integer.` abgelehnt.
- **Plugin Options**: `Please enter Plugin Options string:`.
- **Bootstrap**: `Please enter the Bootstrap file name:`; eine nicht lesbare Datei meldet `Warning cannot open <datei>: ERR=…`.
- **Where**: `Please enter the full path prefix for restore (/ for none):`.
- **File Relocation**: Untermenü mit Strip prefix, Add prefix, Add file suffix, Enter a regexp, Test filename manipulation und `Use this ?`. Mit dem Testpunkt lassen sich Dateinamen ausprobieren (`.` beendet den Test). `Use this ?` übernimmt das Ergebnis als `RegexWhere`.
- **Replace**: Auswahl Always, IfNewer, IfOlder, Never.
- **JobId**: `Please enter a JobId for restore:`.
- **Verify Job**: Auswahlliste der Jobs.

`.` in einer Auswahlliste des mod-Menüs bricht den ganzen Dialog ab (`Selection aborted, nothing done.` und `Job not run.`).

**JobId für Restore.** Ein Restore-Job ohne `jobid=` oder `bootstrap=` fragt zuerst `Please enter a JobId for restore:`; `.` beendet mit `Job not run.`

## API-Modi

- **`.api 0`** (Standard): Zusammenfassung und Frage wie in den Beispielen.
- **`.api 1`**: Die Zusammenfassung beginnt mit `Type:` und `Title:`, die Quellenangaben `(From …)` fehlen, und der Director fordert die Antwort als freie Texteingabe an (oconsole zeigt `input> `). Die Auswahllisten kommen als Liste für grafische Oberflächen; in oconsole sind sie nicht bedienbar.
- **`.api 2`** und **`.api json compact=yes`**: Es gibt **keine Rückfrage**. `run job=<job>` startet den Job sofort, auch ohne `yes`, und antwortet mit `result.run.jobid`. Fehler (unbekannter Job, fehlender Jobname, ungültige Werte) kommen als JSON-RPC-Fehler `"message": "failed"`. Unter `.api 2` daher nur Befehle senden, die wirklich laufen sollen.

## Beispiele

### Job aus einer Liste auswählen

Ohne Jobnamen zeigt der Director alle Jobs; hier wird `BackupCatalog` gewählt und nicht gestartet.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
A job name must be specified.
The defined Job resources are:
 1: backup-bareos-fd
 2: BackupCatalog
 3: parity-admin
 4: parity-consolidate
 5: parity-copy
 6: parity-migrate
 7: parity-restore-regex
 8: parity-verify
 9: RestoreFiles
Select Job resource (1-9):
> 2
Run Backup job
JobName:  BackupCatalog
Level:    Full
Client:   bareos-fd
Format:   Native
FileSet:  Catalog
Pool:     Full (From Job FullPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:29
Priority: 11
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Backup-Job mit Rückfrage, nicht starten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:25
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Parameter auf der Befehlszeile setzen

Werte von der Befehlszeile erscheinen in der Zusammenfassung mit `(From command line)`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd level=Full client=bareos-fd fileset=SelfTest pool=Full storage=File priority=5 comment=doku spooldata=no accurate=yes
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Full
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Full (From command line)
Storage:  File (From command line)
When:     2026-10-16 23:09:16
Priority: 5
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Startzeit mit when=

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd when="2030-01-02 03:04:05"
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2030-01-02 03:04:05
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Eine ungültige Zeitangabe wird durch die aktuelle Zeit ersetzt:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd when="2030-01-02 03:04:05x"
Using Catalog "MyCatalog"
Invalid time, using current time.
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:45:55
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### VirtualFull mit NextPool

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd level=VirtualFull nextpool=Scratch
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    VirtualFull
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job resource)
NextPool: Scratch (From command line)
Storage:  File (From Job resource)
When:     2026-10-16 22:45:46
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Plugin-Optionen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd pluginoptions=a:b
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:45:48
Priority: 10
Plugin Options: a:b
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige Antwort und Abbruch mit Punkt

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:25
Priority: 10
OK to run? (yes/mod/no):
> yes please
Illegal response yes please
OK to run? (yes/mod/no):
> n
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:25
Priority: 10
OK to run? (yes/mod/no):
> .
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Level ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:26
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 1
Levels:
1: Full
2: Incremental
3: Differential
4: Since
5: VirtualFull
Select level (1-5):
> 1
Run Backup job
JobName:  backup-bareos-fd
Level:    Full
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Full (From Job FullPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:26
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Level Since

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:14
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 1
Levels:
1: Full
2: Incremental
3: Differential
4: Since
5: VirtualFull
Select level (1-5):
> 4
Run Backup job
JobName:  backup-bareos-fd
Level:    Since
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:14
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Storage ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:11
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 2
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> 1
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From user selection)
When:     2026-10-16 23:09:11
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: anderen Job wählen

Ein Job desselben Typs (hier `BackupCatalog`) übernimmt dessen Einstellungen:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:15
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 3
The defined Job resources are:
 1: backup-bareos-fd
 2: BackupCatalog
 3: parity-admin
 4: parity-consolidate
 5: parity-copy
 6: parity-migrate
 7: parity-restore-regex
 8: parity-verify
 9: RestoreFiles
Select Job resource (1-9):
> 2
Run Backup job
JobName:  BackupCatalog
Level:    Full
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job resource)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:15
Priority: 11
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein Job anderen Typs beendet den Dialog:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:45:51
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 3
The defined Job resources are:
 1: backup-bareos-fd
 2: BackupCatalog
 3: parity-admin
 4: parity-consolidate
 5: parity-copy
 6: parity-migrate
 7: parity-restore-regex
 8: parity-verify
 9: RestoreFiles
Select Job resource (1-9):
> 6
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: FileSet ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:12
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 4
The defined FileSet resources are:
1: Catalog
2: LinuxAll
3: SelfTest
4: Windows All Drives
Select FileSet resource (1-4):
> 4
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  Windows All Drives
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:12
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Client ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:12
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 5
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2):
> 1
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:12
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Backup Format

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:13
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 6
Please enter Backup Format:
> Native
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:13
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Startzeit

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:45:51
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 7
Please enter desired start time as YYYY-MM-DD HH:MM:SS (return for now):
> 2030-01-02 03:04:05
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2030-01-02 03:04:05
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Eine leere Eingabe setzt die Startzeit auf „jetzt“:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd when="2030-01-02 03:04:05"
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2030-01-02 03:04:05
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 7
Please enter desired start time as YYYY-MM-DD HH:MM:SS (return for now):
>
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:15
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Priorität, mit ungültigem Wert

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:27
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 8
Enter new Priority:
> 0
Priority must be a positive integer.
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:27
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 8
Enter new Priority:
> 7
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:27
Priority: 7
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Pool

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:28
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 9
The defined Pool resources are:
1: Differential
2: Full
3: Incremental
4: Scratch
Select Pool resource (1-4):
> 2
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Full (From user input)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:28
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: Plugin Options

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:14
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> 10
Please enter Plugin Options string:
> a:b
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:14
Priority: 10
Plugin Options: a:b
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod: NextPool bei VirtualFull

Nach der Auswahl des NextPools endet der Dialog wie im Original mit `Job not run.`; der Job muss mit `nextpool=` auf der Befehlszeile neu aufgerufen werden.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd level=VirtualFull nextpool=Scratch
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    VirtualFull
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job resource)
NextPool: Scratch (From command line)
Storage:  File (From Job resource)
When:     2026-10-16 23:09:16
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: NextPool
11: Plugin Options
Select parameter to modify (1-11):
> 10
The defined Pool resources are:
1: Differential
2: Full
3: Incremental
4: Scratch
Select Pool resource (1-4):
> 2
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### mod abbrechen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:26
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Plugin Options
Select parameter to modify (1-10):
> .
Selection aborted, nothing done.
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Parameter mit .mod direkt ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Using Catalog "MyCatalog"
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:28
Priority: 10
OK to run? (yes/mod/no):
> .mod level=Full
Run Backup job
JobName:  backup-bareos-fd
Level:    Full
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:37:28
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Admin-Job starten

Ein Admin-Job braucht weder Client noch Storage. Mit `yes` startet er ohne Rückfrage; seine Meldungen kommen über `messages`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin yes
Using Catalog "MyCatalog"
Job queued. JobId=630
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait
You have messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
16-Oct 22:50 bareos-dir JobId 630: Version: 25.1.2+onesimus.0.2.0 (16 October 2026) Debian GNU/Linux 13 (trixie)
16-Oct 22:50 bareos-dir JobId 630: Start Admin JobId 630, Job=parity-admin.2026-10-16_22.50.08_01
16-Oct 22:50 bareos-dir JobId 630: BAREOS 25.1.2+onesimus.0.2.0 (16Oct26): 16-Oct-2026 22:50:10
  JobId:                  630
  Job:                    parity-admin.2026-10-16_22.50.08_01
  Scheduled time:         16-Oct-2026 22:50:08
  Start time:             16-Oct-2026 22:50:10
  End time:               16-Oct-2026 22:50:10
  Bareos binary info:     Onesimus build: Support: https://onesimus.io
  Job triggered by:       User
  Termination:            Admin OK

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Interaktiv gestartet:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin
Using Catalog "MyCatalog"
Run Admin Job
JobName:  parity-admin
FileSet:  SelfTest
Client:   bareos-fd
Storage:  File
When:     2026-10-16 22:50:10
Priority: 10
OK to run? (yes/mod/no):
> yes
Job queued. JobId=631
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein mit `disable` abgeschalteter Job wird nur vom Zeitplan nicht mehr gestartet; `run` startet ihn trotzdem:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=parity-admin
Job "parity-admin" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin yes
Using Catalog "MyCatalog"
Job queued. JobId=634
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Admin-Job: Level nicht änderbar

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin
Using Catalog "MyCatalog"
Run Admin Job
JobName:  parity-admin
FileSet:  SelfTest
Client:   bareos-fd
Storage:  File
When:     2026-10-16 22:45:35
Priority: 10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
1: Level
2: Storage
3: Job
4: FileSet
5: Client
6: Backup Format
7: When
8: Priority
Select parameter to modify (1-8):
> 1
Level not appropriate for this Job. Cannot be changed.
Run Admin Job
JobName:  parity-admin
FileSet:  SelfTest
Client:   bareos-fd
Storage:  File
When:     2026-10-16 22:45:35
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job für später einplanen

Der Job wartet bis zur angegebenen Zeit; [cancel](cancel.md) bricht ihn vorher ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin when="2099-01-01 00:00:00" yes
Using Catalog "MyCatalog"
Job queued. JobId=635
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore-Job: JobId abfragen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=RestoreFiles
Using Catalog "MyCatalog"
Please enter a JobId for restore:
> 1
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:05
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore-Job mit jobid=

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=RestoreFiles jobid=1
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:05
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beispielausgabe nicht verfügbar: ein Restore-Start würde Daten über den echten File Daemon zurückschreiben.

### Restore: Zielverzeichnis ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=RestoreFiles jobid=1
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:17
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Restore Client
 6: Backup Format
 7: When
 8: Priority
 9: Bootstrap
10: Where
11: File Relocation
12: Replace
13: JobId
14: Plugin Options
Select parameter to modify (1-14):
> 10
Please enter the full path prefix for restore (/ for none):
> /tmp/new
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/new
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:17
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore: Replace ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=RestoreFiles jobid=1
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:19
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Restore Client
 6: Backup Format
 7: When
 8: Priority
 9: Bootstrap
10: Where
11: File Relocation
12: Replace
13: JobId
14: Plugin Options
Select parameter to modify (1-14):
> 12
Replace:
1: Always
2: IfNewer
3: IfOlder
4: Never
Select replace option (1-4):
> 3
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         IfOlder
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:19
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore: File Relocation

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=RestoreFiles jobid=1
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:21
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Restore Client
 6: Backup Format
 7: When
 8: Priority
 9: Bootstrap
10: Where
11: File Relocation
12: Replace
13: JobId
14: Plugin Options
Select parameter to modify (1-14):
> 11
strip_prefix=*None* add_prefix=*None* add_suffix=*None*
This will replace your current Where value
1: Strip prefix
2: Add prefix
3: Add file suffix
4: Enter a regexp
5: Test filename manipulation
6: Use this ?
Select parameter to modify (1-6):
> 1
Please enter the path prefix to strip:
> /tmp
strip_prefix=/tmp add_prefix=*None* add_suffix=*None*
This will replace your current Where value
1: Strip prefix
2: Add prefix
3: Add file suffix
4: Enter a regexp
5: Test filename manipulation
6: Use this ?
Select parameter to modify (1-6):
> 2
Please enter the path prefix to add (/ for none):
> /new
strip_prefix=/tmp add_prefix=/new add_suffix=*None*
This will replace your current Where value
1: Strip prefix
2: Add prefix
3: Add file suffix
4: Enter a regexp
5: Test filename manipulation
6: Use this ?
Select parameter to modify (1-6):
> 3
Please enter the file suffix to add:
> .rst
strip_prefix=/tmp add_prefix=/new add_suffix=.rst
This will replace your current Where value
1: Strip prefix
2: Add prefix
3: Add file suffix
4: Enter a regexp
5: Test filename manipulation
6: Use this ?
Select parameter to modify (1-6):
> 5
strip_prefix=/tmp add_prefix=/new add_suffix=.rst result=!/tmp!!i,!([^/])$!$1.rst!,!^!/new!
Enter a period (.) to stop this test
Please enter filename to test:
> /tmp/a/b
/tmp/a/b -> /new/a/b.rst
Please enter filename to test:
> .
strip_prefix=/tmp add_prefix=/new add_suffix=.rst
This will replace your current Where value
1: Strip prefix
2: Add prefix
3: Add file suffix
4: Enter a regexp
5: Test filename manipulation
6: Use this ?
Select parameter to modify (1-6):
> 6
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
RegexWhere:      !/tmp!!i,!([^/])$!$1.rst!,!^!/new!
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:21
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore: Bootstrap-Datei

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=RestoreFiles jobid=1
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:20
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Restore Client
 6: Backup Format
 7: When
 8: Priority
 9: Bootstrap
10: Where
11: File Relocation
12: Replace
13: JobId
14: Plugin Options
Select parameter to modify (1-14):
> 9
Please enter the Bootstrap file name:
> /nonexistent/c043.bsr
Warning cannot open /nonexistent/c043.bsr: ERR=No such file or directory
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:20
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore: JobId ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=RestoreFiles jobid=1
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:19
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Restore Client
 6: Backup Format
 7: When
 8: Priority
 9: Bootstrap
10: Where
11: File Relocation
12: Replace
13: JobId
14: Plugin Options
Select parameter to modify (1-14):
> 13
Please enter a JobId for restore:
> 1
Run Restore job
JobName:         RestoreFiles
Bootstrap:       *None*
Where:           /tmp/bareos-restores
Replace:         Always
Client:          bareos-fd
Format:          Native
Storage:         File
JobId:           1
When:            2026-10-16 22:45:19
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore-Job mit RegexWhere aus der Konfiguration

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-restore-regex
Using Catalog "MyCatalog"
Run Restore job
JobName:         parity-restore-regex
Bootstrap:       /nonexistent/parity-c043.bsr
RegexWhere:      !/!/tmp/rx/!
Replace:         IfNewer
FileSet:         LinuxAll
Backup Client:   (null)
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 22:45:32
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Verify-Job

Verify-Jobs zeigt Onesimus an und lässt sie ändern, startet sie aber noch nicht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-verify
Using Catalog "MyCatalog"
Run Verify Job
JobName:     parity-verify
Level:       VolumeToCatalog
Client:      bareos-fd
FileSet:     SelfTest
Pool:        Incremental (From Job resource)
Storage:     File (From Job resource)
Verify Job:  backup-bareos-fd
Verify List:
When:        2026-10-16 22:48:12
Priority:    10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-verify
Using Catalog "MyCatalog"
Run Verify Job
JobName:     parity-verify
Level:       VolumeToCatalog
Client:      bareos-fd
FileSet:     SelfTest
Pool:        Incremental (From Job resource)
Storage:     File (From Job resource)
Verify Job:  backup-bareos-fd
Verify List:
When:        2026-10-16 22:45:37
Priority:    10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: Verify Job
Select parameter to modify (1-10):
> 1
Levels:
1: Initialize Catalog
2: Verify Catalog
3: Verify Volume to Catalog
4: Verify Disk to Catalog
5: Verify Volume Data (not yet implemented)
Select level (1-5):
> 4
Run Verify Job
JobName:     parity-verify
Level:       DiskToCatalog
Client:      bareos-fd
FileSet:     SelfTest
Pool:        Incremental (From Job resource)
Storage:     File (From Job resource)
Verify Job:  backup-bareos-fd
Verify List:
When:        2026-10-16 22:45:37
Priority:    10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beispielausgabe nicht verfügbar: `run job=<verify-job> yes` antwortet in Onesimus mit `Running Verify jobs is not available yet.`; im Original würde der Fall einen echten Verify gegen FD bzw. SD starten.

### Copy- und Migrate-Steuerjob

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-copy
Using Catalog "MyCatalog"
Run Copy job
JobName:       parity-copy
Bootstrap:     *None*
Read Storage:  File (From Job resource)
Pool:          Full (From Job resource)
Write Storage: File (From Storage from Run NextPool override)
NextPool:      Scratch (From user input)
JobId:         *None*
When:          2026-10-16 22:48:12
Catalog:       MyCatalog
Priority:      10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-migrate
Using Catalog "MyCatalog"
Run Migration job
JobName:       parity-migrate
Bootstrap:     *None*
Read Storage:  File (From Job resource)
Pool:          Full (From Job resource)
Write Storage: File (From Storage from Run NextPool override)
NextPool:      Scratch (From user input)
JobId:         *None*
When:          2026-10-16 22:45:43
Catalog:       MyCatalog
Priority:      10
OK to run? (yes/mod/no):
> mod
Parameters to modify:
 1: Level
 2: Storage
 3: Job
 4: FileSet
 5: Client
 6: Backup Format
 7: When
 8: Priority
 9: Pool
10: NextPool
Select parameter to modify (1-10):
> 10
The defined Pool resources are:
1: Differential
2: Full
3: Incremental
4: Scratch
Select Pool resource (1-4):
> 4
Run Migration job
JobName:       parity-migrate
Bootstrap:     *None*
Read Storage:  File (From Job resource)
Pool:          Full (From Job resource)
Write Storage: File (From Storage from Run NextPool override)
NextPool:      Scratch (From user input)
JobId:         *None*
When:          2026-10-16 22:45:43
Catalog:       MyCatalog
Priority:      10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Der Steuerjob wählt die Jobs nach dem Selection Type aus. Findet er keine, endet er mit `no files to migrate`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-migrate yes
Using Catalog "MyCatalog"
Job queued. JobId=636
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait
You have messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
16-Oct 22:50 bareos-dir JobId 636: Version: 25.1.2+onesimus.0.2.0 (16 October 2026) Debian GNU/Linux 13 (trixie)
16-Oct 22:50 bareos-dir JobId 636: Regex pattern matched no Jobs to migrate.
16-Oct 22:50 bareos-dir JobId 636: No JobIds found to migrate.
16-Oct 22:50 bareos-dir JobId 636: Bareos bareos-dir 25.1.2+onesimus.0.2.0 (16Oct26):
  OS Information:         Debian GNU/Linux 13 (trixie)
  Current JobId:          636
  Current Job:            parity-migrate.2026-10-16_22.50.53_07
  Catalog:                "MyCatalog" (From Client resource)
  Start time:             16-Oct-2026 22:50:55
  End time:               16-Oct-2026 22:50:55
  Elapsed time:           0 secs
  Priority:               10
  Bareos binary info:     Onesimus build: Support: https://onesimus.io
  Job triggered by:       User
  Termination:            Migration -- no files to migrate

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Consolidate-Job

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-consolidate yes
Using Catalog "MyCatalog"
Job queued. JobId=638
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait
You have messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
16-Oct 22:51 bareos-dir JobId 638: Version: 25.1.2+onesimus.0.2.0 (16 October 2026) Debian GNU/Linux 13 (trixie)
16-Oct 22:51 bareos-dir JobId 638: Start Consolidate JobId 638, Job=parity-consolidate.2026-10-16_22.50.59_09
16-Oct 22:51 bareos-dir JobId 638: BAREOS 25.1.2+onesimus.0.2.0 (16Oct26): 16-Oct-2026 22:51:01
  JobId:                  638
  Job:                    parity-consolidate.2026-10-16_22.50.59_09
  Scheduled time:         16-Oct-2026 22:50:59
  Start time:             16-Oct-2026 22:51:01
  End time:               16-Oct-2026 22:51:01
  Bareos binary info:     Onesimus build: Support: https://onesimus.io
  Job triggered by:       User
  Termination:            Consolidate OK

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ausgabe unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=backup-bareos-fd
Type: Backup
Title: Run Backup Job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental
Storage:  File
When:     2026-10-16 22:37:29
Priority: 10
input> no
OK to run? (yes/mod/no): Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Start unter .api 2

Unter `.api 2` startet `run` ohne Rückfrage.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "run": {
      "jobid": "632"
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Dasselbe mit kompaktem JSON:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin yes
{"jsonrpc":"2.0","id":null,"result":{"run":{"jobid":"630"}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait
{"jsonrpc":"2.0","id":null,"result":{}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=nosuch
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {}
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne run-Recht

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> run
run: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `A job name must be specified.` | Kein `job=` angegeben; es folgt die Auswahlliste. |
| `Job "<name>" not found` | Unbekannter Jobname; es folgt die Auswahlliste. |
| `Job name specified twice.` | `job=` mehrfach angegeben. |
| `Level "<level>" not valid.` | Level passt nicht zum Jobtyp. Der Job läuft nicht (`Job not run.`). |
| `Invalid time, using current time.` | `when=` nicht im Format `JJJJ-MM-TT hh:mm:ss` oder kein gültiges Datum; der Job startet sofort. |
| `Since option expects string in format "yyyy-mm-dd hh:mm:ss", but is: <wert>` | Falsches Format bei `since=`. |
| `Client "<name>" not found.` | Unbekannter Client; im Dialog folgt die Client-Auswahl, bei Restore endet der Befehl. |
| `Storage "<name>" not found.` | Unbekannter Storage; es folgt die Storage-Auswahl. |
| `Catalog "<name>" not found` | Unbekannter Katalog. |
| `JobId <n> not found in catalog.` | Restore oder Verify mit nicht vorhandener JobId. |
| `Where or RegexWhere specified twice.` | `where=` und `regexwhere=` zusammen oder doppelt. |
| `Invalid replace option: <wert>` | `replace=` mit anderem Wert als always, ifnewer, ifolder, never. |
| `Invalid spooldata flag.` | `spooldata=` nicht `yes` oder `no`. |
| `Illegal character "'" in a comment.` | Hochkomma in `comment=`. |
| `Invalid keyword: <wort>` | Unbekanntes oder für den Jobtyp unzulässiges Schlüsselwort. |
| `Value missing for keyword <wort>` | Schlüsselwort ohne `=wert`. |
| `Consolidate Job option is only valid for virtual full jobs.` | `consolidatejob=` ohne `level=VirtualFull`. |
| `Invalid Consolidate Job "<job>". Job type is "<t>" but expected "O".` | `consolidatejob=` nennt keinen Consolidate-Job. |
| `Migration Job "<name>" not found.` / `Verify Job "<name>" not found.` | Unbekannter Job bei `migrationjob=` bzw. `verifyjob=`; es folgt die Auswahl. |
| `Level not appropriate for this Job. Cannot be changed.` | mod-Punkt Level bei Admin, Restore oder Consolidate. |
| `Priority must be a positive integer.` | mod-Punkt Priority mit 0 oder negativem Wert. |
| `Illegal response <eingabe>` | Antwort auf `OK to run?` war nicht yes, mod oder no. |
| `Job not run.` | Mit `no` oder `.` beantwortet oder Dialog abgebrochen. |
| `Running Verify jobs is not available yet.` | Onesimus startet Verify-Jobs noch nicht (C-058). |
| `Running Copy jobs for a JobId is not available yet.` / `Running Migrate jobs for a JobId is not available yet.` | Copy/Migrate mit `jobid=` (Arbeitsjob) ist noch nicht umgesetzt (C-058). |
| `Consolidating Always Incremental jobs is not available yet.` | Ein Consolidate-Job wird abgelehnt, solange Jobs mit `Always Incremental = yes` existieren (C-058). |
| `run: is an invalid command.` | Die Konsole darf `run` nicht ausführen (Command ACL). |

## Siehe auch

- [rerun](rerun.md) — frühere Jobs mit denselben Parametern erneut starten
- [cancel](cancel.md) — laufende oder wartende Jobs abbrechen
- [restore](restore.md) — Dateien interaktiv auswählen und wiederherstellen
- [estimate](estimate.md) — Umfang eines Backups vorab schätzen
- [enable](enable.md) / [disable](disable.md) — Jobs für den Zeitplan ein- und ausschalten
- [status](status.md) — laufende und geplante Jobs anzeigen
- [messages](messages.md) — Jobmeldungen abholen
- [wait](wait.md) — auf das Ende laufender Jobs warten
