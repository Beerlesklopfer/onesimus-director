# restore — Dateien aus Backups wiederherstellen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L402) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Restore files

**Originalsyntax:**

```text
restore where=</path> client=<client-name> storage=<storage-name> bootstrap=<file> restorejob=<job-name> comment=<text> jobid=<jobid> fileset=<fileset-name> replace=<always|never|ifolder|ifnewer> pluginoptions=<plugin-options-string> regexwhere=<regex> fileregex=<regex> restoreclient=<client-name> backupformat=<format> pool=<pool-name> file=<filename> directory=<directory> before=<date> strip_prefix=<prefix> add_prefix=<prefix> add_suffix=<suffix> select=<date> select before current copies done all
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 28 von 28 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-limited.
- Menüpunkte 1, 2, 4–12, Dateibaum-Auswahl, mehrere JobIds und regexwhere fehlen (C-047).
- Verglichen sind Argumentbehandlung und Abbruchpfade; ein echter Restore läuft im Paritätslauf nicht.

**Offene Logeinträge:** `C-047`, `C-051` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (28):** `c012-restore-menu`, `c021-unknown-keyword`, `c021-unknown-backupformat`, `c021-unknown-keyword-json`, `c021-missing-jobid-value`, `c021-missing-file-value`, `c021-jobid-not-a-number`, `c021-jobid-list-invalid`, `c021-jobid-unknown`, `c021-jobid-zero`, `c021-two-dates`, `c021-before-short-date`, `c021-before-garbage`, `c021-pool-unknown`, `c021-fileregex-invalid`, `c021-comment-illegal`, `c021-comment-empty`, `c021-client-unknown`, `c021-client-invalid-name`, `c021-limited-console`, `c021-cancel-menu`, `c021-cancel-menu-client`, `restore-help`, `w011-restore-item-cancel`, `w011-restore-badnumber-cancel`, `w011-restore-jobids-cancel`, `w011-restore-api1-cancel`, `w011-restore-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/restore.rs:86](../../../crates/onesimus-dird/src/commands/restore.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`restore` stellt Dateien aus Backups wieder her. Der Befehl führt in drei Schritten zum Restore-Job: Zuerst werden die JobIds gewählt, die die Dateien enthalten (direkt mit `jobid=` oder über ein Menü mit 12 Methoden). Dann baut der Director aus dem Katalog einen Dateibaum, in dem man mit `mark`, `cd`, `ls` und weiteren Befehlen die Dateien auswählt. Zum Schluss zeigt er die Parameter des Restore-Jobs und fragt `OK to run? (yes/mod/no)`.

Bis zu dieser Frage liest `restore` nur aus dem Katalog; nur die Bootstrap-Datei im Arbeitsverzeichnis des Directors wird geschrieben. Erst mit `yes` (oder dem Schlüsselwort `yes` auf der Befehlszeile) startet ein Job, der über Storage und File Daemon Dateien auf den Client schreibt und je nach `replace` vorhandene Dateien überschreibt.

## Syntax

```text
restore
restore jobid=<id>[,<id>…] [all] [done] [<ziel-optionen>]
restore client=<client> [fileset=<fileset>] [<menüauswahl-optionen>] [<ziel-optionen>]
restore select [current | before=<datum>] [copies] [archive] [pool=<pool>] client=<client> fileset=<fileset>
restore client=<client> file=<pfad> [file=<pfad> …] [<ziel-optionen>]
restore client=<client> file=?<tabelle> [<ziel-optionen>]
restore jobid=<id> client=<client> directory=<verzeichnis> [<ziel-optionen>]
restore [jobid=<id>] fileregex=<regex> client=<client> [fileset=<fileset>] [<ziel-optionen>]
```

`<ziel-optionen>` sind `restorejob=`, `restoreclient=`, `where=` oder `regexwhere=` oder `strip_prefix=`/`add_prefix=`/`add_suffix=`, `replace=`, `storage=`, `bootstrap=`, `pluginoptions=`, `comment=` und `yes`.

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<liste>` | JobIds, durch Komma getrennt | Diese Jobs bilden den Dateibaum; das Menü entfällt. `jobid=0` wählt nichts aus. |
| `current` | – | Mit `select`: neuester Stand (wie Menüpunkt 5). |
| `before=<datum>` | `JJJJ-MM-TT [HH:MM:SS]` | Stand vor diesem Zeitpunkt (wie Menüpunkt 6). Fehlende Teile werden ergänzt; zusammen mit `current` wird nur ein Datum berücksichtigt. |
| `select` | – | Auswahl über Client und FileSet statt über das Menü. |
| `copies` | – | Mit `select`: Copy-Jobs statt der Originale verwenden. |
| `archive` | – | Mit `select`: Archiv-Jobs berücksichtigen. |
| `pool=<pool>` | Pool-Name | Nur Jobs auf Volumes dieses Pools. |
| `client=<client>` | Client-Name | Client, dessen Backup wiederhergestellt wird (Backup Client); ohne `restoreclient=` auch das Ziel. |
| `fileset=<fileset>` | FileSet-Name | FileSet für die Auswahl über Client (Menü 5/6/9/10, `select`, `fileregex`). |
| `file=<pfad>` | vollständiger Pfad, `<datei` (Liste aus Datei) oder `?<tabelle>` (BVFS-Tabelle) | Einzelne Dateien statt Dateibaum; wiederholbar. `file=?b2…` übernimmt eine mit [.bvfs_restore](dot-bvfs-restore.md) gebaute Auswahl. |
| `directory=<verzeichnis>` | Verzeichnis mit `/` am Ende | Ganzes Verzeichnis der gewählten JobIds. |
| `fileregex=<regex>` | POSIX-Regex | Nur Dateien, deren Pfad passt. |
| `all` | – | Alle Dateien des Baums sind beim Aufbau schon markiert. |
| `done` | – | Dateibaum nicht interaktiv öffnen, direkt weiter zur Job-Auswahl. |
| `restorejob=<job>` | Name eines Jobs mit `Type = Restore` | Restore-Job, dessen Vorgaben (Where, Replace …) gelten. Ohne Angabe fragt der Director, wenn mehr als ein Restore-Job definiert ist. |
| `restoreclient=<client>` | Client-Name | Ziel-Client, wenn er vom Backup-Client abweicht. |
| `where=<pfad>` | Pfad, `/` für Originalort | Zielverzeichnis, dem jeder Pfad vorangestellt wird. |
| `regexwhere=<ausdruck>` | `!alt!neu!` … | Pfade beim Zurückschreiben umschreiben (ersetzt `where`). |
| `strip_prefix=<präfix>` | Pfad | Diesen Anfang aus jedem Pfad entfernen (wird zu `regexwhere`). |
| `add_prefix=<präfix>` | Pfad | Vor jeden Pfad setzen (wird zu `regexwhere`). |
| `add_suffix=<endung>` | Text | An jeden Dateinamen anhängen (wird zu `regexwhere`). |
| `replace=<modus>` | `always`, `ifnewer`, `ifolder`, `never` | Umgang mit vorhandenen Dateien am Ziel. |
| `storage=<storage>` | Storage-Name | Storage zum Lesen statt des im Katalog vermerkten. |
| `bootstrap=<datei>` | Pfad | Vorhandene Bootstrap-Datei verwenden. |
| `pluginoptions=<text>` | `plugin:option=wert` | Optionen für ein FD-Plugin. |
| `comment=<text>` | Text ohne Steuerzeichen | Kommentar des Restore-Jobs. |
| `yes` | – | Ohne Rückfrage starten. Startet den Job sofort, nur bewusst verwenden. |

Andere Wörter lehnt `restore` mit `Unknown keyword: <wort>` ab, auch `backupformat=` und `help`.

## Dialoge

### Menü zur Auswahl der JobIds

Ohne `jobid=`, `file=` oder `select` zeigt der Director das Menü `To select the JobIds, you have the following choices:` mit 13 Einträgen und fragt `Select item:  (1-13):`. Eine Zahl außerhalb des Bereichs beantwortet er mit `Please enter a number between 1 and 13` und fragt erneut; `.` bricht mit `Selection aborted, nothing done.` ab, `13` (Cancel) ohne Meldung.

| Punkt | Was danach gefragt wird |
|---|---|
| 1 | Zeigt die letzten 20 Jobs und kehrt ins Menü zurück. |
| 2 | Client, dann ein Dateiname; zeigt die Jobs, die die Datei enthalten. |
| 3 | `Enter JobId(s), comma separated, to restore:` |
| 4 | `Enter SQL list command:`; das Ergebnis erscheint als Tabelle, danach wieder das Menü. |
| 5 | Client, FileSet; wählt Full, letzte Differential und folgende Incrementals. |
| 6 | Datum (`Enter date as YYYY-MM-DD HH:MM:SS :`), Client, FileSet; wie 5, aber vor dem Datum. |
| 7 | Client, dann Dateinamen mit Pfad, eine pro Zeile, leere Zeile beendet. |
| 8 | Datum, Client, Dateinamen. |
| 9 | Client, FileSet; ermittelt die JobIds wie Punkt 5. |
| 10 | Datum, Client, FileSet; ermittelt die JobIds wie Punkt 6. |
| 11 | JobIds, Client, dann Verzeichnisse, eine pro Zeile, leere Zeile beendet. |
| 12 | `Enter JobId to get the state to restore:`; baut den vollständigen Stand zu diesem Job. |
| 13 | Abbruch. |

Fragen nach Client, FileSet oder Restore-Job sind nummerierte Listen; erwartet wird die Nummer. Text wird mit `Expected a positive integer, got: <eingabe>` abgelehnt, eine zu große Zahl mit `Please enter a number between 1 and <n>`.

### Dateiauswahl (Dateibaum)

Nach der Wahl der JobIds baut der Director den Dateibaum (`Building directory tree for JobId(s) … ++++`) und meldet `cwd is: /`. Danach steht vor jeder Eingabe der vom Director gesendete Prompt `$ `; oconsole zeigt ihn als eigene Zeile und darunter seinen Eingabeprompt `> `. Befehle dürfen abgekürzt werden (`es` = `estimate`, `h` = `help`), Groß- und Kleinschreibung ist egal.

| Befehl | Bedeutung |
|---|---|
| `cd <verzeichnis>` | Verzeichnis wechseln (absolut, relativ, `..`). Unbekannte Pfade: `Invalid path given.` |
| `ls [<muster>]` | Einträge des aktuellen Verzeichnisses; `*` vor dem Namen = markiert, `+` = Verzeichnis mit markierten Einträgen. |
| `dir [<muster>]` | Lange Liste mit Rechten, Eigentümer, Größe und Datum. |
| `pwd` | Aktuelles Verzeichnis. |
| `find <muster> …` | Suche im ganzen Baum, Platzhalter erlaubt. |
| `mark <muster> …`, `add` | Dateien und Verzeichnisse (rekursiv) markieren; `*`, `?` und `**` erlaubt, auch mit Pfad. |
| `unmark <muster> …`, `delete` | Markierung rekursiv aufheben. |
| `markdir <muster>` | Nur das Verzeichnis selbst markieren, ohne Inhalt. |
| `unmarkdir <muster>` | Markierung eines Verzeichnisses aufheben, ohne Rekursion. |
| `lsmark` | Markierte Einträge unterhalb des aktuellen Verzeichnisses. |
| `count` | Zahl der Einträge und der markierten Einträge. |
| `estimate` | Zahl und Gesamtgröße der markierten Dateien. |
| `help`, `?` | Befehlsübersicht. |
| `done`, `exit`, `.` | Auswahl beenden und mit dem Restore weitermachen. |
| `quit`, `abort`, `q` | Auswahl verwerfen, `Restore not done.` |

Die Punktbefehle `.ls`, `.dir`, `.lsdir`, `.lsmark`, `.pwd` und `.help` liefern dieselben Daten in einer Form für Programme (ohne Markierungszeichen, `.dir` durch Kommas getrennt).

Sind für eine gewählte JobId keine Dateien mehr im Katalog (Retention), fragt der Director statt des Dateibaums `Do you want to restore all the files? (yes|no):` und danach `Regexp matching files to restore? (empty to abort):`.

### Restore-Job, Client und Bestätigung

Sind mehrere Restore-Jobs definiert und fehlt `restorejob=`, fragt der Director `Select Restore Job (1-2):`; fehlt `client=`, fragt er nach dem Schreiben der Bootstrap-Datei `Select the Client (1-2):`. Danach schreibt er die Bootstrap-Datei, zeigt die benötigten Volumes und die Job-Parameter und fragt:

- `yes` startet den Restore-Job.
- `no` antwortet `Job not run.`
- `mod` zeigt `Parameters to modify:` (1 Level, 2 Storage, 3 Job, 4 FileSet, 5 Restore Client, 6 Backup Format, 7 When, 8 Priority, 9 Bootstrap, 10 Where, 11 File Relocation, 12 Replace, 13 JobId, 14 Plugin Options). Nach der Änderung kommt die Zusammenfassung erneut. `.` im Änderungsmenü bricht den ganzen Restore ab (`Selection aborted, nothing done.` und `Job not run.`).

## API-Modi

- `.api 0`: Text wie in den Beispielen.
- `.api 1`: Ohne die Katalogzeilen. Auswahllisten kommen als Signal-gerahmte Liste (die Menüeinträge ohne Nummern direkt hintereinander), danach erwartet der Director eine Eingabe als Texteingabe. Im Dateibaum steht nach `pwd` kein Zeilenumbruch (`/$ `). Die Zusammenfassung beginnt mit `Type: Restore` und `Title: Run Restore job`; die Frage `OK to run? (yes/mod/no): ` folgt erst mit der Antwort.
- `.api 2`: Ohne Argumente oder bei unvollständiger Auswahl antwortet `restore` mit einem JSON-Fehlerobjekt; Meldungen stehen unter `error.data.messages`. Im Dateibaum laufen die Befehle als Text; erst `quit` oder `done` liefert das JSON-Objekt. **Achtung:** Unter `.api 2` fragt der Director nicht nach `yes/mod/no`. Ist die Auswahl vollständig (zum Beispiel `restore jobid=… client=… restorejob=… all done`), startet der Restore sofort. Das Original meldet den gestarteten Job dabei als `{"result": {"run": {"jobid": "<n>"}}}`, Onesimus antwortet derzeit mit leerem `result` (Abweichung, noch ohne Eintrag im Kompatibilitätslog). Ein Beispiel wird hier bewusst nicht gezeigt, weil es einen echten Restore startet.

## Beispiele

### Menü aufrufen und abbrechen

Ohne Argumente erscheint das Auswahlmenü. `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
to be restored. You will be presented several methods
of specifying the JobIds. Then you will be allowed to
select which files from those JobIds are to be restored.

To select the JobIds, you have the following choices:
 1: List last 20 Jobs run
 2: List Jobs where a given File is saved
 3: Enter list of comma separated JobIds to select
 4: Enter SQL list command
 5: Select the most recent backup for a client
 6: Select backup for a client before a specified time
 7: Enter a list of files to restore
 8: Enter a list of files to restore before a specified time
 9: Find the JobIds of the most recent backup for a client
10: Find the JobIds for a backup for a client before a specified time
11: Enter a list of directories to restore for found JobIds
12: Select full restore to a specified Job date
13: Cancel
Select item:  (1-13): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Falsche Menünummer

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
to be restored. You will be presented several methods
of specifying the JobIds. Then you will be allowed to
select which files from those JobIds are to be restored.

To select the JobIds, you have the following choices:
[…]
Select item:  (1-13): 
> 99
Please enter a number between 1 and 13
Select item:  (1-13): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Letzte 20 Jobs anzeigen (Menüpunkt 1)

Punkt 1 zeigt eine Tabelle und kehrt ins Menü zurück; `13` beendet.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
to be restored. You will be presented several methods
of specifying the JobIds. Then you will be allowed to
select which files from those JobIds are to be restored.

To select the JobIds, you have the following choices:
[…]
Select item:  (1-13): 
> 1
+-------+-----------+---------------------+----------+----------+-------------+
| jobid | client    | starttime           | joblevel | jobfiles | jobbytes    |
+-------+-----------+---------------------+----------+----------+-------------+
| 15    | bareos-fd | 2026-10-15 21:00:56 | I        | 0        | 0           |
| 13    | bareos-fd | 2026-10-15 21:00:45 | I        | 0        | 0           |
[…]
| 619   | bareos-fd | 2026-06-23 21:10:04 | F        | 67       | 14972       |
+-------+-----------+---------------------+----------+----------+-------------+
To select the JobIds, you have the following choices:
[…]
Select item:  (1-13): 
> 13
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JobIds per SQL suchen (Menüpunkt 4)

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
[…]
Select item:  (1-13): 
> 4
Enter SQL list command: 
> SELECT JobId FROM Job WHERE JobId < 3 ORDER BY JobId
+-------+
| jobid |
+-------+
| 1     |
| 2     |
+-------+
To select the JobIds, you have the following choices:
[…]
Select item:  (1-13): 
> 13
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JobIds eingeben und Dateien im Baum auswählen (Menüpunkt 3)

Mit `client=` und `restorejob=` auf der Befehlszeile entfallen die Rückfragen nach Client und Restore-Job. Nach `done` kommt die Zusammenfassung; `mod` und dann `.` bricht den Restore ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore client=bareos-fd restorejob=RestoreFiles
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
[…]
Select item:  (1-13): 
> 3
Enter JobId(s), comma separated, to restore: 
> 1
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
180 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
remove (unmark) files to be restored. No files are initially added, unless
you used the "all" keyword on the command line.
Enter "done" to leave this mode.

cwd is: /
$ 
> cd /usr/sbin
cwd is: /usr/sbin/
$ 
> mark zic
1 file newly marked.
$ 
> done
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.1.bsr

The job will require the following
   Volume(s)                 Storage(s)                SD Device(s)
===========================================================================
   
    Incremental-47322         File                      FileStorage              

Volumes marked with "*" are online.


1 file selected to be restored.

Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.1.bsr
Where:           /tmp/bareos-restores
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:44
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
> .
Selection aborted, nothing done.
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Neuesten Stand eines Clients wählen (Menüpunkt 5)

Der Director sucht für Client und FileSet die Full-Sicherung, die letzte Differential und alle späteren Incrementals. „You have messages.“ weist auf wartende Director-Meldungen hin (siehe [messages](messages.md)).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore client=bareos-fd restorejob=RestoreFiles
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
[…]
Select item:  (1-13): 
> 5
The defined FileSet resources are:
1: BenchmarkSmall
2: Catalog
3: SelfTest
Select FileSet resource (1-3): 
> 3
+-------+-------+----------+-------------+---------------------+--------------------+
| jobid | level | jobfiles | jobbytes    | starttime           | volumename         |
+-------+-------+----------+-------------+---------------------+--------------------+
|   600 | F     |      599 | 157,762,030 | 2026-06-11 21:00:02 | Full-47321         |
|   615 | D     |        8 |  86,384,936 | 2026-06-20 21:00:02 | Differential-47323 |
|   616 | I     |        2 |  34,079,992 | 2026-06-22 21:00:02 | Incremental-47322  |
|   620 | I     |        2 |  34,117,880 | 2026-06-24 21:00:02 | Incremental-47322  |
|   622 | I     |        2 |  34,117,880 | 2026-06-25 21:00:02 | Incremental-47322  |
|   624 | I     |       26 |  66,023,053 | 2026-06-26 21:00:03 | Incremental-47322  |
|   626 | I     |        5 |  24,154,072 | 2026-07-09 21:00:02 | Incremental-47322  |
|     1 | I     |      180 | 126,345,661 | 2026-10-13 21:00:02 | Incremental-47322  |
|     5 | I     |       12 |     965,864 | 2026-10-15 21:00:02 | Incremental-47322  |
+-------+-------+----------+-------------+---------------------+--------------------+
You have selected the following JobIds: 600,615,616,620,622,624,626,1,5

Building directory tree for JobId(s) 600,615,616,620,622,624,626,1,5 ...  +++++++++++++++
181 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
remove (unmark) files to be restored. No files are initially added, unless
you used the "all" keyword on the command line.
Enter "done" to leave this mode.

cwd is: /
$ 
> quit
Restore not done.
You have messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Stand vor einem Datum (Menüpunkt 6)

Ein Datum im falschen Format wird abgelehnt und neu erfragt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
[…]
Select item:  (1-13): 
> 6
The restored files will the most current backup
BEFORE the date you specify below.

Enter date as YYYY-MM-DD HH:MM:SS :
> nodate
Improper date format.
Enter date as YYYY-MM-DD HH:MM:SS :
> 2026-10-16
Defined Clients:
1: bareos-fd
2: parity-ndmp-fd
Select the Client (1-2): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vollständigen Stand zu einem Job (Menüpunkt 12)

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

First you select one or more JobIds that contain files
[…]
Select item:  (1-13): 
> 12
Enter JobId to get the state to restore: 
> 5
Selecting jobs to build the Full state at 2026-10-15 21:00:02
You have selected the following JobIds: 600,615,616,618,620,622,624,626,628,1,3,5,9,10

Building directory tree for JobId(s) 600,615,616,618,620,622,624,626,628,1,3,5,9,10 ...  +++++++++++++++
181 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> quit
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore mit JobId, Ziel und Ersetzungsmodus

Alle Dateien von JobId 1 nach `/tmp/bareos-restores`, vorhandene Dateien bleiben stehen. Mit `no` wird der Job nicht gestartet.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 client=bareos-fd restorejob=RestoreFiles where=/tmp/bareos-restores replace=never all done
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.3.bsr

The job will require the following
   Volume(s)                 Storage(s)                SD Device(s)
===========================================================================
   
    Incremental-47322         File                      FileStorage              

Volumes marked with "*" are online.


180 files selected to be restored.

Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.3.bsr
Where:           /tmp/bareos-restores
Replace:         Never
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:45
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore-Job auswählen lassen

Ohne `restorejob=` fragt der Director, welcher Restore-Job verwendet wird. Die letzte Zeile „You have messages.“ kündigt wartende Director-Meldungen an.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 client=bareos-fd all done
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.
The defined Restore Job resources are:
1: parity-restore-regex
2: RestoreFiles
Select Restore Job (1-2): 
> 2
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.10.bsr

The job will require the following
   Volume(s)                 Storage(s)                SD Device(s)
===========================================================================
   
    Incremental-47322         File                      FileStorage              

Volumes marked with "*" are online.


180 files selected to be restored.

Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.10.bsr
Where:           /tmp/bareos-restores
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:48
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
You have messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Pfade umschreiben

`strip_prefix`, `add_prefix` und `add_suffix` werden zu einem `regexwhere`-Ausdruck zusammengesetzt (Zeile `RegexWhere:`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 client=bareos-fd restorejob=RestoreFiles strip_prefix=/usr add_prefix=/tmp/r add_suffix=.old all done
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.4.bsr

The job will require the following
[…]
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.4.bsr
RegexWhere:      !/usr!!i,!([^/])$!$1.old!,!^!/tmp/r!
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:45
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Mit `regexwhere=` direkt:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 client=bareos-fd restorejob=RestoreFiles regexwhere=!/usr!/tmp/usr! all done
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.5.bsr

The job will require the following
[…]
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.5.bsr
RegexWhere:      !/usr!/tmp/usr!
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:46
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Plugin-Optionen übergeben

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 client=bareos-fd restorejob=RestoreFiles pluginoptions=python:x=y all done
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.7.bsr

The job will require the following
[…]
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.7.bsr
Where:           /tmp/bareos-restores
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:47
Catalog:         MyCatalog
Priority:        10
Plugin Options:  python:x=y
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Zielverzeichnis mit mod ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 client=bareos-fd restorejob=RestoreFiles all done
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.1.bsr

The job will require the following
[…]
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.1.bsr
Where:           /tmp/bareos-restores
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:09:05
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
> /tmp/bareos-restores-doc
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.1.bsr
Where:           /tmp/bareos-restores-doc
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:09:05
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ersetzungsmodus mit mod ändern

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 client=bareos-fd restorejob=RestoreFiles all done
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.2.bsr

The job will require the following
[…]
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.2.bsr
Where:           /tmp/bareos-restores
Replace:         Always
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:09:06
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
> 2
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.2.bsr
Where:           /tmp/bareos-restores
Replace:         IfNewer
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:09:06
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateibaum: Hilfe

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
180 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> help
  Command    Description
  =======    ===========
  abort      abort and do not do restore
  add        add dir/file to be restored recursively, wildcards allowed
  cd         change current directory
  count      count marked files in and below the cd
  delete     delete dir/file to be restored recursively in dir
  dir        long list current directory, wildcards allowed
  done       leave file selection mode
  estimate   estimate restore size
  exit       same as done command
  find       find files, wildcards allowed
  help       print help
  ls         list current directory, wildcards allowed
  lsmark     list the marked files in and below the cd
  mark       mark dir/file to be restored recursively, wildcards allowed
  markdir    mark directory name to be restored (no files)
  pwd        print current working directory
  unmark     unmark dir/file to be restored recursively in dir
  unmarkdir  unmark directory name only no recursion
  quit       quit and do not do restore
  ?          print help

$ 
> ?
[…]
> .help
abort
add
cd
count
delete
dir
done
estimate
exit
find
help
ls
lsmark
mark
markdir
pwd
unmark
unmarkdir
quit
?
$ 
> abort
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateibaum: Navigieren und Auflisten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
180 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> ls
usr/
$ 
> pwd
cwd is: /
$ 
> .pwd
/$ 
> cd /etc
Invalid path given.
cwd is: /
$ 
> ls
usr/
$ 
> cd ..
cwd is: /
$ 
> cd nosuch
Invalid path given.
cwd is: /
$ 
> cd a b
Too few or too many arguments. Try using double quotes.
$ 
> dir
----------   0 0 (root) 0 (root)             0  1970-01-01 00:00:00   /usr/
$ 
> .dir
----------,0,0(root),0(root),0,1970-01-01 00:00:00, ,/usr/
$ 
> .ls
usr/
$ 
> .lsdir
usr/
$ 
> quit
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateibaum: Markieren, Zählen, Größe schätzen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1,5 client=bareos-fd restorejob=RestoreFiles
Using Catalog "MyCatalog"
You have selected the following JobIds: 1,5

Building directory tree for JobId(s) 1,5 ...  +++++++++++++++++++++++++++++++++++++++++++++
181 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> cd /usr/sbin
cwd is: /usr/sbin/
$ 
> mark zic
1 file newly marked.
$ 
> dir zic
-rwxr-xr-x   1 0 (root) 0 (root)         63816  2026-07-25 15:37:27 * /usr/sbin/zic
$ 
> count
181 total files/dirs. 1 marked to be restored.
$ 
> estimate
181 total files; 2 marked to be restored; 63,816 bytes.
$ 
> unmark zic
1 file newly unmarked.
$ 
> count
181 total files/dirs. 0 marked to be restored.
$ 
> quit
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateibaum: Muster und Markierungen anzeigen

`mark zic e2*` markiert mehrere Dateien auf einmal; `ls` und `lsmark` zeigen markierte Einträge mit `*`, Verzeichnisse mit markiertem Inhalt mit `+`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1,5
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobIds: 1,5

Building directory tree for JobId(s) 1,5 ...  +++++++++++++++++++++++++++++++++++++++++++++
181 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> cd /usr/sbin
cwd is: /usr/sbin/
$ 
> mark zic e2*
9 files newly marked.
$ 
> ls e*
*e2freefrag
*e2fsck
*e2image
*e2label
*e2mmpstatus
*e2scrub
*e2scrub_all
*e2undo
e4crypt
e4defrag
exicyclog
exigrep
exim
exim4
exim_checkaccess
exim_convert4r4
exim_dbmbuild
exim_dumpdb
exim_fixdb
exim_id_update
exim_lock
exim_msgdate
exim_tidydb
eximstats
exinext
exipick
exiqgrep
exiqsumm
exiwhat
$ 
> lsmark
*e2freefrag
*e2fsck
*e2image
*e2label
*e2mmpstatus
*e2scrub
*e2scrub_all
*e2undo
*zic
$ 
> .lsmark
e2freefrag
e2fsck
e2image
e2label
e2mmpstatus
e2scrub
e2scrub_all
e2undo
zic
$ 
> count
181 total files/dirs. 9 marked to be restored.
$ 
> estimate
181 total files; 10 marked to be restored; 530,503 bytes.
$ 
> cd ..
cwd is: /usr/
$ 
> lsmark
+sbin/
  *e2freefrag
  *e2fsck
  *e2image
  *e2label
  *e2mmpstatus
  *e2scrub
  *e2scrub_all
  *e2undo
  *zic
$ 
> ls
+sbin/
$ 
> find zic
*/usr/sbin/zic
$ 
> unmark e2fsck
No files newly unmarked.
$ 
> estimate
181 total files; 10 marked to be restored; 530,503 bytes.
$ 
> cd /
cwd is: /
$ 
> lsmark
+usr/
  +sbin/
    *e2freefrag
    *e2fsck
    *e2image
    *e2label
    *e2mmpstatus
    *e2scrub
    *e2scrub_all
    *e2undo
    *zic
$ 
> unmark *
9 files newly unmarked.
$ 
> count
181 total files/dirs. 0 marked to be restored.
$ 
> quit
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateibaum: Pfade, rekursive Muster und markdir

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
180 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> mark /usr/sbin/zic
1 file newly marked.
$ 
> mark usr/**/e2*
8 files newly marked.
$ 
> mark /
171 files newly marked.
$ 
> unmark /usr/**
180 files newly unmarked.
$ 
> markdir usr
No directories newly marked.
$ 
> cd usr
cwd is: /usr/
$ 
> markdir sbin
1 directory newly marked.
$ 
> unmarkdir s*
1 directory newly unmarked.
$ 
> lsmark
$ 
> add sbin/zic
1 file newly marked.
$ 
> delete sbin/
1 file newly unmarked.
$ 
> count
180 total files/dirs. 0 marked to be restored.
$ 
> abort
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateibaum: Dateien suchen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
180 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> find
No file specification given.
$ 
> find *.conf
/usr/sbin/update-exim4.conf
$ 
> find passwd hosts
$ 
> quit
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateibaum: Ungültige Eingaben und Abkürzungen

`d` ist die Abkürzung für `delete`, `e` für `estimate`, `q` für `quit`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
180 files inserted into the tree.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> 
Invalid command "". Enter "done" to exit.
$ 
> bogus
Invalid command "bogus". Enter "done" to exit.
$ 
> .bogus
$ 
> d
No files newly unmarked.
$ 
> e
180 total files; 0 marked to be restored; 0 bytes.
$ 
> markdir etc
No directories newly marked.
$ 
> unmarkdir etc
No directories newly unmarked.
$ 
> add
No files marked.
$ 
> delete
No files newly unmarked.
$ 
> q
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit all alle Dateien vormarkieren

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=1 all
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobId: 1

Building directory tree for JobId(s) 1 ...  ++++++++++++++++++++++++++++++++++++++++++++
179 files inserted into the tree and marked for extraction.

You are now entering file selection mode where you add (mark) and
[…]
cwd is: /
$ 
> count
180 total files/dirs. 180 marked to be restored.
$ 
> estimate
180 total files; 180 marked to be restored; 126,345,661 bytes.
$ 
> lsmark
$ 
> abort
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JobId ohne Dateien im Katalog

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore jobid=3
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have selected the following JobId: 3

Building directory tree for JobId(s) 3 ...  

For one or more of the JobIds selected, no files were found,
so file selection is not possible.
Most likely your retention policy pruned the files.

Do you want to restore all the files? (yes|no): 
> no

Regexp matching files to restore? (empty to abort): 
> 
Restore not done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auswahl aus BVFS übernehmen (file=?tabelle)

So übernimmt `restore` eine mit [.bvfs_restore](dot-bvfs-restore.md) gebaute Auswahl. Danach wird die Tabelle mit [.bvfs_cleanup](dot-bvfs-cleanup.md) gelöscht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 fileid=1 path=b2105
OK
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore file=?b2105 client=bareos-fd restorejob=RestoreFiles where=/tmp/bareos-restores replace=never
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.11.bsr

The job will require the following
[…]
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.11.bsr
Where:           /tmp/bareos-restores
Replace:         Never
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:49
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup path=b2105
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekanntes Schlüsselwort unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore help
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
          "Unknown keyword: help\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Restore ohne Argumente unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore
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

### Konsole ohne Recht auf restore

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> restore help
restore: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Unknown keyword: <wort>` | Schlüsselwort ist `restore` unbekannt (auch `help`, `backupformat=`). Schreibweise prüfen. |
| `Missing value for keyword: <wort>` | Schlüsselwort wie `jobid` oder `file` ohne `=wert`. |
| `Invalid JobId in list.` | `jobid=` enthält etwas anderes als Zahlen und Kommas. |
| `No Jobs selected.` | `jobid=0` oder eine leere Auswahl. |
| `Unable to get Job record for JobId=<n>: ERR=… No Job found for JobId <n>` | JobId gibt es im Katalog nicht; mit [list jobs](list.md) prüfen. |
| `Improper date format: <text>` | `before=` ist kein Datum der Form `JJJJ-MM-TT HH:MM:SS`. Im Menü: `Improper date format.` |
| `Error: Pool resource "<name>" does not exist.` | `pool=` nennt keinen konfigurierten Pool. |
| `Regex compile error: …` und `Invalid "FileRegex" value.` | `fileregex=` ist kein gültiger regulärer Ausdruck. |
| `Illegal character "<z>" in a comment.` | `comment=` enthält ein unzulässiges Zeichen. |
| `Comment must be at least one character long.` | `comment=` ist leer. |
| `invalid client argument: <name>` | `client=` nennt keinen konfigurierten Client. |
| `client argument: Illegal character "<z>" in name.` | Client-Name mit unzulässigem Zeichen. |
| `Error getting FileSet "<name>": ERR=… FileSet record "<name>" not found.` | FileSet ist nicht im Katalog; der Director bietet die vorhandenen zur Auswahl an. |
| `No FileSet found for client "<client>".` | FileSet-Auswahl abgebrochen. |
| `No database record found for: <pfad>` | Datei aus `file=` oder Menüpunkt 7/8 ist in keinem Backup des Clients. |
| `No JobId specified cannot continue.` | `directory=` ohne `jobid=`. |
| `Query failed: …` | SQL aus Menüpunkt 4 ist fehlerhaft; die Meldung von PostgreSQL folgt. |
| `"where" specification not authorized.` / `"RegexWhere" specification not authorized.` | Die Konsole darf dieses Ziel nicht verwenden (Where ACL). |
| `Invalid path given.` | `cd` im Dateibaum auf einen Pfad, den es nicht gibt (oder auf eine Datei). |
| `Too few or too many arguments. Try using double quotes.` | `cd` mit mehreren Wörtern; Namen mit Leerzeichen in Anführungszeichen setzen. |
| `Invalid command "<text>". Enter "done" to exit.` | Unbekannter Befehl im Dateibaum. |
| `No file specification given.` | `find` ohne Muster. |
| `Restore not done.` | Dateiauswahl mit `quit`/`abort` beendet. |
| `Selection aborted, nothing done.` | Auswahl mit `.` abgebrochen. |
| `Job not run.` | Abschlussfrage mit `no` beantwortet oder `mod` abgebrochen. |
| `restore: is an invalid command.` | Die Konsole hat kein Recht auf `restore` (Command ACL). |

## Siehe auch

- [run](run.md) — Restore-Job mit `run job=<restorejob> jobid=…` starten
- [estimate](estimate.md) — Umfang eines Backups schätzen
- [list](list.md) — Jobs und Dateien im Katalog anzeigen
- [.bvfs_restore](dot-bvfs-restore.md) — Auswahl für `file=?<tabelle>` erzeugen
- [.ovfs_tree](dot-ovfs-tree.md) — Restore-Baum einer Job-Kette (Onesimus)
- [.ovfs_latest](dot-ovfs-latest.md) — Job-Kette des neuesten Stands (Onesimus)
- [messages](messages.md) — wartende Meldungen abholen
