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

Statt eines fremden Crates entsteht eine eigene PostgreSQL-Extension `ovfs`
(Rust/pgrx, Repo `oenesimus-postgres-plugin`, Verzeichnis `ovfs/`). Sie liest
den Bareos-Katalog unverändert und liefert für den Restore den Verzeichnisbaum
als JSON. Getestet wird gegen die neuesten Versionen: PostgreSQL 19 und
Bareos 25.1.2-pre (Katalogschema 2250).

Stand 16.10.2026: committet auf `development` (`4041335`) in
https://github.com/Beerlesklopfer/oenesimus-postgres-plugin (öffentlich; `development` ist
Default, `master`/`main` erst zur Produktion). Doku im GitHub-Wiki:
https://github.com/Beerlesklopfer/oenesimus-postgres-plugin/wiki (User-Installation,
User-Restore-Modes, User-Functions, User-Tree-Model, Developer-Architecture,
Developer-Bareos-Semantics, Developer-Development). Zielumgebung:
Katalog-DB auf einem SBC mit 1–2 GB RAM, Katalog in PC-Backup-Größe (~1 Mio. Dateien).
Rückgabe als `json` (nicht `jsonb`: jsonb-Arrays sind auf 256 MB begrenzt).

- `ovfs.tree(jobid | jobids int[], root text, depth int, max_entries bigint DEFAULT 1000000)`:
  Baum ab `root`, `depth` Pflicht (1 = root plus direkte Kinder). Verzeichnisse an der
  Tiefengrenze haben keine `children`, tragen aber `entries` (lebende Einträge darunter) und
  werden per `ls` nachgeladen. `max_entries` zählt Knoten im Ergebnis. Bei einer Jobkette
  gewinnt je (Pfad, Name) die neueste JobTDate, `FileIndex = 0` löscht; Hardlink-Regel aus
  `ua_tree.cc`.
- `ovfs.tree_latest(client, root, depth, fileset DEFAULT NULL, max_entries)` mit
  `ovfs.jobids_latest(client, fileset)`: neuester Stand wie Restore-Option 5
  (`uar_last_full_no_pool`, `uar_dif`, `uar_inc`); WARNING, wenn der Full auf keinem
  aktivierten Volume liegt (die Kette enthält ihn dann wie bei Bareos nicht).
- `ovfs.ls(jobids int[], dir) -> rows` (name, name_b64, path, path_b64, kind, jobid, fileid,
  fileindex, delta_seq, delta_parts json, mode, uid, gid, size, mtime, link_fileindex,
  entries): direkte Kinder eines Verzeichnisses, zum Nachladen in ein MVC-Baummodell
  (`canFetchMore` = `entries > 0` und noch keine Kinder).
- `ovfs.versions(client, path, name, copies DEFAULT false) -> rows`: alle Versionen einer
  Datei, neueste zuerst, mit Volumes und InChanger (wie `.bvfs_versions`).
- BaseFiles und DeltaSeq werden wie bei Bareos aufgelöst
  (`select_recent_version_with_basejob_and_delta` plus Delta-Kette aus `ua_tree`);
  Delta-Dateien tragen `delta_seq` und `delta_parts [{jobid, fileindex}, …]`.
- Reihenfolge der Kinder: erst Dateien (nach Namensbytes), dann Verzeichnisse (Name + `/`),
  also Indexreihenfolge; andere Sortierung in der Ansicht.
- Speicher: Zeilen werden sortiert über einen Cursor gestreamt; `ls` auf 1,25 Mio. Zeilen
  braucht im Spitzenwert 160 MB Backend-RSS (davon ~133 MB Shared Buffers), ein voller Baum
  etwa das Doppelte seiner Textgröße.
- Jeder Knoten trägt `jobid` und `fileindex` (Restore-Kennung); `link_fileindex` verweist auf
  denselben Job. Parität gegen die Bareos-Restore-Abfrage auf 1.230.502 Einträgen: identisch.
  Getestet mit PostgreSQL 19rc1 und dem DDL von 25.1.2-pre.

Rolle im Director: BVFS (`.bvfs_*`, `crates/onesimus-dird/src/commands/bvfs.rs`) ist
byte-gleich portiert und bleibt nur zur Abwärtskompatibilität für Bareos-Clients erhalten
(Paritätsfälle `c022`); OVFS ist der Onesimus-Weg.

