# TODO-COMMANDS — Vollständigkeitsprüfung und Roadmap

Stand: **16.10.2026**, abgeleitet auf `development` = `7993ce6`. Diese Roadmap
führt den Befehlsabgleich mit den Belegen des Paritätsvergleichs zusammen.
Maßgeblich sind die [Quellreferenz](wiki/commands/bareos-reference.md), die
[LLM-JSON](wiki/commands/bareos-director-commands.json), die
[Einzelbefunde](wiki/commands/audit-notes.json), das
[Kompatibilitätslog](COMPATIBILITY-LOG.md) und der [Testplan](TESTPLAN.md).
Weitere Architekturvorhaben stehen im [Backlog](BACKLOG.md).

Die frühere Aufteilung 23 teilweise / 50 ungeprüft / 2 abweichend / 13 Platzhalter
stammte aus der statischen Prüfung **vor** der Paritätsarbeit (15.10.2026) und ist
überholt: alle 13 Platzhalter sind inzwischen echte Implementierungen (BVFS C-022,
`estimate` C-017, `automount` C-005, `setip` C-012, `use`/`var` C-023), `query`
führt das Original-QueryFile-Menü aus, `move` wertet `srcslots`/`dstslots` aus.

## Geprüfter Stand

| Kategorie | Hauptbefehle | Bedeutung |
| --- | ---: | --- |
| Originalnamen registriert | 88/88 | Aufrufname vorhanden |
| Paritätsfälle vorhanden | 84/88 | Mindestens ein Fall in `tests/parity/cases/*.jsonl` ruft den Befehl auf |
| `parity_verified` | 53 | Alle Fälle des Befehls byte-identisch (Text, API 1/2, eingeschränkte Konsolen und Fehlerpfade, soweit abgedeckt), kein offener Logeintrag, keine bekannte ungedeckte Wirkung |
| `partial` | 31 | Fälle vorhanden, aber offene Logeinträge, abweichende Fälle, ungedeckte Varianten oder nur einheitengetestete Seiteneffekte |
| `registered_unverified` | 4 | Handler vorhanden, kein Paritätsfall |
| `stub` / `semantic_mismatch` / `missing` | 0 | — |
| Restore-Dateiauswahl (`missing_context`) | 26/26 | Eigener Dialogkontext fehlt (C-047) |

Paritätslauf: `python3 tests/parity/parity.py run --port 39930 --work target/parity`
(jede Seite auf eigener Katalogkopie) auf `7993ce6`: **1479 von 1481 Fällen
byte-identisch**, 2 Unterschiede, 0 Fehler. Abweichend: `c035-delete-noarg-api2`, `c042-status-client-star`.

**`parity_verified` heißt nicht „vollständig kompatibel“.** Der Status gilt nur
für die in `audit-notes.json` je Befehl unter `parity_cases` genannten Varianten.
Der Vergleich prüft Antwortbytes, Signale und Endgrund je Schritt sowie das
Audit-Log, nicht den Katalogzustand danach. Seiteneffekte auf echte FD/SD
(Labeln, Unmount, Bandbreiten-/Debuglimits, Cancel laufender Jobs, echte
Backup-/Restore-Jobs) werden im Paritätslauf nicht ausgeführt; sie sind nur durch
Einheitentests belegt, und die betroffenen Befehle stehen deshalb auf `partial`.

### Statusregeln

- Zuordnung Fall → Befehl: erstes Wort jedes Schritts, aufgelöst wie im Original
  (erster Tabelleneintrag mit diesem Präfix, ohne Groß-/Kleinschreibung);
  Antworten auf Dialoge (nach `BNET_SUB_PROMPT` bzw. `.`, Zahlen, `yes/no/mod`)
  zählen nicht als Befehl. `setup`-Schritte werden nicht verglichen.
- Instabile Fälle laut Kompatibilitätslog (`status-client`, `list-jobs`,
  `c015-list-jobs-enabled-count`) sowie `c042-status-client-star` (im vollen Lauf
  eine zusätzliche Director-Verbindung am gemeinsamen echten FD, allein grün)
  werden als instabil genannt, stufen aber nicht herab.
