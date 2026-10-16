# Kompatibilitätslog Bareos ↔ Onesimus

Fortlaufendes Protokoll aller belegten Fehlinterpretationen des Bareos-Verhaltens
durch onesimus-dird. Quelle jedes Eintrags ist ein Paritätsfall aus
`tests/parity/cases/*.jsonl` (Lauf: `ci/local-ci.py` bzw.
`tests/parity/parity.py run`, Rohdaten unter `target/parity/out/`) oder ein
Befund beim Aufbau der Testumgebung.

Referenz: Original `bareos-dir 25.1.2~pre43.18be77e68`, Quellstand `18be77e6`
(`WIP/25.1.2-pre-43-g18be77e68`, Checkout `/tmp/bareos-director-reference-18be77e6`) seit
16.10.2026; vorher `fc2c79b6` (43 Commits älter, Unterschiede in `docs/TODO-COMMANDS.md`).
Beide Directors laufen nacheinander mit byte-identischer Konfiguration, jede Seite auf einer
eigenen Kopie des Katalog-Templates, `LC_ALL=C`, `TZ=UTC`.

Status: **offen** · **behoben** (Commit) · **Entscheidung** (bewusste
Abweichung, braucht Freigabe) · **Harness** (Fehler lag in der Testumgebung).

Neue Einträge unten anhängen; behobene Einträge bleiben mit Commit stehen.

