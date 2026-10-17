# cancel — Laufende Jobs abbrechen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L315) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Cancel a job

**Originalsyntax:**

```text
cancel storage=<storage-name> | jobid=<jobid> | job=<job-name> | ujobid=<unique-jobid> | state=<job_state> | all yes
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 24 von 24 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Verglichen sind Selektion und Fehlertexte nur mit nicht existierenden bzw. nicht laufenden Jobs. Abbruch laufender Jobs über FD/SD ist nur durch Einheitentests belegt.
- RunScripts „AfterJob“ bei Cancel und Warte-Zustände fehlen (C-048).

**Offene Logeinträge:** `C-048` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (24):** `c006-cancel-none-api2`, `c006-cancel-noargs-api2`, `c006-cancel-job`, `c006-cancel-ujobid`, `c006-cancel-badjobid`, `c006-cancel-all`, `c034-storage-nosuch-job`, `c034-storage-nosuch-job-api2`, `c034-storage-badjobid`, `c034-storage-nojobid`, `c034-storage-jobid-novalue`, `c034-storage-unknown`, `c034-storage-after-jobid`, `c034-storage-novalue`, `c034-sd-keyword`, `c034-state-running`, `c034-state-idle`, `c034-state-novalue`, `c034-all-yes`, `c034-all-api1`, `c035-cancel-job-nosuch-api1`, `c035-cancel-job-nosuch-api2`, `cancel-none`, `cancel-dialog`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/cancel.rs:63](../../../crates/onesimus-dird/src/commands/cancel.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`cancel` bricht Jobs ab, die laufen oder auf ihren Start warten. Der Director markiert den Job als abgebrochen, benachrichtigt beteiligte File und Storage Daemons und schließt den Job mit dem Status `A` (Canceled) im Katalog ab. Ein Job, der auf seine geplante Startzeit wartet, endet spätestens nach 30 Sekunden. Bereits beendete Jobs lassen sich nicht abbrechen. Mit `storage=` und `jobid=` kann man zusätzlich einen Job direkt im Storage Daemon abbrechen, den der Director selbst nicht mehr kennt.

## Syntax

```text
cancel
cancel jobid=<n> [yes]
cancel job=<name> [yes]
cancel ujobid=<eindeutiger-jobname> [yes]
cancel all [yes]
cancel state=created|blocked|waiting|running [yes]
cancel storage=<storage> jobid=<n>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<n>` | JobId | Den laufenden Job mit dieser JobId abbrechen. |
| `job=<name>` | Name der Job-Ressource | Laufende Jobs dieses Namens; bei mehreren folgt eine Auswahl. |
| `ujobid=<name>` | eindeutiger Jobname, z. B. `parity-admin.2026-10-16_22.50.21_06` | Genau diesen Job abbrechen. |
| `all` | – | Alle laufenden und wartenden Jobs. |
| `state=<zustand>` | `created`, `blocked`, `waiting`, `running` | Alle Jobs in diesem Zustand abbrechen. |
| `storage=<storage>`, `sd=<storage>` | Name einer Storage-Ressource | Abbruch direkt im Storage Daemon; braucht `jobid=`. |
| `yes` | – | Ohne Bestätigungsfrage abbrechen. |

Ohne Argument zeigt der Director die laufenden Jobs zur Auswahl.

## Dialoge

- Gibt es genau einen passenden Job, meldet der Director `Automatically selected Job: …` bzw. `Selected Job <n> for cancelling` und fragt `Confirm cancel (yes/no):`. Nur `yes` bricht ab; `no` und `.` lassen den Job weiterlaufen.
- Bei mehreren laufenden Jobs folgt eine Auswahlliste; `.` bricht die Auswahl ab.
- Mit `yes` als Argument entfällt die Bestätigung.
- Gibt es keine laufenden Jobs, antwortet der Director `No Jobs running.`

## API-Modi

Unter `.api 0` und `.api 1` erscheinen dieselben Texte. Unter `.api 2` gibt es keine Rückfragen; ein Abbruch ohne Treffer liefert ein leeres `result`, eine nicht laufende JobId einen JSON-RPC-Fehler mit dem Text in `messages.error`.

## Beispiele

### Wartenden Job über den Namen abbrechen

Der Admin-Job wartet auf seine Startzeit im Jahr 2099 und wird dann abgebrochen; die Abschlussmeldung kommt nach spätestens 30 Sekunden.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin when="2099-01-01 00:00:00" yes
Using Catalog "MyCatalog"
Job queued. JobId=635
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Später in derselben Sitzung:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list jobs jobstatus=C
+-------+--------------+-----------+-----------+----------+------+-------+----------+----------+-----------+
| jobid | name         | client    | starttime | duration | type | level | jobfiles | jobbytes | jobstatus |
+-------+--------------+-----------+-----------+----------+------+-------+----------+----------+-----------+
|   635 | parity-admin | bareos-fd |           |          | D    |       |        0 |        0 | C         |
+-------+--------------+-----------+-----------+----------+------+-------+----------+----------+-----------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel job=parity-admin
Automatically selected Job: JobId=635 Job=parity-admin.2026-10-16_22.50.21_06
Confirm cancel (yes/no):
> yes
JobId 635, Job parity-admin.2026-10-16_22.50.21_06 marked to be canceled.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait
You have messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
16-Oct 22:50 bareos-dir JobId 635: Version: 25.1.2+onesimus.0.2.0 (16 October 2026) Debian GNU/Linux 13 (trixie)
16-Oct 22:50 bareos-dir JobId 635: BAREOS 25.1.2+onesimus.0.2.0 (16Oct26): 16-Oct-2026 22:50:53
  JobId:                  635
  Job:                    parity-admin.2026-10-16_22.50.21_06
  Scheduled time:         01-Jan-2099 00:00:00
  Start time:             16-Oct-2026 22:50:21
  End time:               16-Oct-2026 22:50:53
  Bareos binary info:     Onesimus build: Support: https://onesimus.io
  Job triggered by:       User
  Termination:            Admin Canceled

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alle Jobs abbrechen

`cancel all` fragt für jeden Job einzeln nach. `messages` zeigt hier auch die Meldungen des Admin-Jobs 630, der kurz vorher gelaufen ist.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> run job=parity-admin when="2099-01-01 00:00:00" yes
Using Catalog "MyCatalog"
Job queued. JobId=631
You have messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel all
Selected Job 631 for cancelling
Confirm cancel (yes/no):
> yes
JobId 631, Job parity-admin.2026-10-16_23.09.29_02 marked to be canceled.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
16-Oct 23:09 bareos-dir JobId 630: Version: 25.1.2+onesimus.0.2.0 (16 October 2026) Debian GNU/Linux 13 (trixie)
16-Oct 23:09 bareos-dir JobId 630: Start Admin JobId 630, Job=parity-admin.2026-10-16_23.09.26_01
16-Oct 23:09 bareos-dir JobId 630: BAREOS 25.1.2+onesimus.0.2.0 (16Oct26): 16-Oct-2026 23:09:29
  JobId:                  630
  Job:                    parity-admin.2026-10-16_23.09.26_01
  Scheduled time:         16-Oct-2026 23:09:26
  Start time:             16-Oct-2026 23:09:28
  End time:               16-Oct-2026 23:09:29
  Bareos binary info:     Onesimus build: Support: https://onesimus.io
  Job triggered by:       User
  Termination:            Admin OK

16-Oct 23:09 bareos-dir JobId 631: Job parity-admin.2026-10-16_23.09.29_02 waiting 2278716631 seconds for scheduled start time.
16-Oct 23:09 bareos-dir JobId 631: Version: 25.1.2+onesimus.0.2.0 (16 October 2026) Debian GNU/Linux 13 (trixie)
16-Oct 23:09 bareos-dir JobId 631: BAREOS 25.1.2+onesimus.0.2.0 (16Oct26): 16-Oct-2026 23:09:59
  JobId:                  631
  Job:                    parity-admin.2026-10-16_23.09.29_02
  Scheduled time:         01-Jan-2099 00:00:00
  Start time:             16-Oct-2026 23:09:29
  End time:               16-Oct-2026 23:09:59
  Bareos binary info:     Onesimus build: Support: https://onesimus.io
  Job triggered by:       User
  Termination:            Admin Canceled

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Keine laufenden Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel
No Jobs running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel all
No Jobs running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JobId, die nicht läuft

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel jobid=999999
JobId 999999 is not running. Use Job name to cancel inactive jobs.
No Jobs running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter eindeutiger Jobname

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel ujobid=nosuch
Warning Job nosuch is not running. Continuing anyway ...
No Jobs running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Abbruch im Storage Daemon

Der Storage Daemon antwortet selbst, hier mit `3904`, weil er keinen Job mit dieser Nummer kennt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel storage=File jobid=999999
3904 Job 999999 not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel storage=File
Missing jobid=JobId specification
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> cancel jobid=999999
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
          "JobId 999999 is not running. Use Job name to cancel inactive jobs.\n"
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
| `No Jobs running.` | Es läuft oder wartet kein passender Job. |
| `JobId <n> is not running. Use Job name to cancel inactive jobs.` | Die JobId läuft nicht (beendet oder nie gestartet). |
| `Warning Job <name> is not running. Continuing anyway ...` | `ujobid=` nennt keinen laufenden Job. |
| `Missing jobid=JobId specification` | `storage=` ohne `jobid=`. |
| `JobId <wert> not a number` | `storage=` mit nicht numerischer JobId. |
| `Illegal state either created, blocked, waiting or running` | `state=` mit unbekanntem Wert (nur geprüft, wenn Jobs laufen). |
| `3904 Job <n> not found.` | Antwort des Storage Daemons: dort läuft kein solcher Job. |
| `Storage resource "<name>": not found` | Unbekannter Storage; es folgt die Storage-Auswahl. |

## Siehe auch

- [run](run.md) — Jobs starten und einplanen
- [status](status.md) — laufende und wartende Jobs anzeigen
- [wait](wait.md) — auf das Ende von Jobs warten
- [messages](messages.md) — Abschlussmeldung des abgebrochenen Jobs lesen
- [list](list.md) — Jobstatus im Katalog prüfen
