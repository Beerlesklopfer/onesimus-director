# TODO-COMMANDS — Vollständigkeitsprüfung und Roadmap

Stand: **17.10.2026**, abgeglichen auf `development` = `7d4fad2`. Diese Roadmap
führt den Befehlsabgleich mit den Belegen des Paritätsvergleichs, dem
[Stubs-Backlog](STUBS.md), der [typisierten gRPC-API](wiki/grpc-api.md) und dem
[MCP-Server](wiki/mcp.md) zusammen. Quellen: [Quellreferenz](wiki/commands/bareos-reference.md),
[LLM-JSON](wiki/commands/bareos-director-commands.json),
[Einzelbefunde](wiki/commands/audit-notes.json),
[Kompatibilitätslog](COMPATIBILITY-LOG.md), [Testplan](TESTPLAN.md),
Paritätsfälle `tests/parity/cases/*.jsonl` mit der Ratsche `tests/parity/baseline.json`.
Weitere Architekturvorhaben stehen im [Backlog](BACKLOG.md).

## Zusammenfassung (17.10.2026)

| Kennzahl | Wert | Quelle |
| --- | ---: | --- |
| Hauptbefehle registriert und echt implementiert | 88/88 | JSON, Registry |
| Befehle mit mindestens einem Paritätsfall | 86/88 (ohne `.bvfs_restore`, `umount`) | `tests/parity/cases` |
| Paritätsfälle in 64 Dateien | 2178 | `tests/parity/cases` |
| davon in der Baseline-Ratsche, alle `match` | 2163 (+ Harnessfall `audit-log` = 2164 Einträge) | `baseline.json` |
| Fälle außerhalb der Baseline | 15 (`c074-*`: Onesimus-Erweiterungen `.acl`, `.grpc`, `.sql*`, bewusst ungleich, Entscheidung C-074) | `c074.jsonl` |
| Letzter dokumentierter Volllauf | 16.10.2026 auf `7993ce6`: 1479/1481 gleich; beide Unterschiede stehen inzwischen als `match` in der Baseline | JSON `parity_evidence` |
| Wiki-Status (Stand 16.10., **nicht aufgefrischt**) | 53 `parity_verified`, 31 `partial`, 4 `registered_unverified` | JSON `summary` |
| Kompatibilitätslog C-001 … C-087 | 81 behoben, 3 Entscheidung (C-074, C-085, C-087), 1 geprüft (C-086), 1 Vorfall/Ursache (C-039), 1 „teilweise behoben“ (C-058) | COMPATIBILITY-LOG |
| Stubs S-01 … S-36 | 35 erledigt/behoben, 1 offen (S-36) | STUBS |
| Typisierte gRPC-RPCs (`DirectorApi`) | 181 definiert: 94 lesend im Produktions-Build, 87 schreibend/Sitzung/Restore-Baum nur mit `grpc-development` (davon 4 nicht anwendbar: Gui, `.api`, `.grpc`, SetIp) | grpc-api.md |
| MCP-Tools Produktions-Build | 35 typisierte Lese-Tools + `query` + 5 `ovfs_*` | mcp.md |
| MCP-Tools Entwicklungs-Build (`--mcp-allow-write`) | zusätzlich 23 Schreib-/Restore-Tools + `execute` | mcp.md |
| MCP-Vollausbau nach C-087 (alle 88 Befehle lokal, HTTP lesend) | offen (S-36) | STUBS, C-087 |

Seit dem 16.10.2026 umgesetzt (Commits auf `development`): alle Jobtypen über
`run` und Scheduler — Verify (S-04), Copy/Migrate einschließlich
SD-zu-SD-Replikation (S-03, S-05, S-20), Consolidate mit Always-Incremental (S-06),
RunScripts (S-07, S-24); nativer Backup-Ablauf mit Accurate, Restore-Objekten,
Quotas, Write Bootstrap, Volume-Pruning und Purge/Recycle Oldest Volume
(S-19, S-23, S-27, S-28); Scheduler startet fällige Jobs (S-15, S-16, C-081);
Joblimits, Duplikatprüfung, Watchdog, Reschedule (S-22, S-26, C-083);
Standard-Wartezeit 10 min auf ein Volume (S-33, C-085); label/relabel-Protokoll
zum SD (S-32, C-084); eigener FD `bareos-filed` mit Sicherung, Restore, Estimate,
Verify (S-12, S-13); `-t` wie `bareos-dir -t` (S-29); typisierte gRPC-API (S-34,
S-35) und typisierte MCP-Tools (S-30, S-31).

