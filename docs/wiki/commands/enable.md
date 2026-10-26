# enable — Job, Client oder Zeitplan einschalten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L331) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Enable a job/client/schedule

**Originalsyntax:**

```text
enable job=<job-name> client=<client-name> schedule=<schedule-name>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 29 von 29 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-device, parity-limited, parity-operator.

**Paritätsfälle (29):** `c006-enable-nojob-api2`, `c006-enable-noclient`, `c011-enable-cancel`, `c012-enable-storage`, `c025-enable-select-limited`, `c025-enable-select-limited-api2`, `c025-enable-client-select-limited`, `c025-enable-client-select-limited-api2`, `c025-enable-select-operator`, `c025-enable-select-operator-api2`, `c025-enable-client-select-operator`, `c025-enable-client-select-operator-api2`, `c025-enable-select-bvfs`, `c025-enable-select-bvfs-api2`, `c025-enable-client-select-bvfs`, `c025-enable-client-select-bvfs-api2`, `c025-enable-select-acl`, `c025-enable-select-acl-api2`, `c025-enable-client-select-acl`, `c025-enable-client-select-acl-api2`, `c032-job-disable-enable`, `c032-job-disable-enable-api1`, `c032-job-disable-enable-api2`, `c032-client-disable-enable`, `c032-client-disable-enable-api2`, `c032-job-select-disable-enable`, `c032-limited-job`, `c032-limited-job-api2`, `enable-nojob`.

**Lokaler Handler:** [crates/odir/src/commands/job.rs:2273](../../../crates/odir/src/commands/job.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`enable` schaltet einen zuvor mit [disable](disable.md) abgeschalteten Job, Client oder Zeitplan (Schedule) wieder ein. Ein eingeschalteter Job wird vom Zeitplan wieder gestartet, ein eingeschalteter Client wieder gesichert. Die Änderung gilt nur im Speicher des laufenden Directors und geht beim nächsten [reload](reload.md) oder Neustart verloren; dauerhaft wirkt nur die Direktive `Enabled` in der Konfiguration. Der Katalog wird nicht verändert.

## Syntax

```text
enable
enable job=<job>
enable client=<client>
enable schedule=<schedule>
enable job | client | schedule
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `job=<job>` | Name einer Job-Ressource | Diesen Job einschalten. |
| `client=<client>` | Name einer Client-Ressource | Diesen Client einschalten. |
| `schedule=<schedule>` | Name einer Schedule-Ressource | Diesen Zeitplan einschalten. |
| `job`, `client`, `schedule` ohne Wert | – | Auswahl unter den abgeschalteten Ressourcen dieser Art. |

Ohne Argument wählt `enable` unter den abgeschalteten Jobs. Andere Wörter (etwa `storage=`) werden übergangen; dann gilt ebenfalls die Jobauswahl.

## Dialoge

Die Auswahlliste enthält nur abgeschaltete Ressourcen, die die Konsole sehen darf. Ist es genau eine, wählt der Director sie ohne Frage (`Automatically selected Job: …`). Ist es keine, meldet er `Selection list for "Job" is empty!` und beendet den Befehl. Sonst fragt er `Select Job resource (1-n):`; `.` bricht ab.

## API-Modi

Unter `.api 0` und `.api 1` kommt derselbe Text. Unter `.api 2` steht das Ergebnis in `result.enable` mit den Listen `jobs`, `clients` bzw. `schedules`; eine leere Auswahl liefert ein leeres `result`, ein unbekannter Name einen JSON-RPC-Fehler.

## Beispiele

### Job ein- und ausschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=backup-bareos-fd
Job "backup-bareos-fd" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable job=backup-bareos-fd
Job "backup-bareos-fd" enabled
Enabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Aus mehreren abgeschalteten Jobs auswählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=BackupCatalog
Job "BackupCatalog" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=parity-admin
Job "parity-admin" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable
The defined Job resources are:
1: BackupCatalog
2: parity-admin
Select Job resource (1-2):
> 2
Job "parity-admin" enabled
Enabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable job=BackupCatalog
Job "BackupCatalog" enabled
Enabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Einzigen abgeschalteten Job automatisch wählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable
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
> 1
Job "backup-bareos-fd" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable
Automatically selected Job: backup-bareos-fd
Job "backup-bareos-fd" enabled
Enabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nichts abgeschaltet

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable
The defined Job resources are:
Selection list for "Job" is empty!
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Client einschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable client=bareos-fd
Client "bareos-fd" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable client=bareos-fd
Client "bareos-fd" enabled
Enabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Zeitplan einschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable schedule=WeeklyCycle
Schedule "WeeklyCycle" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable schedule=WeeklyCycle
Schedule "WeeklyCycle" enabled
Enabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable client=bareos-fd
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "enable": {
      "clients": [
        "bareos-fd"
      ]
    }
  }
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable schedule=ParityPreview
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "enable": {
      "schedules": [
        "ParityPreview"
      ]
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Job unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> enable job=nosuch
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

### Konsole ohne enable-Recht

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> enable
enable: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Job "<name>" not found.` | Unbekannter oder für die Konsole nicht sichtbarer Job. |
| `Client "<name>" not found.` | Unbekannter oder nicht sichtbarer Client. |
| `Schedule "<name>" not found.` | Unbekannter oder nicht sichtbarer Zeitplan. |
| `Selection list for "Job" is empty!` | Es ist kein Job abgeschaltet (entsprechend für Client und Schedule). |
| `enable: is an invalid command.` | Die Konsole darf `enable` nicht ausführen (Command ACL). |

## Siehe auch

- [disable](disable.md) — Job, Client oder Zeitplan abschalten
- [show](show.md) — Konfiguration und `Enabled`-Zustand anzeigen
- [reload](reload.md) — Konfiguration neu laden (setzt `enable`/`disable` zurück)
- [run](run.md) — Job von Hand starten (auch wenn er abgeschaltet ist)
