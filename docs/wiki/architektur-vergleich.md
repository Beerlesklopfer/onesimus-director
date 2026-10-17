# Architekturvergleich: Bareos Director (C++) und Onesimus Director (Rust)

Stand: 17.10.2026. Für Entwickler und Architekten des Onesimus-Projekts.

Verglichen werden:

- **Original:** Bareos Director `bareos-dir 25.1.2~pre43.18be77e68`, Quellstand `18be77e6`
  (Checkout `/tmp/bareos-director-reference-18be77e6`, Verzeichnisse `core/src/dird`, `lib`,
  `cats`, `console`, `stored`, `filed`, `include`, `findlib`). Pfade unten relativ zu `core/src/`.
- **Onesimus:** dieses Repository, vor allem
  [`crates/onesimus-dird`](../../crates/onesimus-dird/src/main.rs) und die Crates
  [`bareos-bnet`](../../crates/bareos-bnet/src/lib.rs),
  [`bareos-config`](../../crates/bareos-config/src/lib.rs),
  [`bareos-orm`](../../crates/bareos-orm/src/lib.rs),
  [`bareos-catalog`](../../crates/bareos-catalog/src/lib.rs),
  [`onesimus-proto-rs`](../../crates/onesimus-proto-rs/src/lib.rs),
  [`oconsole`](../../crates/oconsole/src/main.rs),
  [`bareos-filed`](../../crates/bareos-filed/src/main.rs).

Jede Aussage nennt Datei und Funktion. Wo eine Aussage auf einer Lesart des Codes
beruht und nicht durch einen Test oder Paritätsfall belegt ist, steht das dabei
(„nicht geprüft“). Die Entscheidungen und Abweichungen tragen die Nummern aus dem
[Kompatibilitätslog](../COMPATIBILITY-LOG.md) (C-…) und dem [Stubs-Backlog](../STUBS.md) (S-…).

## Inhalt

