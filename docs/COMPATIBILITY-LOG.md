# Kompatibilitätslog Bareos ↔ Onesimus

Fortlaufendes Protokoll aller belegten Fehlinterpretationen des Bareos-Verhaltens
durch onesimus-dird. Quelle jedes Eintrags ist ein Paritätsfall aus
`tests/parity/cases/*.jsonl` (Lauf: `ci/local-ci.py` bzw.
`tests/parity/parity.py run`, Rohdaten unter `target/parity/out/`) oder ein
Befund beim Aufbau der Testumgebung.

Referenz: Original `bareos-dir 25.1.2~pre43.18be77e68`, Quellstand
`fc2c79b6` (`WIP/25.1.2-pre`). Beide Directors laufen nacheinander mit
byte-identischer Konfiguration, demselben Katalog, `LC_ALL=C`, `TZ=UTC`.

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
| C-008 | 15.10.2026 | Banner | Verbindungsaufbau | Ein Frame `1000 OK: <dir> Version: <v> (<date>) -- <cipher>` (Konsole < 18.2) bzw. 1000/1002 mit Community-Hinweis | Kein Datum/Cipher, zusätzliche 1002-Zeile | `authenticate_console.cc:129` | offen |
| C-009 | 15.10.2026 | Version | `version`, `status dir`, `.status dir header`, `memory` | `bareos-dir Version: 25.1.2~pre43… (09 October 2026) <distname> <distver> …` | `25.1.2+onesimus.0.2.0 (Debian …)` ohne Datum | `ua_cmds.cc` `VersionCmd` | offen — Entscheidung 15.10.2026: Bareos-Format byte-gleich, nur Versionstoken `25.1.2+onesimus.<gui>` eigen; Parität normalisiert ausschließlich dieses Token. Folgeentscheidung: Binary-Info/Support-Hinweis im gleichen Format, aber eigener Onesimus-Text (kein Bareos-Community-Anspruch), ebenfalls gezielt normalisiert |
| C-010 | 15.10.2026 | whoami | `whoami` (Default-Konsole) | `root` | `*UserAgent*` | `ua_cmds.cc:2758` | behoben (C-010 merge) |
| C-011 | 15.10.2026 | Dialoge | `run`, `prune`, `purge`, `delete`, `update`, `label`, `relabel`, `create`, `add`, `query`, `sqlquery`, `trace`, `setdebug`, `setbandwidth` | Interaktive Auswahl/Eingabe mit Signal `BNET_SUB_PROMPT` (-27) | Fehlermeldung bzw. Usage, Ende mit EOD | `ua_input.cc`, `ua_select.cc` | offen |
| C-012 | 15.10.2026 | **Seiteneffekte** | `truncate`, `setip`, `import`, `export`, `move` ohne Argumente | Abbruch mit Usage/Auswahl, keine Wirkung | `truncate` führt ein Truncate aus, `setip` meldet „Client address updated“, `import/export/move` senden Befehle an den SD | `ua_cmds.cc`, `ua_impexp.cc` | teilweise behoben (truncate, setip, import/export/move, run, restore, label/relabel, delete/create/add, enable/disable, rerun; prune/purge folgen) |
| C-013 | 15.10.2026 | Storage-Auswahl | `mount`, `release`, `status` ohne Ziel | Einzige Storage wird automatisch gewählt (`Automatically selected Storage: File`) | `Storage '?' not found` | `ua_select.cc:1228` | offen |
| C-014 | 15.10.2026 | status | `status scheduler`, `status subscriptions`, `status configuration`, `status dir`, `.status dir …` | Eigene Unterabschnitte | Fehlen bzw. Delegation an Gesamtstatus | `ua_status.cc` | offen |
| C-015 | 15.10.2026 | list | `list nextvolume`, `list mediatypes`, Volumes mit `NULL`-Datum, Leerzeile nach Einzelpool, `llist pools` Prune-Summen, `list jobs` Jobauswahl | Original-Filter und Formatierung | Fehlende Schlüsselwörter, `NULL` statt leer, fehlende Felder, andere Jobmenge | `ua_output.cc` | offen |
| C-016 | 15.10.2026 | .sql / sqlquery | `.sql query=…`, `sqlquery` | Tabellen-Rahmen wie `list` | Psql-artige Spalten | `ua_dotcmds.cc` `SqlCmd` | offen |
| C-017 | 15.10.2026 | estimate | `estimate job=…` | FD-Anfrage, `2000 OK estimate files=… bytes=…`, Listing | Platzhalter „Estimate pending“ | `ua_cmds.cc` `EstimateCmd` | offen |
| C-018 | 15.10.2026 | resolve | `resolve`, `resolve client=` | Reihenfolge ipv6 vor ipv4, doppelte Adresse wie getaddrinfo, TLS-Handshakezeile | Andere Reihenfolge, Duplikate entfernt, keine Handshakezeile | `ua_cmds.cc` `ResolveCmd` | offen |
| C-019 | 15.10.2026 | configure | `configure export`, `configure nosuch`, `help configure` | Export in Datei unter `bareos-dir-export/`, Original-Usage, Profile-ACL-Direktiven in Usage | Inline-Ausgabe, eigene CRUD-Usage, ACL-Direktiven fehlen | `ua_configure.cc`, `dird_conf.cc` | offen |
| C-020 | 15.10.2026 | .api json | `.api json compact=yes` | Kompaktes JSON | Pretty-Print | `ua_dotcmds.cc` | offen |
| C-021 | 15.10.2026 | restore | `restore help` | `Unknown keyword: help` | Versucht Bootstrap zu erzeugen | `ua_restore.cc` | offen |
| C-022 | 15.10.2026 | bvfs | `.bvfs_get_jobids`, `.bvfs_lsdirs`, `.bvfs_lsfiles`, `.bvfs_versions` | Echte Katalogabfrage | Echo des Befehls (Platzhalter) | `ua_dotcmds.cc` | offen |
| C-023 | 15.10.2026 | var / use / memory | `var`, `use catalog=…`, `memory` | `var`: Leerzeile; `use`: keine Ausgabe; `memory`: Statuskopf | Eigene Texte bzw. `/proc`-Werte | `ua_cmds.cc` | offen |
| C-024 | 15.10.2026 | Client-Status | `status client=…` | `Probing client protocol…`, Handshakezeile | `Connected to Client … Version: 54` | `ua_status.cc` | offen |
| C-025 | 15.10.2026 | ACL in Listen | `dot_jobs-limited`, `dot_pools-limited`, `run` mit eingeschränkter Konsole | `.jobs`, `.jobdefs`, `.pools`, `.storages`, `.clients`, `.catalogs`, `.filesets`, `.schedule` und die `run`-Jobauswahl filtern nach der Ressourcen-ACL der Konsole | Ungefiltert (nur Reihenfolge korrekt) | `ua_dotcmds.cc:697,728,780,797,819,893,917,1127`, `ua_select.cc:368` | offen |
| C-026 | 15.10.2026 | ACL in list | `c002-limited-twice`, `list-jobs-limited` | `list pools`/`list jobs` filtern nach Pool-/Job-ACL der Konsole | Ungefiltert | `ua_output.cc:779-812` (`SetAclFilter`) | offen |
| C-027 | 15.10.2026 | GUI-Modus | `c002-gui` | Nach `gui on` sind numerische Spalten in Listen linksbündig | Rechtsbündig | `ua_output.cc:309` (`jcr->gui`) | offen |
| C-028 | 15.10.2026 | API 1 | `c002-api1` | `.api 1`: Listenzeilen ohne Rahmen und Kopf | Vollständige Tabelle | `ua_output.cc`, `lib/output_formatter.cc` | offen |
| C-029 | 15.10.2026 | .jobstatus | `dot-jobstatus` | Katalog-SQL, gerahmte Tabelle jobstatus/jobstatuslong/severity/exitlevel/exitstatus; Filter nur über `.jobstatus=X` | Fest codierte Tab-Liste, Filter auf erstes Argument | `ua_dotcmds.cc:744-772` | offen |
| C-030 | 15.10.2026 | prune/purge | — (Einheitentests) | `AclHasRestrictions`-Prüfung vor Katalogöffnung; Warntext bei `add`/`purge` vor `OpenClientDb` | Fehlt | `ua_prune.cc:379`, `ua_purge.cc:86,97`, `ua_cmds.cc:620` | offen |
| C-031 | 15.10.2026 | setdevice ACL | Einheitentest | `Access to storage "X" forbidden` | `Storage "X" not found.` | `ua_cmds.cc:1600` | offen |
| C-032 | 15.10.2026 | enable/disable Warnung | `.api 2` | Warnung „Enabling is a temporary operation until the director reloads…“ mit eigenem Signal und Warnliste in API2 | Kein Warnkanal | `ua_cmds.cc:1094` | offen |
| C-033 | 15.10.2026 | rerun | — | `yes` nur wenn angegeben; Auswahl per `since_jobid`/`days`/`hours` | Automatisches `yes` (wird von C-012 entfernt), Zeitauswahl fehlt | `ua_run.cc:208` | teilweise behoben (automatisches `yes` entfernt; Zeitauswahl offen) |
| C-034 | 15.10.2026 | cancel | — | `cancel storage=…` (SD-Abbruch), `cancel all`, `state=` bei laufenden Jobs | Nicht implementiert | `ua_cmds.cc:809`, `ua_select.cc:1566` | offen |
| C-035 | 15.10.2026 | Warnkanal | `.api 2` | Warnungen (WarningMsg) mit Signal und eigener JSON-Liste | Warnungen als Klartext | `ua_output.cc`, `json_output` | offen |
| C-036 | 15.10.2026 | Autodisplay | — | Bei `autodisplay on` werden Meldungen per `DotMessagesCmd` angezeigt statt „You have messages.“; vorher ACL-Prüfung `messages` | „You have messages.“ auch bei autodisplay on | `ua_server.cc` (HandleUserAgentClientRequest) | offen |
| C-037 | 15.10.2026 | Audit-Log | Arbeitsverzeichnis der Paritätsinstanz | Bei `Auditing = yes`: `Console [default] from [127.0.0.1] cmdline …`, `… Audit acl failure for Command …`, Info-Zeilen; AuditEvents-Filter | Kein Audit-Log, `MsgType::Audit` wird nie erzeugt | `ua_audit.cc:53-158` | offen |
| C-038 | 15.10.2026 | **run ohne Bestätigung** | Vorfall bei C-011-Test | `run job=X` ohne `yes` zeigt die Zusammenfassung und fragt `OK to run? (yes/mod/no):` | Startet den Job sofort (zwei echte Jobs 7/8 gestartet; mit Freigabe bereinigt: Worker beendet, Zeilen gelöscht) | `ua_run.cc` | behoben (Guard `350a7ae`→ development; Dialog selbst → C-011) |
| C-039 | 15.10.2026 | **Katalog-Jobid-Sequenz** | Katalog `bareos_cppref` | Jobids steigen monoton (Sequenz ≥ max(jobid)) | `job_jobid_seq` steht bei 8 trotz max(jobid)=629 — seit 13.10. werden alte Jobids wiederverwendet (JobId 1: Log von März, Jobzeile von Oktober). Verdacht: Rust-ORM-Migration auf Schema 2252 | `cats/ddl`, `crates/bareos-orm` | offen, Priorität hoch; Sequenz unverändert bis Freigabe |

Hinweis C-002: Text-Sitzungen über gRPC/Command-Queue nutzen denselben SessionManager und erhalten die Katalogzeilen jetzt ebenfalls einmal pro Sitzung (konsistent mit dem Original, aber neue Ausgabe an dieser Schnittstelle).
