# rerun -- Re-run a Previously Failed Job

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L424) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Rerun a job

**Originalsyntax:**

```text
rerun jobid=<jobid> | since_jobid=<jobid> [ until_jobid=<jobid> ] | days=<nr_days> | hours=<nr_hours> | yes
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Unterstützt eine JobId mit Katalog-Lookup und delegiert an run; since_jobid/until_jobid/days/hours und die Originalauswahl mehrerer Jobs fehlen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:246](../../../crates/onesimus-dird/src/commands/extra.rs).

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