| ID | Datum | Bereich | Fälle | Bareos (Referenz) | Onesimus (vorher) | Upstream | Status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| C-001 | 15.10.2026 | Konfiguration | Start der Testumgebung | `DirPort`, `Dir Port`, `FdPort`, `SdPort` sind Aliase von `Port`; Leerzeichen und Groß-/Kleinschreibung in Direktivnamen werden ignoriert | Nur `Port` erkannt; Schlüssel mit Leerzeichen trafen nicht → Director lauscht stillschweigend auf 9101 | `dird_conf.cc:123` (Alias `DirPort`) | behoben (dieser Lauf) |
| C-002 | 15.10.2026 | Katalog | ~80 `list`/`llist`/`.sql`/`.media`/`.bvfs_*`/`create`/`delete` … | Beim ersten Katalogzugriff einer Sitzung: `Automatically selected Catalog: MyCatalog` und `Using Catalog "MyCatalog"` (nicht im API-Modus) | Keine Meldung | `ua_db.cc:157` `OpenDb`, `ua_select.cc:1228` | behoben (`ab7ec90`; Rest `status subscriptions` → C-014, Mehrkatalog-Auswahl → C-011) |
| C-003 | 15.10.2026 | Ressourcenreihenfolge | `.jobs`, `.pools`, `.filesets`, `.schedule`, `.msgs`, `.profiles`, `.consoles`, `run` | Punkt-Listen in Original-Ladereihenfolge der Ressourcen (Dateien alphabetisch geladen, Liste umgekehrt: `.pools` → Scratch, Incremental, Full, Differential); `.consoles` endet mit `*UserAgent*` | Alphabetisch sortiert, ohne `*UserAgent*` | `ua_dotcmds.cc` (`foreach_res`) | behoben (`5a84680`; Rest: ACL-Filter → C-025, Dialogformat → C-011) |
| C-004 | 15.10.2026 | Sitzungsende | `quit`, `exit`, `.quit` | Keine Ausgabe, Verbindung wird ohne `BNET_TERMINATE` geschlossen | Sendet `bye` und Signal -4 | `ua_cmds.cc` `QuitCmd` | behoben (`07e2ae3`→ development) |
| C-005 | 15.10.2026 | Stille Schalter | `autodisplay on/off`, `gui on/off`, `automount on` | Keine Ausgabe | Bestätigungstext (`autodisplay off`, `gui mode on`, …) | `ua_cmds.cc` | behoben (C-005 merge; API-Fehlersignal-Fälle über C-006) |
| C-006 | 15.10.2026 | Fehlertexte | `foobar`, `enable/disable job=nosuch`, `cancel jobid=…`, `rerun`, `setdevice` | `foobar: is an invalid command.`, `Job "nosuch" not found.`, `JobId … is not running. Use Job name to cancel inactive jobs.` usw. | Eigene Formulierungen | `ua_cmds.cc:596` u. a. | behoben (`a90cb0f`) |
| C-007 | 15.10.2026 | Leere Eingabe | Leerzeile | Antwort `BNET_EOD` ohne Daten | Keine Antwort (Timeout) | `ua_server.cc` | behoben (`d83c82d`) |
| C-008 | 15.10.2026 | Banner | Verbindungsaufbau | Ein Frame `1000 OK: <dir> Version: <v> (<date>) -- <cipher>` (Konsole < 18.2) bzw. 1000/1002 mit Community-Hinweis | Kein Datum/Cipher, zusätzliche 1002-Zeile | `authenticate_console.cc:129` | behoben (in development, 15.10.2026) |
| C-009 | 15.10.2026 | Version | `version`, `status dir`, `.status dir header`, `memory` | `bareos-dir Version: 25.1.2~pre43… (09 October 2026) <distname> <distver> …` | `25.1.2+onesimus.0.2.0 (Debian …)` ohne Datum | `ua_cmds.cc` `VersionCmd` | behoben (`version_strings.rs`; BINARY_INFO „Onesimus“, Support-Hinweis onesimus.io) — Entscheidung 15.10.2026: Bareos-Format byte-gleich, nur Versionstoken `25.1.2+onesimus.<gui>` eigen; Parität normalisiert ausschließlich dieses Token. Folgeentscheidung: Binary-Info/Support-Hinweis im gleichen Format, aber eigener Onesimus-Text (kein Bareos-Community-Anspruch), ebenfalls gezielt normalisiert |
| C-010 | 15.10.2026 | whoami | `whoami` (Default-Konsole) | `root` | `*UserAgent*` | `ua_cmds.cc:2758` | behoben (C-010 merge) |
| C-011 | 15.10.2026 | Dialoge | `run`, `prune`, `purge`, `delete`, `update`, `label`, `relabel`, `create`, `add`, `query`, `sqlquery`, `trace`, `setdebug`, `setbandwidth` | Interaktive Auswahl/Eingabe mit Signal `BNET_SUB_PROMPT` (-27) | Fehlermeldung bzw. Usage, Ende mit EOD | `ua_input.cc`, `ua_select.cc` | behoben (Merge 15.10.2026; `cancel`-Dialog noch ohne Test) |
| C-012 | 15.10.2026 | **Seiteneffekte** | `truncate`, `setip`, `import`, `export`, `move` ohne Argumente | Abbruch mit Usage/Auswahl, keine Wirkung | `truncate` führt ein Truncate aus, `setip` meldet „Client address updated“, `import/export/move` senden Befehle an den SD | `ua_cmds.cc`, `ua_impexp.cc` | behoben (alle Guards + Seiteneffekt-Tests; Dialoge → C-011) |
| C-013 | 15.10.2026 | Storage-Auswahl | `mount`, `release`, `status` ohne Ziel | Einzige Storage wird automatisch gewählt (`Automatically selected Storage: File`) | `Storage '?' not found` | `ua_select.cc:1228` | behoben (in development, 15.10.2026) |
| C-014 | 15.10.2026 | status | `status scheduler`, `status subscriptions`, `status configuration`, `status dir`, `.status dir …` | Eigene Unterabschnitte | Fehlen bzw. Delegation an Gesamtstatus | `ua_status.cc` | behoben (Merge 15.10.2026; offen: SD-Teil von `status all`/`status storage`, Normalisierung alter Fälle) |
| C-015 | 15.10.2026 | list | `list nextvolume`, `list mediatypes`, Volumes mit `NULL`-Datum, Leerzeile nach Einzelpool, `llist pools` Prune-Summen, `list jobs` Jobauswahl | Original-Filter und Formatierung | Fehlende Schlüsselwörter, `NULL` statt leer, fehlende Felder, andere Jobmenge | `ua_output.cc` | behoben (Merge 16.10.2026; Prompt bei unbekanntem Pool → C-011-Dialog anbinden, ACL → C-026) |
| C-016 | 15.10.2026 | .sql / sqlquery | `.sql query=…`, `sqlquery` | Tabellen-Rahmen wie `list` | Psql-artige Spalten | `ua_dotcmds.cc` `SqlCmd` | behoben (in development, 15.10.2026) |
| C-017 | 15.10.2026 | estimate | `estimate job=…` | FD-Anfrage, `2000 OK estimate files=… bytes=…`, Listing | Platzhalter „Estimate pending“ | `ua_cmds.cc` `EstimateCmd` | behoben (in development, 15.10.2026) |
| C-018 | 15.10.2026 | resolve | `resolve`, `resolve client=` | Reihenfolge ipv6 vor ipv4, doppelte Adresse wie getaddrinfo, TLS-Handshakezeile | Andere Reihenfolge, Duplikate entfernt, keine Handshakezeile | `ua_cmds.cc` `ResolveCmd` | behoben (in development, 15.10.2026) |
| C-019 | 15.10.2026 | configure | `configure export`, `configure nosuch`, `help configure` | Export in Datei unter `bareos-dir-export/`, Original-Usage, Profile-ACL-Direktiven in Usage | Inline-Ausgabe, eigene CRUD-Usage, ACL-Direktiven fehlen | `ua_configure.cc`, `dird_conf.cc` | behoben (in development, 15.10.2026) |
| C-020 | 15.10.2026 | .api json | `.api json compact=yes` | Kompaktes JSON | Pretty-Print | `ua_dotcmds.cc` | behoben (in development, 15.10.2026) |
| C-021 | 15.10.2026 | restore | `restore help` | `Unknown keyword: help` | Versucht Bootstrap zu erzeugen | `ua_restore.cc` | behoben (Merge 16.10.2026; Restore-Menü → C-011-Dialog in `restore.rs prompt_menu()` anbinden; Dateibaum-Auswahl offen) |
| C-022 | 15.10.2026 | bvfs | `.bvfs_get_jobids`, `.bvfs_lsdirs`, `.bvfs_lsfiles`, `.bvfs_versions` | Echte Katalogabfrage | Echo des Befehls (Platzhalter) | `ua_dotcmds.cc` | behoben (Merge 16.10.2026; .bvfs_restore/cleanup/clear_cache nur Einheitentests) |
| C-023 | 15.10.2026 | var / use / memory | `var`, `use catalog=…`, `memory` | `var`: Leerzeile; `use`: keine Ausgabe; `memory`: Statuskopf | Eigene Texte bzw. `/proc`-Werte | `ua_cmds.cc` | behoben (in development, 15.10.2026) |
| C-024 | 15.10.2026 | Client-Status | `status client=…` | `Probing client protocol…`, Handshakezeile | `Connected to Client … Version: 54` | `ua_status.cc` | behoben (in development, 15.10.2026) |
| C-025 | 15.10.2026 | ACL in Listen | `dot_jobs-limited`, `dot_pools-limited`, `run` mit eingeschränkter Konsole | `.jobs`, `.jobdefs`, `.pools`, `.storages`, `.clients`, `.catalogs`, `.filesets`, `.schedule` und die `run`-Jobauswahl filtern nach der Ressourcen-ACL der Konsole | Ungefiltert (nur Reihenfolge korrekt) | `ua_dotcmds.cc:697,728,780,797,819,893,917,1127`, `ua_select.cc:368` | behoben (Merge 16.10.2026) |
| C-026 | 15.10.2026 | ACL in list | `c002-limited-twice`, `list-jobs-limited` | `list pools`/`list jobs` filtern nach Pool-/Job-ACL der Konsole | Ungefiltert | `ua_output.cc:779-812` (`SetAclFilter`) | behoben (Merge 16.10.2026) |
| C-027 | 15.10.2026 | GUI-Modus | `c002-gui` | Nach `gui on` sind numerische Spalten in Listen linksbündig | Rechtsbündig | `ua_output.cc:309` (`jcr->gui`) | behoben (Merge 16.10.2026) |
| C-028 | 15.10.2026 | API 1 | `c002-api1` | `.api 1`: Listenzeilen ohne Rahmen und Kopf | Vollständige Tabelle | `ua_output.cc`, `lib/output_formatter.cc` | behoben (Merge 16.10.2026) |
| C-029 | 15.10.2026 | .jobstatus | `dot-jobstatus` | Katalog-SQL, gerahmte Tabelle jobstatus/jobstatuslong/severity/exitlevel/exitstatus; Filter nur über `.jobstatus=X` | Fest codierte Tab-Liste, Filter auf erstes Argument | `ua_dotcmds.cc:744-772` | behoben (Merge 16.10.2026; .mediatypes/.locations/.media aus dem Katalog) |
| C-030 | 15.10.2026 | prune/purge | — (Einheitentests) | `AclHasRestrictions`-Prüfung vor Katalogöffnung; Warntext bei `add`/`purge` vor `OpenClientDb` | Fehlt | `ua_prune.cc:379`, `ua_purge.cc:86,97`, `ua_cmds.cc:620` | behoben (Merge 16.10.2026; prune/purge-Katalogarbeit mit Original-Bestätigungen; ActionOnPurge offen) |
| C-031 | 15.10.2026 | setdevice ACL | Einheitentest | `Access to storage "X" forbidden` | `Storage "X" not found.` | `ua_cmds.cc:1600` | behoben (Merge 16.10.2026) |
| C-032 | 15.10.2026 | enable/disable Warnung | `.api 2` | Warnung „Enabling is a temporary operation until the director reloads…“ mit eigenem Signal und Warnliste in API2 | Kein Warnkanal | `ua_cmds.cc:1094` | behoben (Merge 16.10.2026) |
| C-033 | 15.10.2026 | rerun | — | `yes` nur wenn angegeben; Auswahl per `since_jobid`/`days`/`hours` | Automatisches `yes` (wird von C-012 entfernt), Zeitauswahl fehlt | `ua_run.cc:208` | behoben (Merge 16.10.2026; since_jobid/days/hours) |
| C-034 | 15.10.2026 | cancel | — | `cancel storage=…` (SD-Abbruch), `cancel all`, `state=` bei laufenden Jobs | Nicht implementiert | `ua_cmds.cc:809`, `ua_select.cc:1566` | behoben (Merge 16.10.2026; RunScripts AfterJob bei Cancel fehlen) |
| C-035 | 15.10.2026 | Warnkanal | `.api 2` | Warnungen (WarningMsg) mit Signal und eigener JSON-Liste | Warnungen als Klartext | `ua_output.cc`, `json_output` | behoben (Merge 16.10.2026; offen: `list nextvolume` Batch-Fehler, `delete` leeres JSON-Objekt) |
| C-036 | 15.10.2026 | Autodisplay | — | Bei `autodisplay on` werden Meldungen per `DotMessagesCmd` angezeigt statt „You have messages.“; vorher ACL-Prüfung `messages` | „You have messages.“ auch bei autodisplay on | `ua_server.cc` (HandleUserAgentClientRequest) | behoben (Merge 16.10.2026; Autodisplay-Default jetzt off wie Original) |
| C-037 | 15.10.2026 | Audit-Log | Arbeitsverzeichnis der Paritätsinstanz | Bei `Auditing = yes`: `Console [default] from [127.0.0.1] cmdline …`, `… Audit acl failure for Command …`, Info-Zeilen; AuditEvents-Filter | Kein Audit-Log, `MsgType::Audit` wird nie erzeugt | `ua_audit.cc:53-158` | behoben (Merge 16.10.2026; Audit-Log-Paritätsfall identisch) |
| C-038 | 15.10.2026 | **run ohne Bestätigung** | Vorfall bei C-011-Test | `run job=X` ohne `yes` zeigt die Zusammenfassung und fragt `OK to run? (yes/mod/no):` | Startet den Job sofort (zwei echte Jobs 7/8 gestartet; mit Freigabe bereinigt: Worker beendet, Zeilen gelöscht) | `ua_run.cc` | behoben (Guard `350a7ae`→ development; Dialog selbst → C-011) |
| C-039 | 15.10.2026 | **Katalog-Jobid-Sequenz** | Katalog `bareos_cppref` | Jobids steigen monoton (Sequenz ≥ max(jobid)) | `job_jobid_seq` steht bei 8 trotz max(jobid)=629 — seit 13.10. werden alte Jobids wiederverwendet (JobId 1: Log von März, Jobzeile von Oktober). Verdacht: Rust-ORM-Migration auf Schema 2252 | `cats/ddl`, `crates/bareos-orm` | Ursache: manueller Katalogimport am 13.10.2026 (COPY mit expliziten IDs ohne setval) in einer früheren Sitzung, nicht Rust-Code; Schutz in `bareos-orm` (keine destruktiven Migrationen, Sequenzprüfung mit Warnung) übernommen; Katalog am 15.10.2026 mit Freigabe repariert: 7 Sequenzen (path, job, fileset, jobmedia, media, pool, log) per setval auf max(id) |
| C-040 | 15.10.2026 | Lokale Erweiterungen | — | — (keine Entsprechung) | `.catalog reset` / `.catalog drop` führen `DROP TABLE … CASCADE` aus, nur durch ACL-TODO geschützt | `crates/onesimus-dird/src/commands/catalog.rs:290,340` | behoben (in development, 15.10.2026) |
| C-041 | 15.10.2026 | Konfiguration JobDefs | `c011-run-select-no` | Explizites `Level = Full` im Job hat Vorrang vor JobDefs | JobDefs-Merge in `director_config` überschreibt das explizite Level (BackupCatalog zeigt Incremental) | `dird_conf.cc` (JobDefs-Vererbung) | behoben (Merge 16.10.2026; gemeinsame JobDefs-Vererbung in `bareos-config/src/jobdefs.rs`) |
| C-042 | 16.10.2026 | Storage-Status | `status-storage`, `status-bogus`, `c014-status-all` | SD-Statusbericht nach `Connecting to Storage daemon File at …` mit Verschlüsselungszeile und Original-Einrückung (StorageStatus, ua_status.cc) | „Connecting to Storage File …“, „Connected to Storage“, NUL-Bytes, fehlende Abschnitte | `ua_status.cc` DoStorageStatus | behoben (Merge 16.10.2026; Autochanger-Slottabelle → C-045) |
| C-043 | 16.10.2026 | run Restore-Job | — | `run job=RestoreFiles` fragt „Please enter a JobId for restore:“; `no` → „Expected a positive integer, got: no“ | Direkt „OK to run?“, dann „Job not run.“ | `ua_run.cc` (Restore-Job-Prompts) | offen |
| C-044 | 16.10.2026 | .pools type= | — | Typvergleich als Präfix ohne Groß-/Kleinschreibung (`bstrncasecmp` mit Argumentlänge) | Exakter Vergleich | `ua_dotcmds.cc:914` | behoben (Merge 16.10.2026; Präfix-/FindArg-Semantik in dot-Listen, list, status, Speicher-/Zeitangaben) |
| C-045 | 16.10.2026 | status slots | — (kein Autochanger im Fixture) | `status storage=… slots`: Slottabelle, API-Zeilen `D|`/`S|`, JSON „contents“, Katalog-Lookups | Endet nach der Volume-Liste | `ua_status.cc:1703,1804,1917`, `storage.cc:553` | behoben (Merge 16.10.2026; nur gegen simulierten Autochanger getestet) |
| C-046 | 16.10.2026 | .api 2 Ergebnisobjekte | `c035-delete-noarg-api2`, `c035-list-nextvol-days-nojob-api2` | `delete` liefert `"deleted": {}`; `list nextvolume` mit unbekanntem Job scheitert im Batchmodus (CMD_FAILED) | Leeres Ergebnis bzw. Erfolg | `ua_cmds.cc:2226`, `ua_output.cc` | behoben (Merge 16.10.2026; `delete` mit echter Purge-Logik, `status` ohne Argument mit Auswahl, `show`/`wait`/`configure`-Rückgaben; `run`-Objekt → C-043) |
| C-047 | 16.10.2026 | restore | — | Menüpunkte 1, 2, 4–12 (inkl. `with_gui` für 1/2/4), Dateibaum-Auswahl (mark/cd/ls …), mehrere JobIds, regexwhere | „Menu item … is not available yet“, „Restore not done.“ | `ua_restore.cc:675-835,1294`, `ua_tree.cc` | weitgehend behoben (Merge 16.10.2026: Restore-Baum mit 26 Befehlen, alle 13 Menüpunkte, Schlüsselwörter, Bootstrap byte-gleich; offen: Run-Zusammenfassung → C-043, Info-Signale in `.api 2`, Nicht-UTF-8-Namen auf SQL_ASCII) |
| C-048 | 16.10.2026 | cancel / Jobzustände | — | RunScripts „AfterJob“ bei Cancel, Warte-Zustände (FD/SD/Ressourcen/Priorität), JCR-Liste inkl. beendeter Jobs | Fehlen | `job.cc:684-741`, `director_jcr_impl.h:54` | behoben (Merge 16.10.2026; Job-Queue wie jobq.cc, Wartezustände, AfterJob bei Cancel; offen: Console-RunScripts, Reschedule/MaxStartDelay/MaxWaitTime) |
| C-049 | 16.10.2026 | purge action= | — | ActionOnPurge (Relabel über SD) | Lokale Meldung „not supported“ | `ua_purge.cc:690ff` | behoben (Merge 16.10.2026; SD-Relabel nur gegen simulierten SD getestet) |
| C-050 | 16.10.2026 | Speicherprotokoll | — | NDMP-Storages/-Clients werden gesondert behandelt (setdevice überspringt, Status, Mount) | Kein Protokollfeld in StorageResource, nur natives Protokoll | `ua_cmds.cc:1552`, `storage.cc:727` | offen |
| C-051 | 16.10.2026 | Jobmeldungen | — | Originaltexte der Jobmeldungen („Start Backup JobId …“, Abschlussbericht), Jmsg der Konsolen-JCR | Eigene Texte (nur Kopf und Routing wie Original) | `backup.cc`, `job.cc`, `ua_run.cc:80-187` | offen |
| C-052 | 16.10.2026 | Katalogschema (ORM) | — | `PathVisibility` Primärschlüssel `(JobId, PathId)` | Schemabeschreibung `crates/bareos-orm/schema/v2250_bareos_catalog.json` nennt nur `jobid` (wirkt nur, wenn Rust das Schema selbst anlegt; Testkatalog korrekt aus Original-DDL) | `ddl/creates/postgresql.sql:356-362` | behoben (Merge 16.10.2026; Schemabeschreibung = Bareos-DDL 2250, Drift-Test `bareos-orm/tests/upstream_ddl.rs`; `.catalog create/reset` repariert) |
| C-053 | 16.10.2026 | truncate / purge action=truncate | — | Truncate über SD-Relabel; Antwort `3000 OK label. VolFiles=… VolBytes=…` gilt als Erfolg (18be77e6) | `truncate` sendet nicht-Bareos-Befehl `truncate Volume=`, wertet keine Relabel-Antwort aus; `action=truncate` fehlt | `ua_purge.cc:654` | offen |
| C-054 | 16.10.2026 | update slots | — | Slots aus dem Autochanger, `MakeInchangerUnique`, Warnung „Could not set inchanger to 0 for slot %d.“ | „not supported“ | `ua_update.cc:1228`, `sql_update.cc:364,445` | behoben (Merge 16.10.2026; nur gegen simulierten Autochanger getestet) |
| C-055 | 16.10.2026 | CatReq UpdateMedia | — | Plausibilitätsprüfungen beim Medien-Update (VolFiles rückwärts → VolBlocks zurücksetzen, „Ignoring Volume Blocks…“) | Keine Prüfungen, VolBlocks nicht behandelt | `catreq.cc:258-280` | behoben (Merge 16.10.2026; PruneVolumes/Purge-/Recycle-Oldest und LabelFormat mit Variablen offen) |
| C-056 | 16.10.2026 | setbandwidth | — | `SetbwlimitCmd`: Auswahl per job/jobid/ujobid/all/state, Original-Fehlertexte | Nur `client=`/`storage=` mit `limit=`, eigene Fehlertexte | `ua_cmds.cc:935` | offen |
| C-057 | 16.10.2026 | reload | — | Nach `reload` zeigt `show clients` beim Client `Catalog = "MyCatalog"` | Zeile fehlt nach `reload`; Fehlerpfad von reload nicht verglichen | `dird.cc` (reload) | offen |

