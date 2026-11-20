# purge — Einträge ohne Rücksicht auf Aufbewahrungszeiten entfernen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L390) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Purge records from catalog

**Originalsyntax:**

```text
purge [files [job=<job> | jobid=<jobid> | client=<client> | volume=<volume>]] | [jobs [client=<client> | volume=<volume>] | pool=<pool>] [jobstatus=<status>]] | [volume[=<volume>] [storage=<storage>] [pool=<pool> | allpools] [devicetype=<type>] [drive=<drivenum>] [action=<action>]] | [quota [client=<client>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 34 von 34 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-c030-client, parity-c030-job, parity-c030-pool, parity-c030-storage.
- `action=` (ActionOnPurge, Relabel über SD) fehlt (C-049); `action=truncate` fehlt (C-053).
- Verglichen sind Dialoge, ACL-Prüfungen und Bestätigungstexte auf der Katalogkopie; der Katalogzustand danach wird nicht verglichen.

**Offene Logeinträge:** `C-049`, `C-053` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (34):** `c011-purge-cancel`, `c011-purge-volume-cancel`, `c011-purge-jobs-cancel`, `c011-purge-files-cancel`, `c011-purge-quota-cancel`, `c011-purge-volume-select-cancel`, `c011-purge-api1-cancel`, `c012-purge-jobs-menu`, `c030-purge-job`, `c030-purge-storage`, `c030-purge-client`, `c030-purge-pool`, `c030-purge-client-api1`, `c030-purge-pool-api2`, `c030-purge-jobs-client-no`, `c030-purge-jobs-client-cancel`, `c030-purge-jobs-client-invalid-answer`, `c030-purge-jobs-client-api1`, `c030-purge-jobs-client-api2`, `c030-purge-jobs-client-jobstatus-bad`, `c030-purge-jobs-client-jobstatus-no`, `c030-purge-files-client-no`, `c030-purge-files-menu-client-no`, `c030-purge-jobs-pool-no`, `c030-purge-jobs-pool-incremental-no`, `c030-purge-volume-no`, `c030-purge-volume-error-no`, `c030-purge-volume-jobstatus-bad`, `c030-purge-volume-jobid`, `c030-purge-volume-two-no`, `c030-purge-volume-menu-no`, `c030-purge-volume-api1`, `c030-purge-quota-client`, `purge-dialog`.

**Lokaler Handler:** [crates/odir/src/commands/admin.rs:1239](../../../crates/odir/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`purge` entfernt Dateieinträge oder ganze Jobs aus dem Katalog, ohne die Aufbewahrungszeiten zu beachten: für einen Job, einen Client, ein Volume oder einen Pool. Ein Volume, auf dem danach keine Jobs mehr stehen, bekommt den Status `Purged` und kann wiederverwendet werden. Mit `action=` lassen sich solche Volumes zusätzlich über den Storage Daemon behandeln (ActionOnPurge, etwa `truncate`). Der Befehl ist gefährlich: er warnt bei jedem Aufruf, braucht vollen Zugriff auf alle Ressourcen und lässt sich nicht rückgängig machen. Im Normalfall ist [prune](prune.md) die richtige Wahl.

## Syntax

```text
purge
purge files [job=<job> | jobid=<n> | client=<client> | volume=<volume>]
purge jobs [client=<client> | volume=<volume> | pool=<pool>] [jobstatus=<status>[,<status>…]]
purge volume[=<volume>] [volume=<volume> …] [jobstatus=<status>]
purge volume[=<volume>] action=<aktion> [storage=<storage>] [pool=<pool> | allpools] [devicetype=<typ>] [drive=<n>]
purge quota [client=<client>]
purge volume all pool=<pool> | allpools [volstatus=<status>[,<status>…]] dryrun | yes   # Onesimus-Erweiterung
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `files` | – | Nur Dateieinträge entfernen, Jobs bleiben. |
| `jobs` | – | Jobs mit allen Einträgen entfernen. |
| `volume[=<volume>]` | Volume-Name, mehrfach möglich | Alle Jobs auf diesem Volume entfernen; ohne Wert Auswahl. |
| `quota` | – | Quota-Zähler eines Clients zurücksetzen. |
| `job=<job>`, `jobid=<n>` | Name bzw. JobId | Mit `files`: Dateien dieses Jobs. |
| `client=<client>` | Client | Einträge dieses Clients. |
| `pool=<pool>` | Pool | Mit `jobs`: Jobs dieses Pools; mit `action=`: Volumes dieses Pools. |
| `jobstatus=<liste>` | Statusbuchstaben (`T`, `E`, `f`, …) oder Namen wie `error`, durch Komma getrennt | Nur Jobs mit diesem Status. |
| `action=<aktion>` | `truncate`, `all` | ActionOnPurge für Volumes im Status `Purged` ausführen; ein Name mit unzulässigen Zeichen gilt als `all`. |
| `storage=<storage>` | Storage | Mit `action=`: Storage, über den die Volumes behandelt werden; sonst Auswahl. |
| `allpools` | – | Mit `action=`: Volumes aller Pools. |
| `devicetype=<typ>`, `drive=<n>` | Gerätetyp, Laufwerksnummer | Mit `action=`: Gerät im Storage Daemon. |

