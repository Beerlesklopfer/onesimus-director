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
| C-002 | 15.10.2026 | Katalog | ~80 `list`/`llist`/`.sql`/`.media`/`.bvfs_*`/`create`/`delete` … | Beim ersten Katalogzugriff einer Sitzung: `Automatically selected Catalog: MyCatalog` und `Using Catalog "MyCatalog"` (nicht im API-Modus) | Keine Meldung | `ua_db.cc:157` `OpenDb`, `ua_select.cc:1228` | offen |
| C-003 | 15.10.2026 | Ressourcenreihenfolge | `.jobs`, `.pools`, `.filesets`, `.schedule`, `.msgs`, `.profiles`, `.consoles`, `run` | Punkt-Listen in Original-Ladereihenfolge der Ressourcen (Dateien alphabetisch geladen, Liste umgekehrt: `.pools` → Scratch, Incremental, Full, Differential); `.consoles` endet mit `*UserAgent*` | Alphabetisch sortiert, ohne `*UserAgent*` | `ua_dotcmds.cc` (`foreach_res`) | offen |
| C-004 | 15.10.2026 | Sitzungsende | `quit`, `exit`, `.quit` | Keine Ausgabe, Verbindung wird ohne `BNET_TERMINATE` geschlossen | Sendet `bye` und Signal -4 | `ua_cmds.cc` `QuitCmd` | offen |
| C-005 | 15.10.2026 | Stille Schalter | `autodisplay on/off`, `gui on/off`, `automount on` | Keine Ausgabe | Bestätigungstext (`autodisplay off`, `gui mode on`, …) | `ua_cmds.cc` | offen |
| C-006 | 15.10.2026 | Fehlertexte | `foobar`, `enable/disable job=nosuch`, `cancel jobid=…`, `rerun`, `setdevice` | `foobar: is an invalid command.`, `Job "nosuch" not found.`, `JobId … is not running. Use Job name to cancel inactive jobs.` usw. | Eigene Formulierungen | `ua_cmds.cc:596` u. a. | offen |
| C-007 | 15.10.2026 | Leere Eingabe | Leerzeile | Antwort `BNET_EOD` ohne Daten | Keine Antwort (Timeout) | `ua_server.cc` | offen |
| C-008 | 15.10.2026 | Banner | Verbindungsaufbau | Ein Frame `1000 OK: <dir> Version: <v> (<date>) -- <cipher>` (Konsole < 18.2) bzw. 1000/1002 mit Community-Hinweis | Kein Datum/Cipher, zusätzliche 1002-Zeile | `authenticate_console.cc:129` | offen |
| C-009 | 15.10.2026 | Version | `version`, `status dir`, `.status dir header`, `memory` | `bareos-dir Version: 25.1.2~pre43… (09 October 2026) <distname> <distver> …` | `25.1.2+onesimus.0.2.0 (Debian …)` ohne Datum | `ua_cmds.cc` `VersionCmd` | Entscheidung (Versionsstring) |
| C-010 | 15.10.2026 | whoami | `whoami` (Default-Konsole) | `root` | `*UserAgent*` | `ua_cmds.cc:2758` | offen |
| C-011 | 15.10.2026 | Dialoge | `run`, `prune`, `purge`, `delete`, `update`, `label`, `relabel`, `create`, `add`, `query`, `sqlquery`, `trace`, `setdebug`, `setbandwidth` | Interaktive Auswahl/Eingabe mit Signal `BNET_SUB_PROMPT` (-27) | Fehlermeldung bzw. Usage, Ende mit EOD | `ua_input.cc`, `ua_select.cc` | offen |
| C-012 | 15.10.2026 | **Seiteneffekte** | `truncate`, `setip`, `import`, `export`, `move` ohne Argumente | Abbruch mit Usage/Auswahl, keine Wirkung | `truncate` führt ein Truncate aus, `setip` meldet „Client address updated“, `import/export/move` senden Befehle an den SD | `ua_cmds.cc`, `ua_impexp.cc` | offen, Priorität hoch |
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
