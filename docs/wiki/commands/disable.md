# disable — Job, Client oder Zeitplan abschalten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L328) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Disable a job/client/schedule

**Originalsyntax:**

```text
disable job=<job-name> client=<client-name> schedule=<schedule-name>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-device, parity-limited, parity-operator.

**Paritätsfälle (30):** `c006-disable-nojob-api2`, `c006-disable-noschedule`, `c011-disable-cancel`, `c012-disable-storage`, `c025-disable-select-limited`, `c025-disable-select-limited-api2`, `c025-disable-schedule-select-limited`, `c025-disable-schedule-select-limited-api2`, `c025-disable-select-operator`, `c025-disable-select-operator-api2`, `c025-disable-select-bvfs`, `c025-disable-select-bvfs-api2`, `c025-disable-schedule-select-bvfs`, `c025-disable-schedule-select-bvfs-api2`, `c025-disable-select-acl`, `c025-disable-select-acl-api2`, `c025-disable-schedule-select-acl`, `c025-disable-schedule-select-acl-api2`, `c032-job-disable-enable`, `c032-job-disable-enable-api1`, `c032-job-disable-enable-api2`, `c032-client-disable-enable`, `c032-client-disable-enable-api2`, `c032-schedule-disable`, `c032-schedule-disable-api1`, `c032-schedule-disable-api2`, `c032-job-select-disable-enable`, `c032-limited-job`, `c032-limited-job-api2`, `disable-nojob`.

**Lokaler Handler:** [crates/odir/src/commands/job.rs:2277](../../../crates/odir/src/commands/job.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`disable` schaltet einen Job, einen Client oder einen Zeitplan (Schedule) ab. Ein abgeschalteter Job wird vom Zeitplan nicht mehr gestartet, ein abgeschalteter Client nicht mehr gesichert, ein abgeschalteter Zeitplan löst keine Jobs mehr aus. Von Hand lässt sich ein abgeschalteter Job mit [run](run.md) weiterhin starten. Die Änderung gilt nur im Speicher des laufenden Directors bis zum nächsten [reload](reload.md) oder Neustart; dauerhaft wirkt nur die Direktive `Enabled = no` in der Konfiguration.

## Syntax

```text
disable
disable job=<job>
disable client=<client>
disable schedule=<schedule>
disable job | client | schedule
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `job=<job>` | Name einer Job-Ressource | Diesen Job abschalten. |
| `client=<client>` | Name einer Client-Ressource | Diesen Client abschalten. |
| `schedule=<schedule>` | Name einer Schedule-Ressource | Diesen Zeitplan abschalten. |
| `job`, `client`, `schedule` ohne Wert | – | Auswahl unter den eingeschalteten Ressourcen dieser Art. |

Ohne Argument wählt `disable` unter den eingeschalteten Jobs. Andere Wörter (etwa `storage=`) werden übergangen; dann gilt ebenfalls die Jobauswahl.

## Dialoge

Die Auswahlliste enthält die eingeschalteten Ressourcen, die die Konsole sehen darf, und fragt `Select Job resource (1-n):` (bzw. `Select Client resource`, `Select Schedule resource`). Eine Nummer schaltet ab, `.` bricht ab (`Selection aborted, nothing done.`). Ist die Liste leer, meldet der Director `Selection list for "<Art>" is empty!`.

## API-Modi

Unter `.api 0` und `.api 1` kommt derselbe Text. Unter `.api 2` steht das Ergebnis in `result.disable` mit den Listen `jobs`, `clients` bzw. `schedules`; eine leere Auswahl liefert ein leeres `result`, ein unbekannter Name einen JSON-RPC-Fehler.

## Beispiele

### Job abschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=backup-bareos-fd
Job "backup-bareos-fd" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job aus der Liste wählen

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auswahl abbrechen

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
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Client aus der Liste wählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable client
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client resource (1-2):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Zeitplan abschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable schedule=WeeklyCycle
Schedule "WeeklyCycle" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1 und .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable schedule=WeeklyCycle
Schedule "WeeklyCycle" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable schedule=WeeklyCycle
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "disable": {
      "schedules": [
        "WeeklyCycle"
      ]
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannte Namen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=nosuch
Job "nosuch" not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable schedule=nosuch
Schedule "nosuch" not found.
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=nosuch
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

### Konsole mit eingeschränkter Job-ACL

Die Konsole `parity-device` darf nur bestimmte Jobs sehen; andere gelten als unbekannt.

```text
parity-device@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=BackupCatalog
Job "BackupCatalog" not found.
parity-device@bareos-dir[25.1.2+onesimus.0.2.0]> disable job=backup-bareos-fd
Job "backup-bareos-fd" disabled
Disabling is a temporary operation until the director reloads.
For a permanent setting, please set the value of the "Enabled"
directive in the relevant configuration resource file.
parity-device@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Job "<name>" not found.` | Unbekannter oder für die Konsole nicht sichtbarer Job. |
| `Client "<name>" not found.` | Unbekannter oder nicht sichtbarer Client. |
| `Schedule "<name>" not found.` | Unbekannter oder nicht sichtbarer Zeitplan. |
| `Selection list for "Schedule" is empty!` | Keine passende Ressource sichtbar (entsprechend für Job und Client). |
| `disable: is an invalid command.` | Die Konsole darf `disable` nicht ausführen (Command ACL). |

## Siehe auch

- [enable](enable.md) — wieder einschalten
- [show](show.md) — Konfiguration und `Enabled`-Zustand anzeigen
- [reload](reload.md) — Konfiguration neu laden (setzt `enable`/`disable` zurück)
- [status](status.md) — geplante Jobs anzeigen