Hinweis C-002: Text-Sitzungen über gRPC/Command-Queue nutzen denselben SessionManager und erhalten die Katalogzeilen jetzt ebenfalls einmal pro Sitzung (konsistent mit dem Original, aber neue Ausgabe an dieser Schnittstelle).

## Stand 16.10.2026

`development` = `1b074e1` (gepusht nach `origin/development`). Alle Agent-Branches beider
Runden sind gemergt. Tests grün: onesimus-dird 365 + 4, oconsole 26, bareos-config 37,
bareos-bnet 46. Voller Paritätslauf auf dem Endstand (16.10.2026, Baseline aktualisiert): 1436 von 1443
Fällen byte-identisch, keine Regression.

Verbleibende Unterschiede im Paritätslauf: nur noch `c035-delete-noarg-api2` (C-046).
Die alten Fälle `banner*`, `status-dir`, `status-subscriptions`, `dot-status-dir-running`
normalisieren seit 16.10.2026 Versionstoken, Build-Datum, Binary-Info und Zeiten wie die
neueren Fälle. `var`: das Original gibt bei nacktem `var` undefinierte Speicherreste aus;
der Fall prüft daher nur noch die Katalogzeilen davor (Hinweis im Fall, Commit `dd377c0`).

Instabile Fälle (gelegentlich rot, allein grün): `status-client` (echter FD wird von
mehreren Directors gleichzeitig benutzt), `list-jobs` (Jobs mit gleicher Startzeit ohne
eindeutige Sortierung), `c015-list-jobs-enabled-count` (undefiniertes Verhalten im Original).