Director-Integration (16.10.2026, Branch der OVFS-Integration): Der Director prüft bei jeder
neuen Katalogverbindung, nach `reload` und mit `.ovfs_status refresh`, ob `ovfs` in Version
0.1.x mit allen genutzten Signaturen, Ergebnistypen (`pg_get_function_result`) und
EXECUTE-Recht vorhanden ist, hält das Ergebnis im Katalog-Handle und protokolliert es.
Befehle (Onesimus-Erweiterungen, nur exakte Namen, Wiki `docs/wiki/commands/dot-ovfs-*.md`):
`.ovfs_status`, `.ovfs_latest`, `.ovfs_tree`, `.ovfs_ls`, `.ovfs_versions`; JSON in `.api 2`
und mit `json`, sonst lesbarer Text; Command-ACL plus Job/Client/FileSet-ACL wie BVFS
(auditiert). Ohne Extension rechnet der Director dasselbe mit reinem SQL (`src/ovfs/`:
Kern aus `ovfs-core` portiert, Zeilen per Server-Cursor in 10.000er-Batches);
Äquivalenztest gegen eine Instanz mit `ovfs` byte-gleich. In oconsole als lesende MCP-Tools
`ovfs_*` (auch im Produktions-Build). BVFS unverändert.

Offen: Restore-Auswahl über die Restore-Kennungen (Bootstrap über `JobMedia`), Restore-Baum
im interaktiven `oconsole`; typisierte gRPC-Aufrufe (derzeit über `ExecuteCommand`). Befund
für das Plugin: In einer `SQL_ASCII`-Datenbank scheitern `ovfs.tree`/`ovfs.ls` mit
`client_encoding = UTF8`, sobald ein Pfad nicht UTF-8 ist („invalid byte sequence“); der
Director setzt dafür je Transaktion `SET LOCAL client_encoding TO 'SQL_ASCII'`.

Integration:

- Eigene Director-Befehle für OVFS (BVFS bleibt unverändert als Kompatibilitätsschicht,
  `commands/bvfs.rs`, Paritätsfälle `c022`)
- Danach als Tools im MCP-Modus von `oconsole` anbieten (Backup-Baum durchsuchen)
- Restore-Baum im interaktiven `oconsole` wie bei bconsole (`cd`, `ls`, `mark`)
- Befund am Rande: `crates/bareos-orm/schema/v2250_bareos_catalog.json` setzt
  den Primärschlüssel von `pathvisibility` auf `jobid`; im Bareos-DDL ist er
  `(JobId, PathId)` (Kompatibilitätslog C-052)

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



###########----------###########----------###########

## Addendum: Plattformübergreifender File Daemon und ereignisgesteuerte Backups

**Stand:** 15.10.2026  
**Status:** Geplant / Implementierung offen  
**Geltungsbereich:** `crates/bareos-filed`, `crates/onesimus-dird`, `proto` (Submodule `onesimus-proto`)

### Zielsetzung

Onesimus soll als Bareos-kompatibler Director beginnen und schrittweise eine eigenständige, gRPC-basierte Backup-Architektur erhalten. Für onesimus Home wird ein nativer Rust File Daemon benötigt, der auf Windows, macOS und Linux als Hintergrunddienst betrieben werden kann.

Die neue Architektur soll neben zeitgesteuerten Backups auch ereignisgesteuerte Ausführung unterstützen: Ist ein Client zum geplanten Zeitpunkt nicht erreichbar, bleibt der Backup-Bedarf erhalten und wird bei erneuter Verfügbarkeit unter Berücksichtigung der konfigurierten Richtlinien ausgeführt.

Die vorhandene BNET-Kompatibilität bleibt während der Migration erhalten. Bestehende C++ File Daemons und Storage Daemons dienen weiterhin als Referenz für Ende-zu-Ende-Tests.

### 1. Ist-Zustand des Rust File Daemon

Das Crate `crates/bareos-filed` enthält bereits:

- TCP-Accept-Loop und Session-Threads (`accept.rs`)
- BNET-Kommunikation mit TLS-PSK- und CRAM-MD5-Authentifizierungspfaden
- Job-Kommandobehandlung (`backup.rs`)
- Konfigurationsverarbeitung (`config.rs`)
- Statusausgabe (`status.rs`)
- Heartbeat-Thread (`heartbeat.rs`)
- Daemon-Einstiegspunkt (`main.rs`)

**Offene Funktionslücken:**

- `backup.rs`: Dateiscan und Übertragung zum Storage Daemon sind noch nicht implementiert. Der aktuelle Backup-Pfad quittiert den Auftrag und meldet null gesicherte Dateien und Bytes.
- `restore.rs`: Enthält lediglich einen Platzhalter.
- `heartbeat.rs`: Wartet periodisch, sendet aber noch keine Heartbeats an den Director.
- `status.rs`: Laufende und abgeschlossene Jobs werden noch nicht tatsächlich erfasst.
- `accept.rs`: Implementiert eingehende BNET-Verbindungen, keine vom FD initiierte gRPC-Control-Session.
- `main.rs`: Enthält Unix-spezifische Prozess- und Daemonlogik. Ein plattformübergreifender Service-Lifecycle ist noch nicht implementiert.