## Dialoge

- Jeder Aufruf beginnt mit der Warnung `This command can be DANGEROUS!!!`.
- Ohne Art: `Choose item to purge (1-4):` (files, jobs, volume, quota); `.` bricht ab.
- Fehlt der Client oder das Volume, folgt die Auswahl (Client-Liste bzw. Pool- und Volume-Auswahl).
- Vor dem Löschen zählt der Director die Jobs (`Found <n> Jobs for …`) und fragt `Purge (yes/no)?`. Nur `yes` löscht; `no` und `.` melden `Purge canceled.`, andere Eingaben wiederholen die Frage.
- Bei mehreren `volume=` kommt die Frage für jedes Volume einzeln.

## API-Modi

Unter `.api 1` fehlt die Rückfrage; der Director meldet `Purge canceled.` Unter `.api 2` kommt ebenfalls keine Rückfrage, das Ergebnis ist ein leeres `result`; Fehler kommen als JSON-RPC-Fehler, die Warnung steht dabei in `messages.warning`.

## Beispiele

### Jobs eines Volumes entfernen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge volume=Full-47321

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Found 15 Jobs for media "Full-47321" in catalog.
Purge (yes/no)?
> yes
15 Jobss on Volume "Full-47321" purged from catalog.
There are no more Jobs associated with Volume "Full-47321". Marking it purged.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobs eines Clients mit bestimmtem Status

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge jobs client=bareos-fd jobstatus=E

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Using Catalog "MyCatalog"
Begin purging jobs from Client "bareos-fd"
Found 3 Jobs for client "bareos-fd" in catalog "MyCatalog".
Purge (yes/no)?
> yes
Purging the following 3 JobIds: 2,4,6
3 Jobs on client "bareos-fd" purged from catalog.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Rückfrage ablehnen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge jobs pool=Full

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Found 44 Jobs for pool "Full" in catalog.
Purge (yes/no)?
> no
Purge canceled.
No Media found for pool Full.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige Antwort

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge jobs client=bareos-fd

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Using Catalog "MyCatalog"
Begin purging jobs from Client "bareos-fd"
Found 161 Jobs for client "bareos-fd" in catalog "MyCatalog".
Purge (yes/no)?
> maybe
Purge (yes/no)?
> no
Purge canceled.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Volumes

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge volume=Full-47321 volume=Incremental-47322

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Found 15 Jobs for media "Full-47321" in catalog.
Purge (yes/no)?
> no
Purge canceled.

Found 10 Jobs for media "Incremental-47322" in catalog.
Purge (yes/no)?
> no
Purge canceled.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Volume über das Menü

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
You have the following choices:
1: files
2: jobs
3: volume
4: quota
Choose item to purge (1-4):
> 3
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
> 2
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
| mediaid | volumename | volstatus | enabled | volbytes       | volfiles | volretention | recycle | slot | inchanger | mediatype | lastwritten         | storage | hasencryptionkey |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
|  47,318 | Full-0001  | Error     |       1 | 23,902,210,671 |        5 |            0 |       1 |    0 |         0 | File      | 2026-03-30 22:54:15 | File    |                0 |
|  47,320 | Full-0002  | Error     |       1 |    124,004,811 |        0 |            0 |       1 |    0 |         0 | File      |                     |         |                0 |
|  47,321 | Full-47321 | Append    |       1 |    158,201,165 |        0 |   31,536,000 |       1 |    0 |         0 | File      | 2026-07-10 21:10:05 | File    |                0 |
+---------+------------+-----------+---------+----------------+----------+--------------+---------+------+-----------+-----------+---------------------+---------+------------------+
Enter the volume name or MediaId of the volume prefixed with an asterisk (*).
E.g. "full-0001" or "*42":
> Incremental-47322
Found 10 Jobs for media "Incremental-47322" in catalog.
Purge (yes/no)?
> no
Purge canceled.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dateien eines Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge files jobid=599

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Quota zurücksetzen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge quota client=bareos-fd

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Using Catalog "MyCatalog"
Purged quota for Client "bareos-fd"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### ActionOnPurge

Ohne Volumes im Status `Purged` gibt es nichts zu tun:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge volume=c049-none action=truncate storage=File pool=Full

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
cats/sql_get.cc:1177 Media record for Volume "c049-none" not found.