### Offen

- Einträge mit Status „offen“ in der Tabelle: C-043, C-047 (Rest), C-050, C-051, C-053, C-056, C-057 (Agenten am 16.10.2026 durch das Ausgabenlimit unterbrochen; Arbeitsstand liegt in ihren Worktrees).
- Befehlsstatus im Wiki (`docs/wiki/commands`): 53 `parity_verified`, 31 `partial`, 4 `registered_unverified`, 26 Restore-Baum-Befehle `missing_context` (Stand vor den C-045/C-049/C-055-Merges, Auffrischung offen).
- OVFS-Integration im Director gemergt (16.10.2026): Erkennung beim Connect (Version 0.1.x + Signaturen), Befehle `.ovfs_status/_latest/_tree/_ls/_versions`, MCP-Tools in oconsole, SQL-Rückfall byte-gleich zur Extension (Äquivalenztest mit gebauter Extension auf PG 17).
- OVFS (eigene PostgreSQL-Extension, paralleles Projekt `oenesimus-postgres-plugin`) ist der Onesimus-Weg für Verzeichnisbäume und Restore-Auswahl. BVFS (`commands/bvfs.rs`, byte-gleich portiert, Fälle `c022`) bleibt nur zur Abwärtskompatibilität für Bareos-Clients (z. B. bareos-webui) erhalten und darf sich dabei nicht ändern.
- Erledigt 16.10.2026: C-012s `update`-Validierung nach dem C-011-Merge geprüft; `maxvoljobs`/`maxvolfiles` war verloren und ist wiederhergestellt (Fälle `t001`).
- Erledigt 16.10.2026: Referenzquelle auf `18be77e6` umgestellt; Generatoren neu erzeugt, bvfs-Pfad-Escaping und query-Prompts angepasst.