Der Rust-FD darf deshalb noch nicht als funktionaler Ersatz für den C++-FD betrachtet werden.

### 2. Rust-FD: Funktionsparität und Backup-Engine

**Priorität: Hoch, nach beziehungsweise abhängig von den aktuellen Director-Paritätsarbeiten.**

- [ ] Tatsächlichen Dateiscan anhand der FileSet-Include-/Exclude-Regeln implementieren.
- [ ] Dateiinhalte und erforderliche Metadaten an den Storage Daemon übertragen.
- [ ] Full-, Incremental- und Differential-Backups implementieren und testen.
- [ ] Dateiänderungen, Löschungen und Umbenennungen korrekt behandeln.
- [ ] Restore

###########----------###########----------###########

## Addendum: Rust-only Plugin-System mit verpflichtender Zertifizierung

**Status:** Architekturentscheidung / Umsetzung offen  
**Geltungsbereich:** onesimus Director, File Daemon, Storage Daemon und zukünftige Erweiterungsschnittstellen  
**Sicherheitsanforderung:** Fail-closed – ohne gültige Zertifizierung keine Plugin-Ausführung

### 1. Architekturentscheidung

Onesimus verwendet künftig ausschließlich Plugins, die in Rust implementiert und durch eine von onesimus anerkannte Zertifizierungsstelle freigegeben wurden.

Das bisherige Python-Plugin-System auf Basis von PyO3 wird nicht weitergeführt. Python-, Shell- und andere externe Plugin-Runtimes werden nicht als alternative Ausführungswege unterstützt.

Ein Plugin darf nur geladen werden, wenn seine Identität, Integrität, Zertifizierung und Berechtigung erfolgreich überprüft wurden.

**Verbindliche Regel: Kein gültiges Zertifikat – kein Plugin-Load.**

### 2. Plugin-Zertifizierung

Jedes Plugin benötigt ein signiertes Zertifizierungsmanifest.

Das Manifest enthält mindestens:

- [ ] Eindeutige Plugin-ID und Herstelleridentität
- [ ] Plugin-Version und unterstützte onesimus-API-Versionen
- [ ] Kryptografischen Hash des freigegebenen Plugin-Artefakts
- [ ] Zertifizierungsstelle und Signaturinformationen
- [ ] Gültigkeitszeitraum
- [ ] Freigegebene Fähigkeiten und Ressourcenberechtigungen
- [ ] Unterstützte Betriebssysteme und Architekturen
- [ ] Zertifizierungsrichtlinie beziehungsweise Sicherheitsprofil

Die Zertifizierung muss kryptografisch an das konkrete Plugin-Artefakt gebunden sein. Eine Zertifizierung für Version A darf nicht automatisch Version B autorisieren.

### 3. Verbindliche Prüfung beim Laden

Der Plugin-Loader muss vor jeder Ausführung:

1. Das Plugin-Manifest lesen und validieren.
2. Den Hash des Plugin-Artefakts überprüfen.
3. Die digitale Signatur und Zertifikatskette validieren.
4. Die Zertifizierungsstelle gegen den konfigurierten Trust Store prüfen.
5. Gültigkeit und gegebenenfalls Widerrufsstatus prüfen.
6. API-Kompatibilität und Plattformfreigabe prüfen.
7. Angeforderte Berechtigungen gegen die lokale Sicherheitsrichtlinie prüfen.
8. Erst danach die Plugin-Initialisierung erlauben.

Bei jeder fehlgeschlagenen Prüfung wird das Plugin abgewiesen. Ein automatischer Fallback auf unsignierte Plugins ist ausgeschlossen.

Auch Entwicklungs- und Debug-Builds dürfen die Zertifizierungsprüfung nicht stillschweigend umgehen. Für die Entwicklung können ausdrücklich konfigurierte Test-Zertifizierungsstellen verwendet werden.

### 4. Plugin-Ausführung und Isolation

Die Implementierungssprache Rust allein stellt keine ausreichende Sicherheitsgrenze dar.

Deshalb muss zwischen der Zertifizierung eines Plugins und seinen tatsächlichen Laufzeitberechtigungen unterschieden werden.

