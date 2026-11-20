# Planung: Ressourcen lesen, anlegen, ändern, löschen über gRPC (CRUD)

Stand: 19.11.2026. Status: **Mit der GUI abgestimmt (19.11.2026); erste Ausbaustufe (Schedule, Job, JobDefs), Client und FileSet umgesetzt.**

Auftrag des Nutzers: volles CRUD für Ressourcen der Director-Konfiguration,
zuerst für Zeitpläne („Weekly Cycle kein CRUD möglich“, „ein drag führt zu einem
neuen schedule und nicht zu einem update“). Entscheidung des Nutzers: „gRPC
ermöglicht es uns, smart vom Bareos-Standard abzuweichen“. Die Erweiterung liegt
deshalb im typisierten DirectorApi (`proto/director_api.proto`); der
Konsolen-Befehlssatz bleibt beim Bareos-Standard (`configure add`, `configure
export`). `bareos-dir` kennt weder diese RPCs noch Ändern/Löschen; eine GUI
erkennt den Umfang über `GetConfigCapabilities` und fällt ohne diese RPC auf
„nur add“ zurück.

## Grundsätze

1. **Rohtext statt Felder:** Gelesen und geschrieben wird der Text einer
   Ressource, wie er in der Datei steht (Bareos-Konfigurationssyntax). Damit
   bleiben Kommentare, Reihenfolge, Formatierung und Direktiven, die die GUI
   nicht kennt, Zeichen für Zeichen erhalten. Der Text kommt als UTF-8-`string`
   über gRPC, ohne Quoting- oder base64-Umweg.
2. **Optimistische Sperre:** Jede Antwort trägt den SHA-256 des gelieferten
   Textes. Ändern und Löschen verlangen den SHA-256 des Standes, auf dem der
   Client aufbaut; weicht der aktuelle ab, antwortet der Director mit
   `CONFLICT` und dem aktuellen Text, ohne etwas zu ändern.
3. **Prüfen vor Schreiben:** Der Director setzt die neue Gesamtkonfiguration im
   Speicher zusammen und prüft sie wie `odir -t` (Parser, Verweise,
   Pflichtdirektiven, Adressen). Nur wenn sie gültig ist, wird geschrieben.
   Fehler kommen als `INVALID` mit Datei, Zeile und Spalte.
4. **Atomar und nur diese Ressource:** Geschrieben wird über eine temporäre
   Datei und `rename`. Steht die Ressource in einer Datei mit weiteren
   Ressourcen (`shared_file`), wird nur ihr Bereich ersetzt bzw. entfernt.
5. **Danach neu laden:** Nach dem Schreiben folgt ein Reload wie bei
   `reload`, damit Scheduler und Jobs die neue Konfiguration nutzen (J-9).
   Schlägt der Reload fehl, wird die alte Datei zurückgeschrieben.
6. **Referenzen:** Löschen verweigert mit `REFERENCED`, solange andere
   Ressourcen die Ressource nennen (z. B. Jobs einen Zeitplan); die Antwort
   nennt sie. Kein force.
7. **Rechte:** Command-ACL `configure` (sonst gRPC `PERMISSION_DENIED`), dazu
   die Ressourcen-ACL des Typs (Job-, Client-, Pool-, Schedule-, FileSet-,
   Storage-, Catalog-ACL wie bei `show`); verweigert ergibt `DENIED`.
8. **Neue Dateien:** `CreateResource` schreibt
   `bareos-dir.d/<typ>/<Name>.conf` (wie `configure add`) und verweigert mit
   `EXISTS`, wenn der Name schon vergeben ist.
9. **Umbenennen** kommt später (mit Anpassung aller Verweise); bis dahin fehlt
   `RENAME` in den Fähigkeiten.
10. **Prüfen ohne Schreiben:** `validate_only` in Create, Update und Delete
    liefert dieselbe Antwort wie der echte Aufruf (status, problem,
    referenced_by), schreibt aber nichts und lädt nicht neu.
11. **Konflikt mit aktuellem Stand:** `CONFLICT` enthält immer `text` und
    `sha256` des aktuellen Stands, damit der Client den Unterschied zeigen kann,
    ohne neu zu lesen.
