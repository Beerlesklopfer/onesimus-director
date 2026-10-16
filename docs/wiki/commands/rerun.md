# rerun -- Re-run a Previously Failed Job

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

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:416](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Re-run a job that previously failed or was canceled, using the same parameters as the original run.

## Syntax

```
rerun jobid=<id> [since_jobid=<id>] [until_jobid=<id>] [days=<days>] [hours=<hours>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `jobid=<id>` | Re-run this specific failed job |
| `since_jobid=<id>` | Re-run all failed jobs since this JobId |
| `until_jobid=<id>` | Re-run all failed jobs up to this JobId |
| `days=<days>` | Re-run all failed jobs from the last N days |
| `hours=<hours>` | Re-run all failed jobs from the last N hours |

## Examples

### Re-run a specific failed job
```
* rerun jobid=15
Rerunning job 15
```

### Re-run all failures from the last 24 hours
```
* rerun jobid=0 hours=24
Rerunning job 15
Rerunning job 18
```

### Re-run failures in a range
```
* rerun since_jobid=10 until_jobid=20
Rerunning job 15
Rerunning job 18
```

## Behavior

- The Director looks up the original job record in the catalog and starts a new job with the same Job resource, level, client, and other parameters.
- Only jobs with a failed status (`E`, `f`, `A`) are eligible for rerun.
- The new job receives a new JobId.
- When using `since_jobid`/`until_jobid` or `days`/`hours`, multiple jobs may be rerun at once.
- This is an audited command.

## Related Commands

- [run](run.md) -- Start a new job manually
- [cancel](cancel.md) -- Cancel a running job
- [list](list.md) -- List jobs to find failed ones