Using Catalog "MyCatalog"
No Volumes found to perform truncate action.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beispielausgabe nicht verfügbar: eine ausgeführte Aktion würde Volumes über den echten Storage Daemon neu beschriften.

### Unter .api 1 und .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge jobs client=bareos-fd

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Begin purging jobs from Client "bareos-fd"
Found 161 Jobs for client "bareos-fd" in catalog "MyCatalog".
Purge canceled.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
parity-c030-pool@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
parity-c030-pool@bareos-dir[25.1.2+onesimus.0.2.0]> purge
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "warning": [
          "\nThis command can be DANGEROUS!!!\n\nIt purges (deletes) all Files from a Job,\nJobId, Client or Volume; or it purges (deletes)\nall Jobs from a Client or Volume without regard\nto retention periods. Normally you should use the\nPRUNE command, which respects retention periods.\nThis command requires full access to all resources.\n"
        ],
        "error": [
          "Permission denied: need full pool permission.\n"
        ]
      }
    }
  }
}
parity-c030-pool@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Permission denied: need full <job\|client\|storage\|pool> permission.` | `purge` braucht vollen Zugriff auf alle Ressourcen. |
| `invalid jobstatus parameter` | Unbekannter Wert in `jobstatus=`. |
| `Purge canceled.` | Rückfrage mit `no` oder `.` beantwortet, oder API-Modus. |
| `No Jobs found for media <volume> to purge from catalog.` | Auf dem Volume stehen keine passenden Jobs. |
| `No Media found for pool <pool>.` | Der Pool hat keine Volumes. |
| `cats/sql_get.cc:1177 Media record for Volume "<name>" not found.` | Volume nicht im Katalog. |
| `No Volumes found to perform <aktion> action.` | Kein Volume im Status `Purged` mit dieser ActionOnPurge. |
| `Error: Pool resource "<name>" does not exist.` | Unbekannter Pool bei `action=`; es folgt die Pool-Auswahl. |
| `Storage resource "<name>": not found` | Unbekannter Storage bei `action=`; es folgt die Storage-Auswahl. |

## Mehrere Volumes auf einmal (Onesimus-Erweiterung)

**Nicht in Bareos.** `bareos-dir` kennt diese Form nicht; Entscheidung des Nutzers vom 20.11.2026 („Ja, mit Vorschau“).

```text
purge volume all pool=<pool> | allpools [volstatus=<status>[,<status>…]] dryrun | yes
```

Purgt alle Volumes eines Pools (oder aller Pools) mit dem gewählten VolStatus auf einmal. Wie jedes `purge` ignoriert das die Aufbewahrungszeiten und löscht die Job- und Dateieinträge aus dem Katalog. Deshalb gibt es immer zuerst eine Vorschau:

| Argument | Bedeutung |
|---|---|
| `all` | Das bloße Wort wählt die Erweiterung. `action=all` ist etwas anderes (ActionOnPurge) und bleibt unverändert. |
| `pool=<pool>` \| `allpools` | Genau eines von beiden ist Pflicht. Ein Pool, den die Pool-ACL der Konsole nicht erlaubt, gilt als unbekannt; bei `allpools` fallen seine Volumes weg. |
| `volstatus=<liste>` | Nur Volumes mit diesem VolStatus, durch Komma getrennt: `Full`, `Used`, `Error`, `Append`, `Archive`, `Disabled`, `Read-Only`. Ohne Angabe: `Full`, `Used`, `Error`. Volumes im Status `Purged` oder `Recycle` werden nie angefasst. |
| `dryrun` | Vorschau: zeigt je Volume, was geschähe; ändert nichts. |
| `yes` | Purgt. Genau eines von `dryrun` und `yes` ist Pflicht; ohne beide lehnt der Director ab und fragt **nicht** nach. |

Andere Argumente (`volume=<name>`, `action=`, `jobstatus=`, `storage=` …) werden mit dieser Form abgelehnt.

**Sicherheit.** Jedes Volume läuft mit `yes` durch genau denselben Code wie `purge volume=<name> yes`, mit allen Prüfungen. Lehnt diese Form ein Volume ab, überspringt die Sammelform es mit dem Grund; sie umgeht keine Prüfung. Die Vorschau stellt dieselben Prüfungen, ohne etwas zu schreiben. Mögliche Gründe für `skipped`:

- `Volume "<v>" has VolStatus "<s>" and cannot be purged. The VolStatus must be: Append, Full, Used or Error to be purged.` – z. B. bei `volstatus=Archive`, `Disabled`, `Read-Only`.
- `Volume "<v>" holds the latest Full chain of client "<c>" FileSet "<f>" (JobIds …); not purged.` – die jüngste Kette jedes Clients bleibt immer erhalten (siehe [Betrieb](../betrieb-odir.md#autoprune-am-job-ende)).
- `Cannot determine whether volume "<v>" holds the latest Full chain of a client (…); not purged.` – Katalogfehler bei der Prüfung.
- `in use by running job <JobIds>` – ein laufender Job hat schon auf das Volume geschrieben (JobMedia). Ein Volume, auf das ein laufender Job erst schreiben wird oder das nur zum Schreiben gemountet ist, erkennt der Director nicht; dafür sorgt die Statusauswahl (`Append` ist nicht in der Vorgabe).
- `no longer in pool <p>`, `VolStatus changed to <s>` – das Volume hat sich zwischen Auswahl und Purge geändert.
- ein Katalogfehler beim Löschen (das Volume bleibt unverändert, die Transaktion wird zurückgerollt).

Die Zahlen je Volume stammen aus dem Katalog: Jobs = verschiedene JobIds in JobMedia, Dateien = Summe von `Job.JobFiles` dieser Jobs, Bytes = `Media.VolBytes`. Bei `yes` sind es die Zahlen vor dem Purge.

**Ausgabe (Text, `.api 1`).** Nach Warnung und Katalogauswahl eine Zeile je Volume und eine Zusammenfassung (Beispiel nach den Tests):

```text
*purge volume all pool=Full dryrun
…
Volume "Full-A" (Pool Full, Full, 2 jobs, 12 files): would purge
Volume "Full-B" (Pool Full, Full, 1 jobs, 5 files): skipped: Volume "Full-B" holds the latest Full chain of client "fd1" FileSet "fs" (JobIds 3); not purged.
Volume "Full-C" (Pool Full, Used, 1 jobs, 5 files): skipped: in use by running job 5
Volume "Full-D" (Pool Full, Error, 0 jobs, 0 files): would purge
Dry run: 4 volumes selected, 2 would be purged, 2 skipped. Nothing was changed.
*purge volume all pool=Full yes
…
Volume "Full-A" (Pool Full, Full, 2 jobs, 12 files): purged
…
4 volumes selected, 2 purged, 2 skipped.
```

**Ausgabe unter `.api 2`.** Die Einzelformen behalten ihr Ergebnis (leeres `result`). Die Sammelform liefert:

```json
{"purge": {"mode": "dryrun", "pool": "Full", "allpools": false, "volstatus": ["Full", "Used", "Error"],
  "volumes": [{"volume": "Full-A", "pool": "Full", "volstatus": "Full", "jobs": 2, "files": 12, "bytes": 1000,
               "action": "would_purge", "reason": null}],
  "summary": {"selected": 1, "purged": 1, "skipped": 0}}}