- Beobachtung: Der Statusbericht des echten FD zeigt `bwlimit=100kB/s`; die Fälle
  `c035-setbandwidth-api1/-api2` setzen vermutlich dieses Limit live am gemeinsamen FD
  (nicht bereinigt; Bereinigung nur mit Freigabe).
- Ein offener Eintrag im Kompatibilitätslog stuft den betroffenen Befehl auf
  `partial`, auch wenn alle vorhandenen Fälle grün sind.
- Kein Statuswechsel auf `parity_verified` allein aufgrund lokaler Unit-Tests.

## Befehle nach Status

### `parity_verified` (53)

`.`, `.actiononpurge`, `.api`, `.authorized`, `.catalogs`, `.clients`, `.consoles`, `.users`, `.defaults`, `.filesets`, `.help`, `.jobdefs`, `.jobs`, `.jobstatus`, `.levels`, `.locations`, `.messages`, `.media`, `.mediatypes`, `.msgs`, `.profiles`, `.quit`, `.sql`, `.schedule`, `.storages`, `.types`, `.volstatus`, `.bvfs_lsdirs`, `.bvfs_lsfiles`, `.bvfs_update`, `.bvfs_get_jobids`, `.bvfs_versions`, `autodisplay`, `automount`, `configure`, `disable`, `enable`, `estimate`, `exit`, `gui`, `help`, `messages`, `memory`, `quit`, `query`, `resolve`, `show`, `sqlquery`, `time`, `use`, `var`, `version`, `whoami`.

### `partial` (31)