12. **Exakte Bytes:** `text` ist der Bereich der Ressource genau so, wie er in
    der Datei steht, vom ersten Zeichen des Ressourcennamens (`Schedule`) bis
    einschließlich der schließenden Klammer und des folgenden Zeilenendes,
    mit unveränderten Zeilenenden (CRLF bleibt CRLF) und ohne Normalisierung.
    `sha256` wird über genau diese Bytes gebildet. Geschriebener Text wird
    unverändert übernommen.
13. **Fähigkeiten je Sitzung:** `GetConfigCapabilities` spiegelt die ACLs der
    angemeldeten Konsole: Ohne Command-ACL `configure` keine schreibenden
    Operationen; Typen ohne erlaubte Ressource erscheinen nicht.
14. **Protokoll:** Jede schreibende Operation (nicht `validate_only`) geht ins
    Journal und als Director-Meldung (Messages-Ressource des Directors):
    Konsole, Operation, Typ, Name, alter und neuer SHA-256, Datei.
15. **Was der Reload nicht angleicht:** Der Reload schreibt wie beim Start
    (`CheckCatalog`, C-066) die Pool-Datensätze des Katalogs aus der
    Konfiguration neu (`check_catalog::ensure_basic_records`, aufgerufen in
    `reload.rs`). Die Volumes eines Pools behalten ihre Werte (Retention,
    Recycle, Max Volume Bytes …), bis `update volume … frompool` bzw.
    `update pool from resource` läuft. Die Antwort nennt solche Nacharbeiten in
    `followups` (z. B. „update volume pool=Full allfrompool“).
16. **Übersicht:** `ListResources(type)` liefert je Ressource Name, Datei und
    SHA-256, damit ein Client veraltete Stände ohne einzelne Reads erkennt.
17. **Reihenfolge der Typen:** Schedule, dann Job und JobDefs, dann die übrigen.
    `GetConfigCapabilities` nennt je Typ, was schon geht.

## proto (Ergänzung von `proto/director_api.proto`)