```

`mode` ist `dryrun` oder `yes`, `pool` ist `null` bei `allpools`, `action` je Volume `would_purge`, `purged` oder `skipped` (dann mit `reason`). `summary.purged` zählt bei `dryrun` die Volumes, die purgt würden. Fehler (unbekannter Pool, fehlendes `dryrun`/`yes`, Berechtigung) kommen im üblichen Fehlerumschlag.

**Rechte.** Wie jedes `purge` braucht die Form vollen Zugriff auf Clients, Jobs, Pools und Storages (`Permission denied: need full pool permission.`); zusätzlich prüft sie die Pool-ACL je Pool.

**gRPC und MCP.** `PurgeVolume` mit `all = true` (siehe [grpc-api](../grpc-api.md#schreibende-rpcs-entwicklungs-build)); MCP-Tool `purge` mit `kind: "volumes"`, `dryrun: true` ohne `confirm`, der echte Lauf mit `confirm: true` (siehe [MCP](../mcp.md)).

Zusätzliche Fehlermeldungen:

| Meldung | Ursache und Abhilfe |
|---|---|
| `purge volume all: give exactly one of dryrun (preview, changes nothing) or yes (purge); this form never asks for confirmation.` | Weder oder beide von `dryrun` und `yes`. |
| `purge volume all: give exactly one of pool=<pool> or allpools.` | Weder oder beide von `pool=` und `allpools`. |
| `purge volume all: invalid volstatus "<s>"; allowed: …` | Unbekannter oder nicht zulässiger VolStatus (`Purged`, `Recycle`). |
| `purge volume all: argument "<a>" does not apply to this form.` | Argument der Einzelformen. |
| `purge volume all: "all" cannot be combined with volume=<name>.` | `volume=<name>` zusammen mit `all`. |
| `Could not find Pool "<p>": ERR=Pool record not found in Catalog.` | Pool unbekannt oder durch die Pool-ACL verboten. |

## Siehe auch

- [prune](prune.md) — abgelaufene Einträge nach Aufbewahrungszeit entfernen
- [delete](delete.md) — Volumes, Jobs oder Pools löschen
- [truncate](truncate.md) — Volumes im Status Purged leeren
- [list](list.md) — Jobs und Volumes anzeigen