| Befehl | Grund | Offene Logeinträge |
| --- | --- | --- |
| `.pools` | 21/21 Fälle gleich. `type=` vergleicht exakt statt als Präfix ohne Groß-/Kleinschreibung (C-044); die Fälle prüfen nur vollständige Typnamen. | C-044 |
| `.status` | 22/22 Fälle gleich. Warte-Zustände laufender Jobs (FD/SD/Ressourcen/Priorität) und beendete Jobs in der JCR-Liste fehlen (C-048); im Fixture läuft kein Job. | C-048 |
| `.bvfs_cleanup` | 1/1 Fälle gleich. Nur `.bvfs_cleanup` ohne Argumente verglichen; das Löschen einer Handle-Tabelle ist nur durch Einheitentests belegt. | — |
| `.bvfs_clear_cache` | 3/3 Fälle gleich. Verglichen sind Ausgaben ohne `yes`; `.bvfs_clear_cache yes` ändert den Katalog und ist nur durch Einheitentests belegt. | — |
| `add` | 9/9 Fälle gleich. Verglichen sind Dialog, ACL-Warnung und Abbruch; das tatsächliche Anlegen von Volumes und der Katalogzustand danach sind nicht verglichen. | — |
| `cancel` | 24/24 Fälle gleich. Verglichen sind Selektion und Fehlertexte nur mit nicht existierenden bzw. nicht laufenden Jobs. Abbruch laufender Jobs über FD/SD ist nur durch Einheitentests belegt. RunScripts „AfterJob“ bei Cancel und Warte-Zustände fehlen (C-048). | C-048 |
| `create` | 4/4 Fälle gleich. Verglichen sind Dialog und unbekannter Pool; das Anlegen eines Pools im Katalog ist nicht verglichen. | — |
| `delete` | 15/16 Fälle gleich. Abweichende Fälle: `c035-delete-noarg-api2`. `.api 2`: Original liefert `"deleted": {}`, Onesimus ein leeres Ergebnis (C-046, Fall `c035-delete-noarg-api2`). Löschen von Volume/Pool/Job wird nur über Dialog- und Abbruchpfade verglichen; die Katalogwirkung ist nur durch Einheitentests belegt. | C-046 |
| `export` | 2/2 Fälle gleich. Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen sind nur durch Einheitentests belegt. | C-045 |
| `import` | 2/2 Fälle gleich. Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen und Katalogupdates sind nur durch Einheitentests belegt. | C-045 |
| `label` | 13/13 Fälle gleich. Verglichen sind Dialog-, Namens- und Abbruchpfade; das Labeln selbst (SD-Wirkung, Katalogeintrag) ist laut Testregeln nicht live erlaubt und nur durch Einheitentests belegt. | — |
| `list` | 299/299 Fälle gleich. `list nextvolume` mit unbekanntem Job scheitert im Original im Batchmodus (CMD_FAILED), Onesimus meldet Erfolg (C-046). | C-046 |
| `llist` | 110/110 Fälle gleich. `llist nextvolume` teilt den Pfad von `list nextvolume` (C-046). | C-046 |
| `mount` | 16/16 Fälle gleich. Autochanger-Slots und NDMP fehlen bzw. sind ungeprüft (C-045/C-050 Umfeld); verglichen sind Auswahl und Fehlerpfade am File-Storage. | C-050 |
| `move` | 2/2 Fälle gleich. Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen sind nur durch Einheitentests belegt. Die Originalsyntax srcslots/dstslots wird inzwischen ausgewertet. | C-045 |
| `prune` | 35/35 Fälle gleich. Verglichen sind Dialoge, ACL-Prüfungen (C-030) und Bestätigungstexte auf der Katalogkopie; der Katalogzustand nach dem Prunen wird nicht verglichen. | — |
| `purge` | 34/34 Fälle gleich. `action=` (ActionOnPurge, Relabel über SD) fehlt (C-049); `action=truncate` fehlt (C-053). Verglichen sind Dialoge, ACL-Prüfungen und Bestätigungstexte auf der Katalogkopie; der Katalogzustand danach wird nicht verglichen. | C-049, C-053 |
| `restore` | 28/28 Fälle gleich. Menüpunkte 1, 2, 4–12, Dateibaum-Auswahl, mehrere JobIds und regexwhere fehlen (C-047). Verglichen sind Argumentbehandlung und Abbruchpfade; ein echter Restore läuft im Paritätslauf nicht. | C-047, C-051 |
| `relabel` | 8/8 Fälle gleich. Verglichen sind Dialog- und Abbruchpfade; Relabel über den SD ist nur durch Einheitentests belegt (Antwortauswertung siehe C-053). | C-053 |
| `release` | 7/7 Fälle gleich. Verglichen sind Auswahl und Fehlerpfade am File-Storage; NDMP (C-050) ungeprüft. | C-050 |
| `rerun` | 32/32 Fälle gleich. Auswahl und Fehlertexte sind verglichen; der tatsächliche Jobstart wird im Paritätslauf nicht ausgeführt (Jobmeldungen C-051). | C-051 |
| `run` | 30/30 Fälle gleich. `run job=RestoreFiles` fragt im Original nach der JobId (C-043); Onesimus geht direkt zu „OK to run?“. Der Paritätslauf startet keine echten Jobs (nur Zusammenfassung, `mod`/`no`/Abbruch). Jobablauf, Jobmeldungen (C-051) und CatReq-Updates (C-055) sind nur durch Einheitentests belegt. | C-043, C-051, C-055 |
| `status` | 51/52 Fälle gleich. Instabil (gemeinsamer echter FD bzw. nicht eindeutige Sortierung, allein grün): `c042-status-client-star`. `status storage=… slots` endet ohne Slottabelle (C-045; kein Autochanger im Fixture, der Fall `c042-status-storage-slots` deckt nur den Nicht-Autochanger ab). Warte-Zustände laufender Jobs fehlen (C-048); NDMP-Storages werden nicht gesondert behandelt (C-050). | C-045, C-048, C-050 |
| `setbandwidth` | 6/6 Fälle gleich. Verglichen sind Dialog/Abbruch und `setbandwidth limit=100 client=bareos-fd` in API 1/2 (setzt das Limit am gemeinsamen echten FD; die tatsächliche Drosselung wird nicht geprüft). SD-Limits und Jobselektoren sind nur durch Einheitentests belegt (NDMP: C-050). | C-050 |
| `setdebug` | 7/7 Fälle gleich. Verglichen sind nur Dialog/Abbruch und `setdebug level=0 dir`; Debug-Pegel an echte FD/SD sind laut Testregeln nicht live erlaubt und nur durch Einheitentests belegt (NDMP: C-050). | C-050 |
| `setdevice` | 23/23 Fälle gleich. NDMP-Storages werden nicht übersprungen (C-050). Verglichen sind ACL, Argumentprüfung und SD-Antwort auf dem Fixture-Storage. | C-050 |
| `setip` | 2/2 Fälle gleich. Verglichen ist nur die Ablehnung für Konsolen ohne gleichnamigen Client; das Setzen der Client-Adresse ist nur durch Einheitentests belegt. | — |
| `trace` | 3/3 Fälle gleich. Verglichen sind Dialog und Abbruch; das Schreiben der Trace-Datei ist nur durch Einheitentests belegt. | — |
| `truncate` | 8/8 Fälle gleich. Truncate über SD-Relabel mit Auswertung von `3000 OK label.` fehlt (C-053); verglichen sind nur Guards und Abbruchpfade. | C-053 |
| `update` | 47/47 Fälle gleich. `update slots` meldet „not supported“ (C-054). Volume-/Pool-Parameter sind verglichen, `update stats` und Autochanger-Pfade nicht vollständig. | C-054 |
| `wait` | 5/5 Fälle gleich. Verglichen sind nur `wait` ohne laufende Jobs und `wait mount timeout=`; das Warten auf echte Jobenden ist nur durch Einheitentests belegt (C-048). | C-048 |

