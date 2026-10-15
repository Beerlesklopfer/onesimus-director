# TOD-COMMANDS — Vollständigkeitsprüfung und Roadmap

Stand: **15.10.2026**. Diese Roadmap führt den aktuellen Befehlsabgleich zusammen
und priorisiert zunächst die ursprünglich 21 teilweise implementierten
Hauptbefehle samt notwendigen Abhängigkeiten. Maßgeblich sind die
[Quellreferenz](wiki/commands/bareos-reference.md), die
[LLM-JSON](wiki/commands/bareos-director-commands.json), die
[Einzelbefunde](wiki/commands/audit-notes.json) und der
[Testplan](TESTPLAN.md). Weitere Architekturvorhaben stehen im [Backlog](BACKLOG.md).

## Geprüfter Stand

| Kategorie | Hauptbefehle | Bedeutung |
| --- | ---: | --- |
| Originalnamen registriert | 88/88 | Aufrufname vorhanden; kein Funktionsnachweis |
| Teilweise implementiert | 23 | Belegte offene Teilabläufe oder Abnahmelücken |
| Platzhalter | 13 | Benötigte Wirkung fehlt |
| Abweichende Semantik | 2 | `move`, `query` |
| Registriert, funktional ungeprüft | 50 | Abschlussstatus nicht belegbar |
| Vollständige Funktionsparität nachgewiesen | 0/88 | Originalvergleich einschließlich SHA-256 noch offen |

**38 Hauptbefehle besitzen belegte Restlücken; alle 88 benötigen noch die
vollständige Abnahme.** Die 38 zählen betroffene Befehle, keine einzelnen Bugs.
Zusätzlich werden **26 Restore-Auswahlbefehle** und **8 lokale
Erweiterungsnamen** separat geführt.

Ausgangsstand waren 21 Teilimplementierungen und 52 ungeprüfte Befehle.
Die unabhängige Nachprüfung hat `cancel` und `.help` als zusätzliche
Teilimplementierungen belegt. Dadurch lautet die aktuelle Aufteilung
23/13/2/50. Die ursprüngliche Priorität der 21 Befehle bleibt bestehen;
`cancel` und `.help` gehören als Abhängigkeiten von `wait` und `help` dazu.

### Was die Prüfung belegt

- Registry/JSON/Audit wurden gegen die gepinnte Bareos-Tabelle abgeglichen:
  Commit `fc2c79b6c43eb5752e98ebd86116593dbbbe0c68` (`WIP/25.1.2-pre`).
- Drei unabhängige Agenten prüften Quellen, Gegenbeispiele und Änderungen in
  mehreren Runden. Gegenseitige Reviews ersetzten die jeweiligen eigenen
  positiven Tests ausdrücklich nicht.
- Bei `wait` wurden vorzeitiger Cancelabschluss, verlorene Endzähler,
  unsichtbare Katalogfehler und ein Snapshot-Race gefunden und korrigiert.
  Reale gehaltene Kindprozesse prüfen Abschlussreihenfolge und Fehlerszenarien.
- Bei `help/.help` wurden falsche Varianten, Suchregeln, Ausgabeformate und
  der inkompatible Completion-Consumer korrigiert. Dynamische `configure`-Usage
  und ACL bleiben offen.
- Bei `.defaults` deckte das Gegenreview Exponenten-/Unterlauf-, Integer- und
  Selektorfehler trotz zunächst grüner Tests auf. Diese Fälle sind korrigiert
  und durch eigene Regressionstests abgesichert.

Prüfstand dieses Arbeitspakets: **142 Director-Unit-Tests und 4
BNET-Integrationstests bestanden; 26 oconsole-Produktionstests,
27 Entwicklungstests und 4 reale stdin/PTY-Fälle bestanden.**
Das ist lokale Regressionsevidenz, noch kein bconsole/oconsole-Rohdatenabgleich.

### Grenzen und Referenzversion

Der installierte Original-Director meldet
`25.1.2~pre43.18be77e68 (09 October 2026)`. Er ist damit nicht derselbe
Quellstand wie die gepinnte Referenz. Vor einem bindenden SHA-256-Abgleich
müssen die ausführbaren Referenzversionen und ihre Quellen eindeutig
zusammengeführt werden. Für diese Roadmap wurde keine erfolgreiche
Live-Paritätsmatrix behauptet oder erzeugt.

## Reihenfolge und gemeinsame Grundlagen

