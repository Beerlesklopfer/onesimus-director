# setbandwidth — Bandbreite begrenzen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L454) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets bandwidth

**Originalsyntax:**

```text
setbandwidth [ client=<client-name> | storage=<storage-name> | jobid=<jobid> | job=<job-name> | ujobid=<unique-jobid> state=<job_state> | all ] limit=<nn-kbs> [ yes ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 6 von 6 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Verglichen sind Dialog/Abbruch und `setbandwidth limit=100 client=bareos-fd` in API 1/2 (setzt das Limit am gemeinsamen echten FD; die tatsächliche Drosselung wird nicht geprüft). SD-Limits und Jobselektoren sind nur durch Einheitentests belegt (NDMP: C-050).

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (6):** `c011-setbandwidth-cancel`, `c011-setbandwidth-badlimit-cancel`, `c011-setbandwidth-api2`, `c035-setbandwidth-api1`, `c035-setbandwidth-api2`, `setbandwidth-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bandwidth.rs:78](../../../crates/onesimus-dird/src/commands/bandwidth.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`setbandwidth` begrenzt die Bandbreite eines File Daemons, eines Storage Daemons oder laufender Jobs, ohne die Konfiguration zu ändern. Der Director baut dazu eine Verbindung zum Daemon auf und übermittelt das neue Limit; es gilt bis zum nächsten Neustart des Daemons bzw. bis zum Ende des Jobs. Ein Limit von 0 hebt die Begrenzung auf.

## Syntax

```text
setbandwidth [limit=<kb/s>] client[=<client-name>]
setbandwidth [limit=<kb/s>] storage[=<storage-name>]
setbandwidth [limit=<kb/s>] job=<job-name> | jobid=<jobid> | ujobid=<unique-job-name> | all | state=<zustand>
setbandwidth [limit=<kb/s>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `limit=<kb/s>` | Ganzzahl in kB/s | Neues Limit; 0 hebt es auf. Fehlt es, fragt der Director danach. |
| `client=<client-name>` | Name einer Client-Ressource | Limit für diesen File Daemon. Ohne Wert oder bei unbekanntem Namen erscheint die Auswahlliste der Clients. |
| `storage=<storage-name>` | Name einer Storage-Ressource | Limit für diesen Storage Daemon. NDMP-Storages lassen sich nicht begrenzen. |
| `job=<job-name>`, `jobid=<jobid>`, `ujobid=<unique-job-name>` | Name, JobId oder eindeutiger Jobname | Laufende Jobs auswählen, deren Bandbreite begrenzt wird. Bei Backup und Restore gilt das Limit über den File Daemon, bei Copy und Migrate über den lesenden Storage Daemon. Läuft der Job nicht, wird kein Job begrenzt; bei `job=` folgt dann die Auswahl des Clients. |
| `all`, `state=<zustand>` | – | Alle laufenden Jobs bzw. laufende Jobs in einem Zustand. |

Ohne `client`, `storage` oder Job-Auswahl fragt der Director nach dem Client.

## Dialoge

Fehlt `limit=`, fragt der Director `Enter new bandwidth limit kb/s:`. Erwartet wird eine nicht negative Ganzzahl; andere Eingaben werden abgelehnt und die Frage wiederholt. `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth
Enter new bandwidth limit kb/s: 
> x
Expected a positive integer, got: x
Enter new bandwidth limit kb/s: 
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Fehlt das Ziel, erscheint die Auswahlliste der Clients; `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth limit=100
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## API-Modi

Unter `.api 1` kommen dieselben Texte wie im Standardmodus. Unter `.api 2` fragt der Director nicht nach: `setbandwidth` ohne Limit endet sofort mit leerem `result`.

## Beispiele

### Limit für einen Client oder Storage setzen

Beispielausgabe nicht verfügbar: der Befehl würde das Bandbreitenlimit des produktiven File Daemons bzw. Storage Daemons ändern.

### Unbekannter Client

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth limit=100 client=nosuch
Error: Client resource nosuch does not exist.
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth limit=100 storage=nosuch
Storage resource "nosuch": not found
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### NDMP-Storage

Für NDMP-Storages lässt sich kein Limit setzen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth storage=ParityNdmp limit=100
Storage selected is NDMP storage which cannot have a bandwidth limit
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Limit für laufende Jobs

Läuft der angegebene Job nicht, meldet der Director das. Bei `job=` fragt er anschließend nach dem Client.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth limit=100 jobid=999999
JobId 999999 is not running. Use Job name to limit inactive jobs.
No Jobs running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth limit=100 job=backup-bareos-fd
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 2 ohne Argumente

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eingeschränkte Konsole

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> setbandwidth limit=100
setbandwidth: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Expected a positive integer, got: <eingabe>` | Bei der Frage nach dem Limit wurde keine nicht negative Ganzzahl eingegeben. |
| `Error: Client resource <name> does not exist.` | Den Client gibt es nicht oder die Konsole darf ihn nicht sehen. Es folgt die Auswahlliste. |
| `Selection aborted, nothing done.` | Auswahl mit `.` abgebrochen. |
| `Storage resource "<name>": not found` | Den Storage gibt es nicht oder die Konsole darf ihn nicht sehen. Es folgt die Auswahlliste. |
| `JobId <n> is not running. Use Job name to limit inactive jobs.` / `No Jobs running.` | Bei `jobid=` läuft kein solcher Job. |
| `Storage selected is NDMP storage which cannot have a bandwidth limit` | Der gewählte Storage spricht NDMP; dort gibt es kein Bandbreitenlimit. |
| `Failed to connect to Client.` / `Failed to connect to Storage daemon.` | Der Daemon ist nicht erreichbar. Adresse und Port der Ressource sowie den Daemon prüfen. |
| `Failed to set bandwidth limit on Client.` | Der File Daemon hat das Limit abgelehnt. Im Erfolgsfall meldet der Director `OK Limiting bandwidth to <n>kb/s <job>`. |
| `setbandwidth: is an invalid command.` | Die Konsole darf `setbandwidth` nicht ausführen (Command ACL). |

## Siehe auch

- [status](status.md) — zeigt beim Client und Storage das aktuelle `bwlimit`
- [setdebug](setdebug.md) — Debug-Level von Director, Client und Storage setzen
- [run](run.md) — Jobs starten