### `registered_unverified` (4)

| Befehl | Grund |
| --- | --- |
| `.bvfs_restore` | Kein Paritätsfall: ändert den Katalog (Handle-Tabelle); nur durch Einheitentests belegt. |
| `reload` | Kein Paritätsfall (Reload ändert die laufende Konfiguration beider Directors). |
| `unmount` | Kein Paritätsfall: Unmount am echten SD ist laut Testregeln nicht live erlaubt; Verhalten nur durch Einheitentests belegt. |
| `umount` | Alias von unmount; kein Paritätsfall (siehe unmount). |

### Restore-Auswahlkontext (26, `missing_context`)

`abort`, `add`, `cd`, `count`, `delete`, `dir`, `.dir`, `done`, `estimate`, `exit`, `find`, `help`, `ls`, `.ls`, `.lsdir`, `lsmark`, `.lsmark`, `mark`, `markdir`, `pwd`, `.pwd`, `unmark`, `unmarkdir`, `quit`, `.help`, `?`.

Diese Befehle benötigen einen eigenen Restore-Dialogzustand (C-047); ihre
Registrierung als globaler Hauptbefehl wäre keine korrekte Umsetzung. Die Fälle
`c021-*` decken nur Argumentbehandlung und Abbruch vor dem Dateibaum ab.
OVFS ist der vorgesehene Onesimus-Weg für Verzeichnisbäume; BVFS bleibt
unverändert für Bareos-Clients erhalten.

### Lokale Erweiterungen (8)

Werden separat geführt (siehe [Quellreferenz](wiki/commands/bareos-reference.md));
sie haben kein Original und daher keinen Paritätsstatus.

## Roadmap

| Etappe | Ergebnis | Offene Logeinträge / Abnahme |
| --- | --- | --- |
| P0 | Erledigt: Paritätsharness (identische Konfiguration, Katalogkopie je Seite, Rohbytes, Signale, SHA-256, Baseline-Ratsche), Session-/Command-/Ressourcen-ACL | C-001 bis C-042 behoben; Fälle mit eingeschränkten Konsolen grün |
| P1.1 | Laufende Jobs: Warte-Zustände, Cancel mit AfterJob-RunScripts, Jobmeldungen im Originaltext | C-048, C-051; betrifft `cancel`, `wait`, `status`, `.status`, `run`, `rerun`, `restore` |
| P1.2 | Nicht-Backup-Jobs über `run` | C-043 (Restore-JobId-Prompt) |
| P1.3 | Restore-Menü und Dateibaum | C-047; 26 Kontextbefehle, Bootstrap, mehrere JobIds, regexwhere |
| P1.4 | Autochanger und Volumes | C-045 (`status … slots`), C-054 (`update slots`), C-053 (`truncate`, `purge action=truncate`), C-049 (ActionOnPurge); `import`/`export`/`move` brauchen ein Autochanger-Fixture |
| P1.5 | API-2-Ergebnisobjekte und Präfixvergleich | C-046 (`delete`, `list nextvolume`), C-044 (`.pools type=`) |
| P1.6 | NDMP | C-050; `setdevice`, `setdebug`, `setbandwidth`, `mount`, `unmount`, `release`, `status` |
| P1.7 | Katalog-Nebenpfade ohne Konsolenbefehl | C-052 (ORM-Schema), C-055 (CatReq UpdateMedia) |
| P2 | Seiteneffekte belegen | Folgeschritte (`list`/`llist`/`show`) in mutierende Fälle aufnehmen; für FD/SD-Wirkung eigene Wegwerf-Daemons statt der gemeinsamen echten; danach `label`, `relabel`, `unmount`, `setbandwidth`, `setdebug`, `trace`, `setip`, `add`, `create`, `prune`, `purge`, `delete` neu bewerten |
| P3 | Fehlende Fälle | `reload` (eigene Director-Instanz je Fall), `.bvfs_restore`, `.bvfs_cleanup` mit Handle-Tabelle, `.bvfs_clear_cache yes` |
| P4 | Scheduler | Fällige Jobs starten keinen Worker (`scheduler.rs`, Backlog „KRITISCH“); vom Paritätslauf nicht erfasst |

