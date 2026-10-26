# estimate — Umfang eines Backups schätzen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L334) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Performs FileSet estimate, listing gives full listing

**Originalsyntax:**

```text
estimate fileset=<fileset-name> client=<client-name> level=<level> accurate=<yes/no> job=<job-name> listing
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 38 von 38 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Fragt den echten FD ab (lesend).

**Paritätsfälle (38):** `c011-estimate-cancel`, `c017-full`, `c017-default-level`, `c017-incremental`, `c017-differential`, `c017-catalog-job`, `c017-listing`, `c017-listing-full`, `c017-accurate-yes`, `c017-accurate-incr`, `c017-accurate-no`, `c017-accurate-bad`, `c017-accurate-missing`, `c017-unknown-job`, `c017-job-missing`, `c017-restore-job`, `c017-client-override`, `c017-fd-override`, `c017-fileset-override`, `c017-client-fileset-nojob`, `c017-unknown-client`, `c017-client-missing`, `c017-unknown-fileset`, `c017-fileset-missing`, `c017-bad-level`, `c017-level-missing`, `c017-twice`, `c017-api2`, `c017-api2-unknown-job`, `c017-select-job`, `c041-estimate-backupcatalog`, `c041-estimate-backupfd`, `estimate`, `estimate-listing`, `w011-estimate-badnumber-cancel`, `w011-estimate-level-cancel`, `w011-estimate-api1-cancel`, `w011-estimate-api2`.

**Lokaler Handler:** [crates/odir/src/commands/estimate.rs:146](../../../crates/odir/src/commands/estimate.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`estimate` fragt den File Daemon, wie viele Dateien und Bytes ein Backup-Job mit dem gewählten Level sichern würde, ohne einen Job zu starten. Der Director verbindet sich dazu mit dem Client, übergibt FileSet und Level und gibt die Antwort des File Daemons aus. Mit `listing` kommt zusätzlich jede Datei, die gesichert würde. Der Befehl verändert weder Katalog noch Daten; für Incremental und Differential liest er nur den letzten passenden Job aus dem Katalog.

## Syntax

```text
estimate
estimate job=<job> [level=<level>] [client=<client>] [fileset=<fileset>] [accurate=yes|no] [listing]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `job=<job>` | Name eines Backup-Jobs | Job, dessen Client, FileSet und Level verwendet werden. Pflicht; fehlt er, folgt eine Auswahl. |
| `level=<level>` | `Full`, `Incremental`, `Differential` | Level der Schätzung; ohne Angabe das Level des Jobs. |
| `client=<client>`, `fd=<client>` | Name einer Client-Ressource | Anderer Client als im Job. Nur native Clients (kein NDMP). |
| `fileset=<fileset>` | Name einer FileSet-Ressource | Anderes FileSet als im Job. |
| `accurate=yes\|no` | `yes`, `no` | Accurate-Modus für die Schätzung. |
| `listing` | – | Jede Datei ausgeben. |

## Dialoge

Ohne `job=` zeigt der Director alle Jobs und fragt `Select Job resource (1-n):`. Eine Nummer startet die Schätzung, `0` oder eine zu große Zahl führt zu `Please enter a number between 1 and n`, `.` bricht ab.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Ausgabe. Unter `.api 2` liefert eine erfolgreiche Schätzung ein leeres `result` (wie im Original); Fehler kommen als JSON-RPC-Fehler mit dem Text in `messages.error`.

## Beispiele

### Schätzung für einen Job

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
2000 OK estimate files=597 bytes=176,344,408
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Bestimmtes Level

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd level=Full
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
2000 OK estimate files=597 bytes=176,344,408
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd level=Differential
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
2000 OK estimate files=181 bytes=126,957,165
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Anderes FileSet

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd fileset=Catalog
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
2000 OK estimate files=68 bytes=15,675
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit Dateiliste

Die Liste ist hier gekürzt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=BackupCatalog listing
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
drwxr-xr-x   1 bareos   bareos             0 2026-03-21 10:38:56  /etc/bareos/bareos-dir-export/client
drwxr-xr-x   1 bareos   bareos            12 2026-03-21 10:38:56  /etc/bareos/bareos-dir-export
-rw-r-----   1 bareos   bareos            94 2026-03-21 16:06:13  /etc/bareos/bareos-dir.d/catalog/MyCatalog.conf
[…]
drwxr-sr-x   1 bareos   bareos           258 2026-10-15 13:48:03  /etc/bareos
2000 OK estimate files=68 bytes=15,675
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job aus der Liste wählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate
The defined Job resources are:
 1: backup-bareos-fd
 2: BackupCatalog
 3: parity-admin
 4: parity-consolidate
 5: parity-copy
 6: parity-migrate
 7: parity-restore-regex
 8: parity-verify
 9: RestoreFiles
Select Job resource (1-9):
> 2
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
2000 OK estimate files=68 bytes=15,675
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige Werte

Ein ungültiges Level oder ein ungültiger Accurate-Wert wird gemeldet; die Schätzung läuft dann mit den Werten des Jobs.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd level=bogus
Level "bogus" not valid.
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
2000 OK estimate files=597 bytes=176,344,408
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd accurate=maybe
Invalid value for accurate. It must be yes or no.
Using Catalog "MyCatalog"
Connecting to Client bareos-fd at localhost:9102
2000 OK estimate files=597 bytes=176,344,408
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlende Werte und falsche Jobtypen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd level
Level value missing.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=RestoreFiles
Wrong job specified of type Restore.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd client=parity-ndmp-fd
Estimate is only supported on native clients.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=backup-bareos-fd
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> estimate job=nosuch
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
          "Job \"nosuch\" not found.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Job "<name>" not found.` | Unbekannter Job. |
| `Job name missing.` / `Client name missing.` / `Fileset name missing.` / `Level value missing.` / `Accurate value missing.` | Schlüsselwort ohne `=wert`. |
| `Client "<name>" not found.` / `Fileset "<name>" not found.` | Unbekannter Client bzw. unbekanntes FileSet. |
| `Level "<level>" not valid.` | Ungültiges Level; die Schätzung nutzt das Level des Jobs. |
| `Invalid value for accurate. It must be yes or no.` | `accurate=` mit anderem Wert; der Wert des Jobs gilt. |
| `No job specified.` | `client=`/`fileset=` ohne `job=`. |
| `Wrong job specified of type Restore.` | Nur Backup-Jobs lassen sich schätzen. |
| `Estimate is only supported on native clients.` | Der Client spricht NDMP. |
| `2000 OK estimate files=<n> bytes=<n>` | Erfolgreiche Antwort des File Daemons. |

## Siehe auch

- [run](run.md) — Backup tatsächlich starten
- [status](status.md) — Client-Status abfragen (`status client=<client>`)
- [show](show.md) — FileSet-Definition ansehen
