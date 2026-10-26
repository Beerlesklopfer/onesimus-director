# list — Katalogdatensätze auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L367) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List objects from catalog

**Originalsyntax:**

```text
list basefiles ( jobid=<jobid> | ujobid=<complete_name> ) | backups client=<client-name> [fileset=<fileset-name>] [jobstatus=<status>] [level=<level>] [order=<asc|desc>] [limit=<number>] [days=<number>] [hours=<number>] | client=<client-name> | clients | copies jobid=<jobid> | files jobid=<jobid> | files ujobid=<complete_name> | filesets | fileset [ filesetid=<filesetid> ] | fileset [ jobid=<jobid> ] | fileset [ ujobid=<complete_name> ] | jobs [job=<job-name>] [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [pool=<pool>] [days=<number>] [hours=<number>] [last] [count] | job=<job-name> [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [days=<number>] [hours=<number>] | jobid=<jobid> | ujobid=<complete_name> | joblog jobid=<jobid> [count] | joblog ujobid=<complete_name> [count] | jobmedia jobid=<jobid> | jobmedia ujobid=<complete_name> | jobtotals | jobstatistics jobid=<jobid> | log [client=<client-name>] [reverse] | media [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] | media=<media-name> | nextvolume job=<job-name> [days=<number>] | pools | pool=<pool-name> | poolid=<poolid> | storages | volumes [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] [count] | volume=<volume-name> | volumeid=<volumeid> | mediaid=<volumeid> | [current] | [enabled | disabled] | [limit=<number> [offset=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 299 von 299 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-bvfs, parity-limited, parity-listacl, parity-operator.
- `list nextvolume` mit unbekanntem Job scheitert im Original im Batchmodus (CMD_FAILED), Onesimus meldet Erfolg (C-046).

**Offene Logeinträge:** `C-046` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (299):** `c002-list-twice`, `c002-private-then-shared`, `c002-shared-then-private`, `c002-client-keyword`, `c002-job-keyword`, `c002-catalog-keyword`, `c002-use-then-list`, `c002-api1`, `c002-api2`, `c002-gui`, `c002-limited-twice`, `c002-bvfs-noargs`, `c002-sql-then-list`, `c015-list-no-keyword`, `c015-list-mediatypes-unknown`, `c015-list-jobs-jobstatus-T_E`, `c015-list-jobs-jobstatus-terminated_fatal`, `c015-list-jobs-jobstatus-bogus`, `c015-list-jobs-jobstatus-f`, `c015-list-jobs-level-Full`, `c015-list-jobs-level-I`, `c015-list-jobs-level-Incremental_Differential`, `c015-list-jobs-level-bogus`, `c015-list-jobs-jobtype-B`, `c015-list-jobs-jobtype-Backup`, `c015-list-jobs-jobtype-Copy`, `c015-list-jobs-jobtype-bogus`, `c015-list-jobs-client-bareos-fd-limit-2`, `c015-list-jobs-client-nosuch`, `c015-list-jobs-job-BackupCatalog`, `c015-list-jobs-jobname-BackupCatalog-limit-2`, `c015-list-jobs-job-nosuch`, `c015-list-jobs-days-1`, `c015-list-jobs-hours-1`, `c015-list-jobs-hours-1000000`, `c015-list-jobs-last`, `c015-list-jobs-last-jobstatus-T`, `c015-list-jobs-limit-2-offset-1`, `c015-list-jobs-offset-3`, `c015-list-jobs-reverse-limit-3`, `c015-list-jobs-count`, `c015-list-jobs-count-jobstatus-T`, `c015-list-jobs-count-last`, `c015-list-jobs-volume-Full-47321`, `c015-list-jobs-pool-Full-limit-2`, `c015-list-jobs-pool-nosuch`, `c015-list-jobs-current`, `c015-list-jobs-enabled`, `c015-list-jobs-disabled`, `c015-list-jobs-limit-0`, `c015-list-jobs-limit-abc`, `c015-list-jobs-value`, `c015-list-job-missing`, `c015-list-jobid-missing`, `c015-list-jobid-14`, `c015-list-jobid-99999`, `c015-list-ujobid`, `c015-list-jobname`, `c015-list-jobtotals-limit`, `c015-list-files-jobid-5`, `c015-list-files-limit-offset`, `c015-list-files-nojobid`, `c015-list-files-jobid-99999`, `c015-list-files-jobid-14`, `c015-list-basefiles-nojobid`, `c015-list-files-ujobid`, `c015-list-fileset-jobid-1`, `c015-list-fileset-filesetid-3`, `c015-list-fileset-none`, `c015-list-filesets-limit`, `c015-list-fileset-jobid-14`, `c015-list-jobmedia-all`, `c015-list-jobmedia-99999`, `c015-list-joblog-none`, `c015-list-joblog-99999`, `c015-list-joblog-count`, `c015-list-joblog-limit`, `c015-list-log`, `c015-list-log-reverse`, `c015-list-log-client`, `c015-list-log-client-nosuch`, `c015-list-pools`, `c015-list-pool-nosuch`, `c015-list-poolid-3`, `c015-list-poolid-none`, `c015-list-pool-Incremental`, `c015-list-pool-Scratch`, `c015-list-pools-current`, `c015-list-client`, `c015-list-client-none`, `c015-list-client-nosuch`, `c015-list-clients-enabled`, `c015-list-storages-current`, `c015-list-volumes-all`, `c015-list-media-all`, `c015-list-volumes-jobid-1`, `c015-list-volumes-jobid-99999`, `c015-list-volumes-pool-nosuch`, `c015-list-volumes-limit`, `c015-list-volumes-count`, `c015-list-volumes-pool-count`, `c015-list-volume-nosuch`, `c015-list-mediaid`, `c015-list-mediaid-none`, `c015-list-volumes-all-limit`, `c015-list-copies-jobid`, `c015-list-copies-jobid-list`, `c015-list-copies-jobid-bad`, `c015-list-backups-none`, `c015-list-backups-fileset`, `c015-list-backups-fileset-any`, `c015-list-backups-fileset-nosuch`, `c015-list-backups-order-desc`, `c015-list-backups-order-asc`, `c015-list-backups-order-bad`, `c015-list-backups-jobstatus`, `c015-list-backups-jobstatus-bad`, `c015-list-backups-level`, `c015-list-backups-level-diff`, `c015-list-backups-client-nosuch`, `c015-list-jobstats`, `c015-list-jobstatistics-none`, `c015-list-nextvol`, `c015-list-nextvolume-days-3`, `c015-list-nextvolume-days-99`, `c015-list-nextvolume-catalog`, `c015-list-nextvolume-restore`, `c015-list-nextvolume-days-0`, `c015-list-jobs-x-jobtype-backup`, `c015-list-jobs-x-level-VirtualFull`, `c015-list-jobs-x-level-f`, `c015-list-jobs-x-jobstatus-[`, `c015-list-jobs-x-jobstatus-`, `c015-list-volumes-all-current`, `c015-list-backups-level-bad`, `c015-list-jobs-enabled-count`, `c015-api2x-list-jobs-current-limit-3`, `c015-api2x-list-clients-disabled`, `c015-api2x-list-files-jobid-1-limit-2-current`, `c015-api2-list-jobs-jobid-99999`, `c015-api2-list-files-jobid-5-limit-2-offset-1`, `c015-api2-list-log-limit-2`, `c015-api2-list-volumes-pool-Full`, `c015-api2-list-volumes-all`, `c015-api2-list-copies`, `c015-api2-list-backups-client-bareos-fd-limit-2`, `c015-api2-list-jobtotals`, `c015-api2-list-pool-Full`, `c015-api2-list-joblog-jobid-1-limit-2`, `c015-api2-list-nextvolume-job-backup-bareos-fd`, `c015-api2-list-jobs-count`, `c015-api2-list-volume-Full-47321`, `c015-api2-list-jobmedia-jobid-1`, `c015-api2-list-fileset-jobid-1`, `c015-api2-list-nosuch`, `c015-api2-list`, `c015-api2-list-jobid-14`, `c015-api2-list-basefiles-jobid-1`, `c015-api2-list-clients`, `c015-api2-list-storages`, `c015-api2-list-jobs-limit-2-offset-1`, `c015-api2-list-jobs-job-nosuch`, `c015-api2-list-joblog-jobid-1-count`, `c015-api2-list-mediaid-47321`, `c015-api2-list-volumes-jobid-1`, `c015-api2-list-jobstats-jobid-1`, `c015-api2-list-backups`, `c020-compact-list-pools`, `c026-limited-jobs`, `c026-limited-jobtotals`, `c026-limited-files`, `c026-limited-filesets`, `c026-limited-jobmedia`, `c026-limited-log`, `c026-limited-pools`, `c026-limited-clients`, `c026-limited-volumes`, `c026-limited-misc`, `c026-listacl-jobs`, `c026-listacl-jobtotals`, `c026-listacl-files`, `c026-listacl-filesets`, `c026-listacl-jobmedia`, `c026-listacl-log`, `c026-listacl-pools`, `c026-listacl-clients`, `c026-listacl-volumes`, `c026-listacl-misc`, `c026-operator-jobs`, `c026-operator-jobtotals`, `c026-operator-files`, `c026-operator-filesets`, `c026-operator-jobmedia`, `c026-operator-log`, `c026-operator-pools`, `c026-operator-clients`, `c026-operator-volumes`, `c026-operator-misc`, `c026-listacl-api2-1`, `c026-listacl-api2-2`, `c026-listacl-api2-3`, `c026-bvfs-list-denied`, `c026-bvfs-api2-list-denied`, `c027-gui-list-pools`, `c027-gui-list-jobs`, `c027-gui-list-media`, `c027-gui-list-clients`, `c027-gui-list-jobtotals`, `c027-gui-list-jobmedia`, `c027-gui-list-filesets`, `c027-gui-list-log`, `c027-gui-sqlquery-keyword`, `c027-gui-off-again`, `c027-gui-noarg`, `c027-gui-api1`, `c027-gui-api2`, `c028-list-jobs`, `c028-list-jobs-limit`, `c028-list-jobs-count`, `c028-list-jobs-last`, `c028-list-jobid`, `c028-list-job`, `c028-list-jobtotals`, `c028-list-basefiles`, `c028-list-files`, `c028-list-fileset`, `c028-list-filesets`, `c028-list-jobmedia`, `c028-list-joblog`, `c028-list-log`, `c028-list-pools`, `c028-list-pool`, `c028-list-clients`, `c028-list-client`, `c028-list-storages`, `c028-list-volumes`, `c028-list-volumes-pool`, `c028-list-media-all`, `c028-list-volume`, `c028-list-mediaid`, `c028-list-nextvol`, `c028-list-copies`, `c028-list-copies-jobid`, `c028-list-backups`, `c028-list-jobstats`, `c028-list-jobs-disabled`, `c028-list-clients-disabled`, `c028-list-schedules-disabled`, `c028-list-jobid-none`, `c028-list-files-none`, `c028-list-noarg`, `c028-list-bogus`, `c028-list-files-all`, `c028-list-volumes-count`, `c028-api-toggle`, `c035-list-jobtype-bogus-api1`, `c035-list-jobtype-bogus-api2`, `c035-list-jobtype-bogus-api2-compact`, `c035-list-nextvol-days-api1`, `c035-list-nextvol-days-api2`, `c035-list-nextvol-days-nojob-api2`, `c041-list-nextvol-backupcatalog`, `list-jobs`, `list-jobs-limit-5`, `list-jobs-last`, `list-jobs-jobstatus-T-limit-3`, `list-jobs-client-bareos-fd-limit-3`, `list-jobs-level-F-limit-3`, `list-jobs-days-0`, `list-jobid-1`, `list-job-backup-bareos-fd-limit-2`, `list-jobtotals`, `list-clients`, `list-pools`, `list-pool-Full`, `list-volumes`, `list-volumes-pool-Full`, `list-volume-Full-0001`, `list-media`, `list-filesets`, `list-files-jobid-1-limit-5`, `list-basefiles-jobid-1`, `list-copies`, `list-backups-client-bareos-fd-limit-3`, `list-joblog-jobid-1`, `list-jobmedia-jobid-1`, `list-log-limit-5`, `list-nextvolume-job-backup-bareos-fd`, `list-storages`, `list-mediatypes`, `list-jobs-limit-3-api2`, `list-pools-api2`, `list-volumes-api2`, `list-clients-api2`, `list-jobs-limited`, `list-bogus`, `w011-list-volumes-pool-cancel`, `w011-list-volumes-pool-api2`, `w011-list-nextvol-cancel`.

**Lokaler Handler:** [crates/odir/src/commands/list.rs:17](../../../crates/odir/src/commands/list.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`list` liest Datensätze aus dem Katalog und zeigt sie als Tabelle: Jobs, Job-Protokolle, gesicherte Dateien, FileSets, Volumes, Pools, Clients, Storages, Kopien und Statistiken. Der Befehl liest nur und verändert nichts. Für dieselben Daten mit allen Spalten untereinander gibt es [llist](llist.md); beide Befehle verstehen dieselben Schlüsselwörter.

Beim ersten Katalogzugriff einer Sitzung meldet der Director `Automatically selected Catalog: MyCatalog` und `Using Catalog "MyCatalog"`. Nennt der Befehl einen Client oder Job, aus dem sich der Katalog ergibt, erscheint nur die zweite Zeile. Bei weiteren Befehlen derselben Sitzung fallen beide Zeilen weg.

## Syntax

```text
list jobs [job=<name>] [client=<name>] [jobstatus=<status>[,…]] [jobtype=<typ>[,…]] [joblevel=<level>[,…]] [volume=<volume>] [pool=<pool>] [days=<n>] [hours=<n>] [last] [count] [reverse] [order=asc|desc] [current | enabled | disabled] [limit=<n> [offset=<n>]]
list job=<name> | jobname=<name> [Filter wie bei jobs]
list jobid=<jobid> | ujobid=<eindeutiger-jobname>
list jobtotals
list files jobid=<jobid> | ujobid=<eindeutiger-jobname>
list basefiles jobid=<jobid> | ujobid=<eindeutiger-jobname>
list fileset [jobid=<jobid> | ujobid=<eindeutiger-jobname> | filesetid=<id>]
list filesets
list jobmedia [jobid=<jobid> | ujobid=<eindeutiger-jobname>]
list joblog jobid=<jobid> | ujobid=<eindeutiger-jobname> [count]
list log [client=<name>] [reverse]
list pools | pool=<name> | poolid=<id>
list clients | client=<name>
list storages
list volumes | media [all | jobid=<jobid> | ujobid=<eindeutiger-jobname> | pool=<pool>] [count]
list volume=<name> | media=<name> | volumeid=<id> | mediaid=<id>
list nextvolume | nextvol [job=<name>] [days=<n>]
list copies [jobid=<jobid>[,…]]
list backups client=<name> [fileset=<name> | fileset=any] [jobstatus=<status>[,…]] [level=<level>[,…]] [order=asc|desc] [days=<n>] [hours=<n>]
list jobstatistics | jobstats jobid=<jobid>
list <schlüsselwort> … [catalog=<katalog>] [limit=<n> [offset=<n>]] [current] [enabled | disabled]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobs` | – | Alle Jobs aus dem Katalog, eine Zeile pro Job. Die Filter dieser Tabelle lassen sich kombinieren. |
| `job=<name>`, `jobname=<name>` | Jobname | Nur Jobs mit diesem Namen. Allein (`list job=…`) wirkt es wie `list jobs job=…`. |
| `jobid=<jobid>` | Ganzzahl | Genau ein Job. Unbekannte JobIds liefern eine leere Antwort. |
| `ujobid=<name>` | eindeutiger Jobname, z.B. `backup-bareos-fd.2026-10-13_21.00.00_09` | Genau ein Job über seinen eindeutigen Namen. |
| `client=<name>` | Clientname | Nur Jobs dieses Clients. Bei `log` und `backups` ebenfalls Filter; ein unbekannter Name ergibt `invalid client argument`. |
| `jobstatus=<status>` | Buchstabe (`T`, `W`, `E`, `f`, `A`, `R`, …) oder `terminated`, `warnings`, `error`, `fatal`, `canceled`, `running`; mehrere mit Komma | Nur Jobs mit diesem Status. |
| `jobtype=<typ>` | Buchstabe (`B`, `R`, `V`, `C`, `M`, …) oder exakter Name wie `Backup`, `Copy` (Groß-/Kleinschreibung zählt); mehrere mit Komma | Nur Jobs dieses Typs. |
| `joblevel=<level>`, `level=<level>` | `F`, `I`, `D` oder `Full`, `Incremental`, `Differential`; mehrere mit Komma | Nur Jobs dieses Levels. Ein einzelner Buchstabe wird wörtlich genommen, `level=f` findet also nichts. |
| `volume=<volume>` | Volumename | Nur Jobs, die auf dieses Volume geschrieben haben. |
| `pool=<pool>` | Poolname | Bei `jobs`: nur Jobs dieses Pools. Bei `volumes`: nur Volumes dieses Pools; ein unbekannter Pool öffnet eine Auswahlliste (siehe Dialoge). Allein: der Pool-Datensatz mit Prune-Details. |
| `days=<n>`, `hours=<n>` | Ganzzahl | Nur Jobs, die in den letzten n Tagen bzw. Stunden gestartet sind. Bei `nextvolume`: Anzahl Tage voraus (höchstens 50). |
| `last` | – | Pro Jobname nur der letzte Lauf. |
| `count` | – | Statt der Zeilen nur ihre Anzahl (`jobs`, `joblog`, `volumes`). |
| `reverse` | – | Umgekehrte Reihenfolge (neueste zuerst), bei `jobs` und `log`. |
| `order=asc\|desc` | `asc`, `desc` (auch abgekürzt) | Sortierung nach Startzeit, bei `jobs` und `backups`. Ein anderer Wert ist bei `backups` ein Fehler. |
| `limit=<n>`, `offset=<n>` | Ganzzahl | Höchstens n Zeilen, ab Zeile `offset`. `limit=0` oder ein ungültiger Wert ergeben keine Zeilen. |
| `current` | – | Nur Zeilen, deren Job, Client, Pool, Storage oder FileSet noch in der Konfiguration steht. |
| `enabled`, `disabled` | – | Nur Zeilen, deren Ressource in der Konfiguration aktiviert bzw. deaktiviert ist. |
| `jobtotals` | – | Anzahl, Dateien und Bytes je Jobname und als Summe. |
| `files` | – | Pfade der in einem Job gesicherten Dateien. Braucht `jobid=` oder `ujobid=`. |
| `basefiles` | – | Basisdateien eines Base-Jobs. Braucht `jobid=` oder `ujobid=`. |
| `fileset` | mit `jobid=`, `ujobid=` oder `filesetid=` | Der FileSet-Datensatz mit dem gespeicherten FileSet-Text. |
| `filesets` | – | Alle FileSet-Datensätze im Katalog. |
| `jobmedia` | optional `jobid=`, `ujobid=` | Welche Volumes ein Job (oder alle Jobs) beschrieben hat. |
| `joblog` | mit `jobid=` oder `ujobid=` | Das gespeicherte Jobprotokoll mit Zeitstempeln. |
| `log` | optional `client=`, `reverse` | Die letzten Protokollzeilen aller Jobs. |
| `pools`, `poolid=<id>` | – / Ganzzahl | Pool-Datensätze. |
| `clients`, `client=<name>` | – / Clientname | Client-Datensätze. `client` ohne Wert ist kein gültiges Schlüsselwort. |
| `storages` | – | Storage-Datensätze. |
| `volumes`, `media` | optional `all`, `jobid=`, `ujobid=`, `pool=` | Volumes, nach Pool gruppiert. Mit `all` in einer Tabelle, mit `jobid=` nur die Volumenamen des Jobs. |
| `volume=<name>`, `media=<name>`, `volumeid=<id>`, `mediaid=<id>` | Name / Ganzzahl | Genau ein Volume. |
| `nextvolume`, `nextvol` | optional `job=`, `days=` | Welches Volume der Job als nächstes beschreiben würde. Ohne `job=` folgt eine Auswahlliste. |
| `copies` | optional `jobid=` (Liste mit Komma) | Kopierjobs zu den angegebenen Jobs. |
| `backups` | `client=` Pflicht, dazu `fileset=` (Name oder `any`) und die Job-Filter | Sicherungen eines Clients. |
| `jobstatistics`, `jobstats` | mit `jobid=` | Statistikdatensätze eines laufenden oder gelaufenen Jobs. |
| `catalog=<katalog>` | Katalogname | Diesen Katalog verwenden (siehe [use](use.md)). |

## Dialoge

Zwei Varianten fragen nach, wenn eine Angabe fehlt oder nicht passt:

- `list volumes pool=<pool>` mit einem Pool, den der Katalog nicht kennt, zeigt die definierten Pools und fragt `Select the Pool (1-4):`. Eine Nummer wählt den Pool, `.` bricht ab (`Selection aborted, nothing done.` und `Pool <name> doesn't exist.`).
- `list nextvolume` ohne `job=` zeigt die Job-Ressourcen und fragt `Select Job resource (1-9):`. Eine Nummer wählt den Job, `.` bricht ab.

Eine Zahl außerhalb des Bereichs beantwortet der Director mit `Please enter a number between 1 and <n>` und fragt erneut. Unter `.api 2` wird nicht gefragt: Die Fehlermeldungen kommen direkt als JSON-Fehler zurück.

## API-Modi

- **`.api 0`** (Standard): Tabellen mit Rahmen, Zahlen mit Tausendertrennzeichen.
- **`gui on`**: wie `.api 0`, aber Zahlen ohne Trennzeichen und linksbündig; die Zeile `Automatically selected Catalog` fällt weg.
- **`.api 1`**: nur die Datenzeilen ohne Rahmen und Kopfzeile; Fehler und Warnungen als eigene Meldungen.
- **`.api 2`**: JSON-RPC-Objekt. Der Schlüssel unter `result` heißt wie die Tabelle (`jobs`, `pools`, `volumes`, `clients`, `storages`, `jobmedia`, `joblog`, `filesets`, …), alle Werte sind Zeichenketten. Mit `limit=` kommt `meta.range` mit `limit`, `offset` und `filtered` dazu. Fehler stehen in `error.data.messages.error`, Warnungen in `…warning`. Die Katalogmeldungen fehlen. `list nextvolume` liefert unter `.api 2` ein leeres `result`.
- **`.api json compact=yes`**: dasselbe JSON in einer Zeile.

## Beispiele

### Jobs anzeigen

`list jobs` zeigt alle Jobs. Mit `limit=` begrenzt man die Ausgabe.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs limit=5
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name            | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   479 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
|   480 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
|   481 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
|   482 | benchmark-small | bareos-fd | 2026-03-30 22:26:53 | 00:00:01 | B    | F     |    2,605 | 824,021,543 | T         |
|   483 | benchmark-small | bareos-fd | 2026-03-30 22:37:04 | 00:00:04 | B    | F     |    2,605 | 824,021,543 | T         |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobs filtern

Filter lassen sich kombinieren, hier nach Status, Typ und Level.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs jobstatus=T,W limit=3
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name            | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   479 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
|   480 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
|   481 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs jobtype=B joblevel=I limit=2
+-------+------------------+-----------+---------------------+----------+------+-------+----------+------------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes   | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+------------+-----------+
|   602 | backup-bareos-fd | bareos-fd | 2026-06-12 21:00:02 | 00:00:01 | B    | I     |        7 | 57,746,072 | T         |
|   605 | backup-bareos-fd | bareos-fd | 2026-06-15 21:00:02 | 00:00:01 | B    | I     |        7 | 57,978,936 | T         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Nur Jobs eines Clients oder eines Jobnamens:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs client=bareos-fd limit=2
Using Catalog "MyCatalog"
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name            | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   479 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
|   480 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs jobname=BackupCatalog limit=2
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+---------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
| jobid | name          | client    | starttime           | duration | type | level | jobfiles | jobbytes | jobstatus |
+-------+---------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
|   508 | BackupCatalog | bareos-fd | 2026-04-03 21:30:06 | 00:30:07 | B    | F     |        0 |        0 | f         |
|   511 | BackupCatalog | bareos-fd | 2026-04-06 21:30:12 | 00:30:06 | B    | F     |        0 |        0 | f         |
+-------+---------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Jobs der letzten Tage:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs days=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
|     5 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:02 | 00:00:00 | B    | I     |       12 |  965,864 | T         |
|     9 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:02 | 00:00:00 | B    | I     |        0 |        0 | T         |
|    10 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:02 | 00:00:01 | B    | I     |        0 |        0 | T         |
|    11 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:03 | 00:00:01 | B    | I     |        0 |        0 | T         |
|    12 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:13 | 00:00:00 | B    | I     |        0 |        0 | T         |
|    13 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:45 | 00:00:01 | B    | I     |        0 |        0 | T         |
|    14 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:52 | 00:00:00 | B    | I     |        0 |        0 | f         |
|    15 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:56 | 00:00:00 | B    | I     |        0 |        0 | T         |
|     6 | BackupCatalog    | bareos-fd | 2026-10-15 21:10:02 | 00:00:00 | B    | F     |        0 |        0 | E         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Jobs auf einem bestimmten Volume:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs volume=Full-47321
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   600 | backup-bareos-fd | bareos-fd | 2026-06-11 21:00:02 | 00:00:01 | B    | F     |      599 | 157,762,030 | T         |
|   601 | BackupCatalog    | bareos-fd | 2026-06-11 21:10:04 | 00:00:01 | B    | F     |       67 |      14,972 | W         |
|   603 | BackupCatalog    | bareos-fd | 2026-06-12 21:10:04 | 00:00:00 | B    | F     |       67 |      14,972 | W         |
[…]
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Letzter Lauf je Job, umgekehrte Reihenfolge, Seiten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs last
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   506 | benchmark-small  | bareos-fd | 2026-03-30 22:53:21 | 00:00:54 | B    | F     |    2,605 | 824,021,543 | T         |
|   628 | backup-bareos-fd | bareos-fd | 2026-07-10 21:00:02 | 00:00:01 | B    | I     |        0 |           0 | T         |
|   629 | BackupCatalog    | bareos-fd | 2026-07-10 21:10:04 | 00:00:01 | B    | F     |       67 |      14,972 | W         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs reverse limit=3
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
|     6 | BackupCatalog    | bareos-fd | 2026-10-15 21:10:02 | 00:00:00 | B    | F     |        0 |        0 | E         |
|    15 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:56 | 00:00:00 | B    | I     |        0 |        0 | T         |
|    14 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:52 | 00:00:00 | B    | I     |        0 |        0 | f         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs limit=2 offset=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name            | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   480 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
|   481 | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     |    2,605 | 824,021,543 | T         |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobs zählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs count jobstatus=T
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+
| count |
+-------+
|    53 |
+-------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur Jobs, die noch konfiguriert sind

`current` blendet Jobs aus, deren Job-Ressource es nicht mehr gibt (hier fallen die alten `benchmark-small`-Läufe weg).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs current
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   507 | backup-bareos-fd | bareos-fd | 2026-04-03 21:00:02 | 00:30:01 | B    | F     |        0 |           0 | f         |
|   508 | BackupCatalog    | bareos-fd | 2026-04-03 21:30:06 | 00:30:07 | B    | F     |        0 |           0 | f         |
|   509 | backup-bareos-fd | bareos-fd | 2026-04-04 21:00:02 | 00:30:01 | B    | F     |        0 |           0 | f         |
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Einen Job anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobid=14
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
|    14 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:52 | 00:00:00 | B    | I     |        0 |        0 | f         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list ujobid=backup-bareos-fd.2026-10-13_21.00.00_09
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|     1 | backup-bareos-fd | bareos-fd | 2026-10-13 21:00:02 | 00:00:01 | B    | I     |      180 | 126,345,661 | T         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Summen je Job

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobtotals limit=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+------+--------+----------------+------------------+
| jobs | files  | bytes          | job              |
+------+--------+----------------+------------------+
|   57 |    938 |        209,608 | BackupCatalog    |
|   28 | 72,940 | 23,072,603,204 | benchmark-small  |
|   76 |    865 |    823,720,976 | backup-bareos-fd |
+------+--------+----------------+------------------+
+------+--------+----------------+
| jobs | files  | bytes          |
+------+--------+----------------+
|  161 | 74,743 | 23,896,533,788 |
+------+--------+----------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Gesicherte Dateien eines Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list files jobid=5
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
 /usr/sbin/
 /usr/sbin/bareos-webui-proxy
 /usr/sbin/mkfs.ntfs
 /usr/sbin/mkntfs
 /usr/sbin/mount.lowntfs-3g
 /usr/sbin/mount.ntfs
 /usr/sbin/mount.ntfs-3g
 /usr/sbin/ntfsclone
 /usr/sbin/ntfscp
 /usr/sbin/ntfslabel
 /usr/sbin/ntfsresize
 /usr/sbin/ntfsundelete
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### FileSet eines Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list fileset jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------+----------+------------------------+---------------------+----------------------------------------------------------------------------------------------------+
| filesetid | fileset  | md5                    | createtime          | filesettext                                                                                          |
+-----------+----------+------------------------+---------------------+----------------------------------------------------------------------------------------------------+
|         1 | SelfTest | 35wO25lkbw+icQACU80RdB | 2026-03-21 23:44:48 | FileSet {
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

 |
+-----------+----------+------------------------+---------------------+----------------------------------------------------------------------------------------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Alle FileSets im Katalog (gekürzt):

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list filesets limit=2
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------+--------------+------------------------+---------------------+----------------------------------------------------------------------------------------------------+
| filesetid | fileset      | md5                    | createtime          | filesettext                                                                                          |
+-----------+--------------+------------------------+---------------------+----------------------------------------------------------------------------------------------------+
|         1 | SelfTest     | 35wO25lkbw+icQACU80RdB | 2026-03-21 23:44:48 | FileSet {
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
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volumes eines Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobmedia jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+-------------------+------------+-----------+
| jobid | volumename        | firstindex | lastindex |
+-------+-------------------+------------+-----------+
|     1 | Incremental-47322 |          1 |       180 |
+-------+-------------------+------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volumes jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------------------+
| volumename        |
+-------------------+
| Incremental-47322 |
+-------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobprotokoll

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list joblog jobid=1 limit=5
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
 2026-03-21 23:44:50 bareos-dir JobId 1: Version: 25.0.3~pre51.cd3792b91 (18 March 2026) Debian GNU/Linux 13 (trixie)
 2026-03-21 23:44:50 bareos-dir JobId 1: Start Backup JobId 1, Job=backup-bareos-fd.2026-03-21_23.44.48_04
 2026-03-21 23:44:50 bareos-dir JobId 1: Connected Storage daemon at WS-JOERG.intranet.bernau.family:9103, encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
 2026-03-21 23:44:50 bareos-dir JobId 1:  Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
 2026-03-21 23:44:50 bareos-dir JobId 1: Probing client protocol... (result will be saved until config reload)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list joblog jobid=1 count
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-------+
| count |
+-------+
|    28 |
+-------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Letzte Protokollzeilen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list log limit=5
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
 bareos-sd JobId 629: Elapsed time=00:00:01, Transfer rate=22.73 K Bytes/second
 bareos-dir JobId 629: Insert of attributes batch table with 67 entries start
 bareos-dir JobId 629: Insert of attributes batch table done
 bareos-dir JobId 629: Error: Error running program when updating bootstrap file: |/usr/bin/bsmtp -h localhost -f "(Bareos) " -s "Bootstrap for Job BackupCatalog.2026-07-10_21.10.00_05" root: ERR=Child exited with code 1
 bareos-dir JobId 629: Error: Bareos bareos-dir 25.0.4~pre179.6e6f5b19f (16Jun26):
  OS Information:         Debian GNU/Linux 13 (trixie)
  JobId:                  629
  Job:                    BackupCatalog.2026-07-10_21.10.00_05
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Pools

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+--------+--------------+---------+---------+----------+---------------+
| poolid | name         | numvols | maxvols | pooltype | labelformat   |
+--------+--------------+---------+---------+----------+---------------+
|      1 | Scratch      |       0 |       0 | Scratch  | *             |
|      2 | Incremental  |       1 |     100 | Backup   | Incremental-  |
|      3 | Full         |       3 |     100 | Backup   | Full-         |
|      4 | Differential |       1 |     100 | Backup   | Differential- |
+--------+--------------+---------+---------+----------+---------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein einzelner Pool zeigt zusätzlich, was beim nächsten Prune wegfallen würde:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pool=Full
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+--------+------+---------+---------+----------+-------------+
| poolid | name | numvols | maxvols | pooltype | labelformat |
+--------+------+---------+---------+----------+-------------+
|      3 | Full |       3 |     100 | Backup   | Full-       |
+--------+------+---------+---------+----------+-------------+

Prune details:
  Reason: 
  Prunable volumes: 0
  Prunable jobs: 0
  Prunable bytes: 0 
  Affected statuses:
    none
  Prunable volumes:
    none
  Prunable jobs:
    none
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Clients und Storages

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list clients
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+----------+----------------+---------------+--------------+
| clientid | name           | fileretention | jobretention |
+----------+----------------+---------------+--------------+
|        1 | bareos-fd      |     5,184,000 |   15,552,000 |
|        2 | parity-ndmp-fd |             0 |            0 |
+----------+----------------+---------------+--------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list client=bareos-fd
Using Catalog "MyCatalog"
+----------+-----------+---------------+--------------+
| clientid | name      | fileretention | jobretention |
+----------+-----------+---------------+--------------+
|        1 | bareos-fd |     5,184,000 |   15,552,000 |
+----------+-----------+---------------+--------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list storages
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+-----------+------------------+-------------+
| storageid | name             | autochanger |
+-----------+------------------+-------------+
|         2 | ParityNdmpNative |           0 |
|         3 | ParityNdmp       |           0 |
|         1 | File             |           0 |
+-----------+------------------+-------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volumes

Ohne weitere Angabe gruppiert nach Pool:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list media
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Pool: Incremental
+---------+-------------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename        | volstatus | enabled | volbytes    | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+-------------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,322 | Incremental-47322 | Append    |       1 | 493,607,891 |        0 |    2,592,000 |       1 |    0 |         0 | File      | 2026-10-15 21:00:02 | File    |                0 |
+---------+-------------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
Pool: Full
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes       | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,318 | Full-0001  | Error     |       1 | 23,902,210,671 |        5 |            0 |       1 |    0 |         0 | File      | 2026-03-30 22:54:15 | File    |                0 |
|  47,320 | Full-0002  | Error     |       1 |    124,004,811 |        0 |            0 |       1 |    0 |         0 | File      |                     |         |                0 |
|  47,321 | Full-47321 | Append    |       1 |    158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
Pool: Differential
+---------+--------------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename         | volstatus | enabled | volbytes    | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+--------------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,323 | Differential-47323 | Append    |       1 | 172,444,761 |        0 |    7,776,000 |       1 |    0 |         0 | File      | 2026-06-20 21:00:02 | File    |                0 |
+---------+--------------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
Pool: Scratch
No results to list.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Alle Volumes in einer Tabelle, oder nur ein Pool:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volumes all
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+---------+--------------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename         | volstatus | enabled | volbytes       | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+--------------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,318 | Full-0001          | Error     |       1 | 23,902,210,671 |        5 |            0 |       1 |    0 |         0 | File      | 2026-03-30 22:54:15 | File    |                0 |
|  47,320 | Full-0002          | Error     |       1 |    124,004,811 |        0 |            0 |       1 |    0 |         0 | File      |                     |         |                0 |
|  47,321 | Full-47321         | Append    |       1 |    158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
|  47,322 | Incremental-47322  | Append    |       1 |    493,607,891 |        0 |    2,592,000 |       1 |    0 |         0 | File      | 2026-10-15 21:00:02 | File    |                0 |
|  47,323 | Differential-47323 | Append    |       1 |    172,444,761 |        0 |    7,776,000 |       1 |    0 |         0 | File      | 2026-06-20 21:00:02 | File    |                0 |
+---------+--------------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volumes pool=Full
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes       | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,318 | Full-0001  | Error     |       1 | 23,902,210,671 |        5 |            0 |       1 |    0 |         0 | File      | 2026-03-30 22:54:15 | File    |                0 |
|  47,320 | Full-0002  | Error     |       1 |    124,004,811 |        0 |            0 |       1 |    0 |         0 | File      |                     |         |                0 |
|  47,321 | Full-47321 | Append    |       1 |    158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ein einzelnes Volume über Namen oder ID:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list media=Full-47321
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes    | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,321 | Full-47321 | Append    |       1 | 158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volumeid=47321
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes    | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,321 | Full-47321 | Append    |       1 | 158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volume=Full-47321
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes    | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,321 | Full-47321 | Append    |       1 | 158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+-------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Anzahl der Volumes je Pool:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volumes count
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Pool: Incremental
+-------+
| count |
+-------+
|     1 |
+-------+
Pool: Full
+-------+
| count |
+-------+
|     3 |
+-------+
Pool: Differential
+-------+
| count |
+-------+
|     1 |
+-------+
Pool: Scratch
+-------+
| count |
+-------+
|     0 |
+-------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Pool: Auswahl im Dialog

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volumes pool=nosuch
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
> 2
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes       | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,318 | Full-0001  | Error     |       1 | 23,902,210,671 |        5 |            0 |       1 |    0 |         0 | File      | 2026-03-30 22:54:15 | File    |                0 |
|  47,320 | Full-0002  | Error     |       1 |    124,004,811 |        0 |            0 |       1 |    0 |         0 | File      |                     |         |                0 |
|  47,321 | Full-47321 | Append    |       1 |    158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Abbruch mit `.`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list volumes pool=nosuch
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

### Nächstes Volume eines Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list nextvolume job=backup-bareos-fd days=99
Using Catalog "MyCatalog"
Ignoring invalid value for days. Max is 50.
The next Volume to be used by Job "backup-bareos-fd" (Pool=Incremental, Level=Differential) will be Incremental-47322
The next Volume to be used by Job "backup-bareos-fd" (Pool=Incremental, Level=Incremental) will be Incremental-47322
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ohne `job=` fragt der Director nach dem Job:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list nextvolume
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
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
> 1
The next Volume to be used by Job "backup-bareos-fd" (Pool=Incremental, Level=Differential) will be Incremental-47322
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list nextvolume
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
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
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Sicherungen eines Clients

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list backups client=bareos-fd fileset=SelfTest limit=3
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
|   600 | backup-bareos-fd | bareos-fd | 2026-06-11 21:00:02 | 00:00:01 | B    | F     |      599 | 157,762,030 | T         |
|   602 | backup-bareos-fd | bareos-fd | 2026-06-12 21:00:02 | 00:00:01 | B    | I     |        7 |  57,746,072 | T         |
|   604 | backup-bareos-fd | bareos-fd | 2026-06-13 21:00:02 | 00:00:01 | B    | D     |        8 |  86,042,816 | T         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list backups client=bareos-fd order=desc limit=3
Using Catalog "MyCatalog"
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
|    15 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:56 | 00:00:00 | B    | I     |        0 |        0 | T         |
|    13 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:45 | 00:00:01 | B    | I     |        0 |        0 | T         |
|    12 | backup-bareos-fd | bareos-fd | 2026-10-15 21:00:13 | 00:00:00 | B    | I     |        0 |        0 | T         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kopien und Statistiken

Gibt es keine Kopierjobs, bleibt die Antwort leer. Für Jobs ohne Statistikdaten kommt `No results to list.`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list copies
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobstatistics jobid=600
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
No results to list.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ausgabe für Programme: `.api 1`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobmedia jobid=1
     1 | Incremental-47322 |          1 |       180 |
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ausgabe als JSON: `.api 2`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs limit=2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {
        "jobid": "479",
        "name": "benchmark-small",
        "client": "bareos-fd",
        "starttime": "2026-03-30 22:26:51",
        "duration": "00:00:02",
        "type": "B",
        "level": "F",
        "jobfiles": "2605",
        "jobbytes": "824021543",
        "jobstatus": "T"
      },
      {
        "jobid": "480",
        "name": "benchmark-small",
        "client": "bareos-fd",
        "starttime": "2026-03-30 22:26:51",
        "duration": "00:00:02",
        "type": "B",
        "level": "F",
        "jobfiles": "2605",
        "jobbytes": "824021543",
        "jobstatus": "T"
      }
    ],
    "meta": {
      "range": {
        "limit": 2,
        "filtered": 0
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobtotals
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {
        "jobs": "28",
        "files": "72940",
        "bytes": "23072603204",
        "job": "benchmark-small"
      },
      {
        "jobs": "57",
        "files": "938",
        "bytes": "209608",
        "job": "BackupCatalog"
      },
      {
        "jobs": "75",
        "files": "685",
        "bytes": "697375315",
        "job": "backup-bareos-fd"
      }
    ],
    "jobtotals": {
      "jobs": "160",
      "files": "74563",
      "bytes": "23770188127"
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs jobtype=bogus
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "warning": [
          "Illegal jobtype bogus\n"
        ],
        "error": [
          "invalid jobtype parameter\n"
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
{"jsonrpc":"2.0","id":null,"result":{"pools":[{"poolid":"1","name":"Scratch","numvols":"0","maxvols":"0","pooltype":"Scratch","labelformat":"*"},{"poolid":"2","name":"Incremental","numvols":"1","maxvols":"100","pooltype":"Backup","labelformat":"Incremental-"},{"poolid":"3","name":"Full","numvols":"3","maxvols":"100","pooltype":"Backup","labelformat":"Full-"},{"poolid":"4","name":"Differential","numvols":"1","maxvols":"100","pooltype":"Backup","labelformat":"Differential-"}]}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### GUI-Modus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs limit=5
Using Catalog "MyCatalog"
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name            | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| 479   | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     | 2605     | 824021543   | T         |
| 480   | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     | 2605     | 824021543   | T         |
| 481   | benchmark-small | bareos-fd | 2026-03-30 22:26:51 | 00:00:02 | B    | F     | 2605     | 824021543   | T         |
| 482   | benchmark-small | bareos-fd | 2026-03-30 22:26:53 | 00:00:01 | B    | F     | 2605     | 824021543   | T         |
| 483   | benchmark-small | bareos-fd | 2026-03-30 22:37:04 | 00:00:04 | B    | F     | 2605     | 824021543   | T         |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole mit eingeschränkten Rechten

Eine Konsole mit ACLs sieht nur die erlaubten Zeilen. Hier darf `parity-limited` nur die Pools `Full` und `Scratch` sehen. Bei `list jobs limit=3` wird zuerst begrenzt und dann gefiltert: Die drei ersten Jobs gehören nicht zur Job-ACL, die Tabelle bleibt leer.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+--------+--------------+---------+---------+----------+---------------+
| poolid | name         | numvols | maxvols | pooltype | labelformat   |
+--------+--------------+---------+---------+----------+---------------+
|      1 | Scratch      |       0 |       0 | Scratch  | *             |
|      3 | Full         |       3 |     100 | Backup   | Full-         |
+--------+--------------+---------+---------+----------+---------------+
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs limit=3
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
| jobid | name            | client    | starttime           | duration | type | level | jobfiles | jobbytes    | jobstatus |
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
+-------+-----------------+-----------+---------------------+----------+------+-------+----------+-------------+-----------+
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> list volumes
Pool: Full
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes       | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,318 | Full-0001  | Error     |       1 | 23,902,210,671 |        5 |            0 |       1 |    0 |         0 | File      | 2026-03-30 22:54:15 | File    |                0 |
|  47,320 | Full-0002  | Error     |       1 |    124,004,811 |        0 |            0 |       1 |    0 |         0 | File      |                     |         |                0 |
|  47,321 | Full-47321 | Append    |       1 |    158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
Pool: Scratch
No results to list.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Eine Konsole ohne `list` in ihrer Command-ACL bekommt:

```text
parity-bvfs@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs
list: is an invalid command.
parity-bvfs@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `list command requires a keyword` | `list` ohne Schlüsselwort. Ein Schlüsselwort aus der Syntax angeben. |
| `Unknown list keyword: <wort>` | Unbekanntes Schlüsselwort (z.B. `mediatypes`), Schlüsselwort ohne nötigen Wert (`list client`, `list poolid`, `list mediaid`) oder ungültige Angaben bei `copies`/`backups` (`jobid=x`, `order=x`). |
| `'list jobs' does not allow an jobs parameter` | `list jobs=<wert>`: `jobs` nimmt keinen Wert. `job=<name>` verwenden. |
| `Missing job parameter`, `Missing jobid parameter` | `list job` bzw. `list jobid` ohne `=<wert>`. |
| `invalid jobstatus parameter` | Leerer oder unbekannter Status bei `jobstatus=`. |
| `invalid joblevel parameter` | Unbekanntes Level, z.B. `VirtualFull`. Nur `F`, `I`, `D` bzw. `Full`, `Incremental`, `Differential`. |
| `Illegal jobtype <typ>` / `invalid jobtype parameter` | Unbekannter Jobtyp. Der Name muss exakt geschrieben sein (`Backup`, nicht `backup`). |
| `invalid client argument: <name>` | Der Client steht nicht im Katalog. |
| `jobid not found in db, access to job or client denied by ACL, or client not found in db` | `files`, `basefiles`, `jobmedia`, `joblog` ohne gültige JobId, mit einer unbekannten JobId oder ohne Zugriffsrecht. Bei `fileset` ohne Angabe zusätzlich `or missing filesetid`. |
| `missing parameter: client` | `list backups` ohne `client=`. Danach folgt `Unknown list keyword: backups`. |
| `Access to specified FileSet not allowed.` | `backups … fileset=<name>` mit einem FileSet außerhalb der FileSet-ACL der Konsole. |
| `no jobid given` | `list jobstatistics` ohne `jobid=` oder mit einer JobId ohne Zugriffsrecht. |
| `No results to list.` | Keine passenden Datensätze. |
| `Could not find Pool "<name>": ERR=Pool record not found in Catalog.` | Unbekannter Pool bei `list volumes pool=`; es folgt die Auswahl. Nach Abbruch: `Pool <name> doesn't exist.` |
| `Ignoring invalid value for days. Max is 50.` | `days=` bei `nextvolume` größer als 50 oder negativ; der Wert wird ignoriert. |
| `Could not find next Volume for Job <job>.` | Für den Job lässt sich kein Volume bestimmen, z.B. bei einem Restore-Job. |
| `list: is an invalid command.` | Die Konsole darf `list` nicht ausführen (Command-ACL). |

`list nextvolume` mit einem unbekannten Job meldet unter Onesimus Erfolg, das Original einen Fehler (Kompatibilitätslog C-046).

## Siehe auch

- [llist](llist.md) — dieselben Daten mit allen Spalten untereinander
- [show](show.md) — Ressourcen aus der Konfiguration statt aus dem Katalog
- [use](use.md) — Katalog wählen
- [sqlquery](sqlquery.md) — eigene SQL-Abfragen
- [query](query.md) — vordefinierte Katalogabfragen
- [status](status.md) — laufende und geplante Jobs
- [gui](gui.md) — GUI-Modus ein- und ausschalten
- [dot-api](dot-api.md) — API-Modus umschalten