```proto
service DirectorApi {
  // ... bestehende RPCs ...

  // --- Konfiguration: Ressourcen lesen, anlegen, ändern, löschen ----------
  // Onesimus-Erweiterung, kein Gegenstück in bareos-dir.
  rpc GetConfigCapabilities (ConfigCapabilitiesRequest) returns (ConfigCapabilitiesResponse);
  rpc ReadResource (ReadResourceRequest) returns (ResourceResponse);
  rpc CreateResource (CreateResourceRequest) returns (ResourceResponse);
  rpc UpdateResource (UpdateResourceRequest) returns (ResourceResponse);
  rpc DeleteResource (DeleteResourceRequest) returns (ResourceResponse);
  rpc RenameResource (RenameResourceRequest) returns (ResourceResponse);  // später; bis dahin UNIMPLEMENTED
  rpc ListResources (ListResourcesRequest) returns (ListResourcesResponse);
}

enum ResourceType {
  RESOURCE_TYPE_UNSPECIFIED = 0;
  RESOURCE_TYPE_SCHEDULE = 1;
  RESOURCE_TYPE_JOB = 2;
  RESOURCE_TYPE_JOBDEFS = 3;
  RESOURCE_TYPE_CLIENT = 4;
  RESOURCE_TYPE_FILESET = 5;
  RESOURCE_TYPE_POOL = 6;
  RESOURCE_TYPE_STORAGE = 7;
  RESOURCE_TYPE_MESSAGES = 8;
  RESOURCE_TYPE_CATALOG = 9;
  RESOURCE_TYPE_CONSOLE = 10;
  RESOURCE_TYPE_PROFILE = 11;
  RESOURCE_TYPE_COUNTER = 12;
  RESOURCE_TYPE_USER = 13;
  RESOURCE_TYPE_DIRECTOR = 14;
}

enum ConfigOperation {
  CONFIG_OPERATION_UNSPECIFIED = 0;
  CONFIG_OPERATION_READ_RAW = 1;
  CONFIG_OPERATION_CREATE_RAW = 2;
  CONFIG_OPERATION_UPDATE_RAW = 3;
  CONFIG_OPERATION_DELETE = 4;
  CONFIG_OPERATION_RENAME = 5;
}

enum ConfigStatus {
  CONFIG_STATUS_UNSPECIFIED = 0;
  CONFIG_STATUS_OK = 1;
  CONFIG_STATUS_CONFLICT = 2;    // expected_sha256 passt nicht; text/sha256 = aktueller Stand
  CONFIG_STATUS_INVALID = 3;     // neue Konfiguration ungültig; problem gesetzt, nichts geändert
  CONFIG_STATUS_REFERENCED = 4;  // Löschen verweigert; referenced_by gesetzt
  CONFIG_STATUS_DENIED = 5;      // Ressourcen-ACL
  CONFIG_STATUS_NOT_FOUND = 6;
  CONFIG_STATUS_EXISTS = 7;      // Create: Name vergeben
  CONFIG_STATUS_RELOAD_FAILED = 8; // geschrieben, Reload scheiterte, alter Stand wiederhergestellt
}

message ResourceRef {
  ResourceType type = 1;
  string name = 2;
}

message ConfigCapabilitiesRequest {
  option (onesimus.command_name) = "configure capabilities";
}

message TypeCapabilities {
  ResourceType type = 1;
  repeated ConfigOperation operations = 2;
}

message ConfigCapabilitiesResponse {
  uint32 interface_version = 1;        // 1 = diese Fassung
  repeated TypeCapabilities types = 2;
}

message ReadResourceRequest {
  option (onesimus.command_name) = "configure read";
  ResourceRef resource = 1;
}

message CreateResourceRequest {
  option (onesimus.command_name) = "configure create";
  ResourceType type = 1;
  string text = 2;                     // vollständige Ressource "Schedule { Name = ...; ... }"
  bool validate_only = 3;              // nur prüfen, nichts schreiben, kein Reload
}

message UpdateResourceRequest {
  option (onesimus.command_name) = "configure update";
  ResourceRef resource = 1;
  string expected_sha256 = 2;          // SHA-256 (hex) des Textes, auf dem die Änderung beruht
  string text = 3;                     // neuer vollständiger Text; Name muss resource.name sein
  bool validate_only = 4;
}

message DeleteResourceRequest {
  option (onesimus.command_name) = "configure delete";
  ResourceRef resource = 1;
  string expected_sha256 = 2;
  bool validate_only = 3;
}

message ListResourcesRequest {
  option (onesimus.command_name) = "configure list";
  ResourceType type = 1;
}

message ResourceSummary {
  ResourceRef resource = 1;
  string file = 2;
  string sha256 = 3;
  bool shared_file = 4;
}

message ListResourcesResponse {
  repeated ResourceSummary resources = 1;
}

message RenameResourceRequest {
  option (onesimus.command_name) = "configure rename";
  ResourceRef resource = 1;
  string new_name = 2;
  string expected_sha256 = 3;
}

message ConfigProblem {
  string file = 1;
  uint32 line = 2;
  uint32 column = 3;
  string text = 4;                     // Meldung wie bei odir -t
}

message ResourceResponse {
  ConfigStatus status = 1;
  string message = 2;                  // lesbare Meldung
  ResourceRef resource = 3;
  string file = 4;                     // Pfad der Datei
  string text = 5;                     // Rohtext der Ressource (aktueller Stand)
  string sha256 = 6;                   // SHA-256 (hex) von text
  bool shared_file = 7;                // Datei enthält weitere Ressourcen
  repeated ResourceRef referenced_by = 8;
  ConfigProblem problem = 9;           // bei INVALID
  bool reloaded = 10;                  // Konfiguration neu geladen
  bool validated_only = 11;            // Antwort auf validate_only, nichts geschrieben
  string old_sha256 = 12;              // Stand vor der Änderung (schreibende Operationen)
  repeated string followups = 13;      // was der Reload nicht angleicht, als Konsolenbefehle
}
```

Beispiel `GetConfigCapabilities` (interface_version 1, erste Ausbaustufe):

