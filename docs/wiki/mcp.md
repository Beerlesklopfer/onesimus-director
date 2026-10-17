# MCP-Server von oconsole

`oconsole --mcp` stellt den Director über das [Model Context Protocol](https://modelcontextprotocol.io) bereit (JSON-RPC 2.0; stdio oder mit `--mcp-http` Streamable HTTP mit Bearer-Token). Die Verbindung zum Director (BNET oder `--grpc`) und die Anmeldung sind dieselben wie in der interaktiven Konsole.

## Transport und Rechte

Entscheidung [C-087](../COMPATIBILITY-LOG.md) (17.10.2026, S-36): Das lokale MCP über stdio bekommt den vollen Befehlssatz, auch im Produktions-Build; MCP über HTTP bleibt lesend.

| Build | `--mcp` (stdio) | `--mcp --mcp-read-only` | `--mcp --mcp-http …` | `--mcp --mcp-http … --mcp-allow-write` |
|---|---|---|---|---|
| Produktion (Vorgabe, Feature `mcp-production`, Pakete) | Lese-, Schreib- und Restore-Tools (122) | nur Lese-Tools (65) | nur Lese-Tools (65) | Option nicht vorhanden (Aufruf wird abgelehnt) |
| Entwicklung (`--no-default-features --features mcp-development`) | Lese-, Schreib-, Restore-Tools und `execute` (123) | nur Lese-Tools | nur Lese-Tools | Lese-, Schreib-, Restore-Tools und `execute` |

- **stdio** gilt als lokal (der Prozess läuft mit den Rechten und der Konsolen-Anmeldung des Aufrufers): die Schreib- und Restore-Tools sind ohne weitere Option da. `--mcp-read-only` schaltet sie ab. `--mcp-allow-write` bleibt die Option des Entwicklungs-Builds für HTTP; für stdio ist sie die Vorgabe.
- **HTTP** (`--mcp-http`, Bearer-Token) ist im Produktions-Build immer lesend: Der Server streicht die Schreibrechte für jede HTTP-Sitzung, auch wenn intern etwas anderes angefordert würde. Schreib-Tools über HTTP gibt es nur im Entwicklungs-Build mit `--mcp-allow-write`.
- **`execute`** (beliebige Zeile) bleibt ein Werkzeug des Entwicklungs-Builds; im Produktions-Build ist es nicht einkompiliert.
- Ohne Schreibrechte nennt `tools/list` kein Schreib-Tool, und ein Aufruf endet mit dem JSON-RPC-Fehler `-32602 unknown tool: <name>`, ohne dass ein Befehl an den Director geht.
- Zerstörende Tools verlangen weiter `confirm: true` (siehe unten), unabhängig vom Transport.

### Weg zum Director

Jedes Tool prüft zuerst seine Parameter und baut daraus die Konsolenzeile. Dann:

1. **gRPC** (`--grpc` oder `Grpc = yes` in der Konfiguration): Das Tool ruft die typisierte RPC der [`DirectorApi`](grpc-api.md) auf (Request aus den Parametern, Antwort als typisierte Message). Das Ergebnis ist die Antwort als JSON mit den Feldnamen der Proto-Datei, einmal als Text und als `structuredContent`. Bei `ActionResponse` (Schreib-RPCs) ist der Text die Ausgabe des Directors mit der gesendeten Zeile davor, `isError` folgt `success`.
2. **gRPC, RPC nicht verfügbar**: Antwortet der Director mit `UNIMPLEMENTED` (Produktions-Director: alle Schreib-, Sitzungs- und Restore-Baum-RPCs, S-35) oder hat ein Parameter kein Feld im Request (z. B. `order` bei `list_jobs`, `state` bei `dot_jobs`), läuft die Konsolenzeile über `ExecuteCommand` derselben gRPC-Sitzung. oconsole merkt sich `UNIMPLEMENTED` je RPC und fragt nicht erneut. Rückfragen kann `ExecuteCommand` nicht beantworten: `purge` hängt deshalb `yes` an (GetConfirmation), und Tools, die nur als Dialog funktionieren (`add_volumes`, `sqlquery`, `run_query`, `delete` eines Pools, `restore_tree_*`), melden über diesen Weg einen Fehler mit Hinweis auf BNET bzw. `dot_sql`.
3. **BNET** (z. B. zum Original-bareos-dir): die Konsolenzeile wie bisher, Lese-Tools unter `.api json compact=yes`, Schreib-Tools im Textmodus mit den Antworten auf die Rückfragen, die das Tool nennt.

Über gRPC sind die Ergebnisse der Lese-Tools also typisierte Messages (z. B. `ListJobsResponse` mit `jobs[].jobstatus` als Enum `JOB_STATUS_TERMINATED`), über BNET das JSON-RPC-Objekt der Director-JSON-API. `"format": "text"` erzwingt bei Lese-Tools die Konsolenzeile.

`configure_update`, `configure_delete`, `delete` mit `kind: storage` und `setbandwidth` (auch `limit: 0` und `all: true`) gehen über die typisierte RPC (S-38/S-39: der Entwicklungs-Director baut dafür die Zeilen, die seine Befehle annehmen; `SetBandwidth` hat ein optionales `limit`). `configure_update` kennt über die RPC nur die Direktiven, die `configure update` ändert (client: `address`, `port`, `password`; job: `newclient`, `newstorage`, `newpool`); andere lehnt der Director mit `INVALID_ARGUMENT` ab.

## Typisierte Lese-Tools

Jedes Tool baut genau eine Konsolenzeile aus geprüften Parametern (JSON-Schema mit Ganzzahlen, Enums und Pflichtfeldern; unbekannte Parameter werden abgelehnt). Namen dürfen nur Buchstaben, Ziffern und `: . - _ /` sowie Leerzeichen enthalten (höchstens 127 Zeichen, mit Leerzeichen wird der Wert in Anführungszeichen gesetzt); Zeilenumbrüche, `;`, `=` oder Anführungszeichen in Werten führen zu einer Fehlermeldung, ohne dass ein Befehl an den Director geht. Alle Tools tragen die Annotationen `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`.

**Ausgabe:** Wo der Befehl die JSON-API unterstützt, schaltet das Tool für diesen einen Befehl auf `.api json compact=yes` und danach zurück auf `.api 0` (über gRPC: Antwortformat JSON). Der Tool-Inhalt ist dann das JSON-RPC-Objekt des Directors in einer Zeile; enthält es `error`, ist `isError` gesetzt. Unter der JSON-API fragt der Director nie nach (keine Auswahllisten). Mit `"format": "text"` kommt stattdessen die Textausgabe wie in bconsole. `status_client`, `status_storage`, `status_configuration` und `list_nextvolume` liefern immer Text, weil der Director dort unter der JSON-API ein leeres `result` liefert.

Fehler in den Parametern kommen als Tool-Ergebnis mit `isError: true` zurück, ein unbekannter Toolname als JSON-RPC-Fehler `-32602`.

### Übersicht

| Tool | Director-Befehl | Ausgabe |
|---|---|---|
| [`list_jobs`](#list_jobs) | `list jobs / llist jobs` | JSON (oder Text) |
| [`list_job`](#list_job) | `list jobid= / llist jobid=` | JSON (oder Text) |
| [`list_joblog`](#list_joblog) | `list joblog jobid=` | JSON (oder Text) |
| [`list_jobmedia`](#list_jobmedia) | `list jobmedia` | JSON (oder Text) |
| [`list_files`](#list_files) | `list files jobid=` | JSON (oder Text) |
| [`list_volumes`](#list_volumes) | `list volumes / list volume=` | JSON (oder Text) |
| [`list_pools`](#list_pools) | `list pools / list pool=` | JSON (oder Text) |
| [`list_clients`](#list_clients) | `list clients / list client=` | JSON (oder Text) |
| [`list_storages`](#list_storages) | `list storages` | JSON (oder Text) |
| [`list_filesets`](#list_filesets) | `list filesets / list fileset jobid=\|filesetid=` | JSON (oder Text) |
| [`list_jobtotals`](#list_jobtotals) | `list jobtotals` | JSON (oder Text) |
| [`list_copies`](#list_copies) | `list copies [jobid=…]` | JSON (oder Text) |
| [`list_backups`](#list_backups) | `list backups client=` | JSON (oder Text) |
| [`list_nextvolume`](#list_nextvolume) | `list nextvolume job=` | Text |
| [`show`](#show) | `show` | JSON (oder Text) |
| [`status_director`](#status_director) | `status director` | JSON (oder Text) |
| [`status_client`](#status_client) | `status client=` | Text |
| [`status_storage`](#status_storage) | `status storage=` | Text |
| [`status_scheduler`](#status_scheduler) | `status scheduler` | JSON (oder Text) |
| [`status_subscriptions`](#status_subscriptions) | `status subscriptions` | JSON (oder Text) |
| [`status_configuration`](#status_configuration) | `status configuration` | Text |
| [`messages`](#messages) | `messages` | JSON (oder Text) |
| [`version`](#version) | `version` | JSON (oder Text) |
| [`time`](#time) | `time` | JSON (oder Text) |
| [`dot_jobs`](#dot_jobs) | `.jobs` | JSON (oder Text) |
| [`dot_clients`](#dot_clients) | `.clients` | JSON (oder Text) |
| [`dot_pools`](#dot_pools) | `.pools` | JSON (oder Text) |
| [`dot_storages`](#dot_storages) | `.storages` | JSON (oder Text) |
| [`dot_filesets`](#dot_filesets) | `.filesets` | JSON (oder Text) |
| [`dot_schedule`](#dot_schedule) | `.schedule` | JSON (oder Text) |
| [`dot_levels`](#dot_levels) | `.levels` | JSON (oder Text) |
| [`dot_types`](#dot_types) | `.types` | JSON (oder Text) |
| [`dot_msgs`](#dot_msgs) | `.msgs` | JSON (oder Text) |
| [`dot_defaults`](#dot_defaults) | `.defaults job=\|client=\|storage=\|pool=` | JSON (oder Text) |
| [`dot_jobstatus`](#dot_jobstatus) | `.jobstatus[=code]` | JSON (oder Text) |
| `list_basefiles` | `list basefiles jobid=\|ujobid=` | JSON (oder Text) |
| `list_jobstatistics` | `list jobstatistics jobid=` | JSON (oder Text) |
| `list_log` | `list log [client=] [reverse] [limit= offset=]` | JSON (oder Text) |
| `status_all` | `status all` | JSON (oder Text) |
| `status_job` | `status job=` | Text |
| `dot_status` | `.status dir <wort>` / `.status client=… <wort>` / `.status storage=… <wort>` | JSON (dir) bzw. Text |
| `dot_messages` | `.messages` | JSON (oder Text) |
| `whoami`, `memory`, `dot_noop` | `whoami`, `memory`, `.` | JSON (oder Text) |
| `help`, `dot_help` | `help [befehl]`, `.help [all] [item=] [full]` | JSON (oder Text) |
| `resolve` | `resolve [client=\|storage=] <host>` | Text |
| `var_expand` | `var <text>` | Text |
| `estimate` | `estimate job= [client= fileset= level= accurate= listing]` | Text |
| `dot_list` | `.catalogs`, `.consoles`, `.users`, `.jobdefs`, `.profiles`, `.locations`, `.media`, `.mediatypes`, `.volstatus`, `.actiononpurge` (Parameter `kind`) | JSON (oder Text) |
| `dot_authorized` | `.authorized job=\|client=\|…\|cmd=` | JSON (oder Text) |
| `sql_meta` | `.sqltables`, `.sqlcount [table=]`, `.sqlschema [table=]` | JSON (oder Text) |
| `acl_list`, `profile_list` | `.acl list [console=]`, `.profile list` | JSON (oder Text) |
| `bvfs_get_jobids`, `bvfs_lsdirs`, `bvfs_lsfiles`, `bvfs_versions` | `.bvfs_get_jobids`, `.bvfs_lsdirs`, `.bvfs_lsfiles`, `.bvfs_versions` | JSON (oder Text) |

`list_volumes` kennt zusätzlich `volumeid`, `mediaid` und `media: true` (Alias `list media` / `media=`), `list_pools` zusätzlich `poolid`.

### Beispielsitzung: Anmeldung und Toolliste

Die Beispiele stammen aus einem Integrationslauf vom 17.10.2026 gegen onesimus-dird mit der Paritätskonfiguration (`tests/parity/parity.py setup`) und einer Wegwerfkopie des Testkatalogs. Lange Listen sind gekürzt (`… n weitere`).

```json
→ {"jsonrpc": "2.0", "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "mcp-read-test", "version": "1"}}, "id": 1}
← {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {"listChanged": false}}, "serverInfo": {"name": "oconsole", "version": "25.1.2+onesimus.0.2.0"}, "instructions": "Connected to the Onesimus/Bareos Director 'bareos-dir' (read-only). Commands are bconsole command lines, e.g. 'status di…"}}
→ {"jsonrpc":"2.0","method":"notifications/initialized"}
→ {"jsonrpc":"2.0","method":"tools/list","params":{},"id":2}
← {"jsonrpc":"2.0","id":2,"result":{"tools":[… 41 Tools: query, ovfs_status, ovfs_latest, ovfs_tree, ovfs_ls, ovfs_versions, list_jobs, list_job, list_joblog, list_jobmedia, list_files, list_volumes, list_pools, list_clients, list_storages, list_filesets, list_jobtotals, list_copies, list_backups, list_nextvolume, show, status_director, status_client, status_storage, status_scheduler, status_subscriptions, status_configuration, messages, version, time, dot_jobs, dot_clients, dot_pools, dot_storages, dot_filesets, dot_schedule, dot_levels, dot_types, dot_msgs, dot_defaults, dot_jobstatus]}}
```

Ein Eintrag der Toolliste (hier `list_job`):

```json
{
  "name": "list_job",
  "title": "One job record",
  "description": "One job from the catalog by JobId or unique job name (list jobid= / llist jobid=).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "jobid": {"type": "integer", "minimum": 1, "maximum": 9223372036854775807, "description": "JobId"},
      "ujobid": {"type": "string", "description": "unique job name, e.g. backup-bareos-fd.2026-10-13_21.00.00_09"},
      "long": {"type": "boolean", "description": "true: llist (all fields of every record) instead of list"},
      "format": {
        "type": "string",
        "enum": ["json", "text"],
        "default": "json",
        "description": "json (default): the Director's JSON API result; text: the bconsole text output"
      }
    },
    "required": [],
    "additionalProperties": false
  },
  "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
}
```

### list_jobs

**Jobs auflisten** – Director-Befehl `list jobs / llist jobs` ([Befehlsseite](commands/list.md)). Jobs aus dem Katalog. Die Filter lassen sich kombinieren.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `job` | Zeichenkette | Jobname |
| `client` | Zeichenkette | Clientname |
| `jobstatus` | Zeichenkette oder Liste | Jobstatus: Kennbuchstabe(n) oder Name(n), als Zeichenkette oder Liste (`T`, `W`, `E`, `f`, `A`, `R`, … oder `terminated`, `warnings`, `error`, `fatal`, `canceled`, `running`) |
| `jobtype` | Zeichenkette oder Liste | Jobtyp: Kennbuchstabe(n) oder exakter Name, Zeichenkette oder Liste (`B`/`Backup`, `R`/`Restore`, `V`/`Verify`, `D`/`Admin`, `c`/`Copy`, `g`/`Migrate`, `O`/`Consolidate`, `A`/`Archive`) |
| `level` | Zeichenkette oder Liste | Level, Zeichenkette oder Liste (`F`/`Full`, `I`/`Incremental`, `D`/`Differential`, `f`/`VirtualFull`, `B`/`Base`) |
| `volume` | Zeichenkette | Volumename |
| `pool` | Zeichenkette | Poolname |
| `days` | Ganzzahl (0…100000) | nur Jobs der letzten n Tage |
| `hours` | Ganzzahl (0…10000000) | nur Jobs der letzten n Stunden |
| `last` | Wahrheitswert | nur der letzte Lauf je Jobname |
| `count` | Wahrheitswert | nur die Anzahl |
| `order` | Zeichenkette, Enum | `asc` oder `desc` (Startzeit) |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `limit` | Ganzzahl (1…1000000) | höchstens n Zeilen (1 bis 1 000 000) |
| `offset` | Ganzzahl (≥ 0) | n Zeilen überspringen (nur mit `limit`) |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_jobs", "arguments": {"client": "bareos-fd", "jobstatus": ["T", "W"], "level": "F", "limit": 2}}, "id": 3}
← {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {
        "jobid": "479",
        "name": "benchmark-small",
        "client": "bareos-fd",
        "starttime": "2026-03-30 22:26:51",
        "duration": "00:00:02",
        "type": "B",
        "level": "F",
        "jobfiles": "2605",
        "jobbytes": "824021543",
        "jobstatus": "T"
      },
      {
        "jobid": "480",
        "name": "benchmark-small",
        "client": "bareos-fd",
        "starttime": "2026-03-30 22:26:51",
        "duration": "00:00:02",
        "type": "B",
        "level": "F",
        "jobfiles": "2605",
        "jobbytes": "824021543",
        "jobstatus": "T"
      }
    ],
    "meta": {"range": {"limit": 2, "filtered": 0}}
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_jobs", "arguments": {"job": "backup-bareos-fd", "last": true, "format": "text"}}, "id": 4}
← {"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
| jobid | name             | client    | starttime           | duration | type | level | jobfiles | jobbytes | jobstatus |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
|   628 | backup-bareos-fd | bareos-fd | 2026-07-10 21:00:02 | 00:00:01 | B    | I     |        0 |        0 | T         |
+-------+------------------+-----------+---------------------+----------+------+-------+----------+----------+-----------+
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_jobs", "arguments": {"count": true, "jobtype": "B"}}, "id": 5}
← {"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{"jsonrpc": "2.0", "id": null, "result": {"jobs": [{"count": "161"}]}}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_jobs", "arguments": {"client": "bareos-fd; delete volume=Full-0001 yes"}}, "id": 51}
← {"jsonrpc":"2.0","id":51,"result":{"content":[{"type":"text","text":…}],"isError":true}}
```

Inhalt von `text`:

```text
client: 'bareos-fd; delete volume=Full-0001 yes' is not a valid name (letters, digits and : . - _ / space, at most 127 characters)
```

### list_job

**Ein Job** – Director-Befehl `list jobid= / llist jobid=` ([Befehlsseite](commands/list.md)). Genau ein Job über JobId oder eindeutigen Jobnamen. Eines von beiden ist Pflicht.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `jobid` | Ganzzahl (≥ 1) | JobId |
| `ujobid` | Zeichenkette | eindeutiger Jobname, z. B. `backup-bareos-fd.2026-10-13_21.00.00_09` |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_job", "arguments": {"jobid": 479}}, "id": 6}
← {"jsonrpc":"2.0","id":6,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {
        "jobid": "479",
        "name": "benchmark-small",
        "client": "bareos-fd",
        "starttime": "2026-03-30 22:26:51",
        "duration": "00:00:02",
        "type": "B",
        "level": "F",
        "jobfiles": "2605",
        "jobbytes": "824021543",
        "jobstatus": "T"
      }
    ]
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_job", "arguments": {"jobid": 479, "long": true}}, "id": 7}
← {"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {
        "jobid": "479",
        "job": "benchmark-small.2026-03-30_22.26.51_04",
        "name": "benchmark-small",
        "purgedfiles": "0",
        "type": "B",
        "level": "F",
        "clientid": "1",
        "client": "bareos-fd",
        "jobstatus": "T",
        "schedtime": "2026-03-30 22:26:51",
        "starttime": "2026-03-30 22:26:51",
        "endtime": "2026-03-30 22:26:53",
        "realendtime": "2026-03-30 22:26:53",
        "duration": "00:00:02",
        "…": "12 weitere Felder"
      }
    ]
  }
}
```

### list_joblog

**Jobprotokoll** – Director-Befehl `list joblog jobid=` ([Befehlsseite](commands/list.md)). Das gespeicherte Jobprotokoll mit Zeitstempeln. `jobid` oder `ujobid` ist Pflicht.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `jobid` | Ganzzahl (≥ 1) | JobId |
| `ujobid` | Zeichenkette | eindeutiger Jobname, z. B. `backup-bareos-fd.2026-10-13_21.00.00_09` |
| `count` | Wahrheitswert | nur die Anzahl |
| `limit` | Ganzzahl (1…1000000) | höchstens n Zeilen (1 bis 1 000 000) |
| `offset` | Ganzzahl (≥ 0) | n Zeilen überspringen (nur mit `limit`) |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_joblog", "arguments": {"jobid": 479, "limit": 3}}, "id": 8}
← {"jsonrpc":"2.0","id":8,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "joblog": [
      {
        "time": "2026-03-30 22:26:51",
        "logtext": "bareos-dir JobId 479: Version: 25.0.3~pre96.1f53a5305 (26 March 2026) Debian GNU/Linux 13 (trixie)\n"
      },
      {
        "time": "2026-03-30 22:26:51",
        "logtext": "bareos-dir JobId 479: Start Backup JobId 479, Job=benchmark-small.2026-03-30_22.26.51_04\n"
      },
      {
        "time": "2026-03-30 22:26:51",
        "logtext": "bareos-dir JobId 479: Connected Storage daemon at WS-JOERG.intranet.bernau.family:9103, encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3\n"
      }
    ],
    "meta": {"range": {"limit": 3, "filtered": 0}}
  }
}
```

### list_jobmedia

**JobMedia** – Director-Befehl `list jobmedia` ([Befehlsseite](commands/list.md)). Welche Volumes ein Job (oder jeder Job) beschrieben hat.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `jobid` | Ganzzahl (≥ 1) | JobId; ohne: alle Jobs |
| `ujobid` | Zeichenkette | eindeutiger Jobname, z. B. `backup-bareos-fd.2026-10-13_21.00.00_09` |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_jobmedia", "arguments": {"jobid": 479}}, "id": 9}
← {"jsonrpc":"2.0","id":9,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobmedia": [
      {"jobid": "479", "volumename": "Full-0001", "firstindex": "2604", "lastindex": "2605"},
      {"jobid": "479", "volumename": "Full-0001", "firstindex": "2239", "lastindex": "2604"},
      {"jobid": "479", "volumename": "Full-0001", "firstindex": "1", "lastindex": "2239"}
    ]
  }
}
```

### list_files

**Dateien eines Jobs** – Director-Befehl `list files jobid=` ([Befehlsseite](commands/list.md)). Die in einem Job gesicherten Pfade; `jobid` oder `ujobid` ist Pflicht. Bei großen Jobs `limit` setzen. Im Beispiel sind die Dateieinträge des Jobs bereits aus dem Katalog entfernt (Prune), die Liste ist daher leer.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `jobid` | Ganzzahl (≥ 1) | JobId |
| `ujobid` | Zeichenkette | eindeutiger Jobname, z. B. `backup-bareos-fd.2026-10-13_21.00.00_09` |
| `limit` | Ganzzahl (1…1000000) | höchstens n Zeilen (1 bis 1 000 000) |
| `offset` | Ganzzahl (≥ 0) | n Zeilen überspringen (nur mit `limit`) |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_files", "arguments": {"jobid": 479, "limit": 3}}, "id": 10}
← {"jsonrpc":"2.0","id":10,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{"jsonrpc": "2.0", "id": null, "result": {"filenames": [], "meta": {"range": {"limit": 3, "filtered": 0}}}}
```

### list_volumes

**Volumes** – Director-Befehl `list volumes / list volume=` ([Befehlsseite](commands/list.md)). Volumes nach Pool gruppiert; mit `pool` nur ein Pool, mit `jobid` die Volumes eines Jobs, mit `volume` genau ein Volume, mit `all` eine einzige Tabelle. Höchstens eine dieser Angaben. Ein unbekannter Pool öffnet keine Auswahlliste, sondern liefert einen JSON-Fehler.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `pool` | Zeichenkette | nur Volumes dieses Pools |
| `jobid` | Ganzzahl (≥ 1) | nur die Volumes dieses Jobs |
| `volume` | Zeichenkette | genau ein Volume (`list volume=`) |
| `all` | Wahrheitswert | alle in einer Tabelle |
| `count` | Wahrheitswert | nur die Anzahl |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `limit` | Ganzzahl (1…1000000) | höchstens n Zeilen (1 bis 1 000 000) |
| `offset` | Ganzzahl (≥ 0) | n Zeilen überspringen (nur mit `limit`) |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_volumes", "arguments": {"pool": "Full", "limit": 2}}, "id": 11}
← {"jsonrpc":"2.0","id":11,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "volumes": [
      {
        "mediaid": "47318",
        "volumename": "Full-0001",
        "volstatus": "Error",
        "enabled": "1",
        "volbytes": "23902210671",
        "volfiles": "5",
        "volretention": "0",
        "recycle": "1",
        "slot": "0",
        "inchanger": "0",
        "mediatype": "File",
        "lastwritten": "2026-03-30 22:54:15",
        "storage": "File",
        "hasencryptionkey": "0"
      },
      {
        "mediaid": "47320",
        "volumename": "Full-0002",
        "volstatus": "Error",
        "enabled": "1",
        "volbytes": "124004811",
        "volfiles": "0",
        "volretention": "0",
        "recycle": "1",
        "slot": "0",
        "inchanger": "0",
        "mediatype": "File",
        "lastwritten": "",
        "storage": "",
        "hasencryptionkey": "0"
      }
    ],
    "meta": {"range": {"limit": 2, "filtered": 0}}
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_volumes", "arguments": {"volume": "Full-47321"}}, "id": 12}
← {"jsonrpc":"2.0","id":12,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "volume": {
      "mediaid": "47321",
      "volumename": "Full-47321",
      "volstatus": "Append",
      "enabled": "1",
      "volbytes": "158201165",
      "volfiles": "0",
      "volretention": "31536000",
      "recycle": "1",
      "slot": "0",
      "inchanger": "0",
      "mediatype": "File",
      "lastwritten": "2026-07-10 21:10:05",
      "storage": "File",
      "hasencryptionkey": "0"
    }
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_volumes", "arguments": {"pool": "nosuch"}}, "id": 13}
← {"jsonrpc":"2.0","id":13,"result":{"content":[{"type":"text","text":…}],"isError":true}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "error": [
          "Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\n",
          "Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\n",
          "Pool nosuch doesn't exist.\n"
        ]
      }
    }
  }
}
```

### list_pools

**Pools** – Director-Befehl `list pools / list pool=` ([Befehlsseite](commands/list.md)). Pool-Datensätze aus dem Katalog.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `pool` | Zeichenkette | genau ein Pool mit Prune-Angaben |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_pools", "arguments": {}}, "id": 14}
← {"jsonrpc":"2.0","id":14,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "pools": [
      {
        "poolid": "1",
        "name": "Scratch",
        "numvols": "0",
        "maxvols": "0",
        "pooltype": "Scratch",
        "labelformat": "*"
      },
      {
        "poolid": "2",
        "name": "Incremental",
        "numvols": "1",
        "maxvols": "100",
        "pooltype": "Backup",
        "labelformat": "Incremental-"
      },
      "… 2 weitere"
    ]
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_pools", "arguments": {"pool": "Full", "long": true}}, "id": 15}
← {"jsonrpc":"2.0","id":15,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "pools": [
      {
        "poolid": "3",
        "name": "Full",
        "numvols": "3",
        "maxvols": "100",
        "useonce": "0",
        "usecatalog": "1",
        "acceptanyvolume": "0",
        "volretention": "31536000",
        "voluseduration": "0",
        "maxvoljobs": "0",
        "maxvolbytes": "53687091200",
        "autoprune": "1",
        "recycle": "1",
        "pooltype": "Backup",
        "…": "8 weitere Felder"
      }
    ]
  }
}
```

### list_clients

**Clients** – Director-Befehl `list clients / list client=` ([Befehlsseite](commands/list.md)). Client-Datensätze aus dem Katalog.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `client` | Zeichenkette | genau ein Client |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_clients", "arguments": {}}, "id": 16}
← {"jsonrpc":"2.0","id":16,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "clients": [
      {"clientid": "1", "name": "bareos-fd", "fileretention": "5184000", "jobretention": "15552000"},
      {"clientid": "2", "name": "parity-ndmp-fd", "fileretention": "0", "jobretention": "0"}
    ]
  }
}
```

### list_storages

**Storages** – Director-Befehl `list storages` ([Befehlsseite](commands/list.md)). Storage-Datensätze aus dem Katalog.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_storages", "arguments": {}}, "id": 17}
← {"jsonrpc":"2.0","id":17,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "storages": [
      {"storageid": "2", "name": "ParityNdmpNative", "autochanger": "0"},
      {"storageid": "3", "name": "ParityNdmp", "autochanger": "0"},
      {"storageid": "1", "name": "File", "autochanger": "0"}
    ]
  }
}
```

### list_filesets

**FileSets** – Director-Befehl `list filesets / list fileset jobid=|filesetid=` ([Befehlsseite](commands/list.md)). FileSet-Datensätze; mit `jobid` oder `filesetid` ein FileSet samt gespeichertem FileSet-Text.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `jobid` | Ganzzahl (≥ 1) | FileSet dieses Jobs mit gespeichertem Text |
| `filesetid` | Ganzzahl (≥ 1) | FileSetId |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_filesets", "arguments": {}}, "id": 18}
← {"jsonrpc":"2.0","id":18,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "filesets": [
      {
        "filesetid": "1",
        "fileset": "SelfTest",
        "md5": "35wO25lkbw+icQACU80RdB",
        "createtime": "2026-03-21 23:44:48",
        "filesettext": "FileSet {\n  Name = \"SelfTest\"\n  Description = \"fileset just to backup some files for selftest\"\n  Include {\n    Options {\n      Signature = \"XXH128\"\n      HardLinks = No\n      AclSupport = Yes\n      Xa…"
      },
      {
        "filesetid": "2",
        "fileset": "Benchmark1GB",
        "md5": "96/+USlWI9+OHD++Zj/MxD",
        "createtime": "2026-03-29 15:19:44",
        "filesettext": "FileSet {\n  Name = \"Benchmark1GB\"\n  Include {\n    Options {\n      HardLinks = No\n      AclSupport = Yes\n      XattrSupport = Yes\n    }\n    File = \"/usr/bin\"\n    File = \"/usr/sbin\"\n    File = \"/usr/sha…"
      },
      "… 2 weitere"
    ]
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_filesets", "arguments": {"jobid": 479}}, "id": 19}
← {"jsonrpc":"2.0","id":19,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "filesets": [
      {
        "filesetid": "4",
        "fileset": "BenchmarkSmall",
        "md5": "dR+CTF/GH6/YnQ8ZOk9y0B",
        "createtime": "2026-03-30 22:17:03",
        "filesettext": "FileSet {\n  Name = \"BenchmarkSmall\"\n  Include {\n    Options {\n      HardLinks = No\n      AclSupport = Yes\n      XattrSupport = Yes\n    }\n    File = \"/usr/bin\"\n  }\n}\n\n"
      }
    ]
  }
}
```

### list_jobtotals

**Job-Summen** – Director-Befehl `list jobtotals` ([Befehlsseite](commands/list.md)). Anzahl Jobs, Dateien und Bytes je Jobname und als Summe.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_jobtotals", "arguments": {}}, "id": 20}
← {"jsonrpc":"2.0","id":20,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {"jobs": "57", "files": "938", "bytes": "209608", "job": "BackupCatalog"},
      {"jobs": "28", "files": "72940", "bytes": "23072603204", "job": "benchmark-small"},
      {"jobs": "76", "files": "865", "bytes": "823720976", "job": "backup-bareos-fd"}
    ],
    "jobtotals": {"jobs": "161", "files": "74743", "bytes": "23896533788"}
  }
}
```

### list_copies

**Kopien** – Director-Befehl `list copies [jobid=…]` ([Befehlsseite](commands/list.md)). Kopierjobs, optional nur zu den angegebenen Originaljobs. Der Testkatalog enthält keine Kopien.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `jobids` | Liste | Liste von JobIds der Originaljobs |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_copies", "arguments": {"jobids": [479, 480]}}, "id": 21}
← {"jsonrpc":"2.0","id":21,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{"jsonrpc": "2.0", "id": null, "result": {}}
```

### list_backups

**Sicherungen eines Clients** – Director-Befehl `list backups client=` ([Befehlsseite](commands/list.md)). Sicherungen eines Clients, optional eines FileSets (`any` für alle).

| Parameter | Typ | Bedeutung |
|---|---|---|
| `client` (Pflicht) | Zeichenkette | Clientname |
| `fileset` | Zeichenkette | FileSet-Name oder `any` |
| `jobstatus` | Zeichenkette oder Liste | Jobstatus: Kennbuchstabe(n) oder Name(n), als Zeichenkette oder Liste (`T`, `W`, `E`, `f`, `A`, `R`, … oder `terminated`, `warnings`, `error`, `fatal`, `canceled`, `running`) |
| `level` | Zeichenkette oder Liste | Level, Zeichenkette oder Liste (`F`/`Full`, `I`/`Incremental`, `D`/`Differential`, `f`/`VirtualFull`, `B`/`Base`) |
| `order` | Zeichenkette, Enum | `asc` oder `desc` (Startzeit) |
| `days` | Ganzzahl (0…100000) | Anzahl Tage |
| `hours` | Ganzzahl (0…10000000) | Anzahl Stunden |
| `long` | Wahrheitswert | `true`: `llist` (alle Felder je Datensatz) statt `list` |
| `limit` | Ganzzahl (1…1000000) | höchstens n Zeilen (1 bis 1 000 000) |
| `offset` | Ganzzahl (≥ 0) | n Zeilen überspringen (nur mit `limit`) |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_backups", "arguments": {"client": "bareos-fd", "fileset": "any", "limit": 2}}, "id": 22}
← {"jsonrpc":"2.0","id":22,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "backups": [
      {
        "jobid": "600",
        "name": "backup-bareos-fd",
        "client": "bareos-fd",
        "starttime": "2026-06-11 21:00:02",
        "duration": "00:00:01",
        "type": "B",
        "level": "F",
        "jobfiles": "599",
        "jobbytes": "157762030",
        "jobstatus": "T"
      },
      {
        "jobid": "601",
        "name": "BackupCatalog",
        "client": "bareos-fd",
        "starttime": "2026-06-11 21:10:04",
        "duration": "00:00:01",
        "type": "B",
        "level": "F",
        "jobfiles": "67",
        "jobbytes": "14972",
        "jobstatus": "W"
      }
    ],
    "meta": {"range": {"limit": 2, "filtered": 0}}
  }
}
```

### list_nextvolume

**Nächstes Volume** – Director-Befehl `list nextvolume job=` ([Befehlsseite](commands/list.md)). Welches Volume der Job als nächstes beschreiben würde. Immer Text: unter der JSON-API liefert der Director hier ein leeres `result`.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `job` (Pflicht) | Zeichenkette | Jobname |
| `days` | Ganzzahl (0…50) | Tage voraus (0 bis 50) |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_nextvolume", "arguments": {"job": "backup-bareos-fd"}}, "id": 23}
← {"jsonrpc":"2.0","id":23,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
The next Volume to be used by Job "backup-bareos-fd" (Pool=Incremental, Level=Differential) will be Incremental-47322
```

### show

**Konfiguration anzeigen** – Director-Befehl `show` ([Befehlsseite](commands/show.md)). Ressourcen aus der Director-Konfiguration: mit `name` eine Ressource, ohne `name` alle des Typs, `all` alles, `disabled` die deaktivierten Jobs, Clients und Schedules. Kennwörter gibt der Director nicht aus.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `resource` (Pflicht) | Zeichenkette | Ressourcentyp: `director`, `client`, `job`, `jobdefs`, `storage`, `catalog`, `schedule`, `fileset`, `pool`, `messages`, `counter`, `profile`, `console`, `user`, `all`, `disabled` |
| `name` | Zeichenkette | Ressourcenname; ohne: alle Ressourcen des Typs |
| `disabled_type` | Zeichenkette, Enum | bei `resource=disabled`: nur `jobs`, `clients` oder `schedules` |
| `verbose` | Wahrheitswert | auch Direktiven mit Vorgabewerten |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "show", "arguments": {"resource": "job", "name": "backup-bareos-fd"}}, "id": 24}
← {"jsonrpc":"2.0","id":24,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {"jobs": {"backup-bareos-fd": {"name": "backup-bareos-fd", "client": "bareos-fd", "jobdefs": "DefaultJob"}}}
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "show", "arguments": {"resource": "pool"}}, "id": 25}
← {"jsonrpc":"2.0","id":25,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "pools": {
      "Scratch": {"name": "Scratch", "pooltype": "Scratch", "storage": ["File"], "recyclepool": "Scratch"},
      "Incremental": {
        "name": "Incremental",
        "labelformat": "Incremental-",
        "maximumvolumes": 100,
        "maximumvolumebytes": "1 g",
        "volumeretention": "1 months "
      },
      "Full": {"name": "Full", "labelformat": "Full-", "maximumvolumes": 100, "maximumvolumebytes": "50 g"},
      "Differential": {
        "name": "Differential",
        "labelformat": "Differential-",
        "maximumvolumes": 100,
        "maximumvolumebytes": "10 g",
        "volumeretention": "3 months "
      }
    }
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "show", "arguments": {"resource": "fileset", "name": "LinuxAll", "format": "text"}}, "id": 26}
← {"jsonrpc":"2.0","id":26,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
FileSet {
  Name = "LinuxAll"
  Description = "Backup all regular filesystems, determined by filesystem type."
  Include {
    Options {
      Signature = "XXH128"
      OneFS = No
      HardLinks = No
      AclSupport = Yes
      XattrSupport = Yes
      FsType = "btrfs"
      FsType = "ext2"
      FsType = "ext3"
      FsType = "ext4"
      FsType = "reiserfs"
      FsType = "jfs"
      FsType = "vfat"
      FsType = "xfs"
      FsType = "zfs"
    }
    File = "/"
  }
  Exclude {
    File = "/var/lib/bareos"
    File = "/var/lib/bareos/storage"
    File = "/proc"
    File = "/dev"
    File = "/sys"
    File = "/tmp"
    File = "/var/tmp"
… (4 weitere Zeilen)
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "show", "arguments": {"resource": "disabled"}}, "id": 27}
← {"jsonrpc":"2.0","id":27,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {"disabled": {"jobs": [], "clients": [], "schedules": ["WeeklyCycleAfterBackup", "WeeklyCycle"]}}
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "show", "arguments": {"resource": "job", "name": "x\nrun job=backup-bareos-fd yes"}}, "id": 52}
← {"jsonrpc":"2.0","id":52,"result":{"content":[{"type":"text","text":…}],"isError":true}}
```

Inhalt von `text`:

```text
name: 'x\nrun job=backup-bareos-fd yes' is not a valid name (letters, digits and : . - _ / space, at most 127 characters)
```

### status_director

**Director-Status** – Director-Befehl `status director` ([Befehlsseite](commands/status.md)). Kopfzeile, geplante, laufende und beendete Jobs und Client-Verbindungen.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `days` | Ganzzahl (0…500) | Tage geplanter Jobs (0 bis 500, Vorgabe 1) |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_director", "arguments": {"days": 1}}, "id": 28}
← {"jsonrpc":"2.0","id":28,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "header": {
      "director": "bareos-dir",
      "version": "25.1.2+onesimus.0.2.0",
      "release_date": "17 October 2026",
      "os": "Debian GNU/Linux 13 (trixie)",
      "binary_info": "Onesimus",
      "daemon_started": "17-Oct-26 16:38",
      "jobs_run": 0,
      "jobs_running": 0,
      "config_warnings": false
    },
    "scheduled": [
      {
        "name": "backup-bareos-fd",
        "type": "Backup",
        "level": "Differential",
        "priority": 10,
        "scheduled": "17-Oct-26 21:00",
        "volume": "Incremental-47322",
        "storage": "File"
      }
    ],
    "running": [],
    "terminated": [],
    "client-connection": []
  }
}
```

### status_client

**Client-Status** – Director-Befehl `status client=` ([Befehlsseite](commands/status.md)). Statusbericht eines File Daemons, immer als Text (unter der JSON-API ist `result` leer). Der Name wird vorher gegen `.clients` geprüft, damit der Director keine Auswahlliste öffnet. Das Tool baut eine Verbindung zum File Daemon auf.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `client` (Pflicht) | Zeichenkette | Clientname |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_client", "arguments": {"client": "bareos-fd"}}, "id": 29}
← {"jsonrpc":"2.0","id":29,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
Connecting to Client bareos-fd at localhost:9102
 Handshake: Immediate TLS, Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3

WS-JOERG.intranet.bernau.family-fd Version: 25.1.2~pre43.18be77e68 (09 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 12-Okt-26 13:01. Jobs: run=10 running=0, Bareos community binary
 Sizeof: boffset_t=8 size_t=8 debug=0 trace=0 bwlimit=0kB/s

Running Jobs:
bareos-dir (director) connected at: 17-Okt-26 18:38
No Jobs running.
====

Terminated Jobs:
 JobId  Level    Files      Bytes   Status   Finished        Name 
======================================================================
     1  Incr        180    126.3 M  OK       13-Okt-26 21:00 backup-bareos-fd
     3  Incr          0         0   OK       14-Okt-26 21:00 backup-bareos-fd
     5  Incr         12    965.8 K  OK       15-Okt-26 21:00 backup-bareos-fd
     9  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    10  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    11  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    12  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    13  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    15  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
   630              180    126.3 M  OK       17-Okt-26 01:07 RestoreFiles
====
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_client", "arguments": {"client": "nosuch"}}, "id": 30}
← {"jsonrpc":"2.0","id":30,"result":{"content":[{"type":"text","text":…}],"isError":true}}
```

Inhalt von `text`:

```text
'nosuch' is not a configured resource; known: parity-ndmp-fd, bareos-fd
```

### status_storage

**Storage-Status** – Director-Befehl `status storage=` ([Befehlsseite](commands/status.md)). Statusbericht eines Storage Daemons, immer als Text; der Name wird vorher gegen `.storages` geprüft. Mit `slots` die Slot-Tabelle eines Autochangers.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `storage` (Pflicht) | Zeichenkette | Storage-Name |
| `slots` | Wahrheitswert | Slot-Tabelle des Autochangers |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_storage", "arguments": {"storage": "File"}}, "id": 31}
← {"jsonrpc":"2.0","id":31,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103
 Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3

bareos-sd Version: 25.1.2~pre43.18be77e68 (09 October 2026) Debian GNU/Linux 13 (trixie) 
Daemon started 12-Okt-26 13:01. Jobs: run=12, running=0, Bareos community binary
 Sizes: boffset_t=8 size_t=8 int32_t=4 int64_t=8 bwlimit=0kB/s

There are WARNINGS for this storagedaemon's configuration!
See output of 'bareos-sd -t' for details.

Job inventory:

No Jobs running.
====

Jobs waiting to reserve a drive:
====

Terminated Jobs:
 JobId  Level    Files      Bytes   Status   Finished        Name 
===================================================================
     5  Incr         12    967.0 K  OK       15-Okt-26 21:00 backup-bareos-fd
     9  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    10  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    11  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    12  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    13  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
    15  Incr          0         0   OK       15-Okt-26 23:00 backup-bareos-fd
     7  Incr          0         0   Error    15-Okt-26 23:27 backup-bareos-fd
     8  Incr          0         0   Error    15-Okt-26 23:27 backup-bareos-fd
… (13 weitere Zeilen)
```

### status_scheduler

**Scheduler-Vorschau** – Director-Befehl `status scheduler` ([Befehlsseite](commands/status.md)). Welche Schedules welche Jobs auslösen und die nächsten Ausführungen. Entweder `days` oder `days_from`/`days_to`.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `days` | Ganzzahl (-365…365) | Vorschau für n Tage (−365 bis 365, negativ: zurück; Vorgabe 7) |
| `days_from` | Ganzzahl (-366…366) | Beginn des Zeitraums in Tagen relativ zu jetzt (mit `days_to`) |
| `days_to` | Ganzzahl (-366…366) | Ende des Zeitraums (mit `days_from`) |
| `job` | Zeichenkette | nur dieser Job |
| `client` | Zeichenkette | nur Jobs dieses Clients |
| `schedule` | Zeichenkette | Schedule-Name |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_scheduler", "arguments": {"days": 2}}, "id": 32}
← {"jsonrpc":"2.0","id":32,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "schedules": [],
    "preview": [
      {
        "datetime": "Sun 18-Oct-2026 12:45",
        "runtime": 1792327500,
        "schedule": "ParityPreview",
        "level": "Incremental",
        "priority": 5,
        "storage": "File",
        "messages": "Standard",
        "spool_data": true,
        "accurate": false
      },
      {
        "datetime": "Mon 19-Oct-2026 12:45",
        "runtime": 1792413900,
        "schedule": "ParityPreview",
        "level": "Incremental",
        "priority": 5,
        "storage": "File",
        "messages": "Standard",
        "spool_data": true,
        "accurate": false
      }
    ]
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_scheduler", "arguments": {"days": 2, "format": "text"}}, "id": 33}
← {"jsonrpc":"2.0","id":33,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
Scheduler Jobs:

Schedule               Jobs Triggered
===========================================================
====

Scheduler Preview for 2 days:

Date                   Schedule                Overrides
==============================================================
Sun 18-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
Mon 19-Oct-2026 12:45  ParityPreview           Level=Incremental Priority=5 Spool Data=1 Accurate=0 Storage=File Messages=Standard
====
```

### status_subscriptions

**Subscriptions** – Director-Befehl `status subscriptions` ([Befehlsseite](commands/status.md)). Auswertung der Backup-Einheiten aus dem Katalog.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `clients` | Wahrheitswert | Zusammenfassung je Client |
| `plugins` | Wahrheitswert | Zusammenfassung je Plugin |
| `all` | Wahrheitswert | alle Teilberichte |
| `anonymize` | Wahrheitswert | Clientnamen durch `Client 1`, `Client 2`, … ersetzen |
| `client` | Zeichenkette | Detailbericht je FileSet dieses Clients |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_subscriptions", "arguments": {}}, "id": 34}
← {"jsonrpc":"2.0","id":34,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": "25.1.2+onesimus.0.2.0",
    "os": "Debian GNU/Linux 13 (trixie)",
    "binary-info": "Onesimus",
    "report-time": "2026-10-17 16:38:42",
    "unit-detail": [
      {"client": "bareos-fd", "plugin": "none (file data)", "count": "1", "size_gb": "0.98"},
      {"client": "TOTAL", "plugin": "", "count": "1", "size_gb": "0.98"}
    ],
    "unit-summary": {
      "accounting_mode": "volume-based (1 volume-based units >= 1 count-based units)",
      "used": "1",
      "configured": "0",
      "remaining": "-1"
    },
    "checksum": "{SHA256}892495ee74d1e76d2e5f9e85272b57742269f53bd9bd934872f80d597632d6f"
  }
}
```

### status_configuration

**Konfigurationsprüfung** – Director-Befehl `status configuration` ([Befehlsseite](commands/status.md)). Prüft die Konfiguration auf veraltete Einstellungen; immer Text.

Keine Parameter.

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "status_configuration", "arguments": {}}, "id": 35}
← {"jsonrpc":"2.0","id":35,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
No deprecated configuration settings detected.
```

### messages

**Nachrichten** – Director-Befehl `messages` ([Befehlsseite](commands/messages.md)). Die für die Konsole gesammelten Director-Nachrichten (Jobberichte, Warnungen). Das Lesen leert die Warteschlange; das Tool ist daher nicht idempotent im strengen Sinn, ändert aber weder Konfiguration noch Katalog.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "messages", "arguments": {}}, "id": 36}
← {"jsonrpc":"2.0","id":36,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{"jsonrpc": "2.0", "id": null, "result": {}}
```

### version

**Version** – Director-Befehl `version` ([Befehlsseite](commands/version.md)). Version des Directors.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "version", "arguments": {}}, "id": 37}
← {"jsonrpc":"2.0","id":37,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": {
      "name": "bareos-dir",
      "type": "bareos-director",
      "version": "25.1.2+onesimus.0.2.0",
      "bdate": "17 October 2026",
      "operatingsystem": "Debian GNU/Linux 13 (trixie)",
      "distname": "debian",
      "distversion": "Debian GNU/Linux 13 (trixie)",
      "customversionid": ""
    }
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "version", "arguments": {}}, "id": 53}
← {"jsonrpc":"2.0","id":53,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": {
      "name": "bareos-dir",
      "type": "bareos-director",
      "version": "25.1.2+onesimus.0.2.0",
      "bdate": "17 October 2026",
      "operatingsystem": "Debian GNU/Linux 13 (trixie)",
      "distname": "debian",
      "distversion": "Debian GNU/Linux 13 (trixie)",
      "customversionid": ""
    }
  }
}
```

### time

**Uhrzeit** – Director-Befehl `time` ([Befehlsseite](commands/time.md)). Aktuelle Zeit auf dem Director.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "time", "arguments": {}}, "id": 38}
← {"jsonrpc":"2.0","id":38,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "time": {
      "full": "Sat 17-Oct-2026 16:38:42",
      "year": "2026",
      "month": "10",
      "day": "17",
      "hour": "16",
      "minute": "38",
      "second": "42"
    }
  }
}
```

### dot_jobs

**Jobnamen** – Director-Befehl `.jobs` ([Befehlsseite](commands/dot-jobs.md)). Namen der konfigurierten Jobs.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `type` | Zeichenkette, Enum | nur Jobs dieses Typs (`Backup`, `Restore`, `Verify`, `Admin`, `Copy`, `Migrate`, `Consolidate`, `Archive`) |
| `state` | Zeichenkette, Enum | `enabled` oder `disabled` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_jobs", "arguments": {"type": "Backup"}}, "id": 39}
← {"jsonrpc":"2.0","id":39,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {"jobs": [{"name": "backup-bareos-fd", "enabled": true}, {"name": "BackupCatalog", "enabled": true}]}
}
```

### dot_clients

**Clientnamen** – Director-Befehl `.clients` ([Befehlsseite](commands/dot-clients.md)). Namen der konfigurierten Clients.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `state` | Zeichenkette, Enum | `enabled` oder `disabled` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_clients", "arguments": {"state": "enabled"}}, "id": 40}
← {"jsonrpc":"2.0","id":40,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {"clients": [{"name": "parity-ndmp-fd", "enabled": true}, {"name": "bareos-fd", "enabled": true}]}
}
```

### dot_pools

**Poolnamen** – Director-Befehl `.pools` ([Befehlsseite](commands/dot-pools.md)). Namen der konfigurierten Pools.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `type` | Zeichenkette, Enum | nur Pools dieses Typs (`Backup`, `Scratch`, `Copy`, `Archive`, `Cloned`, `Migration`, `Save`) |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_pools", "arguments": {"type": "Backup"}}, "id": 41}
← {"jsonrpc":"2.0","id":41,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {"pools": [{"name": "Incremental"}, {"name": "Full"}, {"name": "Differential"}]}
}
```

### dot_storages

**Storage-Namen** – Director-Befehl `.storages` ([Befehlsseite](commands/dot-storages.md)). Namen der konfigurierten Storages.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `state` | Zeichenkette, Enum | `enabled` oder `disabled` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_storages", "arguments": {}}, "id": 42}
← {"jsonrpc":"2.0","id":42,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "storages": [
      {"name": "ParityNdmpNative", "enabled": true},
      {"name": "ParityNdmp", "enabled": true},
      {"name": "File", "enabled": true}
    ]
  }
}
```

### dot_filesets

**FileSet-Namen** – Director-Befehl `.filesets` ([Befehlsseite](commands/dot-filesets.md)). Namen der konfigurierten FileSets.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_filesets", "arguments": {}}, "id": 43}
← {"jsonrpc":"2.0","id":43,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {"filesets": [{"name": "Windows All Drives"}, {"name": "SelfTest"}, "… 2 weitere"]}
}
```

### dot_schedule

**Schedule-Namen** – Director-Befehl `.schedule` ([Befehlsseite](commands/dot-schedule.md)). Namen der konfigurierten Schedules.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `state` | Zeichenkette, Enum | `enabled` oder `disabled` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_schedule", "arguments": {"state": "disabled"}}, "id": 44}
← {"jsonrpc":"2.0","id":44,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "schedules": [{"name": "WeeklyCycleAfterBackup", "enabled": false}, {"name": "WeeklyCycle", "enabled": false}]
  }
}
```

### dot_levels

**Level** – Director-Befehl `.levels` ([Befehlsseite](commands/dot-levels.md)). Job-Level, optional nur die eines Jobtyps.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `jobtype` | Zeichenkette, Enum | nur die Level dieses Jobtyps |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_levels", "arguments": {"jobtype": "Backup"}}, "id": 45}
← {"jsonrpc":"2.0","id":45,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "levels": [
      {"name": "Full", "level": 70, "jobtype": 66},
      {"name": "Base", "level": 66, "jobtype": 66},
      "… 4 weitere"
    ]
  }
}
```

### dot_types

**Jobtypen** – Director-Befehl `.types` ([Befehlsseite](commands/dot-types.md)). Die Jobtypen.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_types", "arguments": {}}, "id": 46}
← {"jsonrpc":"2.0","id":46,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{"jsonrpc": "2.0", "id": null, "result": {"jobtypes": [{"name": "Backup"}, {"name": "Admin"}, "… 6 weitere"]}}
```

### dot_msgs

**Messages-Ressourcen** – Director-Befehl `.msgs` ([Befehlsseite](commands/dot-msgs.md)). Namen der Messages-Ressourcen.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_msgs", "arguments": {}}, "id": 47}
← {"jsonrpc":"2.0","id":47,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{"jsonrpc": "2.0", "id": null, "result": {"messages": [{"text": "Standard"}, {"text": "Daemon"}]}}
```

### dot_defaults

**Vorgaben einer Ressource** – Director-Befehl `.defaults job=|client=|storage=|pool=` ([Befehlsseite](commands/dot-defaults.md)). Vorgabewerte eines Jobs (oder Clients, Storages, Pools) als Schlüssel/Wert-Paare.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `resource` | Zeichenkette, Enum | `job` (Vorgabe), `client`, `storage` oder `pool` |
| `name` (Pflicht) | Zeichenkette | Ressourcenname |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_defaults", "arguments": {"name": "backup-bareos-fd"}}, "id": 48}
← {"jsonrpc":"2.0","id":48,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "defaults": {
      "job": "backup-bareos-fd",
      "pool": "Incremental",
      "messages": "Standard",
      "client": "bareos-fd",
      "storage": "File",
      "where": "",
      "level": "Incremental",
      "type": "Backup",
      "fileset": "SelfTest",
      "enabled": 1,
      "catalog": "MyCatalog"
    }
  }
}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_defaults", "arguments": {"resource": "pool", "name": "Full", "format": "text"}}, "id": 49}
← {"jsonrpc":"2.0","id":49,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text`:

```text
pool=Full
pool_type=Backup
label_format=Full-
purge_oldest_volume==0
recycle_oldest_volume=0
max_volumes=100
vol_retention=31536000
vol_use_duration=0
max_vol_jobs=0
max_vol_files=0
max_vol_bytes=53687091200
auto_prune=1
recycle=1
file_retention=0
job_retention=0
```

### dot_jobstatus

**Statuscodes** – Director-Befehl `.jobstatus[=code]` ([Befehlsseite](commands/dot-jobstatus.md)). Die Job-Statuscodes mit Bedeutung, oder nur ein Code.

| Parameter | Typ | Bedeutung |
|---|---|---|
| `code` | Zeichenkette | ein Status-Kennbuchstabe, z. B. `T` |
| `format` | Zeichenkette, Enum | `json` (Vorgabe): JSON-Antwort des Directors; `text`: Textausgabe wie in bconsole |

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "dot_jobstatus", "arguments": {"code": "T"}}, "id": 50}
← {"jsonrpc":"2.0","id":50,"result":{"content":[{"type":"text","text":…}],"isError":false}}
```

Inhalt von `text` (gekürzt, eingerückt):

```json
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobstatus": [
      {
        "jobstatus": "T",
        "jobstatuslong": "Completed successfully",
        "severity": "10",
        "exitlevel": "0",
        "exitstatus": "Ok"
      }
    ]
  }
}
```

### Abgelehnte Eingaben

Werte, die eine zweite Anweisung einschleusen könnten, erreichen den Director nicht:

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_jobs", "arguments": {"client": "bareos-fd; delete volume=Full-0001 yes"}}, "id": 51}
← {"jsonrpc": "2.0", "id": 51, "result": {"content": [{"type": "text", "text": "client: 'bareos-fd; delete volume=Full-0001 yes' is not a valid name (letters, digits and : . - _ / space, at most 127 characters)"}], "isError": true}}
```

```json
→ {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "show", "arguments": {"resource": "job", "name": "x\nrun job=backup-bareos-fd yes"}}, "id": 52}
← {"jsonrpc": "2.0", "id": 52, "result": {"content": [{"type": "text", "text": "name: 'x\\nrun job=backup-bareos-fd yes' is not a valid name (letters, digits and : . - _ / space, at most 127 characters)"}], "isError": true}}
```

## Weitere Tools

- `query`: beliebiger Read-only-Befehl als Zeile (Allowlist: `status`, `list`, `llist`, `show`, `version`, `help`, `time`, `messages`, `estimate` und lesende Punktbefehle).
- `ovfs_status`, `ovfs_latest`, `ovfs_tree`, `ovfs_ls`, `ovfs_versions`: Wiederherstellungsbäume durchsuchen (siehe [.ovfs_tree](commands/dot-ovfs-tree.md)).

## Schreib- und Restore-Tools

Module `oconsole/src/mcp_write.rs`, `mcp_write/admin.rs` (weitere Befehle) und `mcp_tree.rs` (interaktive Wiederherstellung). Sie stehen bereit, wenn der Transport Schreibrechte hat (siehe [Transport und Rechte](#transport-und-rechte)): über stdio in jedem Build, über HTTP nur im Entwicklungs-Build mit `--mcp-allow-write`.

### Gemeinsame Regeln

- **Eine Zeile pro Aufruf.** Jedes Tool baut aus den Parametern genau eine Konsolenzeile und hängt `yes` an, wo der Befehl das Schlüsselwort kennt (`run`, `rerun`, `cancel`, `restore`, `label`, `relabel`, `prune`, `delete volume`, `truncate`). Es läuft im normalen Konsolenmodus (nicht unter der JSON-API), die Ausgabe ist der Text des Directors. Vor der Ausgabe steht die gesendete Zeile mit `> `.
- **Prüfung vor dem Senden.** Namen von Ressourcen und Volumes: Buchstaben, Ziffern, `: . - _ /` und Leerzeichen im Inneren, höchstens 127 Zeichen, kein `/` am Anfang. Pfade, reguläre Ausdrücke und Kommentare werden in Anführungszeichen gesetzt (`\` und `"` maskiert), Steuerzeichen sind verboten. Zeitpunkte nur als `JJJJ-MM-TT HH:MM[:SS]`, JobIds als Ganzzahlen ≥ 1. Unbekannte Parameter, fehlende Pflichtangaben oder widersprüchliche Kombinationen liefern ein Tool-Ergebnis mit `isError: true`. In diesem Fall erreicht kein Befehl den Director.
- **Rückfragen des Directors.** Ein Tool beantwortet nur die Fragen, die zu seinem Zweck gehören: `Purge (yes/no)?` bei `purge` und `Are you sure you want to delete …` bei `delete` mit `yes`; `add_volumes`, `sqlquery` und `run_query` beantworten die Fragen ihres Dialogs aus ihren Parametern (Anzahl/Name/Startnummer/Slot; SQL-Anweisung und Leerzeile; Nummer der Abfrage und ihre Parameter). Jede andere Frage (Auswahlliste bei einem unbekannten Namen, fehlender Restore-Job …) bricht oconsole mit `.` ab. Das Ergebnis ist dann `isError: true` mit dem Hinweis, welcher Parameter fehlt. Die Sitzung bleibt in jedem Fall bedienbar. Über gRPC beantwortet die typisierte RPC des Entwicklungs-Directors diese Fragen selbst; über `ExecuteCommand` (Produktions-Director) gibt es keine Rückfragen (siehe [Weg zum Director](#weg-zum-director)).
- **Bestätigung.** `relabel_volume`, `prune`, `purge`, `delete` und `truncate` verlangen den Parameter `confirm: true`. Ohne ihn lautet die Antwort `<tool> is destructive: ask the user, then call it again with confirm=true`, und nichts wird ausgeführt. Diese Tools sowie `cancel_job` und `restore` tragen `destructiveHint: true`.
- **Gestartete Jobs.** Bei `run_job`, `rerun_job` und `restore` enthält das Ergebnis zusätzlich `structuredContent: {"jobids": [n, …]}` aus den Zeilen `Job queued. JobId=n`.
- **Lange Befehle.** `wait_job` und große Restores belegen die Director-Sitzung, bis der Director antwortet. Läuft ein Befehl länger als das 30-Sekunden-Lesezeitlimit des Sockets, wartet oconsole einfach weiter.

### Übersicht

| Tool | Director-Befehl | Annotationen | `confirm` |
|---|---|---|---|
| `run_job` | `run job=… [level= client= fileset= pool= nextpool= storage= when= since= priority= jobid= spooldata= accurate= comment=] yes` | schreibend | – |
| `rerun_job` | `rerun jobid=…` \| `since_jobid= [until_jobid=]` \| `days= hours=` `yes` | schreibend | – |
| `cancel_job` | `cancel jobid=` \| `job=` \| `ujobid=` \| `all` \| `state=` `[storage=] yes` | destruktiv | – |
| `wait_job` | `wait jobid=` \| `job=` \| `ujobid=` \| `mount [timeout=]` | lesend, idempotent | – |
| `restore` | `restore … done … yes` (siehe unten) | destruktiv | – |
| `label_volume` | `label storage= volume= pool= [slot= drive= encrypt] yes` | schreibend | – |
| `relabel_volume` | `relabel storage= oldvolume= volume= [pool= slot= drive= encrypt] yes` | destruktiv | ja |
| `mount_storage` | `mount storage=\|jobid=\|job=\|ujobid= [drive= slot=]` | idempotent | – |
| `unmount_storage` | `unmount … [drive=] [alldrives]` | idempotent | – |
| `release_storage` | `release … [drive=] [alldrives]` | idempotent | – |
| `update_volume` | `update volume=… volstatus= volretention= voluse= maxvoljobs= maxvolfiles= maxvolbytes= recycle= inchanger= slot= enabled= pool= recyclepool= actiononpurge= storage= frompool=yes`; `update volume allfrompool=…`; `update volume fromallpools` | idempotent | – |
| `update_pool` | `update pool=…` | idempotent | – |
| `prune` | `prune files\|jobs [client= pool=]`, `prune volume=… \| volume all [pool=]`, `prune stats`, `prune directory=… client= [recursive]`, jeweils `yes` | destruktiv | ja |
| `purge` | `purge files jobid=\|job=\|client=\|volume=`, `purge jobs client=\|volume=\|pool= [jobstatus=]`, `purge volume=… [volume=…] [jobstatus=] [action= storage= pool=\|allpools drive=]`, `purge quota [client=]` | destruktiv | ja |
| `delete` | `delete volume=… [pool=] yes`, `delete pool=…`, `delete jobid=1,2,…`, `delete storage=…` | destruktiv | ja |
| `truncate` | `truncate volstatus=Purged [storage= pool= volume= drive=] yes` | destruktiv | ja |
| `enable`, `disable` | `enable\|disable job=\|client=\|schedule=…` | idempotent | – |
| `configure_add` | `configure add <typ> name=… <direktive>="…" …` | schreibend | – |
| `configure_export` | `configure export client=…` | idempotent | – |
| `reload` | `reload` | idempotent | – |
| `setdebug` | `setdebug level= [trace=0\|1 timestamp=0\|1 hangup=] dir\|client=…\|storage=…\|all` | idempotent | – |
| `setbandwidth` | `setbandwidth limit= client=\|storage=\|jobid=\|job=\|ujobid=\|all\|state=` | idempotent | – |
| `add_volumes` | `add pool= storage=` + Dialog (Anzahl, Name, Startnummer, Slot, InChanger) | schreibend | – |
| `create_pool` | `create pool=` | idempotent | – |
| `export_volumes`, `import_volumes`, `move_volumes` | `export\|import storage= [srcslots= dstslots= volume= scan]`, `move storage= srcslots= dstslots=` | schreibend | – |
| `update_slots` | `update slots storage= [scan] [drive=]` | idempotent | – |
| `update_job` | `update jobid= [jobname= starttime= client= filesetid= jobtype=]` | idempotent | – |
| `update_stats` | `update stats [days=]` | idempotent | – |
| `sqlquery` | `sqlquery` + Anweisung im Dialog | destruktiv (ungeprüftes SQL) | – |
| `dot_sql` | `.sql query="…" [catalog=]` | destruktiv (ungeprüftes SQL) | – |
| `run_query` | `query` + Nummer und Parameter im Dialog | idempotent | – |
| `bvfs_update`, `bvfs_restore`, `bvfs_cleanup`, `bvfs_clear_cache` | `.bvfs_update [jobid=]`, `.bvfs_restore path= jobid= [fileid= dirid= hardlink=]`, `.bvfs_cleanup path=`, `.bvfs_clear_cache yes` | schreibend | – |
| `acl_modify` | `.acl set\|delete console= <typ>acl=…`, `.acl clear console=`, `.acl copy from= to=` | destruktiv | – |
| `profile_modify` | `.profile create\|set\|delete\|assign\|unassign …` | destruktiv | – |
| `configure_update` | `configure update <typ>=<name> <direktive>="…" …` | idempotent | – |
| `configure_delete` | `configure delete <typ>=<name> yes` | destruktiv | ja |
| `use_catalog` | `use catalog=` | idempotent | – |
| `setdevice` | `setdevice storage= device= [autoselect=yes\|no]` | idempotent | – |
| `session_option` | `autodisplay\|automount\|trace on\|off` | idempotent | – |
| `restore_tree_*` | interaktive Dateiauswahl, siehe unten | – | – |
| `execute` (nur Entwicklungs-Build) | beliebige einzelne Zeile (nicht `quit`, `.api`, `@…`) | destruktiv | – |

Die Befehle selbst beschreiben [run](commands/run.md), [rerun](commands/rerun.md), [cancel](commands/cancel.md), [wait](commands/wait.md), [restore](commands/restore.md), [label](commands/label.md), [relabel](commands/relabel.md), [mount](commands/mount.md), [unmount](commands/unmount.md), [release](commands/release.md), [update](commands/update.md), [prune](commands/prune.md), [purge](commands/purge.md), [delete](commands/delete.md), [truncate](commands/truncate.md), [enable](commands/enable.md), [disable](commands/disable.md), [configure](commands/configure.md), [reload](commands/reload.md), [setdebug](commands/setdebug.md) und [setbandwidth](commands/setbandwidth.md).

Einschränkungen in den Parametern: `wait_job` verlangt eine Auswahl (ohne Auswahl würde `wait` auf alle Jobs warten). `setdebug` mit `target` `client` oder `storage` braucht `name`, und `setbandwidth` braucht ein Ziel; ohne diese Angaben würde der Director eine Auswahlliste öffnen. Bei `configure_add` sind die Direktivennamen klein und ohne Leerzeichen (`maximumvolumejobs`), Werte vom Typ Liste wiederholen die Direktive, und `@` sowie `;` in Werten werden abgelehnt.

### restore

Die Auswahl der Jobs und Dateien steht vollständig in einer Zeile; der Dateibaum wird nie interaktiv geöffnet:

| Auswahl | Parameter | Zeile |
|---|---|---|
| Jobs | `jobids: [630]` | `restore jobid=630 client=…` |
| neuester Stand | `fileset` ohne `jobids` | `restore select current client=… fileset=…` |
| Stand vor einem Zeitpunkt | `before: "2030-01-01 00:00:00"`, `fileset` | `restore select before="…" client=… fileset=…` |
| alle Dateien | `all: true` | `… all done` |
| einzelne Dateien | `files: ["/etc/passwd", …]` (ohne `jobids`: neueste Version) | `… file="/etc/passwd" … done` |
| Verzeichnisse | `directories: ["/etc/"]` (nur mit `jobids`) | `… directory="/etc/" done` |
| reguläre Ausdrücke | `fileregex: "\\.conf$"` | `… fileregex="\\.conf$" done` |

Genau eine der vier Dateiauswahlen ist erlaubt. Für das Ziel gilt genau eine der Varianten `where`, `regexwhere` oder `strip_prefix`/`add_prefix`/`add_suffix`. Dazu kommen `replace` (`always`, `ifnewer`, `ifolder`, `never`), `restoreclient`, `restorejob` (nötig, wenn mehrere Restore-Jobs definiert sind; sonst fragt der Director, und das Tool bricht ab), `storage`, `pool` und `comment`. Pfade findet man vorher mit `ovfs_latest`, `ovfs_tree`, `ovfs_ls` und `ovfs_versions`.

### Interaktive Wiederherstellung (`restore_tree_*`)

Die 26 Befehle der Dateiauswahl (`ua_tree.cc`) als Tools. `restore_tree_start` nimmt dieselben Parameter wie `restore` ohne Dateiauswahl (`client`, `jobids` oder `fileset` für den neuesten Stand bzw. `before`, Ziel- und Ersetzungsoptionen) und liefert eine `session_id`, die Zahl der Dateien im Baum und das Arbeitsverzeichnis. Alle weiteren Tools nehmen diese `session_id`.

| Tool | Befehle der Dateiauswahl | Ergebnis |
|---|---|---|
| `restore_tree_start` | `restore …` bis zur Dateiauswahl | `session_id`, `jobids`, `files`, `cwd`, `text` |
| `restore_tree_cd` (`path`) | `cd` | `cwd` |
| `restore_tree_pwd` | `pwd`, `.pwd` | `cwd` |
| `restore_tree_ls` (`pattern`, `mode`: `all`/`dirs`/`marked`) | `ls`, `dir`, `.ls`, `.dir` / `.lsdir` / `lsmark`, `.lsmark` | `cwd`, `entries[]` (`name`, `is_dir`, `marked`) |
| `restore_tree_mark` (`pattern`, `dirs_only`) | `mark`, `add` / `markdir` | `marked` |
| `restore_tree_unmark` (`pattern`, `dirs_only`) | `unmark`, `delete` / `unmarkdir` | `marked` |
| `restore_tree_count` | `count` | `total`, `marked` |
| `restore_tree_find` (`pattern`) | `find` | `entries[]` |
| `restore_tree_estimate` | `estimate` | `total`, `marked`, `bytes` |
| `restore_tree_help` | `help`, `.help`, `?` | `text` |
| `restore_tree_done` | `done`, `exit` (beantwortet „OK to run?“ mit yes) | `jobids` des Restore-Jobs (`destructiveHint`) |
| `restore_tree_abort` | `abort`, `quit` | kein Job |

- **gRPC**: `StartRestoreSession` und `RestoreTree*` der DirectorApi; die Sitzung gehört zur Anmeldung und endet nach 10 Minuten ohne Aufruf. Ein Produktions-Director bietet diese RPCs nicht (`UNIMPLEMENTED`), und `ExecuteCommand` hat keinen Dialog: Die Tools melden dann einen Fehler und verweisen auf das einzeilige `restore` oder BNET.
- **BNET**: der Dialog der Konsole selbst. Solange die Auswahl offen ist, steht die Verbindung im Dialog; der Server lehnt dann jedes andere Tool mit einem Hinweis ab, bis `restore_tree_done` oder `restore_tree_abort` kommt.

### Abdeckung

`crates/oconsole/src/mcp_coverage.rs` ordnet jeder RPC der DirectorApi ein Tool mit Beispielparametern zu und prüft gegen [grpc-api.md](grpc-api.md#abdeckung-befehl--rpc) und [`commands/bareos-director-commands.json`](commands/bareos-director-commands.json), dass alle 88 Befehle mit ihren Unterbefehlen, die Onesimus-Erweiterungen und die 26 Befehle der Dateiauswahl ein Tool haben, dass jedes Tool aus den Beispielparametern seine Zeile baut und auf genau diese RPC abbildet, ohne einen Parameter zu verlieren. Ausgenommen sind nur Sitzungsbefehle, die die Sitzung des MCP-Servers selbst beenden oder umschalten würden: `quit`, `exit`, `.quit`, `.api`, `.grpc`, `gui` und `setip` (die DirectorApi beantwortet die letzten vier ebenfalls mit FAILED_PRECONDITION). Die Zuordnung Befehl → Tool folgt der RPC-Tabelle in grpc-api.md; Tools, die mehrere RPCs abdecken, wählen sie über ihre Parameter (`show` `resource`, `prune`/`purge`/`delete` `kind`, `dot_status` `target`, `dot_list` `kind`, `sql_meta` `kind`, `acl_modify`/`profile_modify` `action`, `session_option` `option`, `restore_tree_ls` `mode`, `restore_tree_mark`/`_unmark` `dirs_only`).

### Beispiele

Die Ausgaben stammen aus dem Integrationslauf vom 17.10.2026 gegen onesimus-dird mit eigenem bareos-fd und bareos-sd (25.1.2) und einer Wegwerfkopie des Katalogs.

Backup starten (Ergebnis gekürzt auf Text und `structuredContent`):

```json
→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_job","arguments":{"job":"McpwBackup","level":"Full","client":"mcpw-fd","fileset":"McpwSet","pool":"McpwFull","storage":"McpwFile","priority":10,"spooldata":false,"accurate":true,"comment":"mcp-write test"}}}
```

```text
> run job=McpwBackup level=Full client=mcpw-fd fileset=McpwSet pool=McpwFull storage=McpwFile priority=10 spooldata=no accurate=yes comment="mcp-write test" yes
Using Catalog "MyCatalog"
Job queued. JobId=630
```

`structuredContent`: `{"jobids":[630]}`. Danach `wait_job` mit `{"jobid":630}`:

```text
> wait jobid=630
JobId=630
JobStatus=OK (T)
```

Zwei Dateien aus diesem Backup zurückschreiben:

```json
{"name":"restore","arguments":{"client":"mcpw-fd","jobids":[630],"files":["/tmp/claude-10001/mcpw/data/a.txt","/tmp/claude-10001/mcpw/data/sub/c.txt"],"where":"/tmp/claude-10001/mcpw/restore/mcp1","restorejob":"McpwRestore","replace":"always","comment":"mcp restore 1"}}
```

```text
> restore jobid=630 client=mcpw-fd file="/tmp/claude-10001/mcpw/data/a.txt" file="/tmp/claude-10001/mcpw/data/sub/c.txt" done where="/tmp/claude-10001/mcpw/restore/mcp1" replace=always restorejob=McpwRestore comment="mcp restore 1" yes
Using Catalog "MyCatalog"
Bootstrap records written to /tmp/claude-10001/mcpw/dirwork/mcpw-dir.restore.1.bsr

The job will require the following
   Volume(s)                 Storage(s)                SD Device(s)
===========================================================================
   
    McpwFull-47324            McpwFile                  McpwFile                 

Volumes marked with "*" are online.


2 files selected to be restored.

Job queued. JobId=631
```

Purge mit Bestätigung. Die Frage des Directors beantwortet das Tool mit `yes`:

```json
{"name":"purge","arguments":{"kind":"jobs","volume":["McpwFull-47324"],"confirm":true}}
```

```text
> purge jobs volume=McpwFull-47324

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Found 2 Jobs for media "McpwFull-47324" in catalog.
Purge (yes/no)? yes
2 Jobss on Volume "McpwFull-47324" purged from catalog.
There are no more Jobs associated with Volume "McpwFull-47324". Marking it purged.
```

Ohne `confirm` (`{"name":"prune","arguments":{"kind":"jobs","client":"mcpw-fd"}}`), `isError: true`:

```text
prune is destructive: ask the user, then call it again with confirm=true
```

Unbekannter Job: Die Auswahlliste des Directors wird mit `.` abgebrochen, `isError: true`:

```text
> run job=NoSuchJob yes
Job "NoSuchJob" not found
The defined Job resources are:
1: McpwBackup
2: McpwRestore
Select Job resource (1-2): .
Selection aborted, nothing done.

[oconsole] The Director asked 'Select Job resource (1-2):', which this tool does not answer; it was cancelled. Give the missing argument (e.g. a valid name) and call the tool again.
```

### Tests

- S-36 (C-087): Einheitentests für die Routen mit Fake-Runnern (typisierte RPC → `ExecuteCommand` bei `UNIMPLEMENTED` → BNET-Dialog, Purge mit `yes` über gRPC), typisierte Requests und Ergebnisse, Restore-Baum über BNET-Dialog und gRPC, Freigabe-Matrix (Produktion stdio: Schreib-Tools da, `execute` nicht; HTTP in Produktion: keine Schreib-Tools, auch nicht auf Anfrage; Entwicklung HTTP mit `--mcp-allow-write`: vorhanden) und der Abdeckungstest. Integrationslauf mit `tests/mcp/mcp_rig.py` (privater `bareos-fd`/`bareos-sd`, eigener Wegwerf-Katalog aus der Bareos-DDL, Restore-Ziel im Scratchpad), jedes der 122 bzw. 123 Tools mindestens einmal über BNET und über gRPC, je 170 Aufrufe: Produktions-Director (Schreib-Tools über `ExecuteCommand`, Lese-Tools typisiert) mit Produktions-oconsole über stdio, und `grpc-development`-Director (alle Tools typisiert außer `add_volumes`, `list_nextvolume` und den damaligen Konsolenzeilen für `configure_update`/`configure_delete`/`delete storage`/`setbandwidth limit=0`, seit S-38/S-39 typisiert) mit Produktions- und Entwicklungs-oconsole. Backup, Restore in einer Zeile und interaktiver Restore (eine von drei Dateien markiert) mit Prüfung der zurückgeschriebenen Dateien; ohne unerwartete Fehler.

  ```sh
  cargo build -p onesimus-dird && cargo build -p oconsole
  python3 tests/mcp/mcp_rig.py --director target/debug/onesimus-dird --oconsole target/debug/oconsole \
      --base /tmp/m36 --port 39250 --restore-dir /tmp/m36-restored --production --report /tmp/m36.json
  ```

- S-31 (Stand vor C-087), Einheitentests (`cargo test -p oconsole` und `cargo test -p oconsole --no-default-features --features mcp-development`): Schemas und Annotationen, erzeugte Zeilen aller Tools, abgelehnte Eingaben, `confirm`, beantwortete und abgebrochene Rückfragen, Weiterlesen nach dem Socket-Zeitlimit. Dazu die Freigabe: Im Produktions-Build und im Entwicklungs-Build ohne `--mcp-allow-write` ist keines der Schreib-Tools gelistet, und jeder Aufruf endet mit `unknown tool`, ohne den Director zu erreichen. Die Snapshots `crates/oconsole/testdata/tools_list_*.json` und `tool_calls_*.json` halten `tools/list` und die Ergebnisse fester Aufrufe je Build fest.
- S-31 (Stand vor C-087), Integrationslauf: onesimus-dird mit privatem `bareos-fd` und `bareos-sd` (eigene Ports, Storage-Verzeichnis im Scratchpad) und einer Wegwerfkopie des Katalogs, oconsole `--mcp` über stdio mit einem Skript. Produktions-Build (damals): Keines der 23 Schreib-Tools und `execute` ist gelistet, alle Aufrufe enden mit `unknown tool`, und die Binärdatei enthält die Toolnamen nicht. Entwicklungs-Build mit `--mcp-allow-write`: Jedes Tool wurde mindestens einmal aufgerufen (62 Prüfungen). Dazu gehören ein Backup, vier Restores ins Scratchpad (`files`, `all` mit `strip_prefix`/`add_prefix`, `directories` mit `regexwhere`, `before` mit `fileregex`), deren Dateien danach geprüft wurden, sowie label, mount, unmount und release auf einem File-Gerät. Außerdem: update volume/pool, Abbruch eines auf 2030 eingeplanten Jobs, enable/disable für Job, Client und Schedule, setdebug, setbandwidth, `configure add` und `export`, reload, rerun, prune, purge (mit beantworteter Rückfrage), relabel, truncate und delete (jobid, volume, pool).
- Befund am Director (behoben): `label` und `relabel` schickten dem Storage Daemon einen falschen Befehl (`3903 Error scanning label command`) und schrieben den Katalog trotzdem. Seit [C-084](../COMPATIBILITY-LOG.md)/[S-32](../STUBS.md) sendet der Director die Original-Anfrage und schreibt den Katalog erst nach `3000 OK label`; geprüft gegen einen privaten `bareos-sd`. `label_volume`/`relabel_volume` geben die Ausgabe des Directors unverändert weiter. Offen bleibt nur die Option `encrypt` (erzeugt noch keinen Volume-Schlüssel).
