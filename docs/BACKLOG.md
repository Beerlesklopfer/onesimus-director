# Backlog

## Aktueller Fokus: die 21 teilweise implementierten Hauptbefehle

Priorität vom 15.10.2026: Zuerst alle offenen Unterbefehle, Optionen und
Ablaufketten dieser 21 Hauptbefehle vervollständigen. Andere Platzhalter und
die 26 Befehle der Restore-Dateiauswahl folgen danach; Abhängigkeiten, die
für einen dieser 21 Befehle erforderlich sind, gehören zu dessen Umsetzung.

Die verbindlichen Einzelbefunde stehen in
[audit-notes.json](wiki/commands/audit-notes.json). Der Status bleibt offen,
bis auch Fehlerpfade, Seiteneffekte und SHA-256-Ausgabeparität geprüft sind.

| Hauptbefehl | Offene Teilabläufe / Abnahme |
| --- | --- |
| `wait` | Job-/Namensauswahl, Mount-/Medienzustände, Timeout, tatsächlicher Katalogabschluss |
| `.authorized` | Command- und Ressourcen-ACL statt bloßer Existenzprüfung |
| `setdevice` | Storage-ACL, vollständige Daemon-Antwort und Fehlerpfade |
| `trace` | Dialogauswahl und ursprünglicher Trace-Pfad |
| `setdebug` | Dialogauswahl, NDMP und ACL |
| `setbandwidth` | Jobselektoren, Zustandsfilter, all, Dialog, NDMP/Legacy und ACL |
| `resolve` | ACL sowie Legacy-/Nicht-TLS-Verbindungen |
| `.messages` | Globale Spool-Semantik und gRPC-Sitzungsqueue |
| `.defaults` | Vollständige Ressourcen- und Restore-Defaults |
| `help` | Originalsyntax, Textlayout und ACL-gefilterte Hilfe |
| `run` | Accurate/VirtualFull, FileSet-Änderungen und weitere Originaloptionen |
| `rerun` | Jobbereiche, Zeitfilter und Mehrfachauswahl |
| `restore` | Dateiauswahl, Storage-/Client-Overrides und fileregex |
| `status` | Scheduler/subscriptions/configuration, geplante und beendete Jobs |
| `.status` | Original-Unterabschnitte und deren eigenes Ausgabeformat |
| `import` | dstslots, volume und scan |
| `export` | dstslots, volume und scan |
| `configure` | Vollständige Direktiven und konfigurierbarer Persistenzpfad |
| `list` | Originalfilter, Ausgabefelder und Pool-Prune-Summen |
| `llist` | Originalfilter, Ausgabefelder und Pool-Prune-Summen |
| `add` | Originaldialog, storage/jobid und vollständige Kataloganlage |

Für jede Tabellenzeile gilt zusätzlich der unten definierte SHA-256-Abgleich.
Je Hauptbefehl bleibt ein eigener Commit vorgeschrieben.

## KRITISCH: Scheduler startet keine Jobs

`scheduler.rs`: `dispatch_ready_jobs()` erhöht nur einen Zähler
(`TODO(R6-full): Fork worker process`). Zusätzlich vergleicht
`fill_queue_from_config()` Stunde/Minute in UTC statt Ortszeit, ignoriert
Tages-/Wochen-/Monatsangaben der Run-Direktive ("1st Sat", "mon-fri") und
dedupliziert nicht. Geplante Backups laufen mit onesimus-dird derzeit nicht.
Folge: `status director` liefert `scheduled: []`.


## Postgres-Plugin für Verzeichnisbäume (OVFS)

Ein fertiges Postgres-Plugin-Crate (auf crates.io, Name noch eintragen) löst die
Verzeichnisbäume im Catalog direkt in PostgreSQL auf. Damit lässt sich im
Restore-Baum navigieren, wie es bconsole tut (`cd`, `ls`, `mark`).

- Crate-Name offen. Globale crates.io-Suche (13.10.2026, Begriffe: bareos, bacula,
  bvfs, path hierarchy, ltree, pg tree, restore tree …) ohne eindeutigen Treffer;
  entfernt ähnlich: `closure-tree`, `diesel_ltree`, `ltree2viz`.
- Einbinden statt der heutigen BVFS-Stubs in `crates/onesimus-dird/src/commands/dot.rs`
  (`.bvfs_update`, `.bvfs_lsdirs`, `.bvfs_lsfiles`, `.bvfs_versions` …)
- Danach als Tools im MCP-Modus von `oconsole` anbieten (Backup-Baum durchsuchen)
- Restore-Baum im interaktiven `oconsole` wie bei bconsole

## JSON-Ausgabe: Rest

Erledigt: Umschlag/Rahmung, Punkt-Befehle, list/llist, .defaults job=,
.help, time, version, reload, status director, .messages/messages.
Offen: `.help` Usage von configure (aus Direktiventabellen), `.defaults` für
client/storage/pool und Restore-Jobs (where/replace) — brauchen die rohen
Direktiven aus dem show-Port; Reihenfolge der Ressourcen wie C++ (Ladereihenfolge).
`status director`: terminated-Liste (Jobs seit Start) wird nicht geführt.

## `llist pools`: prunablevolumes/-jobs/-bytes