```json
{ "interface_version": 1,
  "types": [
    { "type": "RESOURCE_TYPE_SCHEDULE", "operations": ["CONFIG_OPERATION_READ_RAW", "CONFIG_OPERATION_CREATE_RAW", "CONFIG_OPERATION_UPDATE_RAW", "CONFIG_OPERATION_DELETE"] },
    { "type": "RESOURCE_TYPE_JOB",      "operations": ["CONFIG_OPERATION_READ_RAW", "CONFIG_OPERATION_CREATE_RAW", "CONFIG_OPERATION_UPDATE_RAW", "CONFIG_OPERATION_DELETE"] },
    { "type": "RESOURCE_TYPE_JOBDEFS",  "operations": ["CONFIG_OPERATION_READ_RAW", "CONFIG_OPERATION_CREATE_RAW", "CONFIG_OPERATION_UPDATE_RAW", "CONFIG_OPERATION_DELETE"] } ] }
```

Beispiel `UpdateResource` mit Konflikt:

```json
{ "status": "CONFIG_STATUS_CONFLICT",
  "message": "Schedule \"WeeklyCycle\" changed since it was read",
  "resource": { "type": "RESOURCE_TYPE_SCHEDULE", "name": "WeeklyCycle" },
  "file": "/etc/bareos/bareos-dir.d/schedule/WeeklyCycle.conf",
  "text": "Schedule {\n  Name = \"WeeklyCycle\"\n  Run = Full 1st sat at 21:00\n  ...\n}\n",
  "sha256": "9f2c…", "shared_file": false }
```

Beispiel `DeleteResource`, noch verwendet:

```json
{ "status": "CONFIG_STATUS_REFERENCED",
  "message": "Schedule \"WeeklyCycle\" is used by 9 resources",
  "resource": { "type": "RESOURCE_TYPE_SCHEDULE", "name": "WeeklyCycle" },
  "referenced_by": [ { "type": "RESOURCE_TYPE_JOB", "name": "forgejo" }, { "type": "RESOURCE_TYPE_JOB", "name": "BackupMail" } ] }
```

## Bestehende Onesimus-Erweiterungen der Konsole

`configure update`, `configure delete` und `configure read`
(Zusammenfassung) der Konsole bleiben für ältere Clients bestehen, werden aber
in `GetConfigCapabilities` nicht genannt und von der GUI nicht benutzt.
`configure delete` (19.11.2026) und `configure update` (20.11.2026) laufen
inzwischen durch das CRUD (K-10 behoben, siehe unten). Ob `configure read`
entfernt wird, entscheidet der Nutzer.

## Umsetzung (nach Abstimmung)

1. proto: Ergänzung auf einem neuen Zweig `config-crud` des proto-Submoduls
   (ab `plugin-phase1`; seit 20.11.2026 liegt `config-crud` direkt auf `main`,
   `plugin-phase1` ist in `filed-plugin` und `postgresql` aufgeteilt, alles
   zusammen im Integrationszweig `development` von onesimus-proto).
2. odir: Modul `config_edit` (Bereich einer Ressource in der Datei finden,
   Ersetzen/Entfernen, Prüfen der neuen Gesamtkonfiguration mit derselben
   Logik wie `-t`, atomares Schreiben, Reload, Referenzsuche), dann die RPCs
   in `grpc/api*.rs`.
3. Tests: Schedule anlegen/lesen/ändern/löschen; Konflikt; ungültiger Text
   (Zeile/Spalte); Verweis auf einen gelöschten Zeitplan verhindert; geteilte
   Datei mit zwei Ressourcen; Kommentare und unbekannte Direktiven bleiben
   erhalten; Reload-Fehler stellt den alten Stand wieder her; ACL.

### Stand der Umsetzung (19.11.2026, erste Ausbaustufe)

Umgesetzt: `GetConfigCapabilities`, `ReadResource`, `CreateResource`,
`UpdateResource`, `DeleteResource`, `ListResources` für Schedule, Job und
JobDefs (READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE); `RenameResource` und die
übrigen Typen antworten `UNIMPLEMENTED`. proto-Submodul auf `config-crud`
(3b2b468).

- Code: `crates/odir/src/config_edit.rs` (Bereich, Prüfung, Schreiben,
  Reload, Referenzen, Sperre), `crates/odir/src/grpc/api_config.rs` (RPCs,
  ACL, Capabilities); `config_test` liest Dateien jetzt über einen Leser
  (Datei oder geänderte Kopie im Speicher), `DirectorConfig::from_files`,
  `bareos_config::parser::parse_config_named`, `acl::any_allowed`.