- [ ] Eine versionierte Plugin-API definieren.
- [ ] Berechtigungen nach dem Least-Privilege-Prinzip vergeben.
- [ ] Direkten Zugriff auf Director-Interna, Katalog und Schlüsselmaterial standardmäßig verweigern.
- [ ] Ressourcenlimits und Ausführungszeitgrenzen festlegen.
- [ ] Plugin-Fehler vom Hauptdaemon isolieren.
- [ ] Plugin-Ladevorgänge und sicherheitsrelevante Aktionen protokollieren.
- [ ] Sichere Aktualisierung und Deaktivierung von Plugins implementieren.

**Bevorzugte Architektur:** Rust-Plugins als WebAssembly-Komponenten mit einem kontrollierten Host-Interface und Capability-basierter Freigabe.

Native Rust-Plugins über dynamische Bibliotheken sind nicht automatisch speichersicher gegenüber dem Hostprozess. Eine Zertifizierung ersetzt keine Prozess- oder Sandbox-Isolation.

Die endgültige Laufzeitform ist vor der Implementierung verbindlich festzulegen.

### 5. Zertifizierungs- und Vertrauensmodell

- [ ] onesimus-Trust-Store definieren.
- [ ] Signaturalgorithmus und Manifestformat festlegen.
- [ ] Verfahren zur Ausstellung und Erneuerung von Zertifizierungen definieren.
- [ ] Zertifikatswiderruf und Sperrlisten unterstützen.
- [ ] Offline-Verifikation für NAS-Appliances ermöglichen.
- [ ] Vertrauenswürdige Updates des Trust Stores absichern.
- [ ] Zertifizierungsstatus in der Administrationsoberfläche anzeigen.
- [ ] Sicherheitsereignisse bei abgewiesenen Plugins protokollieren.

Die Zertifizierung soll nicht nur die Herkunft eines Plugins bestätigen, sondern eine nachvollziehbare Freigabe für definierte Funktionen und Berechtigungen darstellen.

### 6. Migration des bestehenden Plugin-Systems

Das bisherige Crate `crates/bareos-plugins` verwendet PyO3 für Python-Plugins.

- [ ] Bestehende Plugin-Schnittstellen und Anwendungsfälle inventarisieren.
- [ ] Python-/PyO3-Abhängigkeiten aus der zukünftigen Plugin-Architektur entfernen.
- [ ] Neue Rust-Plugin-API entwickeln.
- [ ] Zertifizierungsmanifest und Signaturprüfung implementieren.
- [ ] Plugin-Loader mit Fail-closed-Verhalten entwickeln.
- [ ] Benötigte bestehende Plugin-Funktionen nach Rust portieren.
- [ ] Nicht migrierte Python-Plugins ausdrücklich als nicht unterstützt kennzeichnen.
- [ ] Dokumentation und Paketierung entsprechend aktualisieren.

Die Migration darf keine stillschweigende Aktivierung nicht zertifizierter Plugins ermöglichen.

### 7. Sicherheits- und Regressionstests

- [ ] Gültig zertifiziertes Plugin wird geladen.
- [ ] Unsigniertes Plugin wird abgewiesen.
- [ ] Manipuliertes Plugin wird abgewiesen.
- [ ] Plugin mit unbekannter Zertifizierungsstelle wird abgewiesen.
- [ ] Abgelaufenes oder widerrufenes Zertifikat führt zur Ablehnung.
- [ ] Plugin mit nicht unterstützter API-Version wird abgewiesen.
- [ ] Plugin mit nicht genehmigten Berechtigungen wird abgewiesen.
- [ ] Plugin-Absturz beeinträchtigt den Hostdaemon nicht.
- [ ] Fehlgeschlagene Verifikation wird nachvollziehbar protokolliert.
- [ ] Offline-Betrieb mit gültiger lokaler Vertrauensbasis funktioniert gemäß definierter Widerrufsrichtlinie.

### 8. Abnahmekriterien

Das neue Plugin-System gilt erst dann als abgeschlossen, wenn:

1. Ausschließlich Rust-basierte Plugins unterstützt werden.
2. Jedes Plugin vor seiner Initialisierung kryptografisch verifiziert wird.
3. Nicht zertifizierte Plugins technisch nicht ausführbar sind.
4. Plugin-Berechtigungen vom Host erzwungen werden.
5. Ein fehlerhaftes Plugin nicht den Director oder File Daemon kompromittieren kann.
6. Der Betrieb ohne Internetverbindung gemäß dokumentierter Vertrauens- und Widerrufsrichtlinie möglich ist.
7. Alle negativen Sicherheitsprüfungen automatisiert getestet sind.

**Verbindlicher Architekturgrundsatz:** Plugin-Ausführung ist eine explizit gewährte Berechtigung, keine Eigenschaft einer installierten Datei. Die Zertifizierung ist notwendig, aber nicht allein hinreichend; zusätzlich gelten Laufzeitisolation und lokale Berechtigungsrichtlinien.