**Wiki-Status veraltet.** `bareos-director-commands.json` und `audit-notes.json`
stehen noch auf dem 16.10.2026 (`7993ce6`). Viele dort als Grund für `partial`
genannte Logeinträge (C-043 … C-055) sind seither behoben, `reload` hat
inzwischen Paritätsfälle, und die 26 Befehle der Restore-Dateiauswahl sind
umgesetzt (C-047, Fälle `c047-*`). Ein Statuswechsel braucht nach den Regeln
unten einen Paritätslauf und die Auffrischung von `audit-notes.json`; bis dahin
nennt die Tabelle unten den alten Wiki-Status und daneben den aktuellen Stand
laut Log und Stubs.

### Noch offen

Nur Punkte, die in den Quellen ausdrücklich als offen stehen:

- **S-36 / C-087:** MCP-Vollausbau — alle 88 Befehle, Unterbefehle und der
  Restore-Baum als typisierte Tools, Schreiben lokal über stdio auch im
  Produktions-Build, HTTP lesend, Aufrufe über die typisierte `DirectorApi`
  mit Rückfall `ExecuteCommand`/BNET. Laut [STUBS.md](STUBS.md) „offen“
  (Umsetzung in Arbeit). Bis dahin beschreibt [mcp.md](wiki/mcp.md) den alten
  Stand (Schreib-Tools nur im Entwicklungs-Build).
- **`.profile create/set/delete/assign/unassign`** (Onesimus-Erweiterung): die
  RPCs laufen durch, der Director kennt die Unterbefehle nicht (`.profile` ist
  Alias von `.acl`) — [S-35](STUBS.md). [grpc-api.md](wiki/grpc-api.md) führt die
  RPCs nur als „Entwicklungs-Build“ ohne diesen Hinweis.
- **`label`/`relabel … encrypt`:** erzeugt noch keinen Volume-Schlüssel
  ([C-084](COMPATIBILITY-LOG.md), [S-08](STUBS.md); `update volume encrypt=` ist erledigt).
- **C-058:** Tabellenstatus im Log „teilweise behoben“, die Teilpunkte (Verify,
  Copy/Migrate, PoolOccupancy, Consolidate, RunScripts) sind über S-03 … S-07,
  S-19 … S-28 erledigt. Ausdrücklich als Rest genannt bleiben nur: Berichtszeilen
  „Director's comm line to SD dropped“/„Network error with FD …“ und SD
  termination status beim Cancel sowie `*JobCancel*` des Job-Threads ohne
  Jobnamen-Nummer ([S-26](STUBS.md)); passive Clients beim Backup „ungeprüft“ ([S-23](STUBS.md)).
- **Eigener FD `bareos-filed`:** PKI-Verschlüsselung/Signaturen, Plugins,
  Heartbeat, client-initiierte Verbindungen, LMDB für Accurate, VSS/Windows/macOS;
  `breg.rs` noch als Kopie ([S-13](STUBS.md)).
- **`configure update/delete`** (Erweiterungen) prüfen die Ressourcen-ACL nicht
  (Hinweis [C-062](COMPATIBILITY-LOG.md)).
- **Konfigurationsstart:** der normale Start prüft die Konfiguration nicht wie
  `-t`; Lexer-Diagnosen (Zeile/Spalte) gehen nicht ins Log ([C-057](COMPATIBILITY-LOG.md)).
- **`setbandwidth`:** `max_bandwidth` laufender Jobs, Jmsg bei Connect-/Antwortfehlern
  ([C-056](COMPATIBILITY-LOG.md)).
- **`status`:** SD-Teil von `status all`/`status storage` ([C-014](COMPATIBILITY-LOG.md));
  typisierte RPCs für `status client/storage` liefern nur Text, `Estimate` fehlt im
  Integrationstest der gRPC-API ([S-34](STUBS.md)).
- **`restore`:** Nicht-UTF-8-Dateinamen auf SQL_ASCII-Katalogen ([C-047](COMPATIBILITY-LOG.md)).
- **`update jobid=… starttime=`** und Run On Incoming Connect Interval sind nicht
  implementiert (Hinweis [C-059](COMPATIBILITY-LOG.md)).
- **`cancel`-Dialog** ohne eigenen Paritätsfall ([C-011](COMPATIBILITY-LOG.md)).
- **C-086:** auf PDC per Katalogabfrage klären, warum `list volumes pool=BGLAGE`
  nur BGLAGE-0015 zeigte ([C-086](COMPATIBILITY-LOG.md)).
- **Dokumentation/Status:** `bareos-director-commands.json`/`audit-notes.json`
  nach einem Paritätslauf auffrischen (P6 unten).

## Geprüfter Stand

