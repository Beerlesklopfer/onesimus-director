# .config — Konfigurationsressourcen als Rohtext lesen, anlegen, ändern, löschen

## Zweck

`.config` ist der BNET-Weg des Konfigurations-CRUD (Planung: [CONFIG-CRUD-GRPC.md](../../plans/CONFIG-CRUD-GRPC.md), Abschnitt „BNET-Weg“). Er bietet dieselben Operationen wie die gRPC-RPCs `GetConfigCapabilities`, `ListResources`, `ReadResource`, `CreateResource`, `UpdateResource` und `DeleteResource` ([grpc-api.md](../grpc-api.md)), mit derselben Kernlogik (`crates/odir/src/config_edit.rs`) und derselben Sperre: Rohtext Byte für Byte, SHA-256 als optimistische Sperre, Prüfung wie `odir -t` vor dem Schreiben, atomares Schreiben, Reload mit Rückfall, `REFERENCED` bei Verweisen.

Onesimus-Erweiterung ohne Gegenstück in Bareos; bareos-dir antwortet `.config: is an invalid command.` Der Name kollidiert mit keinem Bareos-Befehl (kein Bareos-Befehl beginnt mit `.config`; `configure` bleibt unverändert).

## Syntax

```text
.config capabilities
.config list   type=<typ>
.config read   type=<typ> name=<name>
.config create type=<typ> text64=<base64> [validate_only]
.config update type=<typ> name=<name> sha256=<alt> text64=<base64> [validate_only]
.config delete type=<typ> name=<name> sha256=<alt> [validate_only]
```

`<typ>`: `schedule`, `job`, `jobdefs`, `client` (Groß-/Kleinschreibung egal). `text64` ist der Ressourcentext in Base64 (Standardalphabet, Auffüllung `=` optional), weil eine Konsolenzeile keine Zeilenenden trägt. Das Schlüsselwort `json` gibt das JSON-Objekt in jedem Modus als eine Zeile aus.

## Rechte

Command-ACL `.config` (wie jeder Befehl) und zusätzlich `configure` (wie die gRPC-RPCs). Ohne `configure` nennt `capabilities` keine Typen, alle anderen Operationen scheitern. Je Name entscheidet die Ressourcen-ACL des Typs wie bei `show` (Schedule-ACL, Job-ACL für Job und JobDefs, Client-ACL): `status` `DENIED`, `list` lässt Verweigertes weg. Schreibende Operationen gehen wie beim gRPC-Weg ins Journal und als Director-Meldung. Im Audit-Protokoll der Befehlszeile steht `text64=<N bytes>` statt des Textes (er kann Passwörter enthalten).

## API-Modi

`.api 2` liefert als `result` die Felder von `ResourceResponse` (`status`, `message`, `resource` {`type`, `name`}, `file`, `text`, `sha256`, `shared_file`, `referenced_by`, `problem` {`file`, `line`, `column`, `text`}, `reloaded`, `validated_only`, `old_sha256`, `followups`), bei `list` `resources` (je `resource`, `file`, `sha256`, `shared_file`) und bei `capabilities` `interface_version` und `types` (je `type`, `operations`). `status` und Typen stehen als Text: `OK`, `CONFLICT`, `INVALID`, `REFERENCED`, `DENIED`, `NOT_FOUND`, `EXISTS`, `RELOAD_FAILED`; `SCHEDULE`, `JOB`, `JOBDEFS`, `CLIENT`; Operationen `READ_RAW`, `CREATE_RAW`, `UPDATE_RAW`, `DELETE`. Ein Status ungleich `OK` ist kein Befehlsfehler (wie beim gRPC-Weg); Befehlsfehler sind fehlende oder falsche Argumente und fehlendes `configure`.

`.api 0`/`.api 1` zeigen dieselben Felder als Zeilen `feld: wert`, am Ende `text:` und den Rohtext.

## Beispiele

```text
*.config capabilities
interface_version: 1
SCHEDULE: READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE
JOB: READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE
JOBDEFS: READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE
CLIENT: READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE
*.config read type=client name=fd
status: OK
resource: Client "fd"
file: /etc/bareos/bareos-dir.d/client/fd.conf
sha256: 5b1e…
text:
Client {
  Name = fd
  Address = localhost
  Password = p
}
*.config delete type=client name=fd sha256=5b1e…
status: REFERENCED
message: Client "fd" is used by 1 resource
resource: Client "fd"
file: /etc/bareos/bareos-dir.d/client/fd.conf
sha256: 5b1e…
referenced_by: job "a"
text:
…
```

## Siehe auch

- [configure](configure.md) — Bareos-Befehl (`configure add/export`) und die älteren Onesimus-Erweiterungen
- [grpc-api.md](../grpc-api.md) — dieselben Operationen über gRPC