1. [Überblick: Prozesse, Threads, Laufzeitmodell](#1-überblick-prozesse-threads-laufzeitmodell)
2. [Konfiguration](#2-konfiguration)
3. [Konsole und Netzwerk](#3-konsole-und-netzwerk)
4. [Job-Lebenszyklus](#4-job-lebenszyklus)
5. [Katalog](#5-katalog)
6. [Meldungen und Logging](#6-meldungen-und-logging)
7. [Scheduler, Watchdog, Limits](#7-scheduler-watchdog-limits)
8. [Sicherheitsmodell](#8-sicherheitsmodell)
9. [Erweiterbarkeit](#9-erweiterbarkeit)
10. [Unterschiede, Entscheidungen, Lücken, Stärken und Risiken](#10-unterschiede-entscheidungen-lücken-stärken-und-risiken)

---

## 1. Überblick: Prozesse, Threads, Laufzeitmodell

### Kurzfassung

| | Bareos (C++) | Onesimus (Rust) |
|---|---|---|
| Prozesse | ein Prozess `bareos-dir` | ein Director-Prozess `onesimus-dird` **plus ein geforkter Prozess je laufendem Job** mit Daemon-Kontakt (`job:<id>`) |
| Jobs | ein POSIX-Thread je laufendem Job aus `jobq.cc` (`jobq_server`, höchstens `MaxConcurrentJobs` Threads) | Warteschlange im `JobManager` (`job_manager.rs`), Ausführung im geforkten Worker (`job_worker.rs::spawn_job_worker`); Copy/Migrate-Steuerjobs und Consolidate laufen im Director selbst (`internal_jobs.rs`) |
| Jobzustand | `JobControlRecord` (JCR, `lib/jcr.h`, `dird/director_jcr_impl.h`), globale JCR-Kette | `DirJob` + `JcrState` im `JobManager` (Director), `JobParams` (serialisiert in den Worker) |
| Konsolen | ein Thread je Verbindung aus `lib/thread_list.cc` (`BnetThreadServerTcp`, `lib/bnet_server_tcp.cc`) | ein benannter Thread je Verbindung `dir:session:<n>` (`accept.rs::accept_loop`) |
| Katalog | eine DB-Verbindung je JCR/UaContext (`dird/get_database_connection.cc`, Pooling `cats/sql_pooling.cc`) | ein Katalog-Thread `dir:catalog` mit tokio-Runtime und sqlx-Pool (max. 8), Anfragen über einen `flume`-Kanal (`catalog_worker.rs`) |
| Scheduler | läuft im **Hauptthread** (`dird.cc:main` → `Scheduler::GetMainScheduler().Run()`) | eigener Thread `dir:scheduler` (`scheduler.rs::scheduler_loop`); Hauptthread wartet nur auf Signale |
| Watchdog | `lib/watchdog.cc` (Timer-Thread mit Callbacks, u. a. `JobMonitorWatchdog`, `job.cc:717`) | `dir:watchdog` (`watchdog.rs`: Zombies, Scheduler-Herzschlag); Joblimits in `job_manager/limits.rs`, aufgerufen von `dir:job-monitor` |
| Async | keine; blockierende Sockets, `pthread_cond_timedwait` | tokio nur für gRPC (`dir:grpc`) und Katalog (`dir:catalog`); Kern synchron mit `std::thread`, `crossbeam`/`flume`-Kanälen und `Mutex`/`RwLock` (`main.rs`, Modulkommentar „tokio only for gRPC“) |
| Statistik | `dird/stats.cc` `StartStatisticsThread` | kein Gegenstück gefunden |

### Prozess- und Threadmodell Bareos (C++)

Quelle: `dird/dird.cc:main` (Zeile 136 ff.): `ParseConfigOrExit`, `daemon_start`,
`LoadDirPlugins`, `CheckCatalog`, `InitConsoleMsg`, `StartWatchdog`, `InitJobServer`,
`StartStatisticsThread`, `StartSocketServer`, dann `Scheduler::Run()` im Hauptthread.

```mermaid
flowchart TB
    subgraph P["Prozess bareos-dir"]
        MAIN["Hauptthread<br/>dird.cc main<br/>Scheduler::Run"]
        WD["watchdog_thread<br/>lib/watchdog.cc"]
        STAT["statistics_thread<br/>dird/stats.cc"]
        TCP["tcp_server_tid<br/>socket_server.cc<br/>BnetThreadServerTcp"]
        subgraph TL["ThreadList lib/thread_list.cc"]
            UA1["Konsolen-Thread<br/>HandleUserAgentClientRequest"]
            UA2["Konsolen-Thread ..."]
            FDC["FD-Verbindung<br/>HandleFiledConnection"]
        end
        subgraph JQ["jobq.cc job_queue"]
            JS1["jobq_server Thread<br/>job_thread JCR 1"]
            JS2["jobq_server Thread<br/>job_thread JCR 2"]
            SW["sched_wait Thread<br/>verzögerte Startzeit"]
        end
        MT1["msg_thread JCR 1<br/>msgchan.cc<br/>SD-Meldungen, CatReq"]
        MT2["msg_thread JCR 2"]
    end
    MAIN -- "RunJob / JobqAdd" --> JQ
    UA1 -- "run, restore ..." --> JQ
    TCP --> TL
    JS1 --> MT1
    JS2 --> MT2
    WD -- "JobMonitorWatchdog alle 60 s" --> JQ
    JS1 -. "jcr db" .-> DB[("PostgreSQL")]
    MT1 -. "jcr db" .-> DB
    UA1 -. "ua db" .-> DB
```

Alle Jobs, Konsolen und der Scheduler teilen einen Adressraum. Ein Speicherfehler in
einem Job-Thread beendet den gesamten Director (Signalhandler `InitStackDump`,
`lib/signal.cc`).

### Prozess- und Threadmodell Onesimus (Rust)

Quelle: [`main.rs`](../../crates/onesimus-dird/src/main.rs) (`main`, `spawn_named_threads`),
[`job_worker.rs`](../../crates/onesimus-dird/src/job_worker.rs) (`spawn_job_worker`),
[`catalog_worker.rs`](../../crates/onesimus-dird/src/catalog_worker.rs) (`spawn_catalog_worker`).

```mermaid
flowchart TB
    subgraph D["Prozess onesimus-dird"]
        MAIN["main<br/>wait_for_shutdown<br/>SIGTERM, SIGINT"]
        HUP["dir:sighup<br/>reload.rs sighup_loop"]
        ACC["dir:accept<br/>accept.rs accept_loop"]
        S1["dir:session:N<br/>session.rs"]
        SCH["dir:scheduler<br/>scheduler.rs"]
        WD["dir:watchdog<br/>watchdog.rs"]
        MON["dir:job-monitor<br/>job_manager.rs monitor_loop"]
        JM[["JobManager<br/>Arc Mutex"]]
        CW["dir:cmdworker<br/>command_worker.rs"]
        CQ[["CommandQueue<br/>command_queue.rs"]]
        GRPC["dir:grpc<br/>tokio + tonic"]
        MDNS["dir:mdns"]
        CAT["dir:catalog<br/>tokio current_thread<br/>sqlx PgPool max 8"]
        RS["dir:runscript-N<br/>dir:accurate-N"]
    end
    subgraph W1["Prozess job:17 fork"]
        WK1["run_worker<br/>backup_worker.rs, sd_copy.rs, verify.rs"]
        SDM1["sd-msg:17 / sd-cat:17"]
    end
    subgraph W2["Prozess job:18 fork"]
        WK2["run_worker"]
    end
    ACC --> S1
    S1 -- "command_executor::dispatch" --> JM
    GRPC -- "submit" --> CQ --> CW
    CW --> JM
    SCH -- "commands::job::start_job" --> JM
    MON -- "poll_all" --> JM
    JM -- "fork + 2 Pipes JSON" --> W1
    JM -- "fork" --> W2
    WK1 -- "PipeMessage CatReq, Jmsg, JobEnd" --> MON
    S1 -- "CatalogHandle flume" --> CAT
    CW --> CAT
    MON --> CAT
    JM --> RS
    CAT -.-> DB[("PostgreSQL")]
    WK1 -. "BNET TLS-PSK" .-> FD["File Daemon"]
    WK1 -. "BNET TLS-PSK" .-> SD["Storage Daemon"]
```

Kernpunkte:

- **Fork je Job.** `JobManager::start_ready_jobs` (`job_manager.rs`) ruft den `Spawner`
  (Standard `job_worker::spawn_job_worker`), der mit `nix::unistd::fork` einen Kindprozess
  anlegt, im Kind alle Dateideskriptoren 3…1023 außer den zwei Pipes schließt, den
  Prozessnamen `job:<id>` setzt (`prctl(PR_SET_NAME)`) und `run_worker` ausführt.
  Kommunikation: zwei Pipes, eine JSON-Zeile je `PipeMessage` (`State`, `Status`,
  `CatReq`/`CatResp`, `UpdCat`, `Jmsg`, `JobEnd`, `RunScripts`/`RunScriptsDone`,
  `AccurateRequest`, `QuotaCheck`, `Verify` …). Abbruch per `SIGTERM`
  (`JobHandle::cancel`); `pause`/`resume` (`SIGUSR1`/`SIGUSR2`) sind vorhanden, haben aber
  keinen Aufrufer (`JobManager::pause_job`/`resume_job`, Suche ohne Treffer).
- **Der Worker hat keine Katalogverbindung.** Katalogwünsche des Storage Daemons laufen
  als `PipeMessage::CatReq`/`UpdCat` zum Director und werden dort von `dir:job-monitor`
  (`JobManager::poll_all` → `catreq.rs`) über den Katalog-Thread beantwortet.
- **Ein Lock für alle Jobs.** `JobManager` liegt hinter `Arc<std::sync::Mutex<…>>`.
  `monitor_loop` nimmt die Sperre für jeden Durchlauf (`poll_all`), pollt alle Pipes und
  schläft 100 µs (Jobs aktiv) bzw. 50 ms (leer). Director-eigene Jobs
  (`run_internal_job`: Copy/Migrate-Auswahl, Consolidate) laufen nach Lesart des Codes
  innerhalb dieses Durchlaufs (nicht gemessen). RunScripts laufen auf eigenen Threads
  `dir:runscript-<id>`, damit Konsolenbefehle die Sperre nicht verklemmen (Hinweis S-07).
- **Katalog seriell.** `spawn_catalog_worker` bedient den Kanal in einer Schleife
  `while let Ok((req, resp_tx)) = rx.recv_async().await` und wartet jede Anfrage ab, bevor
  es die nächste nimmt. Nur `CatalogRequest::Dedicated` (reservierte Verbindung, z. B. BVFS
  mit `BEGIN/LOCK/COMMIT`) läuft parallel per `spawn_local`. Der Pool hat 8 Verbindungen,
  genutzt wird für normale Anfragen also praktisch eine zur Zeit.
- **gRPC seriell.** Jede gRPC-Anfrage (Dienst `Director` und `DirectorApi`) wird über
  `CommandQueue::submit` an den einen Thread `dir:cmdworker` übergeben
  (`grpc/service.rs::run_command`, `command_worker.rs::command_worker_loop`). BNET-Konsolen
  führen ihre Befehle dagegen im eigenen Sitzungsthread aus (`session.rs`).

### Komponenten Bareos (C++)

```mermaid
flowchart LR
    subgraph lib["lib/"]
        PC["parse_conf.cc, lex.cc, res.cc<br/>Konfigurationsparser"]
        BS["bsock.cc, bnet.cc, tls_openssl.cc,<br/>cram_md5.cc"]
        MSG["message.cc<br/>Jmsg, Qmsg, DispatchMessage"]
        JCRL["jcr.cc"]
        WDL["watchdog.cc"]
        PLG["plugins.cc"]
    end
    subgraph dird["dird/"]
        CONF["dird_conf.cc<br/>Ressourcentabellen"]
        UA["ua_*.cc<br/>UaContext, Befehle, ACL"]
        JOB["job.cc, jobq.cc"]
        FLOW["backup.cc, restore.cc, verify.cc,<br/>migrate.cc, vbackup.cc, consolidate.cc, admin.cc"]
        DAEM["fd_cmds.cc, sd_cmds.cc, msgchan.cc,<br/>getmsg.cc, catreq.cc"]
        SCHED["scheduler*.cc"]
        DP["dir_plugins.cc"]
        NDMP["ndmp_dma_*.cc"]
    end
    subgraph cats["cats/"]
        BDB["BareosDb, postgresql.cc"]
        SQLX["sql_create/get/update/find/list/delete.cc"]
        DML["dml/*, postgresql_queries.inc"]
        BVFS["bvfs.cc"]
    end
    UA --> JOB --> FLOW --> DAEM
    DAEM --> BS
    UA --> BDB
    FLOW --> BDB
    DAEM --> BDB
    BDB --> SQLX --> DML
    UA --> BVFS --> BDB
    CONF --> PC
    JOB --> DP --> PLG
    FLOW --> MSG
    SCHED --> JOB
```

### Komponenten Onesimus (Rust)

```mermaid
flowchart LR
    subgraph crates["Workspace crates/"]
        BNET["bareos-bnet<br/>wire, tls_psk openssl,<br/>tls rustls, cram_md5, hello"]
        CFG["bareos-config<br/>parser.rs typisiert,<br/>raw.rs Rohsicht, jobdefs.rs"]
        ORM["bareos-orm<br/>JSON-Schema, Migrator"]
        CATC["bareos-catalog<br/>im Director nicht benutzt"]
        PROTO["onesimus-proto-rs<br/>tonic, typed.rs Codec"]
        STO["bareos-stored<br/>SD-Protokollhilfen"]
        OCON["oconsole<br/>Konsole, gRPC-Client, MCP"]
        FILED["bareos-filed<br/>Rust-FD"]
    end
    subgraph dird["onesimus-dird/src"]
        ACC["accept.rs, session.rs,<br/>session_manager.rs"]
        REG["command_registry.rs,<br/>command_executor.rs, commands/*"]
        UAS["ua_select.rs, acl.rs, audit.rs"]
        JMG["job_manager.rs, job_manager/limits.rs"]
        JW["job_worker.rs, backup_worker.rs,<br/>sd_copy.rs, verify.rs"]
        JD["backup.rs, migrate.rs, vbackup.rs,<br/>consolidate.rs, internal_jobs.rs, runscript.rs"]
        CR["catreq.rs"]
        CWK["catalog_worker.rs, sql_list.rs, ua_db.rs"]
        OV["ovfs/, commands/bvfs.rs"]
        MSGS["job_messages.rs, console_msg.rs,<br/>message_bus.rs"]
        SCH["scheduler.rs, scheduled_run.rs"]
        GR["grpc/"]
        DC["director_config.rs, reload.rs,<br/>config_check.rs"]
    end
    ACC --> REG --> UAS
    REG --> JMG --> JW
    JMG --> JD
    JMG --> CR --> CWK
    REG --> CWK
    REG --> OV --> CWK
    JW --> BNET
    ACC --> BNET
    JW --> STO
    DC --> CFG
    GR --> PROTO
    GR --> REG
    OCON --> PROTO
    OCON -.-> BNET
    SCH --> JMG
    JMG --> MSGS
    CWK -. "Start: Migrator" .-> ORM
```

`bareos-catalog` ist als Abhängigkeit eingetragen, im Code von `onesimus-dird` gibt es aber
keinen Aufruf `bareos_catalog::` (Suche vom 17.10.2026). Von `bareos-orm` nutzt der Director
nur den `Migrator` beim Start (`main.rs`).

### Laufzeitmodell im Vergleich

| Aspekt | C++ | Rust | Folge |
|---|---|---|---|
| Gemeinsamer Zustand | globale Variablen (`me`, `my_config`, JCR-Kette), Sperren `LockJobs`, `LockJcrChain`, `ResLocker` | `Arc<RwLock<DirectorConfig>>` (`SharedConfig`), `Arc<Mutex<JobManager>>`, `SessionManager`, statische `Mutex`e (z. B. `audit.rs::FILES`, `catreq::RUNNING_JOBIDS`) | Rust erzwingt Synchronisation zur Übersetzungszeit; Engpass ist das eine `JobManager`-Lock |
| Job-Engine | `job_thread` (`job.cc:415`) ruft je Typ `Do…`/`…Cleanup` im Job-Thread | Init im Director (`backup.rs`, `verify.rs::run_verify` …), Daemon-Teil im Worker, Ende im Director (`JobManager`) | Ein Job ist auf zwei Prozesse verteilt; Zustand wird über Pipes gespiegelt |
| Speichersicherheit | manuelle Speicherverwaltung, `POOLMEM`, `BStringList` | Safe Rust; `unsafe` an Systemgrenzen (`fork`, `libc::kill`, `from_raw_fd`, `prctl`) | weniger Speicherfehlerklassen; `fork` in einem Mehrthread-Prozess bleibt heikel (siehe Risiken) |
| Shutdown | `TerminateDird` (Signalhandler): Socket-Server, Watchdog, Pool, Plugins, Statusdatei, Scheduler, Jobqueue | `SHUTDOWN` (`AtomicBool`) + `join` aller benannten Threads (`main.rs`) | Rust schreibt keine Statusdatei (`WriteStateFile`); nicht weiter geprüft |

---

## 2. Konfiguration

### Parser und Ressourcenmodell

| | C++ | Rust |
|---|---|---|
| Lexer | `lib/lex.cc` | `bareos-config/src/raw.rs` (Lexer-Semantik von `lex.cc`: `#`, `;`, Escapes, verkettete Strings) |
| Parser | `ConfigurationParser::ParseConfig` (`lib/parse_conf.cc:162`), Zustandsmaschine `lib/parse_conf_state_machine.cc`, **zwei Durchläufe** (Pass 1 legt an, Pass 2 löst Verweise) | `bareos-config/src/parser.rs` (`parse_config_dir`, `parse_config_file`, `parse_config`) → typisierte Structs (`resources.rs`); daneben die **Rohsicht** `raw.rs` (alle Direktiven mit Tokens in Original-Reihenfolge) |
| Ressourcen-Schema | `ResourceItem`-Tabellen in `dird/dird_conf.cc` (`dir_items` Zeile 119, `job_items` Zeile 284 …) mit Store-Funktionen und Default-Werten | doppelt: Felder in `bareos-config/src/resources.rs` und eine aus den C++-Tabellen nachgebaute Liste in `commands/show_items.rs` (Name, Typ, Default, Aliase) für `show`, `configure`, `.defaults` |
| JobDefs | `PopulateJobdefaults` in `CheckResources` (`dird/reload.cc:36`) | `bareos-config/src/jobdefs.rs` (C-041) |
| Laufzeit-Container | `my_config` mit `BackupCurrentConfiguration`/`RestoreConfiguration`; alte Konfiguration bleibt für laufende Jobs am Leben (`SetNext`) | `DirectorConfig` (`director_config.rs`) mit typisierten Vektoren, `raw` und Flag `reloaded`; geteilt als `Arc<RwLock<…>>` |
| Ausgabe | `PrintConfig` je Ressource (`lib/res.cc`) | `commands/show.rs` aus `raw` + `show_items.rs` |

Das Rust-Modell hält also zwei Sichten derselben Datei: eine typisierte für die Logik und
eine Rohsicht für Befehle, die die Konfiguration byte-gleich wiedergeben müssen (`show`,
`configure export`). Das ist bewusst (Modulkommentar `raw.rs`), erzeugt aber eine
Konsistenzpflicht zwischen `resources.rs`, `raw.rs` und `show_items.rs`.

### Reload, `omit_defaults`, Konfigurationsprüfung

- **C++** `DoReloadConfig` (`dird/reload.cc:179`): Statistik-Thread stoppen, `LockJobs`,
  `ResLocker`, `DbSqlPoolFlush`, Sicherung der aktuellen Konfiguration, `ParseConfig`,
  `CheckResources`, `CheckCatalog(UPDATE_CATALOG)`, `InitializeSqlPooling`; bei Erfolg
  `ClearQueue` des Schedulers, sonst `RestoreConfiguration`. Das erste `CheckResources`
  setzt `omit_defaults_`; danach druckt `show` ungesetzte Werte anders (C-057, C-064, C-065).
- **Rust** `reload.rs::reload_config`: neu laden (`DirectorConfig::load`), dann
  `config_check::check` (Pass-1/Pass-2-Fehler und `CheckResources`-Regeln, C-057), dann
  `check_catalog::check_connections` je Catalog-Ressource (C-075), erst dann Austausch unter
  dem Schreib-Lock; neue Client/Storage-Sätze per `ensure_basic_records`. `reloaded = true`
  bildet `omit_defaults_` nach. SIGHUP kommt nur beim Thread `dir:sighup` an (`block_sighup`
  in `main` vor allen Threads).
- **Prüfung beim Start:** `-t` (`config_test.rs`, S-29) prüft wie `bareos-dir -t` und endet
  mit denselben Exit-Codes. Der **normale Start prüft nicht** wie `-t`, und Lexer-Diagnosen
  mit Zeile/Spalte fehlen, weil die Rohsicht keine Tokenpositionen hält (offen, C-057,
  [BACKLOG](../BACKLOG.md)).
- **Fehlende Konfigurationsdatei:** `main.rs` startet dann mit einer eingebauten Minimal-
  konfiguration (`Director { Name = bareos-dir … Password = default }`) und warnt nur. Das
  Original bricht ab (`ParseConfigOrExit`). Siehe Risiken.

```mermaid
flowchart TB
    subgraph CPP["C++ DoReloadConfig reload.cc"]
        C1["SIGHUP oder reload"] --> C2["is_reloading test_and_set"]
        C2 --> C3["StopStatisticsThread, LockJobs, ResLocker"]
        C3 --> C4["BackupCurrentConfiguration"]
        C4 --> C5["ParseConfig"]
        C5 --> C6{"CheckResources und<br/>CheckCatalog und<br/>InitializeSqlPooling"}
        C6 -- ja --> C7["Scheduler ClearQueue<br/>alte Konfig an neue haengen<br/>InitDaemonMsg"]
        C6 -- nein --> C8["Please correct ...<br/>RestoreConfiguration"]
        C7 --> C9["UnlockJobs"]
        C8 --> C9
    end
    subgraph RS["Rust reload.rs"]
        R1["SIGHUP an dir:sighup<br/>oder Befehl reload"] --> R2["DirectorConfig::load"]
        R2 --> R3{"config_check::check<br/>C-057"}
        R3 -- Fehler --> R8["failed to reload<br/>Jmsg-Texte ins Daemon-Log<br/>alte Konfig bleibt"]
        R3 -- ok --> R4{"check_catalog::check_connections<br/>C-075"}
        R4 -- Fehler --> R8
        R4 -- ok --> R5["RwLock write: Konfig tauschen<br/>reloaded = true"]
        R5 --> R6["ensure_basic_records<br/>OVFS und Katalog neu registrieren"]
    end
```

Unterschied mit Folgen: Das Original sperrt während des Reloads alle Jobs (`LockJobs`) und
leert die Scheduler-Warteschlange. Onesimus tauscht nur die geteilte Konfiguration; ein
bereits geforkter Worker arbeitet mit seiner Kopie (`JobParams`) weiter, der Director-Teil
desselben Jobs liest nach dem Tausch die neue Konfiguration. Ob das in Randfällen
(Ressource eines laufenden Jobs gelöscht) zu Abweichungen führt, ist nicht geprüft.

---

## 3. Konsole und Netzwerk

### BNET, TLS-PSK, CRAM-MD5

| Schicht | C++ | Rust |
|---|---|---|
| Rahmen `[i32 BE Länge][Nutzdaten]`, negative Länge = Signal | `lib/bsock.cc`, `lib/bnet.cc`, `lib/bnet_protocol_signals.h` | `bareos-bnet/src/wire.rs`, `signal.rs`, `stream.rs` |
| TLS-PSK (Standard seit 18.2) | `lib/tls_openssl.cc`, `TryTlsHandshakeAsAServer` (`lib/try_tls_handshake_as_a_server.cc`) | `bareos-bnet/src/tls_psk.rs` (OpenSSL, weil rustls kein PSK kann); PSK-Identität wählt den Schlüssel (`accept.rs::psk_for_identity`) |
| TLS mit Zertifikat | Ressourcen-Direktiven `TlsCertificate` … (`tls_conf.cc`) | `bareos-bnet/src/tls.rs` (rustls); Zertifikat nur über Kommandozeile `--tls-cert/--tls-key`, `verify_peer: false` (`main.rs`) |
| Erkennung TLS/Klartext | `TryTlsHandshakeAsAServer` | erstes Byte `0x16` (ClientHello) per Peek (`accept.rs::handle_connection`) |
| Hello und Routing | `HandleConnectionRequest` (`dird/socket_server.cc:70`): FD-Hello → `HandleFiledConnection`, sonst `HandleUserAgentClientRequest` | `bareos-bnet/src/hello.rs::parse_hello` → `Hello::DefaultConsole`/`NamedConsole`/`FileDaemon` (`accept.rs::handle_hello`) |
| CRAM-MD5 | `lib/cram_md5.cc` | `bareos-bnet/src/cram_md5.rs` |
| Konsolen-Authentisierung | `dird/authenticate_console.cc` (`ConsoleAuthenticatorBefore_18_2`/`From_18_2`, PAM) | `accept.rs` + `session.rs` (Versionswahl aus dem Hello wie `CreateConsoleAuthenticator`); **kein PAM** |
| Verbindungsgrenze | `MaximumConsoleConnections` (Standard 20) über `ConsoleConnectionLease` (`authenticate_console.cc:398`) | keine Grenze gefunden (nur in `show_items.rs` als Direktive bekannt) |
| Client-initiierte FD-Verbindung | `HandleFiledConnection` hält die Verbindung im `connection_pool` für spätere Jobs | `handle_hello` registriert nur Name und Protokollversion (`SessionManager::add_fd_connection`); Nutzung für Jobs nicht gefunden (S-13 nennt client-initiierte Verbindungen für den eigenen FD als offen) |

### Konsolenbefehl: UaContext gegen session.rs

- **C++:** `HandleUserAgentClientRequest` (`dird/ua_server.cc:85`) erzeugt eine Control-JCR
  (`new_control_jcr("-Console-")`) und einen `UaContext` (`dird/ua.h`), liest Zeilen und ruft
  `Do_a_command` (`dird/ua_cmds.cc:540`), das in der Tabelle `commands[]`
  (`ua_cmds.cc:218`) nachschlägt, die Befehls-ACL prüft und den Handler aufruft. Dialoge
  (`GetCmd`, `DoPrompt`) laufen direkt auf dem Socket (`ua_input.cc`, `ua_select.cc`).
  Ausgabeformate: `OutputFormatter` (`lib/output_formatter.cc`), API-Modi über `.api`.
- **Rust:** `session.rs::handle_console_session_with` liest Zeilen, baut eine `CommandAst`
  (`parse_cli_command`), dann `command_executor::dispatch` → `command_registry::dispatch_registry`
  (`resolve_permitted` prüft Befehl und ACL, sonst „… is an invalid command.“). Die Hilfe- und
  Usage-Texte stammen aus `ua_commands_table.rs`, generiert aus `ua_cmds.cc commands[]`.
  Dialoge: Handler bleiben synchrone Funktionen; `ua_select::with_console` hängt die BNET-
  Verbindung für die Dauer eines Befehls an, ohne Konsole (gRPC, RunScripts) gilt Batch-
  Verhalten wie `ua->batch` (`ua_select.rs`, Modulkommentar).
- **API-Modi:** `ApiMode::Human` (0), `Api` (1, 2) und `Json` (`command_executor.rs`);
  Signale `BNET_ERROR_MSG`/`WARNING_MSG`/`INFO_MSG` vor Meldungen in API-Modi (`session.rs`,
  wie `UaContext::vSendMsg`); JSON-Hülle `json_output.rs::envelope`; Tabellen wie
  `BareosDb::ListResult` in `sql_list.rs`.

```mermaid
sequenceDiagram
    autonumber
    participant B as bconsole
    participant T as Konsolen-Thread ThreadList
    participant UA as UaContext ua_cmds.cc
    participant H as Handler z.B. ua_output.cc
    participant DB as BareosDb ua db
    participant PG as PostgreSQL
    Note over B,PG: C++ bareos-dir
    B->>T: TCP, TLS-PSK Handshake
    B->>T: Hello admin calling version 25
    T->>B: CRAM-MD5 zweiseitig, 1000 OK Banner
    B->>UA: list jobs
    UA->>UA: Do_a_command, ACL Befehl
    UA->>H: ListCmd
    H->>DB: OpenDb, ListJobRecords
    DB->>PG: SQL aus dml oder sql_list.cc
    PG-->>DB: Zeilen
    DB-->>H: ListResult mit OutputFormatter
    H-->>B: Tabelle bzw. JSON, dann EOD
```

```mermaid
sequenceDiagram
    autonumber
    participant B as bconsole oder oconsole BNET
    participant S as dir:session:N session.rs
    participant R as command_registry
    participant H as commands/list.rs
    participant C as dir:catalog catalog_worker
    participant PG as PostgreSQL
    participant M as oconsole MCP
    participant G as dir:grpc tonic
    participant Q as CommandQueue
    participant W as dir:cmdworker
    Note over B,PG: Weg 1 BNET, kompatibel zu bconsole
    B->>S: TLS-PSK, Hello, CRAM-MD5
    B->>S: list jobs
    S->>R: dispatch CommandAst
    R->>R: resolve_permitted ACL
    R->>H: Handler
    H->>C: CatalogHandle query_rows via flume
    C->>PG: sqlx raw_sql
    PG-->>C: Zeilen
    C-->>H: SqlTable
    H-->>S: CommandResult
    S-->>B: Text oder API 2 JSON, EOD
    Note over M,W: Weg 2 gRPC und MCP, Onesimus-Erweiterung
    M->>G: Login HMAC-SHA256, dann ListJobs typisiert
    G->>G: AuthState check_session, ACL Befehl
    G->>Q: submit CommandAst JSON-Modus
    Q->>W: take
    W->>R: dispatch
    R->>H: Handler, Katalog wie oben
    W-->>G: CommandResult
    G-->>M: ListJobsResponse via typed.rs Codec
```

### gRPC und MCP (Onesimus-Erweiterungen)

- **Dienste:** `onesimus.director.Director` (`ExecuteCommand`, Sitzungen, Ereignisse;
  `grpc/service.rs`) und `onesimus.director.api.DirectorApi` (eine RPC je Befehl/Unterbefehl;
  `grpc/api.rs`). Proto-Dateien im Submodul `proto` (Repository onesimus-proto), Rust-Typen
  in `onesimus-proto-rs` (`build.rs`, `typed.rs`). Details: [grpc-api.md](grpc-api.md).
- **Kein zweiter Logikpfad:** Eine typisierte RPC baut eine Konsolenzeile, die durch dieselbe
  Queue, Registry und ACL läuft, im API-2-JSON-Modus; `onesimus_proto_rs::typed` setzt das
  JSON deskriptorgesteuert in die Antwort-Message um (`grpc/api.rs`, Modulkommentar und
  `call`). Damit erbt gRPC die Parität der Konsole, aber auch deren Textformate als
  Zwischenschicht.
- **Build-Trennung:** Schreib-, Sitzungs- und Restore-RPCs nur mit Cargo-Feature
  `grpc-development` (S-35); im Produktions-Build `UNIMPLEMENTED` über
  `generate_default_stubs(true)` (`onesimus-proto-rs/build.rs`).
- **Transport/Anmeldung:** TLS (rustls, Zertifikat aus Konfiguration oder selbst erzeugt per
  `rcgen`, `grpc/tls.rs`), Login per Challenge/Response (Schlüssel = MD5 des Konsolen-
  Passworts, Beweis = HMAC-SHA256 über eine Nonce), danach Sitzungstoken im Header
  `x-onesimus-session` (`grpc/auth.rs`). Port `Grpc Port` bzw. DirPort + 1000, nur mit
  `Enable Grpc = yes`. Entdeckung per mDNS (`mdns.rs`, `_onesimus-dir._tcp.local.` mit
  Zertifikats-Fingerprint als Hinweis).
- **MCP:** `oconsole --mcp` (stdio oder Streamable HTTP mit Bearer-Token). Werkzeuge rufen
  erst die typisierte RPC, bei `UNIMPLEMENTED` `ExecuteCommand`, ohne gRPC die BNET-Zeile.
  Entscheidung C-087: stdio mit vollem Befehlssatz auch im Produktions-Build, HTTP nur lesend.
  Details: [mcp.md](mcp.md).

Das Original hat keinen gRPC- oder MCP-Zugang zum Director; die Bareos-WebUI und
python-bareos sprechen BNET mit `.api 2`.

---

## 4. Job-Lebenszyklus

### Vom `run` zum laufenden Job

| Schritt | C++ | Rust |
|---|---|---|
| Anfrage | `ua_run.cc` `DoRunCmd`, Scheduler `ExecuteJob` | `commands/job.rs::start_job` (auch Scheduler: `scheduled_run.rs` baut eine bestätigte `run … yes`-Zeile plus `RunOverrides`, C-081) |
| JCR | `new_jcr`, `SetJcrDefaults`, `SetupJob` (`job.cc:139`: DB-Verbindung `GetDatabaseConnection`, Job-Record, typabhängiges Init `DoNativeBackupInit` …) | `JobParams` + `JcrState` (`job_manager.rs`); Init im Director, z. B. `backup.rs` (GetOrCreateClientRecord, FileSet-Record, Since-Zeit), `verify.rs::run_verify`, `migrate.rs`, `vbackup.rs` |
| Duplikate | `AllowDuplicateJob` in `SetupJob` | `job_manager/limits.rs` (C-083) |
| Einreihen | `JobqAdd` (`jobq.cc:200`), bei späterer Startzeit eigener Thread `sched_wait` | `JobManager::enqueue_job_with`; Wartende nach Priorität sortiert (`JobqAdd` nachgebaut) |
| Start | `jobq_server` nimmt aus `ready_jobs`, prüft `AcquireResources` (Client/Job/Storage-Zähler), ruft `job_thread` | `start_ready_jobs`: Priorität, `AllowMixedPriority`, Max Concurrent Jobs von Director/Job/Client/Storage; dann `fork` bzw. `run_internal_job` |
| Ausführung | `job_thread` (`job.cc:415`): RunScripts BeforeJob, `switch` auf `JT_BACKUP`/`JT_VERIFY`/`JT_RESTORE`/… → `Do…`, dann `DoAutoprune`, `…Cleanup` | Worker: `run_worker` verzweigt nach `WorkerJobType` (`backup_worker::run`, `sd_copy::run`, `verify::run_worker`, Restore-FSM); BeforeJob fragt per `PipeMessage::RunScripts` den Director |
| Ende | `…Cleanup` im Job-Thread: `UpdateJobEndRecord`, Bericht, AfterJob, `RescheduleJob` | `JobManager::poll_all` nach `JobEnd`/Prozessende: UpdateJobEnd, Bericht (`job_messages.rs`), AfterJob (`runscript.rs`), Reschedule (`take_reruns`) |

### Jobtypen

| Jobtyp | C++ | Rust | Ort der Daemon-Sitzung |
|---|---|---|---|
| Backup (nativ) | `backup.cc` `DoNativeBackupInit`, `DoNativeBackup` (Zeile 503), `NativeBackupCleanup` | `backup.rs` (Director), `backup_worker.rs` (Worker) | Worker |
| Restore | `restore.cc` `DoNativeRestore` (Zeile 382) | `commands/restore.rs`, `commands/restore_bsr.rs` (Bootstrap), Worker-FSM in `job_worker.rs` | Worker |
| Verify | `verify.cc` `DoVerify` (Zeile 112) | `verify.rs` (`run_verify`, `run_worker`, `event`/`finish` im Director) | Worker; Katalogvergleich im Director |
| Copy/Migrate | `migrate.cc` `DoMigrationInit`, `DoMigration` | Auswahl `internal_jobs.rs`, Arbeitsjob `migrate.rs` + `sd_copy.rs` | Steuerjob im Director, Arbeitsjob im Worker; **nur lokales Replizieren** (SD-SD endet fatal, `migrate.rs`) |
| VirtualFull | `vbackup.cc` `DoNativeVbackup` | `vbackup.rs` + `sd_copy.rs` | Worker; Bootstrap/JobId-Liste beim Einreihen statt beim Start (Abweichung laut Modulkommentar) |
| Consolidate | `consolidate.cc` `DoConsolidate` | `consolidate.rs` | Director |
| Admin | `admin.cc` `do_admin` | Worker ohne Daemon-Kontakt (`WorkerJobType::Admin`) | Worker |
| NDMP | `ndmp_dma_*.cc` | `ndmp.rs`: Zweige der Konsolenbefehle; Sitzung → „NDMP protocol not supported“ (C-050) | — |

### Backup-Ablauf mit Daemons und Katalog

```mermaid
sequenceDiagram
    autonumber
    participant J as job_thread C++
    participant MT as msg_thread msgchan.cc
    participant SD as Storage Daemon
    participant FD as File Daemon
    participant DB as BareosDb jcr db
    J->>DB: SetupJob, CreateJobRecord, GetLevelSinceTime
    J->>SD: ConnectToStorageDaemon, StartStorageDaemonJob
    J->>SD: ReserveWriteDevice use storage
    J->>FD: ConnectToFileDaemon, TLS-PSK, CRAM-MD5
    J->>FD: JobId, level, fileset, storage address
    J->>SD: run
    J->>MT: StartStorageDaemonMessageThread
    J->>FD: backup FileIndex=0
    FD->>SD: Datenstrom direkt
    SD->>MT: CatReq Find_media, UpdateMedia, Create_jobmedia
    MT->>DB: CatalogRequest catreq.cc
    DB-->>MT: Ergebnis
    MT-->>SD: Antwort
    SD->>MT: UpdCat Attribute oder BlastAttr Spool
    MT->>DB: CatalogUpdate, Batch-Insert
    FD-->>J: 2800 End Job
    SD-->>MT: 3099 Job end
    J->>DB: UpdateJobEndRecord, Bericht, AfterJob
```

```mermaid
sequenceDiagram
    autonumber
    participant D as Director JobManager
    participant C as dir:catalog
    participant W as Worker job:N
    participant SD as Storage Daemon
    participant FD as File Daemon
    D->>C: backup.rs SetupJob, Job-Record, Since-Zeit
    D->>W: fork, JobParams
    W->>D: RunScripts BeforeJob
    D-->>W: RunScriptsDone
    W->>D: CatalogSql UpdateJobStartRecord
    W->>SD: Job-Befehl, use storage, run
    W->>FD: Hello, CRAM-MD5, JobId, level, fileset
    W->>D: AccurateRequest
    D->>C: AccurateGetJobids, GetFileList
    D-->>W: AccurateReady Datei
    W->>FD: backup FileIndex=0
    FD->>SD: Datenstrom direkt
    SD->>W: CatReq ueber sd-msg Thread
    W->>D: PipeMessage CatReq
    D->>C: catreq.rs FindNextVolume, UpdateMedia
    C-->>D: Ergebnis
    D-->>W: CatResp
    W-->>SD: Antwort
    SD->>W: UpdCat Attribute
    W->>D: PipeMessage UpdCat hex
    D->>C: Attribute einfuegen
    FD-->>W: 2800 End Job
    SD-->>W: 3099 Job end
    W->>D: BackupEnd, JobEnd, Prozessende
    D->>C: UpdateJobEnd
    D->>D: Bericht, AfterJob, Reschedule
```

Jede Katalogantwort an den SD macht damit zwei zusätzliche Sprünge (Pipe zum Director,
Kanal zum Katalog-Thread). `monitor_loop` pollt mit 100 µs Pause, solange Jobs laufen.
Messungen dazu gibt es in diesem Repository nur als Skripte (`tests/benchmark_backup.py`,
`tests/stress`), Ergebnisse habe ich nicht ausgewertet.

### Jobzustände

Die Status-Zeichen sind auf beiden Seiten dieselben (`include/job_status.h`; Rust nutzt die
Zeichen in `job_manager.rs`: `is_queued_status`, `status_priority` mit der Regel aus
`lib/jcr.cc`, dass der erste gleichrangige Fehler bleibt und `I` gewinnt).

```mermaid
stateDiagram-v2
    [*] --> Created: C
    Created --> Waiting: JobqAdd
    state "Wartezustaende t s j c d p" as Waiting
    Waiting --> Running: AcquireResources
    state "Running R" as Running
    Running --> WaitFD: F
    Running --> WaitSD: S
    Running --> WaitMedia: m oder M
    WaitFD --> Running
    WaitSD --> Running
    WaitMedia --> Running
    Running --> Blocked: B Pause
    Blocked --> Running
    Running --> Terminated: T
    Running --> Warnings: W
    Running --> Error: E oder e
    Running --> Fatal: f
    Running --> Canceled: A
    Running --> Incomplete: I
    Waiting --> Canceled: Duplikat oder Max Start Delay
    WaitMedia --> Canceled: Max Wait Time oder C-085
    Terminated --> [*]
    Warnings --> [*]
    Error --> [*]
    Fatal --> [*]
    Canceled --> [*]
    Incomplete --> [*]
```

Zusätzlich führt Onesimus zwei eigene Zustandsautomaten, die das Original nicht hat:

- `DirJobPhase` (`job_manager.rs`): `Queued`, `Starting`, `Running`, `Paused`, `Canceling`,
  `WaitingCatReq`, `WaitingMedia`, `WaitingMount`, `Finishing`, `Completed`, `Error`.
- `WorkerState` (`job_worker.rs`, für den generischen Worker-Pfad, v. a. Restore): `Init` →
  `ConnectingFD` → `ConnectingSD` → `SDJobSetup` → `SDUseDevice` → `SDRun` (`SDBootstrap`)
  → `FDJobSetup` → `FDStorage` → `FDSession` → `FDBackup`/`FDRestore` → `Running` →
  `Finishing` → `Completed`, Seitenwege `Paused`, `Canceling`, `Error`.

```mermaid
stateDiagram-v2
    [*] --> Queued
    Queued --> Starting: start_ready_jobs, fork
    Starting --> Running
    Running --> WaitingCatReq: CatReq offen
    WaitingCatReq --> Running
    Running --> WaitingMedia
    Running --> WaitingMount
    WaitingMedia --> Running
    WaitingMount --> Running
    Running --> Paused: SIGUSR1 ohne Aufrufer
    Paused --> Running: SIGUSR2
    Running --> Canceling: cancel, SIGTERM
    Canceling --> Completed
    Running --> Finishing: JobEnd
    Finishing --> Completed: UpdateJobEnd, Bericht, AfterJob
    Running --> Error: Worker-Absturz oder fork-Fehler
    Completed --> [*]
    Error --> [*]
```

### FD/SD-Protokoll und Katalogwünsche (catreq)

- **C++:** `fd_cmds.cc` (`ConnectToFileDaemon`, `SendIncludeExcludeLists` …), `sd_cmds.cc`,
  `msgchan.cc` (`StartStorageDaemonJob`, `msg_thread`), `getmsg.cc` (`BgetDirmsg` verteilt
  `C`atalog, `U`pdCat, `B`lastAttr), `catreq.cc` (`CatalogRequest` Zeile 117,
  `CatalogUpdate` Zeile 687), `next_vol.cc`, `newvol.cc`, `recycle.cc`, `autoprune.cc`.
- **Rust:** `fd_protocol.rs`, `sd_protocol.rs` (dünne Hülle um `bareos-stored`),
  `backup_worker.rs`, `sd_copy.rs`, `catreq.rs` (Port von `catreq.cc`, der Verteilung aus
  `getmsg.cc` und den aufgerufenen Teilen von `next_vol.cc`, `recycle.cc`, `newvol.cc` und
  `cats/`; Nachrichten werden mit einem Port von `bsscanf`, `lib/scan.cc`, zerlegt, C-055).
  Volume-Auswahl inkl. PruneVolumes, Purge/Recycle Oldest Volume ist nachgezogen (S-27, S-28);
  LabelFormat mit Variablen nennt der Modulkommentar von `catreq.rs` noch als nicht portiert,
  `expand.rs` ist dafür vorgesehen.

---

## 5. Katalog

### Zugriffsschichten

```mermaid
flowchart TB
    subgraph CPP["C++"]
        UAc["UaContext ua->db<br/>ua_db.cc OpenDb"]
        JCRc["JCR jcr->db<br/>GetDatabaseConnection"]
        POOL["sql_pooling.cc"]
        BDB["BareosDb cats.h<br/>BareosDbPostgresql postgresql.cc"]
        FUN["sql_create, sql_get, sql_update,<br/>sql_find, sql_list, sql_delete, bvfs.cc"]
        Q["Abfragetexte: cats/dml/*,<br/>postgresql_queries.inc, FillQuery"]
        LIBPQ["libpq"]
        UAc --> POOL
        JCRc --> POOL
        POOL --> BDB --> FUN --> Q --> LIBPQ
    end
    subgraph RS["Rust"]
        H["Befehle commands/*,<br/>catreq.rs, backup.rs, verify.rs ..."]
        UDB["ua_db.rs SessionCatalog<br/>Using Catalog Meldungen"]
        CH["CatalogHandle<br/>raw_query, query_rows, execute,<br/>dedicated"]
        CH2["flume bounded 256"]
        CWT["dir:catalog<br/>eine Anfrage nach der anderen"]
        SQLX["sqlx PgPool max 8<br/>after_connect: Zeitzone, OVFS-Erkennung"]
        LST["sql_list.rs<br/>ListResult-Format"]
        OVF["ovfs/ native oder fallback"]
        H --> UDB --> CH
        H --> CH --> CH2 --> CWT --> SQLX
        H --> LST
        H --> OVF --> CH
    end
    LIBPQ --> PG[("PostgreSQL<br/>Bareos-Schema 2250")]
    SQLX --> PG
```

| Thema | C++ | Rust |
|---|---|---|
| SQL-Herkunft | zentrale Abfragetexte (`cats/dml/*`, 112 Dateien; `postgresql_queries.inc`) und Funktionen in `sql_*.cc` | SQL-Texte in den Handlern selbst (`catreq.rs`, `commands/*.rs`), als Nachbau der C++-Funktionen; eine eingebundene Datei (`commands/subscription_with.sql`) |
| Verbindungen | je JCR/Konsole eine (gepoolte) Verbindung, parallel nutzbar | ein Katalog-Thread, Anfragen seriell (siehe oben) |
| Ergebnisdarstellung | `BareosDb::ListResult` (`cats/sql.cc`) | `sql_list.rs` (NULL wie `PQgetvalue` als leerer Text, Breite 4) |
| Schema | DDL `cats/ddl`, Upgrade per Skript `update_bareos_tables`; `CheckCatalog` prüft die Version | `bareos-orm`: JSON-Schemabeschreibung (`schema/v2250_bareos_catalog.json`, Drift-Test gegen Original-DDL `tests/upstream_ddl.rs`, C-052) und **automatische Migration beim Start** (`main.rs` → `Migrator::migrate`), wenn das Schemaverzeichnis existiert; v2251/v2252 legen Onesimus-Views und -Funktionen an (`vw_job_summary`, `sp_create_job_record` …) |
| Schutz | — | `bareos-orm` überspringt zerstörende Anweisungen und prüft Sequenzen (C-039) |
| Basisdatensätze beim Start | `CheckCatalog` (`dird/check_catalog.cc`) | `check_catalog.rs::ensure_basic_records` (nur Client/Storage/MediaType; Pools über C-066) |

### Zeitbehandlung (C-082)

Das Original schreibt Job-Zeiten als **lokale Zeit im Text** (`bstrutime`, `lib/btime.cc`,
`cats/sql_update.cc`); libpq-Sitzungen laufen in der Server-Zeitzone. Onesimus schrieb
anfangs `to_timestamp(epoch)` über Stored Procedures und sqlx startete jede Sitzung mit
`TimeZone=UTC`. Behoben: Text wie `bstrutime()` (`job_messages.rs::bstrutime`, genutzt in
`job_manager.rs`, `migrate.rs`, `vbackup.rs`, `catalog_worker.rs`), und
`catalog_worker.rs::server_time_zone` setzt nach dem Verbindungsaufbau die Zone wie libpq
(PGTZ, sonst Rolle/Datenbank, sonst `log_timezone`). Zeitangaben auf der Konsole parst
`btime.rs::str_to_utime` mit der `bsscanf`-Semantik des Originals (C-059).

### OVFS gegen BVFS

| | BVFS (C++ und Rust) | OVFS (nur Onesimus) |
|---|---|---|
| Ort | `cats/bvfs.cc`; in Rust byte-gleich portiert in `commands/bvfs.rs` (C-022) | PostgreSQL-Extension `ovfs` (eigenes Repository) mit Rückfall im Director (`ovfs/`) |
| Arbeitsweise | Cache-Tabellen `PathHierarchy`, `PathVisibility`, Befehle `.bvfs_*` mit eigener Verbindung (`BEGIN/LOCK`) | `ovfs.tree`, `ovfs.ls`, `ovfs.jobids_latest`, `ovfs.versions`; ohne Extension rechnet `ovfs/fallback.rs` + `tree.rs` dasselbe aus den Katalogzeilen per Server-Cursor |
| Erkennung | — | `catalog_worker.rs::detect_ovfs` bei jedem Verbindungsaufbau (`ovfs/capability.rs`) |
| Schnittstelle | `.bvfs_*` (WebUI) | `.ovfs_status/_latest/_tree/_ls/_versions`, MCP-Tools |
| Status | bleibt für Bareos-Clients unverändert erhalten | Onesimus-Weg für Verzeichnisbäume; Äquivalenztest `ovfs/tests.rs` |

---

## 6. Meldungen und Logging

| | C++ | Rust |
|---|---|---|
| Job-Meldung | `Jmsg` (`lib/message.cc:1186`) formatiert und ruft `DispatchMessage` (Zeile 613) | Texte in `job_messages.rs` (Original-Wortlaut), Kopf `<dir> JobId <n>: ` in `console_msg::job_message_text`, Verteilung `console_msg::dispatch_job_message` → `audit::dispatch` |
| Aus dem Worker | — (gleicher Prozess) | `PipeMessage::Jmsg` an den Director (`job_worker.rs`, eine Zeile je `write` unter `JMSG_PIPE`-Lock) |
| Warteschlange | `Qmsg` (Zeile 1396) in `jcr->msg_queue`, ausgegeben von `DequeueMessages` (Zeile 1429) beim nächsten `Jmsg` | nachgebaut für Watchdog-Meldungen (Nachtrag C-083, S-26) |
| Ziele | Messages-Ressource: `console`, `file`, `append`, `catalog`, `director`, `mail`, `mailonerror`, `mailonsuccess`, `operator`, `syslog`, `stdout`, `stderr` | `audit.rs::dispatch` bedient `console`, `file`, `append`, `catalog`. **Mail, Operator, Syslog und die Standardströme werden nicht bedient** (Kommentar an `dispatch`) |
| Konsolen-Warteschlange | `InitConsoleMsg`, Datei `<WorkingDirectory>/<name>.conmsg` (`lib/message.cc`) | `console_msg.rs` mit derselben Datei und denselben Regeln |
| Katalog-Log | `SetDbLogInsertCallback(DirDbLogInsert)` (`dird.cc`) | `job_messages::set_catalog`, `log_insert` |
| Audit | `dird/ua_audit.cc` | `audit.rs` (C-037) |
| Daemon-Log/Debug | `Dmsg`, `debug_level`, Trace-Datei | `tracing` + `tracing-subscriber`; `logging.rs` lädt den Filter bei `setdebug` neu |
| Ereignisse an Clients | — | `message_bus.rs` (crossbeam), Push-Ereignisse über gRPC (`grpc/push_events.rs`) |

```mermaid
flowchart LR
    subgraph CPP["C++"]
        J1["Jmsg jcr, M_INFO ..."] --> DQ["DequeueMessages"]
        Q1["Qmsg"] --> MQ[("jcr->msg_queue")] --> DQ
        DQ --> DM["DispatchMessage"]
        DM --> D1["console .conmsg"]
        DM --> D2["file, append"]
        DM --> D3["catalog Log-Tabelle"]
        DM --> D4["mail, operator<br/>MailCommand bsmtp"]
        DM --> D5["syslog, stdout, stderr"]
    end
    subgraph RS["Rust"]
        WJ["Worker PipeMessage Jmsg"] --> MON["dir:job-monitor"]
        JT["job_messages.rs Texte"] --> CM["console_msg::dispatch_job_message"]
        MON --> CM
        CM --> AD["audit::dispatch"]
        AD --> R1["console .conmsg"]
        AD --> R2["file, append"]
        AD --> R3["catalog log_insert"]
        AD -. "nicht bedient" .-> R4["mail, operator,<br/>syslog, std"]
        CM --> BUS["MessageBus, gRPC-Ereignisse"]
    end
```

Die fehlende Mail-Zustellung ist in [STUBS.md](../STUBS.md) und [BACKLOG.md](../BACKLOG.md)
nicht als offener Punkt geführt (Suche nach „mail“ ohne Treffer). Für Installationen, die
Job-Berichte per `MailCommand` verschicken, ist das eine stille Funktionslücke.

---

## 7. Scheduler, Watchdog, Limits

| | C++ | Rust |
|---|---|---|
| Scheduler | `Scheduler::Run` im Hauptthread (`scheduler.cc`, `scheduler_private.cc`, `scheduler_job_item_queue.cc`): Warteschlange der laufenden und nächsten Stunde, Doppelschutz über `scheduled_last` (61 s) | `scheduler.rs` (gleiche Regeln, S-02) im Thread `dir:scheduler`, Start über `scheduled_run.rs` → `commands::job::start_job` (C-081) |
| Verzögerter Start | Thread `sched_wait` je Job (`jobq.cc:161`) | Warteliste im `JobManager` (Status `t`), kein eigener Thread |
| Watchdog | `lib/watchdog.cc`: ein Thread, registrierte Timer (`RegisterWatchdog`), u. a. `JobMonitorWatchdog` (60 s) und Socket-Timer | `dir:watchdog` (`watchdog.rs`, Intervall 30 s): räumt verwaiste Zombie-Worker gezielt per `waitpid(pid)` ab, meldet einen Scheduler-Herzschlag älter als 120 s; Socket-Timeouts je Verbindung |
| Joblimits | `job_thread` (Max Start Delay, MaxRunSchedTime), `JobMonitorWatchdog` (Max Wait Time, Max Run Time, Level-Max-Runtime) | `job_manager/limits.rs` (S-22, C-083), 60-s-Takt aus `monitor_loop`; Abbruch über `commands::cancel::cancel_from_watchdog` außerhalb der Sperre |
| Onesimus-Standard | Job wartet ohne `Max Wait Time` unbegrenzt auf ein Volume | `DEFAULT_MEDIA_WAIT` = 600 s nur für blockiertes Warten auf Medien (C-085, Entscheidung) |
| Reschedule | `RescheduleJob` (`jobq.cc:567`) | `take_reruns` → `scheduled_run::rerun` (S-26) |
| Run On Incoming Connect Interval | `dird/run_on_incoming_connect_interval.cc` | nicht implementiert (Hinweis C-059) |

Der Modulkommentar von `watchdog.rs` sagt noch, die Joblimits seien „not implemented yet“;
das ist durch `job_manager/limits.rs` überholt.

```mermaid
flowchart TB
    subgraph SCH["dir:scheduler"]
        S1["FillSchedulerJobQueueOrSleep<br/>jetzige und naechste Stunde"] --> S2["WaitForJobsToRun"]
        S2 --> S3["scheduled_run: run job=... yes<br/>plus RunOverrides"]
        S2 --> HB["Herzschlag"]
    end
    S3 --> ST["commands::job::start_job"]
    ST --> DUP{"AllowDuplicateJob<br/>limits.rs"}
    DUP -- "Duplikat verboten" --> CAN1["Status A, vorn in die Queue,<br/>Bericht Canceled"]
    DUP -- ok --> ENQ["enqueue_job_with<br/>Prioritaet, sched_time"]
    ENQ --> RDY{"start_ready_jobs<br/>Prioritaet, Max Concurrent Jobs<br/>Director, Job, Client, Storage"}
    RDY -- "Max Start Delay oder<br/>MaxRunSchedTime ueberschritten" --> CAN2["Fatal, Canceled"]
    RDY -- frei --> RUN["fork Worker oder run_internal_job"]
    subgraph MON["dir:job-monitor alle 60 s"]
        L1["Max Wait Time, Max Run Time,<br/>Level-Max-Runtime, MaxRunSchedTime"]
        L2["C-085: 600 s Warten auf Medien,<br/>wenn keine Max Wait Time"]
    end
    RUN --> MON
    L1 -- Treffer --> CX["cancel_from_watchdog<br/>Qmsg, SIGTERM, FD/SD cancel"]
    L2 -- Treffer --> CX
    subgraph WD["dir:watchdog alle 30 s"]
        W1["Zombie-Worker gezielt reapen"]
        W2["Scheduler-Herzschlag pruefen,<br/>Warnung ab 120 s"]
    end
    HB -.-> W2
```

---

## 8. Sicherheitsmodell

| Thema | C++ | Rust | Bemerkung |
|---|---|---|---|
| Konsolen-ACLs | `UaContext::AclAccessOk` (`dird/ua_acl.cc:40`, `:139`), Profile, POSIX-Regex, `AclHasRestrictions` | `acl.rs` (`allowed`, `access`, `has_restrictions`, `access_audited`); Befehls-ACL vor dem Handler in `resolve_permitted` | Tests in `acl.rs` u. a. „forbidden dispatch never invokes handler through prefix or alias“ |
| ACL in Listen | `SetAclFilter`, Filter in `ua_dotcmds.cc`, `ua_output.cc` | nachgezogen (C-025, C-026) | |
| `show` und Geheimnisse | `DumpResource`/`IsResAllowed`, `hide_sensitive_data` | gleich (C-062): Filter nach ACL, Passwörter als `****************` ohne `configure`-Recht | vorher sah eine eingeschränkte Konsole alle Hashes (Sicherheitsfund) |
| Onesimus-Erweiterungen | — | `configure update/delete` prüfen die Ressourcen-ACL noch nicht (offen, Hinweis C-062); `.sqlcount/.sqltables/.sqlschema` mit derselben ACL wie `.sql` (C-074) | |
| Transport | TLS-PSK aus dem Passwort, optional Zertifikate (`tls_conf.cc`), kTLS (`SetEnableKtls`) | TLS-PSK (OpenSSL), Zertifikat-TLS nur per CLI ohne Peer-Prüfung; gRPC mit rustls und Zertifikats-Pinning beim Client | |
| Authentisierung | CRAM-MD5 zweiseitig; PAM für Konsolen (`dird/auth_pam.cc`, `AuthenticatePamUser`) | CRAM-MD5 zweiseitig; **kein PAM** (nur die Konfigurationsprüfung für PAM-Konsolen in `config_check.rs`); gRPC: HMAC-SHA256-Challenge, Sitzungstoken mit Ablauf (`session_manager.rs::authenticate_session_with_ttl`) | |
| Verbindungsgrenzen | `MaximumConsoleConnections` | nicht durchgesetzt | Angriffsfläche für Verbindungsfluten (jede Verbindung ein Thread) |
| Rechte | `-u`/`-g` (`lib/bsys` `drop`) | `privileges.rs`, vor allen Threads und vor der ersten DB-Verbindung; Fehler sind fatal | |
| MCP | — | stdio voll, HTTP nur lesend, zerstörende Tools mit `confirm: true` (C-087, [mcp.md](mcp.md)) | neue Angriffsfläche, bewusst begrenzt |
| Prozessisolation | keine (ein Adressraum) | Worker als eigener Prozess, schließt geerbte Deskriptoren 3…1023 | Deskriptoren ab 1024 würden vererbt (nicht geprüft, ob es sie gibt) |

---

## 9. Erweiterbarkeit

| | C++ | Rust |
|---|---|---|
| Director-Plugins | `dird/dir_plugins.cc` (`LoadDirPlugins` beim Start, Ereignisse wie Job-Start/-Ende), Python-Director-Plugins über `python-dir` (im Sparse-Checkout nicht enthalten) | **keine** im Director. Das Crate `bareos-plugins` (PyO3, Nachbau des `bareosdir`-Moduls) ist kein Workspace-Mitglied (`Cargo.toml`, `optional-members`) und wird von `onesimus-dird` nicht benutzt. Folge im Code: Plugin-Ereignisse entfallen, z. B. beim Prune (C-030, S-21) |
| FD-Plugins | `filed/fd_plugins.cc`, Python-FD-Plugins (`python-fd`) | eigener FD `bareos-filed` ohne Plugins (S-13 offen); der Onesimus-Director arbeitet mit dem Original-`bareos-fd` samt dessen Python-Plugins, da das Protokoll gleich bleibt |
| Plan | — | [BACKLOG](../BACKLOG.md), Addendum „Rust-only Plugin-System mit verpflichtender Zertifizierung“: nur zertifizierte Rust-Plugins, kein PyO3/Python mehr, fail-closed beim Laden. Status: Entscheidung, Umsetzung offen |
| Externe Schnittstellen | BNET + `.api 2` | zusätzlich gRPC (`Director`, `DirectorApi`), MCP, mDNS; `.help` aus der Registry |
| Konfigurierbare Befehle | `ua_cmds.cc commands[]` | `command_registry.rs` als zentrale Liste (Argumente, Vervollständigung in oconsole, Hilfe) |

---

## 10. Unterschiede, Entscheidungen, Lücken, Stärken und Risiken

### Abbildung der Module

```mermaid
flowchart LR
    subgraph C["Bareos C++"]
        c1["dird.cc main"]
        c2["socket_server.cc, ua_server.cc,<br/>authenticate_console.cc"]
        c3["ua_cmds.cc, ua_dotcmds.cc,<br/>ua_output.cc ..."]
        c4["ua_input.cc, ua_select.cc"]
        c5["ua_acl.cc, ua_audit.cc"]
        c6["job.cc, jobq.cc"]
        c7["backup.cc, fd_cmds.cc,<br/>msgchan.cc"]
        c8["restore.cc, bsr.cc"]
        c9["verify.cc"]
        c10["migrate.cc, vbackup.cc,<br/>consolidate.cc"]
        c11["catreq.cc, getmsg.cc,<br/>next_vol.cc, recycle.cc"]
        c12["cats/ BareosDb, sql_*.cc"]
        c13["cats/bvfs.cc"]
        c14["scheduler*.cc"]
        c15["lib/watchdog.cc"]
        c16["lib/message.cc"]
        c17["lib/parse_conf.cc, dird_conf.cc"]
        c18["reload.cc"]
        c19["lib/bsock.cc, cram_md5.cc,<br/>tls_openssl.cc"]
        c20["dir_plugins.cc"]
        c21["lib/runscript.cc"]
    end
    subgraph R["Onesimus Rust"]
        r1["main.rs"]
        r2["accept.rs, session.rs,<br/>session_manager.rs"]
        r3["command_registry.rs,<br/>commands/*"]
        r4["ua_select.rs"]
        r5["acl.rs, audit.rs"]
        r6["job_manager.rs, limits.rs,<br/>job_worker.rs"]
        r7["backup.rs, backup_worker.rs,<br/>fd_protocol.rs"]
        r8["commands/restore*.rs"]
        r9["verify.rs"]
        r10["migrate.rs, vbackup.rs, consolidate.rs,<br/>internal_jobs.rs, sd_copy.rs"]
        r11["catreq.rs"]
        r12["catalog_worker.rs, sql_list.rs,<br/>bareos-orm"]
        r13["commands/bvfs.rs, ovfs/"]
        r14["scheduler.rs, scheduled_run.rs"]
        r15["watchdog.rs"]
        r16["job_messages.rs, console_msg.rs"]
        r17["bareos-config, director_config.rs"]
        r18["reload.rs, config_check.rs"]
        r19["bareos-bnet"]
        r20["keine"]
        r21["runscript.rs"]
    end
    c1 --> r1
    c2 --> r2
    c3 --> r3
    c4 --> r4
    c5 --> r5
    c6 --> r6
    c7 --> r7
    c8 --> r8
    c9 --> r9
    c10 --> r10
    c11 --> r11
    c12 --> r12
    c13 --> r13
    c14 --> r14
    c15 --> r15
    c15 --> r6
    c16 --> r16
    c17 --> r17
    c18 --> r18
    c19 --> r19
    c20 --> r20
    c21 --> r21
```

### Unterschiedstabelle

| Konzept | C++ | Rust | Folge |
|---|---|---|---|
| Job-Ausführung | Thread je Job (`jobq_server`) | Prozess je Job (`fork`), Director-eigene Jobs im Director | Absturz eines Jobs reißt den Director nicht mit; Zustand über Pipes, zusätzliche Latenz bei CatReq |
| Jobzustand | eine JCR im Speicher | `DirJob`/`JcrState` im Director + Worker-FSM | zwei Wahrheiten, Abgleich über `PipeMessage` |
| Katalogzugriff | parallel, eine Verbindung je JCR/Konsole | seriell über `dir:catalog` (außer `Dedicated`) | einfache Konsistenz, aber Durchsatzgrenze bei vielen gleichzeitigen Jobs/Konsolen (nicht gemessen) |
| SQL-Quelle | `cats/dml`, `sql_*.cc` | SQL in den Handlern | Abfragen müssen bei Upstream-Änderungen einzeln nachgezogen werden |
| Schema | Skripte, Versionsprüfung | Auto-Migration beim Start + Onesimus-Views/-Funktionen | gemeinsamer Katalog mit `bareos-dir` erhält Zusatzobjekte |
| Konfiguration | ein Modell (`ResourceItem`) | typisiert + Rohsicht + `show_items.rs` | drei Stellen pflegen |
| Reload | `LockJobs`, alte Konfig bleibt für laufende Jobs | `RwLock`-Tausch; Worker mit Kopie | kein Job-Stopp beim Reload |
| Start ohne Konfiguration | Abbruch | Minimal-Konfig mit Passwort `default` | siehe Risiken |
| Konsolen | Thread-Pool mit Grenze | Thread je Verbindung ohne Grenze | siehe Risiken |
| gRPC/MCP | keine | `dir:grpc` + `dir:cmdworker` (seriell), oconsole MCP | neue Schnittstellen, aber auf der Konsolenlogik aufgesetzt |
| Meldungsziele | alle inkl. Mail/Syslog | console, file, append, catalog | Mail-Berichte fehlen |
| PAM | ja | nein | PAM-Konsolen nicht nutzbar |
| Plugins | Director- und FD-Plugins | keine im Director | Plugin-Ereignisse entfallen |
| NDMP | ja | nein (Meldung wie Build ohne NDMP) | NDMP-Umgebungen nicht abgedeckt |
| SD-SD-Replikation | ja | nein (nur lokales Replizieren) | Copy/Migrate zwischen zwei SDs endet fatal |
| Statistik-Thread | ja | nein | `Statistics Collect …` ohne Wirkung (nicht geprüft) |

### Bewusste Onesimus-Entscheidungen

| Nr. | Entscheidung |
|---|---|
| C-009 | Versions-/Binary-Info im Bareos-Format, eigenes Versionstoken `25.1.2+onesimus.<gui>` |
| C-074 | `.sqlcount`, `.sqltables`, `.sqlschema` bleiben als dokumentierte Erweiterungen (gleiche ACL wie `.sql`) |
| C-085 | Standard-Wartezeit 600 s auf ein Volume ohne konfigurierte Max Wait Time |
| C-087 | MCP über stdio mit vollem Befehlssatz, über HTTP nur lesend; Werkzeuge nutzen die typisierte `DirectorApi` |
| S-34/S-35 | Typisierte gRPC-API; Schreib-RPCs nur im Entwicklungs-Build |
| OVFS | eigener Weg für Verzeichnisbäume, BVFS nur zur Abwärtskompatibilität (Kompatibilitätslog, Abschnitt „Weiterhin gültig“) |
| Plugins | künftig nur zertifizierte Rust-Plugins ([BACKLOG](../BACKLOG.md), Addendum) |
| Architektur | Fork je Job, tokio nur für gRPC/Katalog (Modulkommentar `main.rs`) |

### Bekannte Lücken

Offene Stubs laut [STUBS.md](../STUBS.md) und Fokus laut [BACKLOG.md](../BACKLOG.md) (17.10.2026):

| ID | Lücke |
|---|---|
| S-37 | `.profile create/set/delete/assign/unassign` ohne Wirkung (`.profile` ist Alias von `.acl`) |
| S-38 | gRPC `ConfigureUpdate`, `ConfigureDelete`, `DeleteStorage` bauen Zeilen, die die eigenen Befehle ablehnen |
| S-39 | `SetBandwidth limit=0` in proto3 nicht übertragbar |
| S-40 | `update jobid=…` auf nie gestarteten Job: SQL-Fehler |
| S-41 | `list nextvolume` findet frisch gelabeltes Volume nicht (gegen Original prüfen) |
| S-13 | Rust-FD ohne PKI, Plugins, Heartbeat, client-initiierte Verbindungen, LMDB-Accurate, Windows/macOS |
| C-014 | SD-Teil von `status all`/`status storage` |
| C-057 | Normaler Start prüft nicht wie `-t`; keine Lexer-Diagnosen |
| C-062 | `configure update/delete` ohne Ressourcen-ACL |
| C-056 | `max_bandwidth` laufender Jobs |
| C-084/S-08 | `label … encrypt` ohne Volume-Schlüssel |
| C-047 | Nicht-UTF-8-Dateinamen auf SQL_ASCII-Katalogen |
| C-059 | `update jobid=… starttime=`, Run On Incoming Connect Interval |
| C-058 | Cancel-Berichtszeilen, `*JobCancel*`-Nummer, passive Clients beim Backup ungeprüft |

Zusätzlich bei dieser Durchsicht gefunden und in keinem der beiden Dokumente geführt (bitte
prüfen, ob sie aufgenommen werden sollen):

- Meldungsziele `mail`, `mailonerror`, `mailonsuccess`, `operator`, `syslog`, `stdout`,
  `stderr` werden nicht bedient (`audit.rs::dispatch`).
- `MaximumConsoleConnections` wird nicht durchgesetzt.
- PAM-Anmeldung für Konsolen fehlt.
- Client-initiierte FD-Verbindungen (`Connection From Client To Director`) werden nur
  registriert.
- Start ohne Konfigurationsdatei mit eingebauter Minimal-Konfiguration (`main.rs`).
- Kein Statistik-Thread, keine Statusdatei (`WriteStateFile`).
- Modulkommentar `watchdog.rs` zu den Joblimits ist veraltet.

### Stärken und Risiken

**Bareos (C++)**

| Stärken | Risiken |
|---|---|
| Ausgereift, 20 Jahre Betrieb, vollständiger Funktionsumfang (NDMP, Plugins, PAM, Mail) | Ein Adressraum: ein Speicherfehler in einem Job oder Plugin beendet alle Jobs |
| Echte Parallelität bei Jobs und Katalogzugriffen (eigene Verbindung je JCR) | Manuelle Speicherverwaltung, viele globale Zustände und Sperren (`LockJobs`, `ResLocker`, JCR-Kette) |
| Zentrale SQL-Quellen (`cats/dml`) | Erweiterung nur über BNET-Text/`.api 2` oder Plugins in C/Python |
| Reload hält alte Konfiguration für laufende Jobs am Leben | Scheduler im Hauptthread: blockiert er, steht die Planung (kein externer Herzschlag) |

**Onesimus (Rust)**

| Stärken | Risiken |
|---|---|
| Prozessisolation je Job: Absturz, Speicherleck oder Hänger eines Jobs bleibt im Worker; Abbruch per Signal | `fork` in einem Mehrthread-Prozess: im Kind existiert nur der forkende Thread; Sperren anderer Threads (Allokator, `tracing`, statische `Mutex`e) können gesperrt geerbt werden. Abgefangen ist das Logging-Reload-Lock (`logging::guard_reload_for_fork`); andere Sperren nicht geprüft |
| Speichersicherheit, `unsafe` nur an Systemgrenzen | Ein `Mutex` für den gesamten `JobManager`, Katalog seriell über einen Thread, gRPC seriell über `dir:cmdworker`: Engpässe bei vielen Jobs/Clients (nicht gemessen) |
| Byte-genaue Parität über einen Paritäts-Harness (`tests/parity`, Baseline 2164 Fälle) | Große Kompatibilitätsfläche: jede Ausgabezeile, jeder Dialog, jedes SQL wird nachgebaut; Upstream-Änderungen müssen einzeln nachgezogen werden |
| Benannte Threads (`dir:*`, `job:<id>`) erleichtern Diagnose mit `ps`/`top` | Doppelte Konfigurationsmodelle (typisiert, roh, `show_items.rs`) |
| Typisierte gRPC-API und MCP auf derselben Befehlslogik, ACL-geprüft | gRPC-Antworten hängen an der API-2-JSON-Ausgabe (Formatänderung → Feldverlust, nur als Debug-Log `unmapped`) |
| Konsolen-Hilfe aus dem Original generiert (`ua_commands_table.rs`), Registry als zentrale Liste | Fehlende Grenzen (Konsolenverbindungen), Start ohne Konfiguration mit Standardpasswort |
| Katalog-Schema mit Drift-Test gegen Original-DDL | Auto-Migration und Onesimus-Objekte im gemeinsam genutzten Katalog; Worker-Polling mit 100 µs Schlaf kostet CPU bei langen Jobs |

### Offene Fragen

- Durchsatz bei vielen gleichzeitigen Jobs: Wie verhalten sich der serielle Katalog-Thread
  und das `JobManager`-Lock gegenüber dem C++-Modell? Ein Lastvergleich fehlt.
- Wirkt ein Reload, der eine von einem laufenden Job genutzte Ressource entfernt, anders als
  im Original?
- Sollen die oben gefundenen, nicht geführten Lücken in [STUBS.md](../STUBS.md) aufgenommen
  werden?
