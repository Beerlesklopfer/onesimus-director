# Planung: Ressourcen lesen, anlegen, ändern, löschen über gRPC (CRUD)

Stand: 19.11.2026. Status: **Mit der GUI abgestimmt (19.11.2026), Umsetzung läuft.**

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

`configure update client|job`, `configure delete` und `configure read`
(Zusammenfassung) der Konsole bleiben für ältere Clients bestehen, werden aber
in `GetConfigCapabilities` nicht genannt und von der GUI nicht benutzt
(K-10: `configure update client` verliert Direktiven). Ob sie entfernt werden,
entscheidet der Nutzer.

## Umsetzung (nach Abstimmung)

1. proto: Ergänzung auf einem neuen Zweig `config-crud` des proto-Submoduls
   (ab `plugin-phase1`).
2. odir: Modul `config_edit` (Bereich einer Ressource in der Datei finden,
   Ersetzen/Entfernen, Prüfen der neuen Gesamtkonfiguration mit derselben
   Logik wie `-t`, atomares Schreiben, Reload, Referenzsuche), dann die RPCs
   in `grpc/api*.rs`.
3. Tests: Schedule anlegen/lesen/ändern/löschen; Konflikt; ungültiger Text
   (Zeile/Spalte); Verweis auf einen gelöschten Zeitplan verhindert; geteilte
   Datei mit zwei Ressourcen; Kommentare und unbekannte Direktiven bleiben
   erhalten; Reload-Fehler stellt den alten Stand wieder her; ACL.
