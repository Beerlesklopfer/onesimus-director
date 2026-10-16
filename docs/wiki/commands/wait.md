# wait -- Wait for a Job to Complete

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L523) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Wait until no jobs are running

**Originalsyntax:**

```text
wait jobname=<name> | jobid=<jobid> | ujobid=<complete_name> | mount [timeout=<number>]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Jobauswahl, mount/timeout und Abschlussausgabe sind portiert. Der Lifecycle verarbeitet jetzt echte Worker-Enden vor dem Katalogabschluss; Canceling bleibt aktiv, finale Zähler bleiben erhalten, Bareos-Statusprioritäten einschließlich Incomplete gelten. Fehlgeschlagene Katalog-Endupdates und der Snapshot-Race werden für wait jobid und wait ohne Argument sichtbar abgefangen; Retention beginnt am tatsächlichen Ende. Gegenprüfungen fanden und Regressionstests reproduzieren diese Fehler. Vollständige FD-/SD-Abbruchkoordination, ACL und Live-SHA-256-Parität bleiben offen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/wait.rs:46](../../../crates/onesimus-dird/src/commands/wait.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Block the console session until a specific job or all jobs have completed.

## Syntax

```
wait [jobid=<id>] [jobuid=<unique-id>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `jobid=<id>` | Wait for this specific job to finish |
| `jobuid=<unique-id>` | Wait for the job with this unique job ID |

## Examples

### Wait for all jobs to complete
```
* wait
No jobs running or queued.
```

### Wait for a specific job
```
* run job=backup-bareos-fd level=Full yes
Job queued. JobId=15
* wait jobid=15
JobId 15 terminated with status OK.
```

### Use in a scripting workflow
```
* run job=backup-bareos-fd level=Full yes
Job queued. JobId=15
* wait jobid=15
JobId 15 terminated with status OK.
* list jobid=15
jobid  name              type level jobstatus starttime            endtime              jobfiles jobbytes
15     backup-bareos-fd  B    F     T         2026-03-29 01:00:00  2026-03-29 01:02:15  458      71683878
```

## Behavior

- Without arguments, waits until no jobs are running or queued, then returns.
- With `jobid` or `jobuid`, waits only for that specific job.
- The console is blocked during the wait -- no other commands can be entered.
- Returns immediately if the specified job has already terminated or if no jobs are running.
- Useful in batch scripts that need to sequence operations after a backup completes.
- This is not an audited command.

## Related Commands

- [run](run.md) -- Start a job
- [cancel](cancel.md) -- Cancel a running job
- [status](status.md) -- Check job status without blocking