- Prüfung wie `odir -t` ohne Katalog: Parser, `config_check::check`
  (CheckResources, Verweise, Pflichtdirektiven), Listen-Adressen.
  `problem.text` ist die Ausgabe von `-t`; Zeile/Spalte, wo `-t` sie kennt
  (scan_err); bei Fehlern des typisierten Parsers nur die Zeile (Spalte 0).
- Reload über `reload::reload_now` (derselbe Weg wie `reload`, J-9).
- Protokoll: `tracing::info` und Director-Meldung (Typ Info) über die
  Messages-Ressource des Directors (`audit::daemon_message`); bei
  RELOAD_FAILED als Warnung mit dem Ergebnis der Wiederherstellung.
- `followups` bleibt leer (Kommentar mit Verweis auf Grundsatz 15).
- Tests: `config_edit/tests.rs` (14), `grpc/api_config.rs` (2), dazu ein
  Block im Integrationstest `tests/grpc_typed_api.rs` (Rohtext-CRUD eines
  Zeitplans gegen den echten Director mit Reload und Katalog).

Entscheidungen, die der Plan offen ließ (konservativ):

1. Die Konfigurations-RPCs gibt es in jedem Build (nicht nur mit
   `grpc-development` wie die S-35-Schreib-RPCs): sie haben eigene
   Schutzmechanismen (Sperre, `-t`-Prüfung, Rückfall) und die GUI braucht sie
   im Paket. Leicht umzustellen, falls nicht gewünscht.
2. Auch `ReadResource` und `ListResources` verlangen die Command-ACL
   `configure` (der Rohtext ist nicht geschwärzt wie `show` ohne
   `configure`). `GetConfigCapabilities` braucht nur die Sitzung und nennt
   ohne `configure` keine Typen; ein Typ erscheint, wenn seine ACL
   (Konsole, dann Profile) mindestens einen positiven Eintrag hat.
3. Der Bereich schließt Leerzeichen und einen Kommentar hinter der
   schließenden Klammer auf derselben Zeile ein; Kommentare vor dem Typwort
   gehören zur Datei, nicht zur Ressource.
4. Der übergebene Text muss genau eine Ressource des Typs sein, die den
   ganzen Text ausfüllt; fehlt nur das letzte Zeilenende, wird es im Stil
   der Datei (LF/CRLF) ergänzt. Leerer bzw. falscher `expected_sha256` →
   CONFLICT.
5. Löschen in einer geteilten Datei entfernt nur den Bereich; bleibt nur
   Leerraum/Kommentar, wird die Datei entfernt (sonst blockiert ein
   verwaister `<Name>.conf` ein späteres Create desselben Namens, und der
   Kommentar beschreibt eine Ressource, die es nicht mehr gibt).
6. Referenzen: generisch über die Direktiventabellen aller Ressourcentypen
   (`Schedule =`, `JobDefs =`, `Job To Verify =`); `run job=<name>` in
   RunScript-`Console` und Job-`Run` nur bei wörtlichem Namen (keine `%`/`$`
   Variablen). Nicht gezählt: ACL-Einträge (Muster, keine Verweise) und
   Selection Pattern von Copy/Migrate (reguläre Ausdrücke).
7. Create nur im Verzeichnis-Layout (`bareos-dir.d`); bei einer einzelnen
   Konfigurationsdatei INVALID mit Hinweis. Neue Dateien: Rechte/Eigentümer
   der ersten `.conf` im Typverzeichnis, sonst der Nachbarverzeichnisse,
   sonst 0640 und Eigentümer des Verzeichnisses; fehlt das Typverzeichnis,
   wird es angelegt.