Die Tabelle unten nennt je Befehl: Implementierung, alten Wiki-Status (JSON,
16.10.), Paritätsfälle (Anzahl in `parity_cases` der JSON / aktuelle Fälle in der
Baseline, deren Schritte den Befehl aufrufen), Stand und offene Punkte mit
Verweisen, die typisierte gRPC-RPC und die MCP-Tools.

Lesehilfe:

- **Fälle „a / b“:** a = in der JSON genannte `parity_cases` (16.10.), b = aktuelle
  Baseline-Fälle mit einem Schritt, dessen erstes Wort den Befehl nennt (erster
  Schritt mit Präfixauflösung wie im Original, spätere Schritte nur mit vollem
  Namen, ohne Zahlen und `yes/no/mod`; `.` nur als erster Schritt). b ist eine
  Näherung, keine Statusaussage.
- **gRPC:** „Prod“ = lesende RPC im Produktions-Build; „Entw.“ = nur mit Cargo-Feature
  `grpc-development` (Produktion: `UNIMPLEMENTED`); „nicht anwendbar“ =
  `FAILED_PRECONDITION`. Zuordnung aus der generierten Abdeckungstabelle in
  [grpc-api.md](wiki/grpc-api.md#abdeckung-befehl--rpc).
- **MCP:** typisierte Lese-Tools gibt es in jedem Build; `query` = generisches
  Lese-Tool mit Allowlist (`status`, `list`, `llist`, `show`, `version`, `help`,
  `time`, `messages`, `estimate`, lesende Punktbefehle); „Entw.“ = nur Build
  `mcp-development` mit `--mcp-allow-write` (`execute` deckt dort jeden Befehl
  außer `quit`/`exit`/`.quit`/`.api`/`@…` ab). Nach C-087 soll sich das mit S-36 ändern.

**`parity_verified` heißt nicht „vollständig kompatibel“.** Der Status gilt nur
für die in `audit-notes.json` je Befehl unter `parity_cases` genannten Varianten.
Der Vergleich prüft Antwortbytes, Signale und Endgrund je Schritt sowie das
Audit-Log, nicht den Katalogzustand danach. Seiteneffekte auf echte FD/SD
werden im Paritätslauf nicht ausgeführt; seit dem 17.10.2026 sind viele davon
mit **privaten** Test-Daemons (eigene `bareos-fd`/`bareos-sd`, eigene Ports und
Wegwerfkatalog) gegen das Original geprüft (siehe S-Einträge), was den
Wiki-Status aber nicht ändert.

### Statusregeln

- Zuordnung Fall → Befehl: erstes Wort jedes Schritts, aufgelöst wie im Original
  (erster Tabelleneintrag mit diesem Präfix, ohne Groß-/Kleinschreibung);
  Antworten auf Dialoge (nach `BNET_SUB_PROMPT` bzw. `.`, Zahlen, `yes/no/mod`)
  zählen nicht als Befehl. `setup`-Schritte werden nicht verglichen.
- Instabile Fälle laut Kompatibilitätslog (`status-client`, `list-jobs`,
  `c015-list-jobs-enabled-count`) sowie `c042-status-client-star` (im vollen Lauf
  eine zusätzliche Director-Verbindung am gemeinsamen echten FD, allein grün)
  werden als instabil genannt, stufen aber nicht herab.
- Die Fälle `c035-setbandwidth-api1/-api2`, die `bwlimit=100kB/s` am gemeinsamen
  echten FD setzten, sind entfernt; der Warnkanal ist per Einheitentest belegt
  (Kompatibilitätslog, „Testumgebung und Regeln“).
- Ein offener Eintrag im Kompatibilitätslog stuft den betroffenen Befehl auf
  `partial`, auch wenn alle vorhandenen Fälle grün sind.
- Kein Statuswechsel auf `parity_verified` allein aufgrund lokaler Unit-Tests
  oder Tests mit privaten Daemons.
- Pfade mit FD/SD-Kontakt nie gegen die laufenden Daemons testen;
  `tests/parity/parity.py` weist Fälle mit möglichem Jobstart ab (`unsafe_job_start`).

## Befehle

| Befehl | Implementiert | Wiki-Status (16.10.) | Fälle | Stand / offen | gRPC (`DirectorApi`) | MCP |
| --- | --- | --- | ---: | --- | --- | --- |
| `.` | ja | parity_verified | 3 / 14 | — | Prod: `Noop` | `query` |
| `.actiononpurge` | ja | parity_verified | 2 / 2 | — | Prod: `DotActiononpurge` | `query` |
| `.api` | ja | parity_verified | 28 / 33 | — | nicht anwendbar | – |
| `.authorized` | ja | parity_verified | 3 / 4 | — | Prod: `DotAuthorized` | `query` |
| `.catalogs` | ja | parity_verified | 9 / 10 | — | Prod: `DotCatalogs` | `query` |
| `.clients` | ja | parity_verified | 20 / 24 | — | Prod: `DotClients` | `dot_clients`, `query` |
| `.consoles` | ja | parity_verified | 10 / 10 | — | Prod: `DotConsoles` | `query` |
| `.users` | ja | parity_verified | 9 / 9 | — | Prod: `DotUsers` | `query` |
| `.defaults` | ja | parity_verified | 9 / 18 | — | Prod: `DotDefaults` | `dot_defaults`, `query` |
| `.filesets` | ja | parity_verified | 10 / 10 | — | Prod: `DotFilesets` | `dot_filesets`, `query` |
| `.help` | ja | parity_verified | 8 / 13 | — | Prod: `DotHelp` | `query` |
| `.jobdefs` | ja | parity_verified | 9 / 9 | — | Prod: `DotJobdefs` | `query` |
| `.jobs` | ja | parity_verified | 31 / 49 | — | Prod: `DotJobs` | `dot_jobs`, `query` |
| `.jobstatus` | ja | parity_verified | 21 / 8 | — | Prod: `DotJobstatus` | `dot_jobstatus`, `query` |
| `.levels` | ja | parity_verified | 6 / 10 | — | Prod: `DotLevels` | `dot_levels`, `query` |
| `.locations` | ja | parity_verified | 4 / 4 | — | Prod: `DotLocations` | `query` |
| `.messages` | ja | parity_verified | 8 / 10 | — | Prod: `DotMessages` | `query` |
| `.media` | ja | parity_verified | 5 / 5 | — | Prod: `DotMedia` | `query` |
| `.mediatypes` | ja | parity_verified | 9 / 9 | — | Prod: `DotMediatypes` | `query` |
| `.msgs` | ja | parity_verified | 10 / 10 | — | Prod: `DotMsgs` | `dot_msgs`, `query` |
| `.pools` | ja | partial | 21 / 34 | [C-044](COMPATIBILITY-LOG.md) behoben, Wiki-Status veraltet | Prod: `DotPools` | `dot_pools`, `query` |
| `.profiles` | ja | parity_verified | 10 / 10 | Erweiterung `.profile create/set/…` wirkt nicht ([S-35](STUBS.md)) | Prod: `DotProfiles` | `query` |
| `.quit` | ja | parity_verified | 3 / 3 | — | Entw.: `DotQuit` | – |
| `.sql` | ja | parity_verified | 30 / 32 | — | Entw.: `DotSql` | Entw.: nur `execute` |
| `.schedule` | ja | parity_verified | 18 / 24 | — | Prod: `DotSchedule` | `dot_schedule`, `query` |
| `.status` | ja | partial | 22 / 28 | [C-048](COMPATIBILITY-LOG.md) behoben, Wiki-Status veraltet | Prod: `DotStatusClient`, `DotStatusDirector`, `DotStatusStorage` | `query` |
| `.storages` | ja | parity_verified | 18 / 22 | — | Prod: `DotStorages` | `dot_storages`, `query` |
| `.types` | ja | parity_verified | 2 / 2 | — | Prod: `DotTypes` | `dot_types`, `query` |
| `.volstatus` | ja | parity_verified | 2 / 3 | — | Prod: `DotVolstatus` | `query` |
| `.bvfs_lsdirs` | ja | parity_verified | 11 / 11 | — | Prod: `DotBvfsLsdirs` | `query` |
| `.bvfs_lsfiles` | ja | parity_verified | 9 / 9 | — | Prod: `DotBvfsLsfiles` | `query` |
| `.bvfs_update` | ja | parity_verified | 3 / 3 | — | Entw.: `DotBvfsUpdate` | Entw.: nur `execute` |
| `.bvfs_get_jobids` | ja | parity_verified | 7 / 8 | — | Prod: `DotBvfsGetJobids` | `query` |
| `.bvfs_versions` | ja | parity_verified | 5 / 5 | — | Prod: `DotBvfsVersions` | `query` |
| `.bvfs_restore` | ja | registered_unverified | 0 / 0 | kein Paritätsfall (P3) | Entw.: `DotBvfsRestore` | Entw.: nur `execute` |
| `.bvfs_cleanup` | ja | partial | 1 / 1 | Löschen der Handle-Tabelle nur Einheitentests (P3) | Entw.: `DotBvfsCleanup` | Entw.: nur `execute` |
| `.bvfs_clear_cache` | ja | partial | 3 / 3 | `yes`-Variante nur Einheitentests (P3) | Entw.: `DotBvfsClearCache` | Entw.: nur `execute` |
| `add` | ja | partial | 9 / 12 | Katalogzustand danach nicht verglichen (P2) | Entw.: `AddVolumes` | Entw.: nur `execute` |
| `autodisplay` | ja | parity_verified | 16 / 17 | — | Entw.: `Autodisplay` | Entw.: nur `execute` |
| `automount` | ja | parity_verified | 14 / 14 | — | Entw.: `Automount` | Entw.: nur `execute` |
| `cancel` | ja | partial | 24 / 27 | [C-048](COMPATIBILITY-LOG.md) behoben; Cancel laufender Jobs nur mit privaten FD/SD ([S-22](STUBS.md), [S-26](STUBS.md)) belegt; [C-011](COMPATIBILITY-LOG.md): Cancel-Dialog ohne Test; `*JobCancel*` des Job-Threads verbraucht keine Jobnamen-Nummer ([S-26](STUBS.md)) | Entw.: `Cancel` | Entw.: `cancel_job` |
| `configure` | ja | parity_verified | 31 / 43 | offen: `configure update/delete` (Erweiterungen) ohne Ressourcen-ACL (Hinweis [C-062](COMPATIBILITY-LOG.md)) | Entw.: 4 RPCs | Entw.: `configure_add`, `configure_export` |
| `create` | ja | partial | 4 / 7 | [C-066](COMPATIBILITY-LOG.md) behoben; Katalogzustand per Folgeschritt nur in `c066` | Entw.: `CreatePool` | Entw.: nur `execute` |
| `delete` | ja | partial | 16 / 36 | [C-046](COMPATIBILITY-LOG.md), [C-077](COMPATIBILITY-LOG.md) behoben; `c035-delete-noarg-api2` in der Baseline `match` | Entw.: 4 RPCs | Entw.: `delete` |
| `disable` | ja | parity_verified | 30 / 35 | — | Entw.: `Disable` | Entw.: `disable` |
| `enable` | ja | parity_verified | 29 / 35 | — | Entw.: `Enable` | Entw.: `enable` |
| `estimate` | ja | parity_verified | 38 / 44 | typisierte RPC nicht im Integrationstest ([S-34](STUBS.md)) | Prod: `Estimate` | `query` |
| `exit` | ja | parity_verified | 2 / 2 | — | Entw.: `Exit` | – |
| `export` | ja | partial | 2 / 4 | [C-071](COMPATIBILITY-LOG.md) behoben, nur simulierter Autochanger | Entw.: `ExportVolumes` | Entw.: nur `execute` |
| `gui` | ja | parity_verified | 30 / 30 | — | nicht anwendbar | Entw.: nur `execute` |
| `help` | ja | parity_verified | 7 / 11 | — | Prod: `Help` | `query` |
| `import` | ja | partial | 2 / 4 | [C-071](COMPATIBILITY-LOG.md) behoben, nur simulierter Autochanger | Entw.: `ImportVolumes` | Entw.: nur `execute` |
| `label` | ja | partial | 13 / 16 | [C-084](COMPATIBILITY-LOG.md)/[S-32](STUBS.md) behoben (privater SD); offen: `encrypt` ([C-084](COMPATIBILITY-LOG.md), [S-08](STUBS.md)) | Entw.: `Label` | Entw.: `label_volume` |
| `list` | ja | partial | 299 / 333 | [C-046](COMPATIBILITY-LOG.md) behoben, Wiki-Status veraltet | Prod: 19 RPCs | `list_*` (14 Tools), `query` |
| `llist` | ja | partial | 110 / 117 | [C-046](COMPATIBILITY-LOG.md) behoben, Wiki-Status veraltet | Prod: 19 RPCs | `list_*` (Parameter `long`), `query` |
| `messages` | ja | parity_verified | 13 / 34 | — | Prod: `GetMessages` | `messages`, `query` |
| `memory` | ja | parity_verified | 3 / 3 | — | Prod: `GetMemory` | Entw.: nur `execute` |
| `mount` | ja | partial | 16 / 19 | [C-050](COMPATIBILITY-LOG.md) behoben (NDMP-Sitzungen: „NDMP protocol not supported“) | Entw.: `Mount` | Entw.: `mount_storage` |
| `move` | ja | partial | 2 / 5 | [C-069](COMPATIBILITY-LOG.md) behoben, Transfer/Release nur Schein-SD | Entw.: `MoveVolumes` | Entw.: nur `execute` |
| `prune` | ja | partial | 35 / 36 | Katalogzustand danach nicht verglichen (P2); [C-030](COMPATIBILITY-LOG.md) behoben | Entw.: 5 RPCs | Entw.: `prune` |
| `purge` | ja | partial | 34 / 50 | [C-049](COMPATIBILITY-LOG.md), [C-053](COMPATIBILITY-LOG.md) behoben (nur simulierter SD) | Entw.: 4 RPCs | Entw.: `purge` |
| `quit` | ja | parity_verified | 5 / 23 | — | Entw.: `Quit` | – |
| `query` | ja | parity_verified | 8 / 9 | — | Entw.: `Query` | Entw.: nur `execute` |
| `restore` | ja | partial | 28 / 99 | [C-047](COMPATIBILITY-LOG.md) behoben (Baum, Menü, Bootstrap); offen: Nicht-UTF-8-Namen auf SQL_ASCII ([C-047](COMPATIBILITY-LOG.md)); echte Restores mit privaten Daemons belegt ([S-01](STUBS.md), [S-12](STUBS.md), [C-078](COMPATIBILITY-LOG.md)–[C-080](COMPATIBILITY-LOG.md)) | Entw.: `Restore`, `StartRestoreSession` | Entw.: `restore` |
| `relabel` | ja | partial | 8 / 8 | [C-084](COMPATIBILITY-LOG.md)/[S-32](STUBS.md) behoben; offen: `encrypt` ([C-084](COMPATIBILITY-LOG.md), [S-08](STUBS.md)) | Entw.: `Relabel` | Entw.: `relabel_volume` |
| `release` | ja | partial | 7 / 9 | [C-050](COMPATIBILITY-LOG.md) behoben (NDMP-Sitzungen nicht unterstützt) | Entw.: `Release` | Entw.: `release_storage` |
| `reload` | ja | registered_unverified | 0 / 23 | [C-057](COMPATIBILITY-LOG.md), [C-075](COMPATIBILITY-LOG.md) behoben, jetzt mit Fällen `c057`/`c075`; offen: Start prüft die Konfiguration nicht, Lexer-Diagnosen nicht im Log ([C-057](COMPATIBILITY-LOG.md)) | Entw.: `Reload` | Entw.: `reload` |
| `rerun` | ja | partial | 32 / 33 | [C-033](COMPATIBILITY-LOG.md) behoben; Jobstart im Paritätslauf nicht ausgeführt | Entw.: `Rerun` | Entw.: `rerun_job` |
| `resolve` | ja | parity_verified | 19 / 19 | — | Prod: `Resolve` | Entw.: nur `execute` |
| `run` | ja | partial | 30 / 201 | [C-043](COMPATIBILITY-LOG.md), [C-051](COMPATIBILITY-LOG.md), [C-055](COMPATIBILITY-LOG.md) behoben; alle Jobtypen ([S-04](STUBS.md)–[S-07](STUBS.md), [S-15](STUBS.md), [S-19](STUBS.md)–[S-28](STUBS.md)); [C-058](COMPATIBILITY-LOG.md) steht im Log noch auf „teilweise“ (siehe Widersprüche) | Entw.: `Run` | Entw.: `run_job` |
| `status` | ja | partial | 52 / 68 | [C-045](COMPATIBILITY-LOG.md), [C-048](COMPATIBILITY-LOG.md), [C-050](COMPATIBILITY-LOG.md) behoben; offen laut [C-014](COMPATIBILITY-LOG.md): SD-Teil von `status all`/`status storage`; typisiert nur Text für Client/Storage ([S-34](STUBS.md)) | Prod: 8 RPCs | `status_director/client/storage/scheduler/subscriptions/configuration`, `query` |
| `setbandwidth` | ja | partial | 6 / 54 | [C-056](COMPATIBILITY-LOG.md) offen-Rest: `max_bandwidth` laufender Jobs, Jmsg bei Connect-/Antwortfehlern | Entw.: `SetBandwidth` | Entw.: `setbandwidth` |
| `setdebug` | ja | partial | 7 / 11 | [C-050](COMPATIBILITY-LOG.md) behoben; FD/SD-Wirkung nur Einheitentests | Entw.: `SetDebug` | Entw.: `setdebug` |
| `setdevice` | ja | partial | 23 / 25 | [C-050](COMPATIBILITY-LOG.md) behoben | Entw.: `SetDevice` | Entw.: nur `execute` |
| `setip` | ja | partial | 2 / 3 | Setzen der Adresse nur Einheitentests; gRPC nicht anwendbar | nicht anwendbar | Entw.: nur `execute` |
| `show` | ja | parity_verified | 53 / 118 | — | Prod: 16 RPCs | `show`, `query` |
| `sqlquery` | ja | parity_verified | 13 / 35 | — | Entw.: `SqlQuery` | Entw.: nur `execute` |
| `time` | ja | parity_verified | 1 / 2 | — | Prod: `GetTime` | `time`, `query` |
| `trace` | ja | partial | 3 / 3 | Trace-Datei nur Einheitentests | Entw.: `Trace` | Entw.: nur `execute` |
| `truncate` | ja | partial | 8 / 18 | [C-053](COMPATIBILITY-LOG.md) behoben (nur simulierter SD) | Entw.: `Truncate` | Entw.: `truncate` |
| `unmount` | ja | registered_unverified | 0 / 1 | 1 Fall (`c050-unmount`); Wirkung am SD nur privat/Einheitentests | Entw.: `Unmount` | Entw.: `unmount_storage` |
| `umount` | ja | registered_unverified | 0 / 0 | Alias von `unmount`, kein eigener Fall | Entw.: `Unmount` | Entw.: `unmount_storage` |
| `update` | ja | partial | 47 / 108 | [C-054](COMPATIBILITY-LOG.md) (nur simulierter Autochanger), [C-067](COMPATIBILITY-LOG.md), [C-068](COMPATIBILITY-LOG.md) behoben; `update jobid=… starttime=` fehlt (Hinweis [C-059](COMPATIBILITY-LOG.md)) | Entw.: 5 RPCs | Entw.: `update_volume`, `update_pool` |
| `use` | ja | parity_verified | 7 / 9 | — | Entw.: `UseCatalog` | Entw.: nur `execute` |
| `var` | ja | parity_verified | 16 / 16 | — | Prod: `ExpandVariables` | Entw.: nur `execute` |
| `version` | ja | parity_verified | 9 / 10 | — | Prod: `GetVersion` | `version`, `query` |
| `wait` | ja | partial | 5 / 40 | [C-048](COMPATIBILITY-LOG.md) behoben | Entw.: `Wait` | Entw.: `wait_job` |
| `whoami` | ja | parity_verified | 9 / 9 | — | Prod: `Whoami` | Entw.: nur `execute` |

### Restore-Dateiauswahl (26)

`abort`, `add`, `cd`, `count`, `delete`, `dir`, `.dir`, `done`, `estimate`, `exit`, `find`, `help`, `ls`, `.ls`, `.lsdir`, `lsmark`, `.lsmark`, `mark`, `markdir`, `pwd`, `.pwd`, `unmark`, `unmarkdir`, `quit`, `.help`, `?`.

Die JSON führt sie noch als `missing_context` (16.10.). Laut Kompatibilitätslog
ist der Restore-Baum mit eigenem Dialogkontext umgesetzt
([C-047](COMPATIBILITY-LOG.md) behoben, `commands/restore_tree.rs`); 46 Fälle
`c047-*` in der Baseline rufen alle Baumbefehle außer `exit` auf (Ende immer mit
`abort`/`quit` oder `done` + `no`, kein echter Restore). Offen nur Nicht-UTF-8-Namen
auf SQL_ASCII. gRPC: `StartRestoreSession` und 16 `RestoreTree*`-RPCs
(Entwicklungs-Build). MCP: nur das Einzeilen-Tool `restore` (Entwicklungs-Build,
`files`/`directories`/`fileregex`/`all`); interaktive Baum-Tools kommen mit S-36.
OVFS ist der vorgesehene Onesimus-Weg für Verzeichnisbäume; BVFS bleibt
unverändert für Bareos-Clients erhalten.

### Lokale Erweiterungen (12)

`.acl`, `.profile`, `.grpc`, `.sqltables`, `.sqlschema`, `.sqlcount`, `.storage`,
`.ovfs_status`, `.ovfs_latest`, `.ovfs_tree`, `.ovfs_ls`, `.ovfs_versions`
(JSON `onesimus_extensions`; `.catalog` ist seit S-17 wie im Original `.catalogs`).
Sie haben kein Original und daher keinen Paritätsstatus; die Fälle `c074-*`
stehen bewusst außerhalb der Baseline (Entscheidung [C-074](COMPATIBILITY-LOG.md),
Nacharbeit S-10 erledigt). gRPC: Lese-RPCs im Produktions-Build (`.acl list`,
`.profile list`, `.sql*`, `.storage`, OVFS), Änderungen an `.acl`/`.profile` nur im
Entwicklungs-Build; `.grpc` nicht anwendbar. MCP: `ovfs_*` in jedem Build,
`.sql*`/`.storage` über `query`. Offen: `.profile create/set/…` wirkt nicht (S-35).

## Roadmap

| Etappe | Ergebnis | Stand 17.10.2026 |
| --- | --- | --- |
| P0 | Paritätsharness, Session-/Command-/Ressourcen-ACL | erledigt (C-001 … C-042) |
| P1.1 | Laufende Jobs: Warte-Zustände, Cancel mit AfterJob-RunScripts, Jobmeldungen | erledigt (C-048, C-051, S-07, S-22, S-26); Rest S-26 siehe „Noch offen“ |
| P1.2 | Nicht-Backup-Jobs über `run` | erledigt (C-043, C-058 Teilpunkte über S-03 … S-06, S-20) |
| P1.3 | Restore-Menü und Dateibaum | erledigt (C-047, S-01, S-14, C-078 … C-080); Rest Nicht-UTF-8 |
| P1.4 | Autochanger und Volumes | erledigt gegen simulierten Autochanger/SD (C-045, C-049, C-053, C-054, C-069, C-071); label/relabel gegen privaten SD (C-084); offen `encrypt` |
| P1.5 | API-2-Ergebnisobjekte und Präfixvergleich | erledigt (C-044, C-046) |
| P1.6 | NDMP | Konsolenzweige erledigt (C-050); NDMP-Sitzungen selbst nicht unterstützt |
| P1.7 | Katalog-Nebenpfade ohne Konsolenbefehl | erledigt (C-052, C-055, S-27, S-28) |
| P2 | Seiteneffekte belegen | teilweise: private FD/SD-Prüfstände für Backup, Restore, Verify, Copy/Migrate, Consolidate, Scheduler, Joblimits, label/relabel, gRPC- und MCP-Schreibpfade; Paritätsfälle mit Folgeschritten und Neubewertung im Wiki offen |
| P3 | Fehlende Fälle | `reload` erledigt (`c057`, `c075`); offen: `.bvfs_restore`, `.bvfs_cleanup` mit Handle-Tabelle, `.bvfs_clear_cache yes`, `umount` |
| P4 | Scheduler | erledigt (S-02, S-15, S-16, C-081); [BACKLOG.md](BACKLOG.md) führt den Punkt noch als „KRITISCH“ |
| P5 | Typisierte Schnittstellen | gRPC erledigt (S-34, S-35); MCP-Lese-/Schreib-Tools erledigt (S-30, S-31); Vollausbau nach C-087 offen (S-36) |
| P6 | Statusauffrischung | Paritätsvolllauf, `audit-notes.json` und Referenz neu erzeugen; Wiki-Status von 16.10. auf aktuellen Stand bringen |

Etappen sind Abhängigkeiten, keine Freigaben. Jeder Hauptbefehl erhält einen
eigenen Commit einschließlich Debian-Changelog, Wiki-/JSON-Status und Tests;
Statuswechsel nur über neue Paritätsfälle und `audit-notes.json`, danach
`python3 scripts/generate_director_reference.py /tmp/bareos-director-reference-18be77e6`
und `--check`.

## Widersprüche zwischen den Quellen (17.10.2026)

- `bareos-director-commands.json`/`audit-notes.json` (16.10., `7993ce6`) nennen für
  25 Befehle offene Logeinträge (C-043 … C-055), die im Kompatibilitätslog
  behoben sind; `reload` gilt dort als „kein Paritätsfall“, obwohl `c057`/`c075`
  in der Baseline stehen; `unmount` ebenso (Fall `c050-unmount`); der
  Restore-Baum steht auf `missing_context`, obwohl C-047 behoben ist;
  `cross_cutting_findings` nennt noch „Scheduler startet keine Worker“;
  `mcp_build_policies` kennt die typisierten MCP-Tools nicht.
- Kompatibilitätslog: Tabellenzeile C-058 „teilweise behoben“ und der Abschnitt
  „Offen“ (C-058-RunScripts offen, C-080 offen) widersprechen den späteren
  Einträgen (S-07 erledigt, C-080 behoben). Der Abschnitt „Stand 16.10.2026“
  ist nicht fortgeschrieben.
- STUBS: S-02 „Unverändert offen: der Scheduler startet fällige Jobs nicht“ ist
  durch S-15 überholt; S-14 „Befund C-080 offen“ durch S-01/C-080 behoben.
- [mcp.md](wiki/mcp.md) (Abschnitt Tests) nennt den label/relabel-Befund als
  aktuell; er ist mit C-084/S-32 behoben.
- [BACKLOG.md](BACKLOG.md) „KRITISCH: Scheduler startet keine Jobs“ ist durch
  S-15 überholt.
- `.profile`-Änderungen: S-35 sagt „laufen durch, der Director kennt sie nicht“;
  [grpc-api.md](wiki/grpc-api.md) erwähnt das nicht.

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