Etappen sind Abhängigkeiten, keine Freigaben. Jeder Hauptbefehl erhält einen
eigenen Commit einschließlich Debian-Changelog, Wiki-/JSON-Status und Tests;
Statuswechsel nur über neue Paritätsfälle und `audit-notes.json`, danach
`python3 scripts/generate_director_reference.py /tmp/bareos-director-reference-18be77e6`
und `--check`.

## Abnahme pro Befehl / Teilablauf

- [ ] Originalquelle und versionsgebundene Syntax, Defaultwerte, Selektoren,
  Dialoge, ACL, Nebenwirkungen und Fehlerpfade vollständig erfassen.
- [ ] Positive und negative Fälle in `tests/parity/cases/*.jsonl` anlegen;
  mutierende Fälle nur auf der Katalogkopie, keine zustandsändernden Befehle an
  die gemeinsamen echten FD/SD.
- [ ] Text, `.api 1`, `.api 2` und eingeschränkte Konsolen abdecken, soweit der
  Originalpfad davon abhängt.
- [ ] Normalisierung nur je Fall und begründet (Version, Datum, Zeiten).
- [ ] Wirkung über Folgeschritte im selben Fall sichtbar machen; wo das nicht
  geht, als „nur Einheitentests“ in `audit-notes.json` führen.
- [ ] Abweichungen als Eintrag im Kompatibilitätslog erfassen; behobene Fälle
  per `--update-baseline` in die Ratsche übernehmen.
- [ ] Erst danach `audit-notes.json` (Status, `parity_cases`,
  `open_log_entries`) anpassen und die Referenz neu erzeugen.

## Grenzen und Referenzversion

Der installierte Original-Director meldet `25.1.2~pre43.18be77e68 (09 October 2026)`;
die gepinnte Quellreferenz ist genau dieser Stand: Commit
`18be77e6832e33f5acd64094b9cba6066f2503f9` (Branch `bareos-25`, 43 Commits
nach `WIP/25.1.2-pre` = `fc2c79b6`). Lokaler, unveränderter Checkout:
`/tmp/bareos-director-reference-18be77e6` (Reproduktion in
[bareos-reference.md](wiki/commands/bareos-reference.md)). Die Live-Parität
vergleicht gegen das installierte Binary; es ist die Grundwahrheit.

Die Änderungen `fc2c79b6..18be77e6` ändern weder die Befehlstabellen
(`ua_cmds.cc`, `ua_tree.cc`, `ua_dotcmds.cc`) noch Hilfetexte oder
Ressourcendirektiven. Konsolenrelevant sind SQL-Escaping der Filter in
`list jobs/clients/log`, einfaches Escaping von `path=` in `.bvfs_versions`,
Prompt-Ersetzung in `query`, `3000 OK label. VolFiles=…` bei
`purge … action=truncate` (C-053), die Warnung in `update slots` (C-054), die
VolBlocks-Prüfung in CatReq-`UpdateMedia` (C-055) und die Nachrichtenverteilung.