8. Namen: zusätzlich zu `/`, `..` und Steuerzeichen auch `\` und ein
   führender Punkt abgelehnt; außerdem die Bareos-Namensregeln
   (`IsNameValid`).
9. `@include` wird wie im Rohparser übersprungen (nicht verfolgt).

### Client (19.11.2026, zweite Ausbaustufe)

Umgesetzt: Client mit READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE und
`ListResources`; `GetConfigCapabilities` nennt ihn, wenn die Client-ACL der
Konsole (oder ihres Profils) einen positiven Eintrag hat. Rechte pro Name über
die Client-ACL wie `show client`.

- Referenzen: `Client =` in Job und JobDefs (einziges Item mit `R_CLIENT` in
  den Direktiventabellen; ein `Restore Client` gibt es in Bareos 25 nicht),
  dazu `client=<name>` in Job-`Run`-Werten und in `run`-Befehlen von
  RunScript-`Console`, nur bei wörtlichem Namen (konservativ: lieber
  REFERENCED als ein verwaister Verweis). Nicht gezählt: Client-ACL-Einträge
  (Muster).
- Der Rohtext bleibt Byte für Byte erhalten (TLS-Direktiven, Kommentare,
  `;`, CRLF); Tests in `config_edit/tests.rs` (`client_*`,
  `a_job_using_the_client_prevents_the_delete`) und `grpc/api_config.rs`.
- `followups` bleibt leer: der Katalog-Datensatz des Clients übernimmt
  AutoPrune und Retentions wie in Bareos beim nächsten Job des Clients.
- Hinweis: `referenced_by` nennt auch Ressourcen, die die ACL der Konsole
  nicht zeigt (wie bei Schedule); die Löschung wird dadurch nur verhindert.

### BNET-Weg (19.11.2026)

Entscheidung des Nutzers: „Das CRUD läuft auch über BNET.“ oconsole und der
MCP-Server sprechen den Director auch über die klassische Konsole, wenn gRPC
nicht verfügbar ist. Dafür gibt es den Onesimus-Punktbefehl `.config`
(`crates/odir/src/commands/config_crud.rs`, Doku
[dot-config.md](../wiki/commands/dot-config.md)):

```text
.config capabilities
.config list   type=<typ>
.config read   type=<typ> name=<name>
.config create type=<typ> text64=<base64> [validate_only]
.config update type=<typ> name=<name> sha256=<alt> text64=<base64> [validate_only]
.config delete type=<typ> name=<name> sha256=<alt> [validate_only]
```

- Name: `.config` kollidiert mit keinem Bareos-Befehl (kein Eintrag der
  Bareos-Befehlstabelle beginnt mit `.config`, die Präfixauflösung bleibt
  eindeutig; `configure` unverändert). Test
  `the_command_is_registered_without_a_bareos_name_clash`.
- Dieselbe Kernlogik und dieselbe Editor-Instanz wie die RPCs
  (`config_edit::shared_editor`, eine Sperre für beide Wege).
- Text als Base64 (`text64=`), weil eine Konsolenzeile keine Zeilenenden
  trägt; die Antwort trägt ihn als UTF-8 im JSON.
- Antwort im `.api 2`-JSON mit den Feldern von `ResourceResponse`,
  `ListResourcesResponse` bzw. `ConfigCapabilitiesResponse`; `status`, Typen
  und Operationen als Text (`OK`, `CONFLICT`, …; `CLIENT`; `READ_RAW`).
- Rechte (konservativ): Command-ACL `.config` wie jeder Befehl **und**
  `configure` wie die RPCs; Ressourcen-ACL je Name wie beim gRPC-Weg.
  Protokoll wie beim gRPC-Weg; im Audit-Protokoll der Befehlszeile wird
  `text64=` durch die Länge ersetzt (Passwörter im Text).

### FileSet und `configure delete` über das CRUD (19.11.2026)

Anlass: Beim Entfernen von BgLage auf PDC hat das alte `configure delete`
(MCP-Werkzeug `configure_delete`) Job, Client und Pool ohne jede
Verweisprüfung aus dem Speicher genommen und die Datei gelöscht; das FileSet
konnte es nicht löschen.

- FileSet mit READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE und `ListResources`;
  Rechte über die FileSet-ACL wie `show fileset`. Referenzen: `FileSet =` in
  Job und JobDefs (`R_FILESET`) und `fileset=<name>` in Job-`Run`-Werten und
  RunScript-`run`-Befehlen bei wörtlichem Namen. Test
  `fileset_create_read_update_delete_and_referenced`.
- `configure delete <typ>=<name> [yes]` (Konsole, `ConfigureDelete` über
  gRPC, damit auch das MCP-Werkzeug) geht jetzt durch denselben Editor: Lesen
  für die aktuelle SHA-256, dann DeleteResource mit dieser Prüfsumme. Damit
  gelten REFERENCED (es wird nichts gelöscht, die Verweise werden genannt),
  die `-t`-Prüfung der Restkonfiguration, atomares Schreiben und Reload mit
  Rücknahme. Typen: client, job, jobdefs, schedule, fileset. Pool, Storage
  und Console werden abgelehnt („cannot be deleted safely yet“), bis sie im
  CRUD sind. Ohne Konfigurationspfad lehnt der Befehl ab, statt nur die
  laufende Konfiguration zu ändern. Test
  `configure_delete_goes_through_the_crud`.

### `configure update` über das CRUD (20.11.2026)

Anlass: Eine GUI-Sitzung muss auf der Produktion Schedule-Ressourcen ändern
(Description, Run-Zeilen). Löschen und neu Anlegen geht nicht (die Jobs
verweisen auf den Schedule, REFERENCED), und das alte `configure update`
(K-10) schrieb eine Client-Datei mit vier Direktiven neu und änderte einen Job
nur im Speicher.

- `configure update <typ>=<name> <direktive>=<wert> …` (Konsole,
  `ConfigureUpdate` über gRPC) geht wie `configure delete` durch den Editor:
  ReadResource für Text und SHA-256, die Direktiven werden im Rohtext
  geändert, UpdateResource mit dieser Prüfsumme (`-t`-Prüfung, atomares
  Schreiben, Reload mit Rücknahme, Ressourcen-ACL). Typen: client, job,
  jobdefs, schedule, fileset; Pool, Storage und Console werden abgelehnt.
  Ohne Konfigurationspfad lehnt der Befehl ab.
- Die Textänderung liegt in `bareos_config::edit` und wird auch vom
  MCP-Werkzeug `configure_update` benutzt: Schlüssel wie Bareos-Schlüsselwörter
  verglichen (Groß-/Kleinschreibung, Leerzeichen und `_` egal), nur
  Direktiven der Ressource selbst (Klammertiefe 1), verschachtelte Blöcke
  bleiben unberührt; Anführungszeichen, `#`-Kommentare und `;`-Trenner
  (mehrere Direktiven auf einer Zeile) werden beachtet, ersetzt wird nur der
  Wertbereich. Alles andere bleibt Byte für Byte.
