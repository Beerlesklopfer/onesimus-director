# rerun — Jobs erneut starten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L424) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Rerun a job

**Originalsyntax:**

```text
rerun jobid=<jobid> | since_jobid=<jobid> [ until_jobid=<jobid> ] | days=<nr_days> | hours=<nr_hours> | yes
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 32 von 32 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Auswahl und Fehlertexte sind verglichen; der tatsächliche Jobstart wird im Paritätslauf nicht ausgeführt (Jobmeldungen C-051).

**Offene Logeinträge:** `C-051` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (32):** `c006-rerun-noargs-api2`, `c006-rerun-badid-api2`, `c006-rerun-invalidid`, `c012-rerun-confirm`, `c033-since-none`, `c033-since-yes-none`, `c033-since-zero`, `c033-since-text`, `c033-since-days`, `c033-until-only`, `c033-since-until-no`, `c033-since-list-no`, `c033-since-prompt-abort`, `c033-since-invalid-answer`, `c033-since-long-answer`, `c033-since-confirm-run-no`, `c033-since-api2`, `c033-since-api1`, `c033-days-no`, `c033-days-negative`, `c033-hours-none`, `c033-days-hours-no`, `c033-jobid-list-no`, `c033-jobid-nopool`, `c033-jobid-fraction`, `c033-jobid-trailing-comma`, `c033-jobid-leading-comma`, `c033-jobid-empty`, `c033-jobid-negative`, `c033-jobid-missing-then-ok`, `rerun-noargs`, `rerun-badid`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:348](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`rerun` startet frühere Jobs noch einmal mit den Parametern, mit denen sie damals liefen (Job, Level, Client, FileSet, Pool). Man wählt die Jobs über ihre JobId, über einen JobId-Bereich oder über ein Zeitfenster. Für jeden Job zeigt der Director die Run-Zusammenfassung von [run](run.md) mit der Frage `OK to run? (yes/mod/no):`. Mit `yes` als Argument starten alle gewählten Jobs ohne Rückfrage.

Bei `since_jobid=`, `days=` und `hours=` berücksichtigt `rerun` nur Jobs mit dem Jobstatus `f` (Fatal Error); bei `days=`/`hours=` zählt die geplante Startzeit (SchedTime). Für die Jobtypen gilt dasselbe wie bei `run` (Kompatibilitätslog C-058).

## Syntax

```text
rerun jobid=<n>[,<n>…] [yes]
rerun since_jobid=<n> [until_jobid=<n>] [yes]
rerun days=<n> [hours=<n>] [yes]
rerun hours=<n> [yes]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<n>[,<n>…]` | JobIds, durch Komma getrennt | Diese Jobs erneut starten. Bruchteile werden abgeschnitten (`599.5` = 599). |
| `since_jobid=<n>` | JobId größer 0 | Alle Jobs mit Status `f` ab dieser JobId. |
| `until_jobid=<n>` | JobId | Obergrenze zu `since_jobid=`; allein nicht zulässig. |
| `days=<n>` | Tage | Jobs mit Status `f` der letzten n Tage. |
| `hours=<n>` | Stunden | Jobs mit Status `f` der letzten n Stunden; mit `days=` addiert. |
| `yes` | – | Ohne Rückfragen starten. |

## Dialoge

- Wählt `rerun` mehrere Jobs über `since_jobid=`, `days=` oder `hours=`, listet der Director die JobIds und fragt `rerun these jobids? (yes/no):`. Nur `yes` und `no` gelten; andere Eingaben (auch leere) wiederholen die Frage, `.` bricht ab.
- Danach folgt für jeden Job die Run-Zusammenfassung mit `OK to run? (yes/mod/no):` wie bei [run](run.md), einschließlich `mod`.
- Bei `jobid=` mit mehreren JobIds arbeitet der Director die Liste nacheinander ab; endet die erste mit `no`, kommen die übrigen nicht mehr an die Reihe. Eine nicht vorhandene JobId beendet die Liste ohne Meldung nach `rerunning jobid <n>`.

## API-Modi

Unter `.api 0` und `.api 1` erscheinen dieselben Texte. Unter `.api 2` kommen Fehler als JSON-RPC-Fehler `"message": "failed"`; die Ja/Nein-Frage wird unter `.api 1` und `.api 2` nicht als Text ausgegeben. Wie bei `run` gibt es unter `.api 2` keine Rückfrage `OK to run?`: die Jobs starten sofort.

## Beispiele

### Einen Job erneut starten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun jobid=14
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
rerunning jobid 14
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:44:19
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere JobIds

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun jobid=14,599
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
rerunning jobid 14
Run Backup job
JobName:  backup-bareos-fd
Level:    Incremental
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Incremental (From Job IncPool override)
Storage:  File (From Job resource)
When:     2026-10-16 22:44:19
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobs ab einer JobId

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun since_jobid=590
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The following ids were selected for rerun:
590,591,592,593,594,595,596,597,598,599
rerun these jobids? (yes/no):
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Bereich bestätigen und dann ablehnen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun since_jobid=599 until_jobid=599
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The following ids were selected for rerun:
599
rerun these jobids? (yes/no):
> yes
rerunning jobid 599
Run Backup job
JobName:  backup-bareos-fd
Level:    Full
Client:   bareos-fd
Format:   Native
FileSet:  SelfTest
Pool:     Full (From command line)
Storage:  File (From Job resource)
When:     2026-10-16 22:44:16
Priority: 10
OK to run? (yes/mod/no):
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige Antwort auf die Bereichsfrage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun since_jobid=599 until_jobid=599
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The following ids were selected for rerun:
599
rerun these jobids? (yes/no):
> nope
rerun these jobids? (yes/no):
>
rerun these jobids? (yes/no):
> No
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Abbruch mit Punkt

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun since_jobid=599 until_jobid=599
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The following ids were selected for rerun:
599
rerun these jobids? (yes/no):
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nach Zeit auswählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun days=-1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
No jobids with the specified options were found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun hours=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
No jobids with the specified options were found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Argumente

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Please specify jobid, since_jobid, hours or days
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültige JobId

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun jobid=abc
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Specified jobid "abc" is not valid
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> rerun
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {}
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Please specify jobid, since_jobid, hours or days` | Kein gültiges Auswahlargument; `since_jobid=` muss eine Zahl größer 0 sein, `until_jobid=` braucht `since_jobid=`. |
| `Specified jobid "<wert>" is not valid` | `jobid=` enthält keinen Zahlenwert (auch bei führendem Komma). |
| `No jobids with the specified options were found.` | Im Zeitfenster bzw. ab der JobId gibt es keine Jobs mit Status `f`. |
| `rerunning jobid <n>` ohne Zusammenfassung | Die JobId gibt es nicht im Katalog. |
| `Job not run.` | Die Run-Frage wurde mit `no` oder `.` beantwortet. |

## Siehe auch

- [run](run.md) — Job starten, Run-Dialog und mod-Menü
- [list](list.md) — Jobs mit Status suchen (`list jobs jobstatus=f`)
- [cancel](cancel.md) — Jobs abbrechen
- [status](status.md) — laufende Jobs anzeigen
