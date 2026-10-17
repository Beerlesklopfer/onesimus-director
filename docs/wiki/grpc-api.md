# Typisierte gRPC-API (`DirectorApi`)

Der Director bietet neben dem Dienst `Director` (`ExecuteCommand` mit Text-/JSON-Ausgabe) den typisierten Dienst `onesimus.director.api.DirectorApi` an: je Konsolenbefehl bzw. Unterbefehl eine RPC, deren Antwort eine strukturierte Protobuf-Message ist. Definiert in [`proto/director_api.proto`](https://github.com/Beerlesklopfer/onesimus-proto/blob/director-typed-api/director_api.proto) (Repository onesimus-proto, Branch `director-typed-api`), implementiert in `crates/onesimus-dird/src/grpc/api.rs` (lesende RPCs im Produktions-Build, S-34; schreibende, Sitzungs- und Restore-RPCs nur im Entwicklungs-Build mit dem Cargo-Feature `grpc-development`, S-35).

Protobuf-Pakete (seit 17.10.2026, ohne Übergangszeit; vorher `bareos.*`): `onesimus` (Optionen in `onesimus_options.proto`), `onesimus.director` (Dienst `Director`), `onesimus.director.commands`, `onesimus.director.api` (Dienst `DirectorApi`), `onesimus.ccc`. Repository [onesimus-proto](https://github.com/Beerlesklopfer/onesimus-proto); Rust-Typen im Crate `onesimus-proto-rs` (`onesimus_proto_rs::onesimus::director::…`). Die BNET-Konsolenschnittstelle bleibt mit der Bareos-Konsole kompatibel.

## Konventionen der Proto-Dateien

Zusammenfassung der bestehenden Dateien (`director_service.proto`, `director_commands.proto`, `onesimus_options.proto`) und wie `director_api.proto` ihnen folgt:

| Konvention (bestehend) | `director_api.proto` |
|---|---|
| Kopf: `SPDX-License-Identifier: AGPL-3.0-only`, Zeile `BAREOS® - Backup Archiving REcovery Open Sourced`, Copyright-Zeile, Leerzeile, Kurzbeschreibung | gleich |
| `syntax = "proto3"`, Paketschema `onesimus.director[.*]` | Paket `onesimus.director.api` |
| Abschnitte mit `// ====…` und Titel, Befehle als `// name — Beschreibung` | gleich |
| Kommentare deutsch, Umlaute als ae/oe/ue (service), Feldkommentare am Zeilenende | gleich |
| Requests sind `*Command`-Messages mit `option (onesimus.command_name)`, `help_text`, `usage`, `allowed_in_runscript`, `audit_event`; Felder mit `bare_flag`, `required` | jede neue Request-Message trägt die Optionen; `command_name` enthält das Unterkommando wie bei `ConfigureAddCommand` (`"configure add"`) |
| Request-Feldnamen = Konsolen-Schlüsselwörter (`jobid`, `since_jobid`, `oldvolume`) | gleich; zusätzlich `llist` (Langform) |
| Antworten `*Response`, Datensätze `*Info` (`ServerInfo`, `CommandInfo`) | gleich |
| Ergebnis-Feldnamen snake_case | snake_case = Schlüssel der API-2-JSON-Ausgabe (`jobid`, `volbytes`, `client_connection`) |
| Enums: `<PREFIX>_UNSPECIFIED = 0` (bzw. `RESPONSE_FORMAT_DEFAULT = 0`) | `JOB_STATUS_UNSPECIFIED = 0` usw.; Werte von `JobStatus`/`JobType`/`JobLevel` = ASCII-Code des Katalogzeichens |
| Zeitstempel als `int64` (Unix-Zeit), keine Well-Known-Types im Dienst; `onesimus_options.proto` importiert `google/protobuf/descriptor.proto` | Unix-Zeiten `int64`; Katalog-Zeitpunkte als Text (Ortszeit des Directors, ohne Zeitzone gespeichert); `google.protobuf.Struct`/`Value` nur für frei geformte Inhalte (Direktiven von `show`, OVFS-Baum) |
| Wiederverwendung statt Duplikat | wo eine `*Command`-Message aus `director_commands.proto` alle Parameter einer Unterform hat, wird sie verwendet (`VersionCommand`, `StatusCommand`, `ShowCommand`, `Dot*Command`, `RunCommand` …); neue Requests nur, wo keine passt |

Jede neue Message wurde gegen diese Liste geprüft. Abweichungen mit Begründung: die ASCII-Code-Nummerierung der Job-Enums (die Werte sind so ohne Tabelle in das Katalogzeichen umrechenbar), Katalog-Zeitpunkte als Text (eine Umrechnung in UTC bräuchte die Zeitzone des Katalogs) und `google.protobuf.Struct` für Direktiven (ihre Form hängt vom Ressourcentyp ab).

**Inkonsistenzen der bestehenden Dateien** (nicht geändert, die API ist rein additiv):

- Kommentarsprache gemischt: `director_service.proto` deutsch mit englischen Abschnittstiteln („Authentication“, „Response Format Negotiation“), `director_commands.proto` deutsch/englisch gemischt.
- Enum-Nullwerte uneinheitlich: `RESPONSE_FORMAT_DEFAULT = 0` statt `_UNSPECIFIED`; `ConfigureResourceType` mit Präfix `CONFIGURE_RESOURCE_` statt `CONFIGURE_RESOURCE_TYPE_`; `SessionMessage.Type` mit `TYPE_UNSPECIFIED`, aber Werten ohne Präfix (`COMMAND`, `RESPONSE` …).
- Feldnamen: `job_id`/`client_name` (snake_case mit Trenner) im Dienst, `jobid` (Konsolen-Schlüsselwort) in den Befehlen; Job-IDs einmal `uint32` (Events), einmal `int32` (Befehle); Zähler/Limits als `int32`.
- `.acl`- und `.profile`-Messages tragen alle `command_name = ".acl"`/`".profile"`, das Unterkommando steht nur im Kommentar; `configure` dagegen `"configure add"`.
- `usage`-Optionen weichen teils vom Director ab (`ListCommand` nennt nur einen Teil der list-Formen, `StatusCommand` kennt weder `scheduler` noch `job=`).
- Mehreren `*Command`-Messages fehlen Parameter des echten Befehls: `ListCommand` (joblog, backups, copies …), `RunCommand` (nextpool, since, verifyjob …), `DeleteCommand`/`RerunCommand` (`yes`), `PruneCommand` (volume, directory, `yes`), `EstimateCommand` (`listing`), `UpdateCommand` (`encrypt`, `scan`), `DotJobsCommand` (`enabled`/`disabled`), `DotBvfsVersionsCommand` (`jobid`, `copies`, `versions`), `DotBvfsGetJobidsCommand` (`all`); Befehle mit Positionsargumenten haben leere Messages (`HelpCommand`, `VarCommand`, `TraceCommand`, `GuiCommand` …). Für die betroffenen Unterformen gibt es neue Requests in `director_api.proto`, sonst wird die bestehende Message verwendet.
- `CommandResponse.json_output` wird vom Onesimus-Director nicht gefüllt (JSON steht in `text_output`).
- Die Copyright-Zeile nennt in allen Dateien die Bareos GmbH; die neue Datei übernimmt die Kopfzeilen unverändert.

## Verbindung, Anmeldung, Rechte

- Der gRPC-Server läuft, wenn im Director-Ressource `Enable Grpc = yes` steht (Port: `Grpc Port`, sonst DirPort + 1000).
- Gleicher Port, gleiches TLS (selbst signiertes Zertifikat, Fingerprint-Pinning) und gleiche Anmeldung wie der Dienst `Director`: `GetChallenge` + `Login`, danach Session-Token im Metadata-Header `x-onesimus-session`.
- Jede RPC baut eine Konsolenzeile und lässt sie durch dieselbe Befehlsimplementierung laufen wie die Konsole (gleiche SQL-Abfragen, gleiche ACL-Filter). Eine Konsole sieht über die RPC also genau das, was `list`/`show`/`status` ihr zeigen.
- Fehler: Befehl nicht in der CommandACL → `PERMISSION_DENIED`; Befehl fehlgeschlagen → `FAILED_PRECONDITION` mit den Fehlermeldungen des Directors (wie `error.data.messages.error` bei `.api 2`); kein/abgelaufener Token → `UNAUTHENTICATED`; im Produktions-Build nicht enthalten → `UNIMPLEMENTED`; ungültige Parameter oder fehlendes `confirm` (Entwicklungs-Build) → `INVALID_ARGUMENT`.

## Abbildung der API-2-Ausgabe

Die Lese-RPCs führen den Befehl im JSON-Modus aus (wie `.api 2`) und wandeln das `result`-Objekt generisch anhand der Protobuf-Deskriptoren um (`crates/onesimus-proto-rs/src/typed.rs`):

- JSON-Schlüssel → gleichnamiges Feld (Kleinschreibung, `-` → `_`, führender `.` entfällt).
- Zeichenketten werden nur verlustfrei umgewandelt (`"0042"` bleibt Text); Wahrheitswerte aus `1/0`, `t/f`, `true/false`, `yes/no`.
- Enums aus Zahl, Katalogzeichen (`"T"`) oder Name (`"VolumeToCatalog"`, `"Read-Only"`).
- `[{"count": n}]` (Option `count`) → Feld `count`.
- Werte ohne Feld oder ohne verlustfreie Umwandlung → `extra_fields` (Zeichenketten unverändert, sonst JSON-Text). Es geht kein Wert verloren.
- `show`: typisierte Felder je Ressourcentyp plus alle Direktiven als `google.protobuf.Struct directives`.
- Befehle, die unter `.api 2` kein Ergebnis liefern (`status client`, `status storage` ohne `slots`, `status configuration`, `status job`, `.status client/storage`, `list nextvolume`, `messages`, `var`, `resolve`, `estimate`), laufen im Textmodus und füllen das Feld `text`; `list nextvolume` liest zusätzlich die Volumes aus dem Text.

## Beispiele

Mit `grpcurl` (ohne Server-Reflection, daher mit den Proto-Dateien; Port = DirPort + 1000 oder `Grpc Port`):

```sh
DIR=localhost:10101
PROTO="-import-path proto -proto director_service.proto -proto director_api.proto"
NONCE=$(grpcurl -insecure $PROTO -d '{"console_name":"admin"}' $DIR onesimus.director.Director/GetChallenge | jq -r .nonce)
KEY=$(printf '%s' "$PASSWORD" | md5sum | cut -d' ' -f1)
PROOF=$(printf '%s' "$NONCE" | openssl dgst -sha256 -hmac "$KEY" | cut -d' ' -f2)
TOKEN=$(grpcurl -insecure $PROTO -d "{\"console_name\":\"admin\",\"nonce\":\"$NONCE\",\"proof\":\"$PROOF\"}" \
        $DIR onesimus.director.Director/Login | jq -r .sessionToken)

grpcurl -insecure $PROTO -H "x-onesimus-session: $TOKEN" -d '{"limit": 2, "llist": true}' \
        $DIR onesimus.director.api.DirectorApi/ListJobs
```

Antwort (gekürzt):

```json
{
  "jobs": [
    {"jobid": 479, "job": "benchmark-small.2026-03-30_22.26.51_04", "name": "benchmark-small",
     "type": "JOB_TYPE_BACKUP", "level": "JOB_LEVEL_FULL", "client": "bareos-fd",
     "jobstatus": "JOB_STATUS_TERMINATED", "starttime": "2026-03-30 22:26:51",
     "jobfiles": "2605", "jobbytes": "824021543", "poolname": "Full", "fileset": "BenchmarkSmall"}
  ],
  "meta": {"range": {"limit": 2}}
}
```

(`-insecure` nur zum Ausprobieren; produktiv den Fingerprint des Zertifikats pinnen, siehe `oconsole --grpc`.)

In Rust mit tonic (wie im Integrationstest `crates/onesimus-dird/tests/grpc_typed_api.rs`):

```rust
use onesimus_proto_rs::onesimus::director::api::{director_api_client::DirectorApiClient, ListJobsCommand, JobStatus};

let mut api = DirectorApiClient::new(channel.clone());          // gleicher Kanal wie DirectorClient
let mut req = tonic::Request::new(ListJobsCommand { client: "bareos-fd".into(), limit: 20, ..Default::default() });
req.metadata_mut().insert("x-onesimus-session", token.clone());
for job in api.list_jobs(req).await?.into_inner().jobs {
    let failed = matches!(job.jobstatus(), JobStatus::ErrorTerminated | JobStatus::FatalError);
    println!("{} {} {} bytes failed={failed}", job.jobid, job.name, job.jobbytes);
}
```

## Stand der Implementierung

- **Produktions-Build** (Paket, Vorgabe): alle lesenden RPCs – Info/Hilfe, `list`/`llist` aller Formen, `status`/`.status`, `show` aller Ressourcentypen, die Punkt-Auflistungen, `.sql*`-Metadaten, `.acl`/`.profile` (Liste), BVFS-Abfragen und OVFS. Alle übrigen RPCs antworten `UNIMPLEMENTED` (ihr Code ist nicht einkompiliert), wie die MCP-Schreib-Tools von oconsole.
- **Entwicklungs-Build** (`cargo build -p onesimus-dird --features grpc-development`, S-35): zusätzlich die schreibenden Befehle, die Sitzungsbefehle und die interaktive Wiederherstellung.

### Schreibende RPCs (Entwicklungs-Build)

- Jede RPC baut genau eine Konsolenzeile aus dem typisierten Request (`… yes`, wo der Befehl es kennt). Die Werte gehen unverändert als Argumente in den Befehl; Steuerzeichen und `"` werden abgelehnt (`INVALID_ARGUMENT`), ebenso unvollständige oder widersprüchliche Angaben (z.B. `Cancel` mit `jobid` und `job`).
- Der Befehl läuft über die Befehlswarteschlange mit den ACLs der angemeldeten Konsole, im Textmodus mit einer Konsole, die nur die Rückfragen beantwortet, die die RPC nennt: `Purge (yes/no)?` (Purge*), `Are you sure you want to delete` (Delete*), die vorgegebenen Antworten von `SqlQuery` (Abfrage, Leerzeile) und `Query` (Nummer, Parameter). Jede andere Rückfrage wird abgebrochen und steht in `ActionResponse.question` (`success = false`).
- **Zerstörende RPCs** (`Relabel`, `Prune*`, `Purge*`, `Delete*`, `Truncate`) verlangen `confirm = true`, sonst `INVALID_ARGUMENT` – wie die MCP-Tools mit `confirm`.
- Antwort `ActionResponse`: `success`, `text` (Ausgabe wie in der Konsole), `errors`, `warnings`, `command` (ausgeführte Zeile), `jobids` (aus „Job queued. JobId=N“ bzw. dem Request) und die typisierten Datensätze nach der Änderung: `jobs` (Run, Rerun, Restore, RestoreTreeDone, UpdateJob, Cancel), `volumes` (Label, Relabel, UpdateVolume, PruneVolume, PurgeVolume, Truncate mit `volume`), `pools` (CreatePool, UpdatePool); `.bvfs_*`/`.acl` liefern das API-2-Ergebnis in `result`. Ein fehlgeschlagener Befehl ist `success = false` mit den Meldungen, kein gRPC-Fehler.
- Neue Request-Messages, wo die Messages aus `director_commands.proto` Parameter nicht kennen: `RunJobCommand`, `RerunJobCommand`, `RestoreJobCommand`, `RestoreSessionCommand`, `LabelVolumeCommand`, `RelabelVolumeCommand`, `DeleteObjectCommand`, `PruneTargetCommand`, `PurgeTargetCommand`, `UpdateVolumeCommand`, `TruncateVolumesCommand`, `SetDebugLevelCommand`; `PruneVolumeCommand`/`PruneDirectoryCommand` haben `confirm` bekommen.
- `Wait` läuft in einem eigenen Thread (es blockiert bis zum Jobende), alle anderen über die Warteschlange.
- **`.profile`-Änderungen ohne Wirkung:** `DotProfileCreate`, `DotProfileSet`, `DotProfileDelete`, `DotProfileAssign` und `DotProfileUnassign` werden angenommen und laufen durch, ändern aber nichts: `.profile` ist im Director ein Alias von `.acl`, der diese Unterbefehle nicht kennt. Profile bleiben Konfigurationsressourcen (`configure add profile`, `reload`). Offen als [S-37](../STUBS.md).

### Sitzungsbefehle (Entwicklungs-Build)

| RPC | Wirkung |
|---|---|
| `UseCatalog`, `Autodisplay`, `Automount`, `Trace` | wie der Konsolenbefehl, für die Sitzung des Session-Tokens |
| `Wait` | wartet auf den Job bzw. alle Jobs |
| `Quit`, `Exit`, `DotQuit` | beenden die Sitzung: der Session-Token wird ungültig, offene Restore-Sitzungen enden |
| `Gui`, `DotApi`, `DotGrpc`, `SetIp` | nicht anwendbar → `FAILED_PRECONDITION`: das Antwortformat legt die RPC fest, das typisierte API fragt nie nach, und `setip` braucht die Adresse einer BNET-Konsole |

### Interaktive Wiederherstellung (Entwicklungs-Build)

`StartRestoreSession` führt `restore … ` bis zur Dateiauswahl aus (dieselbe Implementierung wie in der Konsole, `commands/restore.rs`/`restore_tree.rs`) und liefert eine `session_id`, die Zahl der Dateien im Baum und das Arbeitsverzeichnis. Ohne `jobid` braucht es `fileset` (jüngster Stand) oder `before`. Fragt der Restore vor der Dateiauswahl etwas (z.B. weil ein Parameter fehlt), wird abgebrochen (`FAILED_PRECONDITION` mit der Frage).

Die `RestoreTree*`-RPCs entsprechen den Befehlen der Dateiauswahl: `Cd`, `Ls` (ls/dir), `LsDir`, `LsMark`, `Pwd`, `Find` liefern `RestoreTreeListResponse` (`cwd`, `entries` mit `name`, `is_dir`, `marked`, dazu `text`); `Mark`, `Unmark`, `MarkDir`, `UnmarkDir`, `Count`, `Estimate` liefern `RestoreTreeCountResponse` (`marked` = neu markiert bzw. markiert, `total`, `bytes`). `RestoreTreeDone` beendet die Auswahl, beantwortet „OK to run?“ mit yes und liefert die JobId des Restore-Jobs; `RestoreTreeAbort` verwirft. Die Sitzung gehört zum Session-Token (andere Logins bekommen `NOT_FOUND`) und endet nach 10 Minuten ohne Aufruf.

```sh
grpcurl … -d '{"client":"fd1","jobid":[631],"where":"/tmp/restore"}' $DIR onesimus.director.api.DirectorApi/StartRestoreSession
grpcurl … -d '{"session_id":"<id>","path":"/etc/"}' $DIR onesimus.director.api.DirectorApi/RestoreTreeCd
grpcurl … -d '{"session_id":"<id>","path":"hosts"}' $DIR onesimus.director.api.DirectorApi/RestoreTreeMark
grpcurl … -d '{"session_id":"<id>"}' $DIR onesimus.director.api.DirectorApi/RestoreTreeDone
```

### Tests

Abbildungs-Tests in `crates/onesimus-proto-rs/src/typed.rs` und `crates/onesimus-dird/src/grpc/api.rs`; Konsolenzeilen und Sperren jeder Schreib-RPC in `grpc/api_write.rs` (nur Entwicklungs-Build); `UNIMPLEMENTED` im Produktions-Build und Warteschlange/Folgeabfrage/Abmeldung im Entwicklungs-Build in `grpc/api.rs`; Abdeckungstest `grpc/coverage.rs` (prüft gegen [`commands/bareos-director-commands.json`](commands/bareos-director-commands.json), dass jeder Befehl und jedes Unterkommando eine RPC hat, dass jede RPC entweder Lese-RPC oder im Entwicklungs-Build implementiert ist, und hält die Tabellen unten aktuell). Integrationstests in `crates/onesimus-dird/tests/grpc_typed_api.rs`:

- `typed_api_matches_console_api2`: Director mit der Parity-Konfiguration auf einer Wegwerf-Kopie von `bareos_cppref`, jede Lese-RPC gegen die `.api 2`-Ausgabe derselben Konsolenzeile, auch für eine eingeschränkte Konsole;
- `typed_api_writes_on_private_daemons` (nur Entwicklungs-Build): eigener Katalog (Bareos-DDL), **private** `bareos-fd`/`bareos-sd` mit eigener Konfiguration, eigenen Ports und Dateigerät (nie die laufenden Daemons): Label, Backup über `Run`, Restore über `Restore` und über eine interaktive `RestoreTree`-Sitzung (Inhalt der wiederhergestellten Dateien geprüft), Abbruch einer Sitzung, Mount/Unmount/Release, UpdateVolume, Purge/Truncate/Delete mit `confirm`, Cancel, DeleteJob, Enable/Disable, ConfigureAdd, Reload, `.sql`, sqlquery, nicht anwendbare RPCs, Abmeldung.

```sh
cargo build -p onesimus-dird && cargo build -p bareos-bnet --example bnet_capture
cargo test -p onesimus-dird --test grpc_typed_api -- --ignored --nocapture
# Entwicklungs-Build; Ports 39240 (Director), +1 FD, +2 SD, +3 gRPC; Restore-Ziel TYPED_API_RESTORE_DIR
cargo test -p onesimus-dird --features grpc-development --test grpc_typed_api -- --ignored --nocapture
```

Die CI (`.github/workflows/package.yml`) testet beide Builds; die Pakete bleiben Produktions-Builds.

## RPCs

<!-- rpcs:begin (generiert, crates/onesimus-dird/src/grpc/coverage.rs) -->
| RPC | Request | Response | Stand |
|---|---|---|---|
| `GetVersion` | `commands.VersionCommand` | `api.VersionResponse` | implementiert |
| `GetTime` | `commands.TimeCommand` | `api.TimeResponse` | implementiert |
| `Whoami` | `commands.WhoamiCommand` | `api.WhoamiResponse` | implementiert |
| `GetMemory` | `commands.MemoryCommand` | `api.MemoryResponse` | implementiert |
| `Noop` | `commands.DotCommand` | `api.NoopResponse` | implementiert |
| `Help` | `api.HelpTopicCommand` | `api.HelpResponse` | implementiert |
| `DotHelp` | `api.DotHelpTopicCommand` | `api.HelpResponse` | implementiert |
| `GetMessages` | `commands.MessagesCommand` | `api.MessagesResponse` | implementiert |
| `DotMessages` | `commands.DotMessagesCommand` | `api.MessagesResponse` | implementiert |
| `ExpandVariables` | `api.VarExpandCommand` | `api.TextResponse` | implementiert |
| `Resolve` | `api.ResolveHostCommand` | `api.TextResponse` | implementiert |
| `Estimate` | `commands.EstimateCommand` | `api.TextResponse` | implementiert |
| `ListJobs` | `api.ListJobsCommand` | `api.ListJobsResponse` | implementiert |
| `GetJob` | `api.ListJobidCommand` | `api.ListJobsResponse` | implementiert |
| `ListBackups` | `api.ListBackupsCommand` | `api.ListJobsResponse` | implementiert |
| `ListCopies` | `api.ListCopiesCommand` | `api.ListCopiesResponse` | implementiert |
| `ListJobLog` | `api.ListJoblogCommand` | `api.ListJobLogResponse` | implementiert |
| `ListJobMedia` | `api.ListJobmediaCommand` | `api.ListJobMediaResponse` | implementiert |
| `ListJobTotals` | `api.ListJobtotalsCommand` | `api.ListJobTotalsResponse` | implementiert |
| `ListJobStatistics` | `api.ListJobstatisticsCommand` | `api.ListJobStatisticsResponse` | implementiert |
| `ListFiles` | `api.ListFilesCommand` | `api.ListFilesResponse` | implementiert |
| `ListBaseFiles` | `api.ListBasefilesCommand` | `api.ListFilesResponse` | implementiert |
| `ListLog` | `api.ListLogCommand` | `api.ListLogResponse` | implementiert |
| `ListVolumes` | `api.ListVolumesCommand` | `api.ListVolumesResponse` | implementiert |
| `ListMedia` | `api.ListVolumesCommand` | `api.ListVolumesResponse` | implementiert |
| `GetVolume` | `api.ListVolumeCommand` | `api.GetVolumeResponse` | implementiert |
| `ListPools` | `api.ListPoolsCommand` | `api.ListPoolsResponse` | implementiert |
| `ListClients` | `api.ListClientsCommand` | `api.ListClientsResponse` | implementiert |
| `ListFilesets` | `api.ListFilesetsCommand` | `api.ListFilesetsResponse` | implementiert |
| `ListStorages` | `api.ListStoragesCommand` | `api.ListStoragesResponse` | implementiert |
| `ListNextVolume` | `api.ListNextvolumeCommand` | `api.NextVolumeResponse` | implementiert |
| `GetDirectorStatus` | `commands.StatusCommand` | `api.DirectorStatusResponse` | implementiert |
| `GetStatusAll` | `commands.StatusCommand` | `api.DirectorStatusResponse` | implementiert |
| `GetSchedulerStatus` | `api.StatusSchedulerCommand` | `api.SchedulerStatusResponse` | implementiert |
| `GetClientStatus` | `commands.StatusCommand` | `api.ClientStatusResponse` | implementiert |
| `GetStorageStatus` | `api.StatusStorageCommand` | `api.StorageStatusResponse` | implementiert |
| `GetSubscriptions` | `api.StatusSubscriptionsCommand` | `api.SubscriptionsResponse` | implementiert |
| `GetConfigurationStatus` | `commands.StatusCommand` | `api.TextResponse` | implementiert |
| `GetJobStatus` | `api.StatusJobCommand` | `api.TextResponse` | implementiert |
| `DotStatusDirector` | `api.DotStatusDirCommand` | `api.DirectorStatusResponse` | implementiert |
| `DotStatusClient` | `api.DotStatusClientCommand` | `api.ClientStatusResponse` | implementiert |
| `DotStatusStorage` | `api.DotStatusStorageCommand` | `api.StorageStatusResponse` | implementiert |
| `ShowCatalogs` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowClients` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowConsoles` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowDirectors` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowFilesets` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowJobdefs` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowJobs` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowMessages` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowPools` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowProfiles` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowSchedules` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowStorages` | `commands.ShowCommand` | `api.ShowResponse` | implementiert |
| `ShowUsers` | `api.ShowUserCommand` | `api.ShowResponse` | implementiert |
| `ShowCounters` | `api.ShowCounterCommand` | `api.ShowResponse` | implementiert |
| `ShowAll` | `api.ShowAllCommand` | `api.ShowResponse` | implementiert |
| `ShowDisabled` | `api.ShowDisabledCommand` | `api.ShowDisabledResponse` | implementiert |
| `DotJobs` | `commands.DotJobsCommand` | `api.NameListResponse` | implementiert |
| `DotJobdefs` | `commands.DotJobdefsCommand` | `api.NameListResponse` | implementiert |
| `DotClients` | `commands.DotClientsCommand` | `api.NameListResponse` | implementiert |
| `DotPools` | `commands.DotPoolsCommand` | `api.NameListResponse` | implementiert |
| `DotStorages` | `commands.DotStoragesCommand` | `api.NameListResponse` | implementiert |
| `DotFilesets` | `commands.DotFilesetsCommand` | `api.NameListResponse` | implementiert |
| `DotCatalogs` | `commands.DotCatalogsCommand` | `api.NameListResponse` | implementiert |
| `DotConsoles` | `commands.DotConsolesCommand` | `api.NameListResponse` | implementiert |
| `DotUsers` | `commands.DotUsersCommand` | `api.NameListResponse` | implementiert |
| `DotSchedule` | `commands.DotScheduleCommand` | `api.NameListResponse` | implementiert |
| `DotMsgs` | `commands.DotMsgsCommand` | `api.NameListResponse` | implementiert |
| `DotProfiles` | `commands.DotProfilesCommand` | `api.NameListResponse` | implementiert |
| `DotTypes` | `commands.DotTypesCommand` | `api.NameListResponse` | implementiert |
| `DotVolstatus` | `commands.DotVolstatusCommand` | `api.NameListResponse` | implementiert |
| `DotActiononpurge` | `commands.DotActiononpurgeCommand` | `api.NameListResponse` | implementiert |
| `DotMedia` | `commands.DotMediaCommand` | `api.NameListResponse` | implementiert |
| `DotMediatypes` | `commands.DotMediatypesCommand` | `api.NameListResponse` | implementiert |
| `DotLocations` | `commands.DotLocationsCommand` | `api.NameListResponse` | implementiert |
| `DotLevels` | `commands.DotLevelsCommand` | `api.LevelListResponse` | implementiert |
| `DotJobstatus` | `commands.DotJobstatusCommand` | `api.JobStatusListResponse` | implementiert |
| `DotDefaults` | `commands.DotDefaultsCommand` | `api.DefaultsResponse` | implementiert |
| `DotAuthorized` | `commands.DotAuthorizedCommand` | `api.AuthorizedResponse` | implementiert |
| `DotSqlTables` | `commands.DotSqlTablesCommand` | `api.SqlTablesResponse` | implementiert |
| `DotSqlCount` | `commands.DotSqlCountCommand` | `api.SqlTablesResponse` | implementiert |
| `DotSqlSchema` | `commands.DotSqlSchemaCommand` | `api.SqlSchemaResponse` | implementiert |
| `DotAclList` | `commands.DotAclListCommand` | `api.AclListResponse` | implementiert |
| `DotProfileList` | `commands.DotProfileListCommand` | `api.NameListResponse` | implementiert |
| `DotBvfsGetJobids` | `commands.DotBvfsGetJobidsCommand` | `api.BvfsJobIdsResponse` | implementiert |
| `DotBvfsLsdirs` | `commands.DotBvfsLsdirsCommand` | `api.BvfsListResponse` | implementiert |
| `DotBvfsLsfiles` | `commands.DotBvfsLsfilesCommand` | `api.BvfsListResponse` | implementiert |
| `DotBvfsVersions` | `commands.DotBvfsVersionsCommand` | `api.BvfsListResponse` | implementiert |
| `DotOvfsStatus` | `api.DotOvfsStatusCommand` | `api.OvfsStatusResponse` | implementiert |
| `DotOvfsLatest` | `api.DotOvfsLatestCommand` | `api.OvfsLatestResponse` | implementiert |
| `DotOvfsTree` | `api.DotOvfsTreeCommand` | `api.OvfsTreeResponse` | implementiert |
| `DotOvfsLs` | `api.DotOvfsLsCommand` | `api.OvfsLsResponse` | implementiert |
| `DotOvfsVersions` | `api.DotOvfsVersionsCommand` | `api.OvfsVersionsResponse` | implementiert |
| `Run` | `api.RunJobCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Rerun` | `api.RerunJobCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Cancel` | `commands.CancelCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Restore` | `api.RestoreJobCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Label` | `api.LabelVolumeCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Relabel` | `api.RelabelVolumeCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Mount` | `commands.MountCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Unmount` | `commands.UnmountCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Release` | `commands.ReleaseCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `AddVolumes` | `commands.AddCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `CreatePool` | `commands.CreateCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DeleteVolume` | `api.DeleteObjectCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DeletePool` | `api.DeleteObjectCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DeleteJob` | `api.DeleteObjectCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DeleteStorage` | `api.DeleteObjectCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Enable` | `commands.EnableCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Disable` | `commands.DisableCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `ExportVolumes` | `commands.ExportCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `ImportVolumes` | `commands.ImportCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `MoveVolumes` | `commands.MoveCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PruneFiles` | `api.PruneTargetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PruneJobs` | `api.PruneTargetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PruneVolume` | `api.PruneVolumeCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PruneStats` | `api.PruneTargetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PruneDirectory` | `api.PruneDirectoryCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PurgeFiles` | `api.PurgeTargetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PurgeJobs` | `api.PurgeTargetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PurgeVolume` | `api.PurgeTargetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `PurgeQuota` | `api.PurgeTargetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `UpdateVolume` | `api.UpdateVolumeCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `UpdatePool` | `commands.UpdateCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `UpdateSlots` | `api.UpdateSlotsCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `UpdateJob` | `commands.UpdateCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `UpdateStats` | `commands.UpdateCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Truncate` | `api.TruncateVolumesCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `ConfigureAdd` | `commands.ConfigureAddCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `ConfigureExport` | `commands.ConfigureExportCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `ConfigureDelete` | `commands.ConfigureDeleteCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `ConfigureUpdate` | `commands.ConfigureUpdateCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Reload` | `commands.ReloadCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `SetDebug` | `api.SetDebugLevelCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `SetBandwidth` | `commands.SetbandwidthCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `SetDevice` | `commands.SetdeviceCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `SetIp` | `commands.SetipCommand` | `api.ActionResponse` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `Trace` | `api.TraceSwitchCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotSql` | `commands.DotSqlCommand` | `api.SqlQueryResponse` | Entwicklungs-Build (`grpc-development`) |
| `SqlQuery` | `commands.SqlQueryCommand` | `api.SqlQueryResponse` | Entwicklungs-Build (`grpc-development`) |
| `Query` | `api.QueryRunCommand` | `api.SqlQueryResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotBvfsUpdate` | `commands.DotBvfsUpdateCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotBvfsRestore` | `commands.DotBvfsRestoreCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotBvfsCleanup` | `commands.DotBvfsCleanupCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotBvfsClearCache` | `commands.DotBvfsClearCacheCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotAclSet` | `commands.DotAclSetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotAclDelete` | `commands.DotAclDeleteCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotAclClear` | `commands.DotAclClearCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotAclCopy` | `commands.DotAclCopyCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotProfileCreate` | `commands.DotProfileCreateCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotProfileSet` | `commands.DotProfileSetCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotProfileDelete` | `commands.DotProfileDeleteCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotProfileAssign` | `commands.DotProfileAssignCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotProfileUnassign` | `commands.DotProfileUnassignCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `UseCatalog` | `commands.UseCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Wait` | `commands.WaitCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Gui` | `api.GuiSwitchCommand` | `api.ActionResponse` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `Autodisplay` | `api.AutodisplaySwitchCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Automount` | `api.AutomountSwitchCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotApi` | `api.DotApiModeCommand` | `api.ActionResponse` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `DotGrpc` | `commands.DotGrpcCommand` | `api.ActionResponse` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `Quit` | `commands.QuitCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `Exit` | `commands.ExitCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `DotQuit` | `commands.DotQuitCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `StartRestoreSession` | `api.RestoreSessionCommand` | `api.RestoreSessionResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeCd` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeLs` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeLsDir` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeLsMark` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreePwd` | `api.RestoreTreeSessionCommand` | `api.RestoreTreeListResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeMark` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeUnmark` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeMarkDir` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeUnmarkDir` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeCount` | `api.RestoreTreeSessionCommand` | `api.RestoreTreeCountResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeFind` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeEstimate` | `api.RestoreTreeSessionCommand` | `api.RestoreTreeCountResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeHelp` | `api.RestoreTreeSessionCommand` | `api.HelpResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeDone` | `api.RestoreTreeSessionCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
| `RestoreTreeAbort` | `api.RestoreTreeSessionCommand` | `api.ActionResponse` | Entwicklungs-Build (`grpc-development`) |
<!-- rpcs:end -->

## Abdeckung: Befehl → RPC

Unterbefehl `key=` steht für die Form `key=<wert>`, ein Wort für das Schlüsselwort; `–` für den Befehl selbst.

<!-- coverage:begin (generiert, crates/onesimus-dird/src/grpc/coverage.rs) -->
| Befehl | Unterbefehl | RPC | Stand |
|---|---|---|---|
| `.` | – | `Noop` | implementiert |
| `.actiononpurge` | – | `DotActiononpurge` | implementiert |
| `.api` | – | `DotApi` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `.authorized` | – | `DotAuthorized` | implementiert |
| `.catalogs` | – | `DotCatalogs` | implementiert |
| `.clients` | – | `DotClients` | implementiert |
| `.consoles` | – | `DotConsoles` | implementiert |
| `.users` | – | `DotUsers` | implementiert |
| `.defaults` | – | `DotDefaults` | implementiert |
| `.filesets` | – | `DotFilesets` | implementiert |
| `.help` | – | `DotHelp` | implementiert |
| `.jobdefs` | – | `DotJobdefs` | implementiert |
| `.jobs` | – | `DotJobs` | implementiert |
| `.jobstatus` | – | `DotJobstatus` | implementiert |
| `.levels` | – | `DotLevels` | implementiert |
| `.locations` | – | `DotLocations` | implementiert |
| `.messages` | – | `DotMessages` | implementiert |
| `.media` | – | `DotMedia` | implementiert |
| `.mediatypes` | – | `DotMediatypes` | implementiert |
| `.msgs` | – | `DotMsgs` | implementiert |
| `.pools` | – | `DotPools` | implementiert |
| `.profiles` | – | `DotProfiles` | implementiert |
| `.quit` | – | `DotQuit` | Entwicklungs-Build (`grpc-development`) |
| `.sql` | – | `DotSql` | Entwicklungs-Build (`grpc-development`) |
| `.schedule` | – | `DotSchedule` | implementiert |
| `.status` | `dir` | `DotStatusDirector` | implementiert |
| `.status` | `storage=` | `DotStatusStorage` | implementiert |
| `.status` | `client=` | `DotStatusClient` | implementiert |
| `.storages` | – | `DotStorages` | implementiert |
| `.types` | – | `DotTypes` | implementiert |
| `.volstatus` | – | `DotVolstatus` | implementiert |
| `.bvfs_lsdirs` | – | `DotBvfsLsdirs` | implementiert |
| `.bvfs_lsfiles` | – | `DotBvfsLsfiles` | implementiert |
| `.bvfs_update` | – | `DotBvfsUpdate` | Entwicklungs-Build (`grpc-development`) |
| `.bvfs_get_jobids` | – | `DotBvfsGetJobids` | implementiert |
| `.bvfs_versions` | – | `DotBvfsVersions` | implementiert |
| `.bvfs_restore` | – | `DotBvfsRestore` | Entwicklungs-Build (`grpc-development`) |
| `.bvfs_cleanup` | – | `DotBvfsCleanup` | Entwicklungs-Build (`grpc-development`) |
| `.bvfs_clear_cache` | – | `DotBvfsClearCache` | Entwicklungs-Build (`grpc-development`) |
| `add` | – | `AddVolumes` | Entwicklungs-Build (`grpc-development`) |
| `autodisplay` | – | `Autodisplay` | Entwicklungs-Build (`grpc-development`) |
| `automount` | – | `Automount` | Entwicklungs-Build (`grpc-development`) |
| `cancel` | – | `Cancel` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add client` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add jobdefs` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add job` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add storage` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add catalog` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add schedule` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add fileset` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add pool` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add messages` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add counter` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add profile` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add console` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `add user` | `ConfigureAdd` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `export` | `ConfigureExport` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `delete` | `ConfigureDelete` | Entwicklungs-Build (`grpc-development`) |
| `configure` | `update` | `ConfigureUpdate` | Entwicklungs-Build (`grpc-development`) |
| `create` | – | `CreatePool` | Entwicklungs-Build (`grpc-development`) |
| `delete` | `volume=` | `DeleteVolume` | Entwicklungs-Build (`grpc-development`) |
| `delete` | `volume` | `DeleteVolume` | Entwicklungs-Build (`grpc-development`) |
| `delete` | `pool=` | `DeletePool` | Entwicklungs-Build (`grpc-development`) |
| `delete` | `jobid=` | `DeleteJob` | Entwicklungs-Build (`grpc-development`) |
| `delete` | `storage` | `DeleteStorage` | Entwicklungs-Build (`grpc-development`) |
| `disable` | – | `Disable` | Entwicklungs-Build (`grpc-development`) |
| `enable` | – | `Enable` | Entwicklungs-Build (`grpc-development`) |
| `estimate` | – | `Estimate` | implementiert |
| `exit` | – | `Exit` | Entwicklungs-Build (`grpc-development`) |
| `export` | – | `ExportVolumes` | Entwicklungs-Build (`grpc-development`) |
| `gui` | – | `Gui` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `help` | – | `Help` | implementiert |
| `import` | – | `ImportVolumes` | Entwicklungs-Build (`grpc-development`) |
| `label` | – | `Label` | Entwicklungs-Build (`grpc-development`) |
| `list` | `basefiles` | `ListBaseFiles` | implementiert |
| `list` | `backups` | `ListBackups` | implementiert |
| `list` | `client=` | `ListClients` | implementiert |
| `list` | `clients` | `ListClients` | implementiert |
| `list` | `copies` | `ListCopies` | implementiert |
| `list` | `files` | `ListFiles` | implementiert |
| `list` | `filesets` | `ListFilesets` | implementiert |
| `list` | `fileset` | `ListFilesets` | implementiert |
| `list` | `jobs` | `ListJobs` | implementiert |
| `list` | `job=` | `ListJobs` | implementiert |
| `list` | `jobid=` | `GetJob` | implementiert |
| `list` | `ujobid=` | `GetJob` | implementiert |
| `list` | `joblog` | `ListJobLog` | implementiert |
| `list` | `jobmedia` | `ListJobMedia` | implementiert |
| `list` | `jobtotals` | `ListJobTotals` | implementiert |
| `list` | `jobstatistics` | `ListJobStatistics` | implementiert |
| `list` | `log` | `ListLog` | implementiert |
| `list` | `media` | `ListMedia` | implementiert |
| `list` | `media=` | `GetVolume` | implementiert |
| `list` | `nextvolume` | `ListNextVolume` | implementiert |
| `list` | `pools` | `ListPools` | implementiert |
| `list` | `pool=` | `ListPools` | implementiert |
| `list` | `poolid=` | `ListPools` | implementiert |
| `list` | `storages` | `ListStorages` | implementiert |
| `list` | `volumes` | `ListVolumes` | implementiert |
| `list` | `volume=` | `GetVolume` | implementiert |
| `list` | `volumeid=` | `GetVolume` | implementiert |
| `list` | `mediaid=` | `GetVolume` | implementiert |
| `llist` | `basefiles` | `ListBaseFiles` | implementiert |
| `llist` | `backups` | `ListBackups` | implementiert |
| `llist` | `client=` | `ListClients` | implementiert |
| `llist` | `clients` | `ListClients` | implementiert |
| `llist` | `copies` | `ListCopies` | implementiert |
| `llist` | `files` | `ListFiles` | implementiert |
| `llist` | `filesets` | `ListFilesets` | implementiert |
| `llist` | `fileset` | `ListFilesets` | implementiert |
| `llist` | `jobs` | `ListJobs` | implementiert |
| `llist` | `job=` | `ListJobs` | implementiert |
| `llist` | `jobid=` | `GetJob` | implementiert |
| `llist` | `ujobid=` | `GetJob` | implementiert |
| `llist` | `joblog` | `ListJobLog` | implementiert |
| `llist` | `jobmedia` | `ListJobMedia` | implementiert |
| `llist` | `jobtotals` | `ListJobTotals` | implementiert |
| `llist` | `jobstatistics` | `ListJobStatistics` | implementiert |
| `llist` | `log` | `ListLog` | implementiert |
| `llist` | `media` | `ListMedia` | implementiert |
| `llist` | `media=` | `GetVolume` | implementiert |
| `llist` | `nextvolume` | `ListNextVolume` | implementiert |
| `llist` | `pools` | `ListPools` | implementiert |
| `llist` | `pool=` | `ListPools` | implementiert |
| `llist` | `poolid=` | `ListPools` | implementiert |
| `llist` | `storages` | `ListStorages` | implementiert |
| `llist` | `volumes` | `ListVolumes` | implementiert |
| `llist` | `volume=` | `GetVolume` | implementiert |
| `llist` | `volumeid=` | `GetVolume` | implementiert |
| `llist` | `mediaid=` | `GetVolume` | implementiert |
| `messages` | – | `GetMessages` | implementiert |
| `memory` | – | `GetMemory` | implementiert |
| `mount` | – | `Mount` | Entwicklungs-Build (`grpc-development`) |
| `move` | – | `MoveVolumes` | Entwicklungs-Build (`grpc-development`) |
| `prune` | `files` | `PruneFiles` | Entwicklungs-Build (`grpc-development`) |
| `prune` | `jobs` | `PruneJobs` | Entwicklungs-Build (`grpc-development`) |
| `prune` | `volume` | `PruneVolume` | Entwicklungs-Build (`grpc-development`) |
| `prune` | `stats` | `PruneStats` | Entwicklungs-Build (`grpc-development`) |
| `prune` | `directory` | `PruneDirectory` | Entwicklungs-Build (`grpc-development`) |
| `purge` | `files` | `PurgeFiles` | Entwicklungs-Build (`grpc-development`) |
| `purge` | `jobs` | `PurgeJobs` | Entwicklungs-Build (`grpc-development`) |
| `purge` | `volume` | `PurgeVolume` | Entwicklungs-Build (`grpc-development`) |
| `purge` | `quota` | `PurgeQuota` | Entwicklungs-Build (`grpc-development`) |
| `quit` | – | `Quit` | Entwicklungs-Build (`grpc-development`) |
| `query` | – | `Query` | Entwicklungs-Build (`grpc-development`) |
| `restore` | – | `Restore` | Entwicklungs-Build (`grpc-development`) |
| `restore` | `interaktive Dateiauswahl` | `StartRestoreSession` | Entwicklungs-Build (`grpc-development`) |
| `relabel` | – | `Relabel` | Entwicklungs-Build (`grpc-development`) |
| `release` | – | `Release` | Entwicklungs-Build (`grpc-development`) |
| `reload` | – | `Reload` | Entwicklungs-Build (`grpc-development`) |
| `rerun` | – | `Rerun` | Entwicklungs-Build (`grpc-development`) |
| `resolve` | – | `Resolve` | implementiert |
| `run` | – | `Run` | Entwicklungs-Build (`grpc-development`) |
| `status` | `all` | `GetStatusAll` | implementiert |
| `status` | `dir=` | `GetDirectorStatus` | implementiert |
| `status` | `dir` | `GetDirectorStatus` | implementiert |
| `status` | `director` | `GetDirectorStatus` | implementiert |
| `status` | `days=` | `GetDirectorStatus` | implementiert |
| `status` | `scheduler` | `GetSchedulerStatus` | implementiert |
| `status` | `schedule=` | `GetSchedulerStatus` | implementiert |
| `status` | `client=` | `GetClientStatus` | implementiert |
| `status` | `storage=` | `GetStorageStatus` | implementiert |
| `status` | `job=` | `GetJobStatus` | implementiert |
| `status` | `subscriptions` | `GetSubscriptions` | implementiert |
| `status` | `configuration` | `GetConfigurationStatus` | implementiert |
| `setbandwidth` | – | `SetBandwidth` | Entwicklungs-Build (`grpc-development`) |
| `setdebug` | – | `SetDebug` | Entwicklungs-Build (`grpc-development`) |
| `setdevice` | – | `SetDevice` | Entwicklungs-Build (`grpc-development`) |
| `setip` | – | `SetIp` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `show` | `catalog=` | `ShowCatalogs` | implementiert |
| `show` | `client=` | `ShowClients` | implementiert |
| `show` | `console=` | `ShowConsoles` | implementiert |
| `show` | `director=` | `ShowDirectors` | implementiert |
| `show` | `fileset=` | `ShowFilesets` | implementiert |
| `show` | `jobdefs=` | `ShowJobdefs` | implementiert |
| `show` | `job=` | `ShowJobs` | implementiert |
| `show` | `message=` | `ShowMessages` | implementiert |
| `show` | `pool=` | `ShowPools` | implementiert |
| `show` | `profile=` | `ShowProfiles` | implementiert |
| `show` | `schedule=` | `ShowSchedules` | implementiert |
| `show` | `storage=` | `ShowStorages` | implementiert |
| `show` | `user=` | `ShowUsers` | implementiert |
| `show` | `counter=` | `ShowCounters` | implementiert |
| `show` | `catalog` | `ShowCatalogs` | implementiert |
| `show` | `clients` | `ShowClients` | implementiert |
| `show` | `consoles` | `ShowConsoles` | implementiert |
| `show` | `directors` | `ShowDirectors` | implementiert |
| `show` | `filesets` | `ShowFilesets` | implementiert |
| `show` | `jobdefs` | `ShowJobdefs` | implementiert |
| `show` | `jobs` | `ShowJobs` | implementiert |
| `show` | `messages` | `ShowMessages` | implementiert |
| `show` | `pools` | `ShowPools` | implementiert |
| `show` | `profiles` | `ShowProfiles` | implementiert |
| `show` | `schedules` | `ShowSchedules` | implementiert |
| `show` | `storages` | `ShowStorages` | implementiert |
| `show` | `users` | `ShowUsers` | implementiert |
| `show` | `counters` | `ShowCounters` | implementiert |
| `show` | `disabled` | `ShowDisabled` | implementiert |
| `show` | `all` | `ShowAll` | implementiert |
| `sqlquery` | – | `SqlQuery` | Entwicklungs-Build (`grpc-development`) |
| `time` | – | `GetTime` | implementiert |
| `trace` | – | `Trace` | Entwicklungs-Build (`grpc-development`) |
| `truncate` | – | `Truncate` | Entwicklungs-Build (`grpc-development`) |
| `unmount` | – | `Unmount` | Entwicklungs-Build (`grpc-development`) |
| `umount` | – | `Unmount` | Entwicklungs-Build (`grpc-development`) |
| `update` | `volume=` | `UpdateVolume` | Entwicklungs-Build (`grpc-development`) |
| `update` | `pool=` | `UpdatePool` | Entwicklungs-Build (`grpc-development`) |
| `update` | `slots` | `UpdateSlots` | Entwicklungs-Build (`grpc-development`) |
| `update` | `jobid=` | `UpdateJob` | Entwicklungs-Build (`grpc-development`) |
| `update` | `stats` | `UpdateStats` | Entwicklungs-Build (`grpc-development`) |
| `use` | – | `UseCatalog` | Entwicklungs-Build (`grpc-development`) |
| `var` | – | `ExpandVariables` | implementiert |
| `version` | – | `GetVersion` | implementiert |
| `wait` | – | `Wait` | Entwicklungs-Build (`grpc-development`) |
| `whoami` | – | `Whoami` | implementiert |
| `.acl` | `list` | `DotAclList` | implementiert |
| `.acl` | `set` | `DotAclSet` | Entwicklungs-Build (`grpc-development`) |
| `.acl` | `delete` | `DotAclDelete` | Entwicklungs-Build (`grpc-development`) |
| `.acl` | `clear` | `DotAclClear` | Entwicklungs-Build (`grpc-development`) |
| `.acl` | `copy` | `DotAclCopy` | Entwicklungs-Build (`grpc-development`) |
| `.grpc` | – | `DotGrpc` | nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION) |
| `.ovfs_latest` | – | `DotOvfsLatest` | implementiert |
| `.ovfs_ls` | – | `DotOvfsLs` | implementiert |
| `.ovfs_status` | – | `DotOvfsStatus` | implementiert |
| `.ovfs_tree` | – | `DotOvfsTree` | implementiert |
| `.ovfs_versions` | – | `DotOvfsVersions` | implementiert |
| `.profile` | `list` | `DotProfileList` | implementiert |
| `.profile` | `create` | `DotProfileCreate` | Entwicklungs-Build (`grpc-development`) |
| `.profile` | `set` | `DotProfileSet` | Entwicklungs-Build (`grpc-development`) |
| `.profile` | `delete` | `DotProfileDelete` | Entwicklungs-Build (`grpc-development`) |
| `.profile` | `assign` | `DotProfileAssign` | Entwicklungs-Build (`grpc-development`) |
| `.profile` | `unassign` | `DotProfileUnassign` | Entwicklungs-Build (`grpc-development`) |
| `.sqlcount` | – | `DotSqlCount` | implementiert |
| `.sqlschema` | – | `DotSqlSchema` | implementiert |
| `.sqltables` | – | `DotSqlTables` | implementiert |
| `.storage` | – | `DotStorages` | implementiert |
| Restore-Baum: `abort` | – | `RestoreTreeAbort` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `add` | – | `RestoreTreeMark` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `cd` | – | `RestoreTreeCd` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `count` | – | `RestoreTreeCount` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `delete` | – | `RestoreTreeUnmark` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `dir` | – | `RestoreTreeLs` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `.dir` | – | `RestoreTreeLs` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `done` | – | `RestoreTreeDone` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `estimate` | – | `RestoreTreeEstimate` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `exit` | – | `RestoreTreeDone` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `find` | – | `RestoreTreeFind` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `help` | – | `RestoreTreeHelp` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `ls` | – | `RestoreTreeLs` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `.ls` | – | `RestoreTreeLs` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `.lsdir` | – | `RestoreTreeLsDir` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `lsmark` | – | `RestoreTreeLsMark` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `.lsmark` | – | `RestoreTreeLsMark` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `mark` | – | `RestoreTreeMark` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `markdir` | – | `RestoreTreeMarkDir` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `pwd` | – | `RestoreTreePwd` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `.pwd` | – | `RestoreTreePwd` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `unmark` | – | `RestoreTreeUnmark` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `unmarkdir` | – | `RestoreTreeUnmarkDir` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `quit` | – | `RestoreTreeAbort` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `.help` | – | `RestoreTreeHelp` | Entwicklungs-Build (`grpc-development`) |
| Restore-Baum: `?` | – | `RestoreTreeHelp` | Entwicklungs-Build (`grpc-development`) |
<!-- coverage:end -->