- Ein mehrfach angegebener Schlüssel (`run=… run=…`) ersetzt alle
  vorhandenen Zeilen der Direktive an der Stelle der ersten; `<direktive>=`
  ohne Wert entfernt sie; fehlende Direktiven kommen vor die schließende
  Klammer. Werte ohne Leerzeichen und ohne `{}=;#",\@` stehen ohne
  Anführungszeichen, sonst in Anführungszeichen; Schedule-`Run` immer ohne
  (StoreRun liest eigene Tokens). Job-`Run` ist in Bareos eine Zeichenkette
  (CFG_TYPE_ALIST_STR) und wird deshalb wie jeder andere Wert gesetzt.
- Neu im Editor: Bei Schedule-Ressourcen prüft CREATE/UPDATE jede `Run`-Zeile
  wie C++ StoreRun() (`bareos_config::parser::check_run`); die
  `-t`-Prüfung allein tut das nicht (der typisierte Parser überspringt eine
  falsche Run-Zeile nur mit Warnung).
- gRPC: `ConfigureUpdateCommand.directives` ist `map<string, string>`, trägt
  also einen Wert je Schlüssel. Mehrere Run-Zeilen gehen über gRPC nur mit
  `UpdateResource` (Rohtext); die Planung nimmt Listenwerte nur aus
  JSON-Anfragen an. Das proto bleibt unverändert.
- Tests: `configure_update_goes_through_the_crud`,
  `configure_update_plans_crud_lines`, `bareos_config::edit::tests`.

Offen: RENAME (Grundsatz 9), weitere Typen (Grundsatz 17) mit `followups`
für Pool (Grundsatz 15), ob die alte Konsolenerweiterung `configure read`
entfernt wird.