### Testumgebung und Regeln

- Paritätslauf: `python3 tests/parity/parity.py run`; jede Seite arbeitet auf einer eigenen
  Wegwerfkopie des Katalog-Templates `bareos_cppref` (CREATE DATABASE … TEMPLATE, Rolle
  `joerg.bernau` hat CREATEDB). `--shared-db` nur für rein lesende Fälle.
- Lokale CI: `ci/local-ci.py`; jeder Check ein eigener Schritt, Fehler werden nur berichtet.
- Katalogkopien isolieren nur die Datenbank. FD/SD sind echt und gemeinsam (16.10.2026: die Fälle `c035-setbandwidth-api1/-api2` setzten bei jedem Lauf `bwlimit=100kB/s` am echten FD; entfernt, der Warnkanal von `setbandwidth` bleibt per Einheitentest abgedeckt): keine
  zustandsändernden FD/SD-Befehle live (setbandwidth mit Limit, setdebug client/storage,
  setdevice auf echte Geräte, unmount/label, cancel an Daemons).
- Bereinigungen am Testkatalog nur mit Freigabe des Nutzers. Am 15./16.10.2026 bereinigt:
  Jobs 7/8, 7 Sequenzen, JobHisto, JobRetention bareos-fd, `bwlimit` am echten FD.