| Etappe | Ergebnis | Abhängigkeiten / Abnahme |
| --- | --- | --- |
| P0.1 | Reproduzierbare Original-/Rust-Testinstanzen und Vergleichsmatrix | Versionsanker, identische Konfiguration/Katalogsnapshots, Text/API1/API2, Rohbytes, SHA-256, Seiteneffekte und negative Fälle |
| P0.2 | Echte Session-, Command- und Ressourcen-ACL | Console/Profile/Default-Console, geordnete Allow/Deny/Regex-Regeln, Alias-/Prefixauflösung; verbotener Aufruf bewirkt nichts |
| P1.1 | Jobabschluss und Statusprioritäten | `wait` samt `cancel`, Workerexit, FD-/SD-Abbruch, Katalogfehler, Retention und Statuskonflikte |
| P1.2 | Ressourcenmetadaten und Hilfe | `.defaults`, `help/.help`, dynamische configure-Syntax, ACL, oconsole-Completion |
| P1.3 | Gemeinsame Dialog- und Daemonabläufe | Auswahl, Bestätigung, Abbruch und Fehler in `trace`, `setdebug`, `setbandwidth`, `setdevice`, `resolve`, `add` |
| P1.4 | Katalog- und Changeroperationen | `list/llist`, `configure`, `import/export`; DB- und Hardwarewirkung gemeinsam prüfen |
| P1.5 | Backup-/Restore-Jobketten | `run`, dann `rerun`, dann `restore`; historische Parameter, Zeit-/ID-Auswahl und tatsächliche Datenwirkung |
| P1.6 | Status, Scheduler und Nachrichten | `status/.status`, echte geplante Jobs, beendete Jobs, `.messages` und abhängiges `messages` |
| P2 | Übrige Originalbefehle systematisch abnehmen | 13 Platzhalter, 2 Semantikabweichungen, 50 ungeprüfte Namen; gezielte Abhängigkeiten früher bearbeiten |
| P3 | Restore-Dateiauswahlkontext vervollständigen | 26 separate Kontextbefehle, Baumzustand, Markierung, Abbruch, Bootstrap und Wiederherstellung |

Etappen sind Abhängigkeiten, keine Freigaben. Jeder Hauptbefehl erhält einen
eigenen Commit einschließlich Debian-Changelog, Wiki-/JSON-Status und Tests.
Gemeinsame Infrastruktur erhält eigene Commits. Kein Statuswechsel auf
„abgeschlossen“ allein aufgrund vorhandener Registry oder lokaler Unit-Tests.

## Offene Teilabläufe der priorisierten Befehle

Alle folgenden Zeilen bleiben offen bis zur vollständigen Abnahme. Auch bei
bereits korrigierten Teilpfaden gelten ACL, Fehlerfälle und der Rohdatenabgleich.