Bareos 25.1 gibt bei `llist pools` zusätzlich eine Prune-Zusammenfassung pro
Pool aus (ua_output.cc `PoolPruneSummary`, Auswertung der Aufbewahrungsfristen).
onesimus-dird liefert die Felder noch nicht.

## Quellbasierter Director-Vollständigkeitsabgleich

[Originalbefehle und Status](wiki/commands/bareos-reference.md) · [LLM-JSON](wiki/commands/bareos-director-commands.json).

88/88 Originalnamen sind registriert; dies ist keine Funktionsparität. Belegt sind unter anderem BVFS- und Steuerungs-Stubs, fehlende interaktive Restore-Dateiauswahl, `query`/`move`-Semantikabweichungen, unvollständige ACL-Prüfung und zu großzügige MCP-Punktbefehlsfreigaben. Die Referenz enthält die Quellenbelege und unterscheidet diese Befunde von noch ungeprüften Handlern.

## SHA-256-Ausgabeparität je Befehl (Abnahmekriterium)

- Für jeden der 88 Original-Konsolenbefehle und jeden der 26 Restore-Shell-Befehle konkrete Testfälle, Parameter, Vorbedingungen und erwartete Seiteneffekte festlegen. Lokale Erweiterungen separat erfassen.
- Zuerst `bconsole` und `oconsole` gegen denselben Director und denselben reproduzierbaren Zustand vergleichen; anschließend Bareos- und Onesimus-Director mit identischen Konfigurationen/Katalog-Snapshots vergleichen. Beide Ebenen separat ausweisen.
- Farbgebung bei `oconsole` mit `NO_COLOR=1` deaktivieren (implementiert). Der Director darf selbst keine ANSI-Farben in Befehlsantworten erzeugen. Für beide Prozesse identische Locale, Zeitzone, API-Modus und Terminalparameter setzen.
- Die eigentlichen Befehlsantworten als unveränderte Bytes speichern und SHA-256 berechnen. Für jeden Fall müssen `bconsole` und `oconsole` denselben SHA-256 liefern. CLI-Banner, Prompt/Echo und Diagnose-stderr separat erfassen; eine etwaige Extraktion des Antwortbereichs muss explizit und für beide Clients nachvollziehbar sein.
- Nicht trimmen, Zeilen sortieren, JSON neu serialisieren oder Fehlermeldungen entfernen, um Parität vorzutäuschen. Zeilenenden, Leerzeichen, Feldreihenfolge und Encoding sind Teil der Prüfung.
- Uhrzeit, Laufzeit, IDs, Hostnamen, Speicherwerte und asynchrone Meldungen durch feste Testumgebung/Fixtures kontrollieren. Wo identische Rohwerte nicht reproduzierbar sind, den Fall als offen/nicht deterministisch ausweisen; ein normalisierter Zusatzvergleich ersetzt keinen erfolgreichen Rohdaten-Hashabgleich.
- Zustandsändernde Befehle nur in wegwerfbaren Testinstanzen ausführen; vor jedem Clientlauf denselben Ausgangszustand wiederherstellen. Restore-Shell-Dialoge einschließlich Abbruch und Fehlerfällen testen.
- Ergebnis-JSON pro Testfall: Befehl, Parameter, Kontext, Versionen, Fixtures, Rückgabecodes, SHA-256 beider Antworten, Byteanzahl, Diff und Status. Fehler, Zeitüberschreitungen und nicht ausgeführte Fälle dürfen nicht als bestanden zählen. Rohdateien und Diffs für die Analyse behalten.
- Abschluss erst nach vollständiger Matrix ohne ungeprüfte Originalbefehle. Der bisherige statische Registry-Abgleich ist kein Ersatz für diese Tests.

## Arbeitsfolge und Git-Anker

Anker: `director-parity-anchor-20261015` (`fb5242f`), vor den funktionalen Befehlsänderungen.
Je Hauptbefehl ein eigener Commit einschließlich Tests, Wiki-/JSON-Status und Eintrag in `CHANGELOG.md` (Debian-Format). Gemeinsame Infrastruktur erhält eigene Commits. Offene Teilfunktionen bleiben ausdrücklich im Audit, bis Quellabgleich und SHA-256-Matrix die Parität nachweisen.

## MCP: Entwicklungs- und Produktionsbuild trennen

Implementiert: Der Standardbuild (`mcp-production`) kompiliert ausschließlich MCP-Lesezugriffe. `execute` und der CLI-Schalter `--mcp-allow-write` sind dort nicht aktivierbar. Schreibzugriffe werden nur mit `--no-default-features --features mcp-development` einkompiliert und benötigen zusätzlich `--mcp-allow-write`. Beide Features zusammen oder keines sind Compile-Fehler. Debian baut ausdrücklich die Produktionsvariante. Gilt für stdio und HTTP; die Director-Verbindung bleibt unverändert.

Abnahme: Tests für beide Buildvarianten, Produktions-CLI ohne Schreibschalter, Ablehnung direkt konstruierter execute-Anfragen und Buildfehler bei widersprüchlichen Features. Entwicklungsartefakte getrennt von Produktionsartefakten kennzeichnen/ablegen.
