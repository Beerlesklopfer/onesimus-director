# cancel -- Cancel a Running Job

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L315) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Cancel a job

**Originalsyntax:**

```text
cancel storage=<storage-name> | jobid=<jobid> | job=<job-name> | ujobid=<unique-jobid> | state=<job_state> | all yes
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/job.rs:220](../../../crates/onesimus-dird/src/commands/job.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Cancel a currently running or queued job by its JobId, job name, or unique job identifier.

## Syntax

```
cancel [jobid=<id>] [job=<name>] [ujobid=<unique-id>] [all]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `jobid=<id>` | Cancel the job with this numeric JobId |
| `job=<name>` | Cancel all running jobs with this name |
| `ujobid=<unique-id>` | Cancel by unique job ID (e.g. `backup-bareos-fd.2026-03-29_01.00.00_000000`) |
| `all` | Cancel all running and queued jobs |

## Examples

### Cancel a specific job by ID
```
* cancel jobid=15
Job 15 marked for cancellation
```

### Cancel without arguments (interactive)
```
* cancel
cancel: specify jobid=<id>
No running jobs to cancel.
```

### Cancel after checking status
```
* status dir
Running Jobs:
JobId  Level  Name              Status
==========================================
15     Full   backup-bareos-fd  Running

* cancel jobid=15
Job 15 marked for cancellation
```

## Behavior

- The Director sends a cancellation signal to the job worker process.
- The job status is updated to `A` (Canceled) in the catalog.
- The FD and SD are notified to stop data transfer.
- If the job has already completed, the cancel has no effect.
- Without arguments and no running jobs, the command reports that there are no jobs to cancel.
- This is an audited command.

## Related Commands

- [run](run.md) -- Start a job
- [status](status.md) -- Show running jobs
- [list](list.md) -- List job records from catalog