| Befehl | Nächste konkrete Teilabläufe / Rest | Erforderlicher Abschlussnachweis |
| --- | --- | --- |
| `wait` | FD-/SD-Abbruchkoordination, Regression für State/CatReq nach Cancel, Status-/Ausfallfälle und Originaloptionen prüfen | Cancel verzögert, Workerabsturz, Pipe-/DB-Fehler, status I/E/f/A, Job-/Mount-Auswahl, Text/API/JSON und SHA |
| `.authorized` | Ressourcenexistenz durch echte ACL ersetzen; Selektorpriorität, Case und JSON portieren | Erlaubte/verbotene sowie nicht existente Namen, Profile, negative Regeln, mehrere Selektoren, tatsächliche Dispatch-Wirkung |
| `setdevice` | Storage-ACL; vollständige SD-Antworten, EOF und Fehlerfälle | Fake-SD-Befehlsfolge und realer SD-Zustand; keine Erfolgsmeldung ohne Bestätigung |
| `trace` | Originaldialog und ursprünglicher Trace-Pfad | Auswahl/Abbruch/ungültige Antworten, Dateiöffnung/-wechsel/-fehler, Bytes und Nebenwirkung |
| `setdebug` | Dialogauswahl, NDMP und ACL | Alle Zielkombinationen/Versionen/Flags, Teilfehler und tatsächliche Debugwirkung |
| `setbandwidth` | job/jobid/ujobid/state/all, Dialog und NDMP/Legacy | Auswahl darf nicht versehentlich globale Limits setzen; richtige Job-Zuordnung, Quittung, Unlimited und Fehler |
| `resolve` | ACL, Legacy-/Nicht-TLS-Verbindungen und vollständige Daemonfälle | Mehrere Hosts/FD/SD, Mehrframe-Antworten, DNS-/Verbindungsfehler, unveränderte Antwortbytes |
| `.messages` | Globale Spoolsemantik und gRPC-Sitzungsqueue | Einmalige Zustellung, Reihenfolge, parallele Sessions, erhaltene Meldungsbytes; `messages` mitprüfen |
| `.defaults` | ACL, vollständige Konfigvalidierung und Originalvergleich der portierten Felder | Alle vier Ressourcenselektoren, JobDefs/Overrides, Restore, Mengen/Zeiten, Storage-/Device-Reihenfolge und SHA |
| `help` | Dynamische configure-Usage und ACL-Filter/permission | help/.help/all/full/item, unbekannt/Case/Fehler, API1/API2, Completion gegen Original und SHA |
| `run` | Accurate/VirtualFull, FileSet-Inhaltsänderung und weitere Originaloptionen | Full-/Differential-/Incremental-Ketten, Pool/FileSet-Override, FD-/SD-/DB-Fehler, Backupinhalt und Katalogwirkung |
| `rerun` | JobID-Listen/-Bereiche, days/hours, historische Parameter und gemeinsame Startzeit; run-Abhängigkeiten when/nextpool/Dialog | Inklusive ID-Grenzen, strikte Zeitgrenze, f-Statusfilter, nextpool, SQL-Injection und Fehler mitten in der Auswahl |
| `restore` | Interaktive Dateiauswahl, storage/restoreclient/fileregex und weitere Overrides | Originalselektion bis Bootstrap und wiederhergestellten Dateien; Abbruch und Fehler vor/nach Jobanlage |
| `status` | scheduler/subscriptions/configuration, geplante und beendete Jobs | Jeder Zielpfad, tatsächlicher Schedulerstart, Originaldaten/Felder/Text/JSON und SHA |
| `.status` | Eigene Original-Unterabschnitte statt Delegation | current/last/header/scheduled/running/terminated und alle weiteren Originalzweige getrennt prüfen |
| `import` | dstslots/volume/scan, Changerliste, Slottypen/Belegung/Überlappung/Kapazität und echte Einzelbewegungen | Fake-SD-Protokollfolge, automatische Slots, Typ/Belegung/Kapazität, Teilfehler und Katalogteilupdates |
| `export` | dstslots/volume/scan und komplette Changer-Orchestrierung | InChanger vor Bewegung freigeben; Hardware-/DB-Fehler nach jeder Einzelbewegung, keine falsche Freigabe |
| `configure` | Vollständige Direktiven, Originaldialog/-Syntax, Persistenzpfad und Fehlerbehandlung | Add/Export/Validierung, Ressourcenbeziehungen, Datei-/Reloadfehler und dokumentierte lokale Erweiterungen |
| `list` | Alle Originalfilter/Ausgabefelder, Pool-Prune-Summen | Reproduzierbare Katalog-Fixtures, Zahlen/NULL/Reihenfolge, Text/API/JSON, Retentionsgrenzen und SHA |
| `llist` | Alle Originalfilter/Ausgabefelder, Pool-Prune-Summen | Dieselben Fixtures wie list, vollständige vertikale Darstellung/JSON und identische Filterwirkung |
| `add` | Originaldialog, storage/jobid, vollständige Volume-/Kataloganlage | Namen/Anzahl/Medientyp/Pool/Storage, Fehler-/Abbruchpfade und tatsächlich angelegte Datensätze |

### Neu belegte Abhängigkeiten

- `cancel`: erster Lifecyclefix ist vorhanden; Originalselektoren, Dialog,
  tatsächlicher FD-/SD-Abbruch und Ausgabeparität sind weiterhin offen.
- `.help`: Ausgabevarianten sind korrigiert; dynamische configure-Syntax und
  ACL verhindern auch hier einen Abschluss. Eigenständiger JSON-Status ist
  deshalb jetzt `partial` statt `registered_unverified`.

## Weitere Inventarlücken

### Platzhalter (13)

`.bvfs_lsdirs`, `.bvfs_lsfiles`, `.bvfs_update`, `.bvfs_get_jobids`, `.bvfs_versions`, `.bvfs_restore`, `.bvfs_cleanup`, `.bvfs_clear_cache`, `automount`, `estimate`, `setip`, `use`, `var`.

### Semantikabweichungen (2)

`move`, `query`.

### Funktional ungeprüfte Hauptbefehle (50)

