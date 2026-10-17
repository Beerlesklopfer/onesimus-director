# Typisierte gRPC-API (`DirectorApi`)

Der Director bietet neben dem Dienst `Director` (`ExecuteCommand` mit Text-/JSON-Ausgabe) den typisierten Dienst `bareos.director.api.DirectorApi` an: je Konsolenbefehl bzw. Unterbefehl eine RPC, deren Antwort eine strukturierte Protobuf-Message ist. Definiert in [`proto/director_api.proto`](https://github.com/Beerlesklopfer/baReos-proto/blob/director-typed-api/director_api.proto) (Repository baReos-proto, Branch `director-typed-api`), implementiert in `crates/onesimus-dird/src/grpc/api.rs` (Stand S-34: alle lesenden RPCs; schreibende RPCs folgen mit S-35).

## Konventionen der Proto-Dateien

Zusammenfassung der bestehenden Dateien (`director_service.proto`, `director_commands.proto`, `bareos_options.proto`) und wie `director_api.proto` ihnen folgt:

| Konvention (bestehend) | `director_api.proto` |
|---|---|
| Kopf: `SPDX-License-Identifier: AGPL-3.0-only`, Zeile `BAREOS® - Backup Archiving REcovery Open Sourced`, Copyright-Zeile, Leerzeile, Kurzbeschreibung | gleich |
| `syntax = "proto3"`, Paketschema `bareos.director[.*]` | Paket `bareos.director.api` |
| Abschnitte mit `// ====…` und Titel, Befehle als `// name — Beschreibung` | gleich |
| Kommentare deutsch, Umlaute als ae/oe/ue (service), Feldkommentare am Zeilenende | gleich |
| Requests sind `*Command`-Messages mit `option (bareos.command_name)`, `help_text`, `usage`, `allowed_in_runscript`, `audit_event`; Felder mit `bare_flag`, `required` | jede neue Request-Message trägt die Optionen; `command_name` enthält das Unterkommando wie bei `ConfigureAddCommand` (`"configure add"`) |
| Request-Feldnamen = Konsolen-Schlüsselwörter (`jobid`, `since_jobid`, `oldvolume`) | gleich; zusätzlich `llist` (Langform) |
| Antworten `*Response`, Datensätze `*Info` (`ServerInfo`, `CommandInfo`) | gleich |
| Ergebnis-Feldnamen snake_case | snake_case = Schlüssel der API-2-JSON-Ausgabe (`jobid`, `volbytes`, `client_connection`) |
| Enums: `<PREFIX>_UNSPECIFIED = 0` (bzw. `RESPONSE_FORMAT_DEFAULT = 0`) | `JOB_STATUS_UNSPECIFIED = 0` usw.; Werte von `JobStatus`/`JobType`/`JobLevel` = ASCII-Code des Katalogzeichens |
| Zeitstempel als `int64` (Unix-Zeit), keine Well-Known-Types im Dienst; `bareos_options.proto` importiert `google/protobuf/descriptor.proto` | Unix-Zeiten `int64`; Katalog-Zeitpunkte als Text (Ortszeit des Directors, ohne Zeitzone gespeichert); `google.protobuf.Struct`/`Value` nur für frei geformte Inhalte (Direktiven von `show`, OVFS-Baum) |
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
- Fehler: Befehl nicht in der CommandACL → `PERMISSION_DENIED`; Befehl fehlgeschlagen → `FAILED_PRECONDITION` mit den Fehlermeldungen des Directors (wie `error.data.messages.error` bei `.api 2`); kein/abgelaufener Token → `UNAUTHENTICATED`; noch nicht implementiert → `UNIMPLEMENTED`.

## Abbildung der API-2-Ausgabe

Die Lese-RPCs führen den Befehl im JSON-Modus aus (wie `.api 2`) und wandeln das `result`-Objekt generisch anhand der Protobuf-Deskriptoren um (`crates/bareos-proto-rs/src/typed.rs`):

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
NONCE=$(grpcurl -insecure $PROTO -d '{"console_name":"admin"}' $DIR bareos.director.Director/GetChallenge | jq -r .nonce)
KEY=$(printf '%s' "$PASSWORD" | md5sum | cut -d' ' -f1)
PROOF=$(printf '%s' "$NONCE" | openssl dgst -sha256 -hmac "$KEY" | cut -d' ' -f2)
TOKEN=$(grpcurl -insecure $PROTO -d "{\"console_name\":\"admin\",\"nonce\":\"$NONCE\",\"proof\":\"$PROOF\"}" \
        $DIR bareos.director.Director/Login | jq -r .sessionToken)

grpcurl -insecure $PROTO -H "x-onesimus-session: $TOKEN" -d '{"limit": 2, "llist": true}' \
        $DIR bareos.director.api.DirectorApi/ListJobs
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
use bareos_proto_rs::bareos::director::api::{director_api_client::DirectorApiClient, ListJobsCommand, JobStatus};

let mut api = DirectorApiClient::new(channel.clone());          // gleicher Kanal wie DirectorClient
let mut req = tonic::Request::new(ListJobsCommand { client: "bareos-fd".into(), limit: 20, ..Default::default() });
req.metadata_mut().insert("x-onesimus-session", token.clone());
for job in api.list_jobs(req).await?.into_inner().jobs {
    let failed = matches!(job.jobstatus(), JobStatus::ErrorTerminated | JobStatus::FatalError);
    println!("{} {} {} bytes failed={failed}", job.jobid, job.name, job.jobbytes);
}
```

## Stand der Implementierung

- **Implementiert (Produktions-Build):** alle lesenden RPCs – Info/Hilfe, `list`/`llist` aller Formen, `status`/`.status`, `show` aller Ressourcentypen, die Punkt-Auflistungen, `.sql*`-Metadaten, `.acl`/`.profile` (Liste), BVFS-Abfragen und OVFS.
- **Definiert, Antwort `UNIMPLEMENTED` (S-35, nur Entwicklungs-Build geplant):** schreibende Befehle (run, rerun, cancel, restore, label, relabel, mount/unmount/release, update, prune, purge, delete, truncate, enable/disable, configure, reload, setdebug, setbandwidth, setdevice, setip, `.sql`, sqlquery, query, `.bvfs_update/restore/cleanup/clear_cache`, `.acl`/`.profile`-Änderungen), Sitzungsbefehle (use, wait, gui, autodisplay, automount, trace, `.api`, `.grpc`, quit/exit/`.quit`) und die Dateiauswahl der interaktiven Wiederherstellung (`StartRestoreSession`, `RestoreTree*`).
- Bis dahin gehen Schreibbefehle weiter über `Director/ExecuteCommand`.

Tests: Abbildungs-Tests in `crates/bareos-proto-rs/src/typed.rs` und `crates/onesimus-dird/src/grpc/api.rs`; Abdeckungstest `crates/onesimus-dird/src/grpc/coverage.rs` (prüft gegen [`commands/bareos-director-commands.json`](commands/bareos-director-commands.json), dass jeder Befehl und jedes Unterkommando eine RPC hat, und hält die Tabellen unten aktuell); Integrationstest `crates/onesimus-dird/tests/grpc_typed_api.rs` (startet den Director mit der Parity-Konfiguration auf einer Wegwerf-Kopie von `bareos_cppref` und vergleicht jede Lese-RPC mit der `.api 2`-Ausgabe derselben Konsolenzeile, auch für eine eingeschränkte Konsole):

```sh
cargo build -p onesimus-dird && cargo build -p bareos-bnet --example bnet_capture
cargo test -p onesimus-dird --test grpc_typed_api -- --ignored --nocapture
```

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
| `Run` | `commands.RunCommand` | `api.ActionResponse` | definiert (S-35) |
| `Rerun` | `commands.RerunCommand` | `api.ActionResponse` | definiert (S-35) |
| `Cancel` | `commands.CancelCommand` | `api.ActionResponse` | definiert (S-35) |
| `Restore` | `commands.RestoreCommand` | `api.ActionResponse` | definiert (S-35) |
| `Label` | `commands.LabelCommand` | `api.ActionResponse` | definiert (S-35) |
| `Relabel` | `commands.RelabelCommand` | `api.ActionResponse` | definiert (S-35) |
| `Mount` | `commands.MountCommand` | `api.ActionResponse` | definiert (S-35) |
| `Unmount` | `commands.UnmountCommand` | `api.ActionResponse` | definiert (S-35) |
| `Release` | `commands.ReleaseCommand` | `api.ActionResponse` | definiert (S-35) |
| `AddVolumes` | `commands.AddCommand` | `api.ActionResponse` | definiert (S-35) |
| `CreatePool` | `commands.CreateCommand` | `api.ActionResponse` | definiert (S-35) |
| `DeleteVolume` | `commands.DeleteCommand` | `api.ActionResponse` | definiert (S-35) |
| `DeletePool` | `commands.DeleteCommand` | `api.ActionResponse` | definiert (S-35) |
| `DeleteJob` | `commands.DeleteCommand` | `api.ActionResponse` | definiert (S-35) |
| `DeleteStorage` | `commands.DeleteCommand` | `api.ActionResponse` | definiert (S-35) |
| `Enable` | `commands.EnableCommand` | `api.ActionResponse` | definiert (S-35) |
| `Disable` | `commands.DisableCommand` | `api.ActionResponse` | definiert (S-35) |
| `ExportVolumes` | `commands.ExportCommand` | `api.ActionResponse` | definiert (S-35) |
| `ImportVolumes` | `commands.ImportCommand` | `api.ActionResponse` | definiert (S-35) |
| `MoveVolumes` | `commands.MoveCommand` | `api.ActionResponse` | definiert (S-35) |
| `PruneFiles` | `commands.PruneCommand` | `api.ActionResponse` | definiert (S-35) |
| `PruneJobs` | `commands.PruneCommand` | `api.ActionResponse` | definiert (S-35) |
| `PruneVolume` | `api.PruneVolumeCommand` | `api.ActionResponse` | definiert (S-35) |
| `PruneStats` | `commands.PruneCommand` | `api.ActionResponse` | definiert (S-35) |
| `PruneDirectory` | `api.PruneDirectoryCommand` | `api.ActionResponse` | definiert (S-35) |
| `PurgeFiles` | `commands.PurgeCommand` | `api.ActionResponse` | definiert (S-35) |
| `PurgeJobs` | `commands.PurgeCommand` | `api.ActionResponse` | definiert (S-35) |
| `PurgeVolume` | `commands.PurgeCommand` | `api.ActionResponse` | definiert (S-35) |
| `PurgeQuota` | `commands.PurgeCommand` | `api.ActionResponse` | definiert (S-35) |
| `UpdateVolume` | `commands.UpdateCommand` | `api.ActionResponse` | definiert (S-35) |
| `UpdatePool` | `commands.UpdateCommand` | `api.ActionResponse` | definiert (S-35) |
| `UpdateSlots` | `api.UpdateSlotsCommand` | `api.ActionResponse` | definiert (S-35) |
| `UpdateJob` | `commands.UpdateCommand` | `api.ActionResponse` | definiert (S-35) |
| `UpdateStats` | `commands.UpdateCommand` | `api.ActionResponse` | definiert (S-35) |
| `Truncate` | `commands.TruncateCommand` | `api.ActionResponse` | definiert (S-35) |
| `ConfigureAdd` | `commands.ConfigureAddCommand` | `api.ActionResponse` | definiert (S-35) |
| `ConfigureExport` | `commands.ConfigureExportCommand` | `api.ActionResponse` | definiert (S-35) |
| `ConfigureDelete` | `commands.ConfigureDeleteCommand` | `api.ActionResponse` | definiert (S-35) |
| `ConfigureUpdate` | `commands.ConfigureUpdateCommand` | `api.ActionResponse` | definiert (S-35) |
| `Reload` | `commands.ReloadCommand` | `api.ActionResponse` | definiert (S-35) |
| `SetDebug` | `commands.SetdebugCommand` | `api.ActionResponse` | definiert (S-35) |
| `SetBandwidth` | `commands.SetbandwidthCommand` | `api.ActionResponse` | definiert (S-35) |
| `SetDevice` | `commands.SetdeviceCommand` | `api.ActionResponse` | definiert (S-35) |
| `SetIp` | `commands.SetipCommand` | `api.ActionResponse` | definiert (S-35) |
| `Trace` | `api.TraceSwitchCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotSql` | `commands.DotSqlCommand` | `api.SqlQueryResponse` | definiert (S-35) |
| `SqlQuery` | `commands.SqlQueryCommand` | `api.SqlQueryResponse` | definiert (S-35) |
| `Query` | `api.QueryRunCommand` | `api.SqlQueryResponse` | definiert (S-35) |
| `DotBvfsUpdate` | `commands.DotBvfsUpdateCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotBvfsRestore` | `commands.DotBvfsRestoreCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotBvfsCleanup` | `commands.DotBvfsCleanupCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotBvfsClearCache` | `commands.DotBvfsClearCacheCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotAclSet` | `commands.DotAclSetCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotAclDelete` | `commands.DotAclDeleteCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotAclClear` | `commands.DotAclClearCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotAclCopy` | `commands.DotAclCopyCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotProfileCreate` | `commands.DotProfileCreateCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotProfileSet` | `commands.DotProfileSetCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotProfileDelete` | `commands.DotProfileDeleteCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotProfileAssign` | `commands.DotProfileAssignCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotProfileUnassign` | `commands.DotProfileUnassignCommand` | `api.ActionResponse` | definiert (S-35) |
| `UseCatalog` | `commands.UseCommand` | `api.ActionResponse` | definiert (S-35) |
| `Wait` | `commands.WaitCommand` | `api.ActionResponse` | definiert (S-35) |
| `Gui` | `api.GuiSwitchCommand` | `api.ActionResponse` | definiert (S-35) |
| `Autodisplay` | `api.AutodisplaySwitchCommand` | `api.ActionResponse` | definiert (S-35) |
| `Automount` | `api.AutomountSwitchCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotApi` | `api.DotApiModeCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotGrpc` | `commands.DotGrpcCommand` | `api.ActionResponse` | definiert (S-35) |
| `Quit` | `commands.QuitCommand` | `api.ActionResponse` | definiert (S-35) |
| `Exit` | `commands.ExitCommand` | `api.ActionResponse` | definiert (S-35) |
| `DotQuit` | `commands.DotQuitCommand` | `api.ActionResponse` | definiert (S-35) |
| `StartRestoreSession` | `commands.RestoreCommand` | `api.RestoreSessionResponse` | definiert (S-35) |
| `RestoreTreeCd` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | definiert (S-35) |
| `RestoreTreeLs` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | definiert (S-35) |
| `RestoreTreeLsDir` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | definiert (S-35) |
| `RestoreTreeLsMark` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | definiert (S-35) |
| `RestoreTreePwd` | `api.RestoreTreeSessionCommand` | `api.RestoreTreeListResponse` | definiert (S-35) |
| `RestoreTreeMark` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | definiert (S-35) |
| `RestoreTreeUnmark` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | definiert (S-35) |
| `RestoreTreeMarkDir` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | definiert (S-35) |
| `RestoreTreeUnmarkDir` | `api.RestoreTreePathCommand` | `api.RestoreTreeCountResponse` | definiert (S-35) |
| `RestoreTreeCount` | `api.RestoreTreeSessionCommand` | `api.RestoreTreeCountResponse` | definiert (S-35) |
| `RestoreTreeFind` | `api.RestoreTreePathCommand` | `api.RestoreTreeListResponse` | definiert (S-35) |
| `RestoreTreeEstimate` | `api.RestoreTreeSessionCommand` | `api.RestoreTreeCountResponse` | definiert (S-35) |
| `RestoreTreeHelp` | `api.RestoreTreeSessionCommand` | `api.HelpResponse` | definiert (S-35) |
| `RestoreTreeDone` | `api.RestoreTreeSessionCommand` | `api.ActionResponse` | definiert (S-35) |
| `RestoreTreeAbort` | `api.RestoreTreeSessionCommand` | `api.ActionResponse` | definiert (S-35) |
<!-- rpcs:end -->

## Abdeckung: Befehl → RPC

Unterbefehl `key=` steht für die Form `key=<wert>`, ein Wort für das Schlüsselwort; `–` für den Befehl selbst.

<!-- coverage:begin (generiert, crates/onesimus-dird/src/grpc/coverage.rs) -->
| Befehl | Unterbefehl | RPC | Stand |
|---|---|---|---|
| `.` | – | `Noop` | implementiert |
| `.actiononpurge` | – | `DotActiononpurge` | implementiert |
| `.api` | – | `DotApi` | definiert (S-35) |
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
| `.quit` | – | `DotQuit` | definiert (S-35) |
| `.sql` | – | `DotSql` | definiert (S-35) |
| `.schedule` | – | `DotSchedule` | implementiert |
| `.status` | `dir` | `DotStatusDirector` | implementiert |
| `.status` | `storage=` | `DotStatusStorage` | implementiert |
| `.status` | `client=` | `DotStatusClient` | implementiert |
| `.storages` | – | `DotStorages` | implementiert |
| `.types` | – | `DotTypes` | implementiert |
| `.volstatus` | – | `DotVolstatus` | implementiert |
| `.bvfs_lsdirs` | – | `DotBvfsLsdirs` | implementiert |
| `.bvfs_lsfiles` | – | `DotBvfsLsfiles` | implementiert |
| `.bvfs_update` | – | `DotBvfsUpdate` | definiert (S-35) |
| `.bvfs_get_jobids` | – | `DotBvfsGetJobids` | implementiert |
| `.bvfs_versions` | – | `DotBvfsVersions` | implementiert |
| `.bvfs_restore` | – | `DotBvfsRestore` | definiert (S-35) |
| `.bvfs_cleanup` | – | `DotBvfsCleanup` | definiert (S-35) |
| `.bvfs_clear_cache` | – | `DotBvfsClearCache` | definiert (S-35) |
| `add` | – | `AddVolumes` | definiert (S-35) |
| `autodisplay` | – | `Autodisplay` | definiert (S-35) |
| `automount` | – | `Automount` | definiert (S-35) |
| `cancel` | – | `Cancel` | definiert (S-35) |
| `configure` | `add client` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add jobdefs` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add job` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add storage` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add catalog` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add schedule` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add fileset` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add pool` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add messages` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add counter` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add profile` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add console` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `add user` | `ConfigureAdd` | definiert (S-35) |
| `configure` | `export` | `ConfigureExport` | definiert (S-35) |
| `configure` | `delete` | `ConfigureDelete` | definiert (S-35) |
| `configure` | `update` | `ConfigureUpdate` | definiert (S-35) |
| `create` | – | `CreatePool` | definiert (S-35) |
| `delete` | `volume=` | `DeleteVolume` | definiert (S-35) |
| `delete` | `volume` | `DeleteVolume` | definiert (S-35) |
| `delete` | `pool=` | `DeletePool` | definiert (S-35) |
| `delete` | `jobid=` | `DeleteJob` | definiert (S-35) |
| `delete` | `storage` | `DeleteStorage` | definiert (S-35) |
| `disable` | – | `Disable` | definiert (S-35) |
| `enable` | – | `Enable` | definiert (S-35) |
| `estimate` | – | `Estimate` | implementiert |
| `exit` | – | `Exit` | definiert (S-35) |
| `export` | – | `ExportVolumes` | definiert (S-35) |
| `gui` | – | `Gui` | definiert (S-35) |
| `help` | – | `Help` | implementiert |
| `import` | – | `ImportVolumes` | definiert (S-35) |
| `label` | – | `Label` | definiert (S-35) |
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
| `mount` | – | `Mount` | definiert (S-35) |
| `move` | – | `MoveVolumes` | definiert (S-35) |
| `prune` | `files` | `PruneFiles` | definiert (S-35) |
| `prune` | `jobs` | `PruneJobs` | definiert (S-35) |
| `prune` | `volume` | `PruneVolume` | definiert (S-35) |
| `prune` | `stats` | `PruneStats` | definiert (S-35) |
| `prune` | `directory` | `PruneDirectory` | definiert (S-35) |
| `purge` | `files` | `PurgeFiles` | definiert (S-35) |
| `purge` | `jobs` | `PurgeJobs` | definiert (S-35) |
| `purge` | `volume` | `PurgeVolume` | definiert (S-35) |
| `purge` | `quota` | `PurgeQuota` | definiert (S-35) |
| `quit` | – | `Quit` | definiert (S-35) |
| `query` | – | `Query` | definiert (S-35) |
| `restore` | – | `Restore` | definiert (S-35) |
| `restore` | `interaktive Dateiauswahl` | `StartRestoreSession` | definiert (S-35) |
| `relabel` | – | `Relabel` | definiert (S-35) |
| `release` | – | `Release` | definiert (S-35) |
| `reload` | – | `Reload` | definiert (S-35) |
| `rerun` | – | `Rerun` | definiert (S-35) |
| `resolve` | – | `Resolve` | implementiert |
| `run` | – | `Run` | definiert (S-35) |
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
| `setbandwidth` | – | `SetBandwidth` | definiert (S-35) |
| `setdebug` | – | `SetDebug` | definiert (S-35) |
| `setdevice` | – | `SetDevice` | definiert (S-35) |
| `setip` | – | `SetIp` | definiert (S-35) |
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
| `sqlquery` | – | `SqlQuery` | definiert (S-35) |
| `time` | – | `GetTime` | implementiert |
| `trace` | – | `Trace` | definiert (S-35) |
| `truncate` | – | `Truncate` | definiert (S-35) |
| `unmount` | – | `Unmount` | definiert (S-35) |
| `umount` | – | `Unmount` | definiert (S-35) |
| `update` | `volume=` | `UpdateVolume` | definiert (S-35) |
| `update` | `pool=` | `UpdatePool` | definiert (S-35) |
| `update` | `slots` | `UpdateSlots` | definiert (S-35) |
| `update` | `jobid=` | `UpdateJob` | definiert (S-35) |
| `update` | `stats` | `UpdateStats` | definiert (S-35) |
| `use` | – | `UseCatalog` | definiert (S-35) |
| `var` | – | `ExpandVariables` | implementiert |
| `version` | – | `GetVersion` | implementiert |
| `wait` | – | `Wait` | definiert (S-35) |
| `whoami` | – | `Whoami` | implementiert |
| `.acl` | `list` | `DotAclList` | implementiert |
| `.acl` | `set` | `DotAclSet` | definiert (S-35) |
| `.acl` | `delete` | `DotAclDelete` | definiert (S-35) |
| `.acl` | `clear` | `DotAclClear` | definiert (S-35) |
| `.acl` | `copy` | `DotAclCopy` | definiert (S-35) |
| `.grpc` | – | `DotGrpc` | definiert (S-35) |
| `.ovfs_latest` | – | `DotOvfsLatest` | implementiert |
| `.ovfs_ls` | – | `DotOvfsLs` | implementiert |
| `.ovfs_status` | – | `DotOvfsStatus` | implementiert |
| `.ovfs_tree` | – | `DotOvfsTree` | implementiert |
| `.ovfs_versions` | – | `DotOvfsVersions` | implementiert |
| `.profile` | `list` | `DotProfileList` | implementiert |
| `.profile` | `create` | `DotProfileCreate` | definiert (S-35) |
| `.profile` | `set` | `DotProfileSet` | definiert (S-35) |
| `.profile` | `delete` | `DotProfileDelete` | definiert (S-35) |
| `.profile` | `assign` | `DotProfileAssign` | definiert (S-35) |
| `.profile` | `unassign` | `DotProfileUnassign` | definiert (S-35) |
| `.sqlcount` | – | `DotSqlCount` | implementiert |
| `.sqlschema` | – | `DotSqlSchema` | implementiert |
| `.sqltables` | – | `DotSqlTables` | implementiert |
| `.storage` | – | `DotStorages` | implementiert |
| Restore-Baum: `abort` | – | `RestoreTreeAbort` | definiert (S-35) |
| Restore-Baum: `add` | – | `RestoreTreeMark` | definiert (S-35) |
| Restore-Baum: `cd` | – | `RestoreTreeCd` | definiert (S-35) |
| Restore-Baum: `count` | – | `RestoreTreeCount` | definiert (S-35) |
| Restore-Baum: `delete` | – | `RestoreTreeUnmark` | definiert (S-35) |
| Restore-Baum: `dir` | – | `RestoreTreeLs` | definiert (S-35) |
| Restore-Baum: `.dir` | – | `RestoreTreeLs` | definiert (S-35) |
| Restore-Baum: `done` | – | `RestoreTreeDone` | definiert (S-35) |
| Restore-Baum: `estimate` | – | `RestoreTreeEstimate` | definiert (S-35) |
| Restore-Baum: `exit` | – | `RestoreTreeDone` | definiert (S-35) |
| Restore-Baum: `find` | – | `RestoreTreeFind` | definiert (S-35) |
| Restore-Baum: `help` | – | `RestoreTreeHelp` | definiert (S-35) |
| Restore-Baum: `ls` | – | `RestoreTreeLs` | definiert (S-35) |
| Restore-Baum: `.ls` | – | `RestoreTreeLs` | definiert (S-35) |
| Restore-Baum: `.lsdir` | – | `RestoreTreeLsDir` | definiert (S-35) |
| Restore-Baum: `lsmark` | – | `RestoreTreeLsMark` | definiert (S-35) |
| Restore-Baum: `.lsmark` | – | `RestoreTreeLsMark` | definiert (S-35) |
| Restore-Baum: `mark` | – | `RestoreTreeMark` | definiert (S-35) |
| Restore-Baum: `markdir` | – | `RestoreTreeMarkDir` | definiert (S-35) |
| Restore-Baum: `pwd` | – | `RestoreTreePwd` | definiert (S-35) |
| Restore-Baum: `.pwd` | – | `RestoreTreePwd` | definiert (S-35) |
| Restore-Baum: `unmark` | – | `RestoreTreeUnmark` | definiert (S-35) |
| Restore-Baum: `unmarkdir` | – | `RestoreTreeUnmarkDir` | definiert (S-35) |
| Restore-Baum: `quit` | – | `RestoreTreeAbort` | definiert (S-35) |
| Restore-Baum: `.help` | – | `RestoreTreeHelp` | definiert (S-35) |
| Restore-Baum: `?` | – | `RestoreTreeHelp` | definiert (S-35) |
<!-- coverage:end -->