`.`, `.actiononpurge`, `.api`, `.catalogs`, `.clients`, `.consoles`, `.users`, `.filesets`, `.jobdefs`, `.jobs`, `.jobstatus`, `.levels`, `.locations`, `.media`, `.mediatypes`, `.msgs`, `.pools`, `.profiles`, `.quit`, `.sql`, `.schedule`, `.storages`, `.types`, `.volstatus`, `autodisplay`, `create`, `delete`, `disable`, `enable`, `exit`, `gui`, `label`, `messages`, `memory`, `mount`, `prune`, `purge`, `quit`, `relabel`, `release`, `reload`, `show`, `sqlquery`, `time`, `truncate`, `unmount`, `umount`, `update`, `version`, `whoami`.

Bei jedem ungeprüften Hauptbefehl zuerst alle Originalzweige und Seiteneffekte
gegen den lokalen Handler erfassen. Eine fehlende Auditnotiz ist keine Freigabe.
Befunde, die als Abhängigkeit eines priorisierten Befehls auftreten, sofort
nachtragen und bearbeiten.

### Restore-Auswahlkontext (26)

`abort`, `add`, `cd`, `count`, `delete`, `dir`, `.dir`, `done`, `estimate`, `exit`, `find`, `help`, `ls`, `.ls`, `.lsdir`, `lsmark`, `.lsmark`, `mark`, `markdir`, `pwd`, `.pwd`, `unmark`, `unmarkdir`, `quit`, `.help`, `?`.

Diese Befehle benötigen einen eigenen Restore-Dialogzustand; ihre Registrierung
als globaler Hauptbefehl wäre keine korrekte Umsetzung.

## Abnahme pro Befehl / Teilablauf

- [ ] Originalquelle und versionsgebundene Syntax, Defaultwerte, Selektoren,
  Dialoge, ACL, Nebenwirkungen und Fehlerpfade vollständig erfassen.
- [ ] Reproduzierbaren Ausgangszustand und positive/negative Fälle definieren;
  mutierende Fälle ausschließlich in wegwerfbaren Instanzen ausführen.
- [ ] bconsole und oconsole zunächst gegen denselben Director vergleichen;
  anschließend Original- und Rust-Director gegen identische Fixtures.
- [ ] `NO_COLOR=1`, Locale, Zeitzone, API-Modus und Terminalparameter angleichen.
  Der Drei-Sekunden-Guard bleibt aktiv; Dialogtests passend takten.
- [ ] Rohantworten ohne Trim, Sortierung oder JSON-Neuserialisierung speichern,
  SHA-256/Bytezahl beider Seiten, Diff, Versionen und Status dokumentieren.
  Banner/Prompt/Echo/stderr ausschließlich explizit und nachvollziehbar abtrennen.
- [ ] Gleiche tatsächliche Wirkung nachweisen: Katalog, Dateien, Jobzustände,
  Ressourcen, Daemonlimits oder Changerzustand. Hashgleichheit allein genügt nicht.
- [ ] Fehler, Timeout und ungetestete/nicht deterministische Fälle offen führen.
- [ ] Unabhängigen Gegenreview nach Implementierung und erneut nach Korrekturen
  durchführen; konkrete Gegenbeispiele als Regressionstests bewahren.
- [ ] Erst danach Wiki-/JSON-/Roadmapstatus mit den gespeicherten Nachweisen
  auf abgeschlossen setzen. Alle verbleibenden Unterabläufe bleiben benannt.

## Nachweise dieser Prüfrunden

- `338e86f`: wait-Lifecycle, tatsächlicher Workerexit, Katalogfehler und Statusprioritäten.
- `1041688`: Original-Hilfevarianten und angepasster oconsole-Consumer.
- `36020af`: Ressourcen-Defaults, Vererbung und korrigierte Review-Gegenbeispiele.
- Gegengeprüfte Quellen: `ua_cmds.cc`, `ua_dotcmds.cc`, `ua_acl.cc`,
  `ua_select.cc`, `ua_run.cc`, `ua_impexp.cc`, `job.cc`, `dird_conf.cc`,
  `lib/jcr.cc`, `lib/output_formatter.cc`, `lib/edit.cc` im gepinnten Original.

Der nächste sinnvolle Schwerpunkt ist **P0.1/P0.2**: verlässliche
Originalvergleichs-Fixtures und das gemeinsame ACL-/Sitzungsmodell. Ohne diese
Grundlagen lassen sich mehrere bereits weit portierte Befehle nicht abschließen.
