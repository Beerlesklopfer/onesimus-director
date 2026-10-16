# .jobstatus — List Job Status Codes

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L249) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List jobstatus information

**Originalsyntax:**

```text
.jobstatus [ =<jobstatus> ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:147](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole and automation tools. Returns all job status codes with their descriptions as tab-separated pairs, one per line.

## Syntax

```
.jobstatus [=<status-char>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List all status codes |
| `=<char>` | Show only the entry for the given status character |

## Examples

### List all status codes
```
* .jobstatus
A	Canceled by user
B	Blocked
C	Created, not yet running
D	Verify found differences
E	Terminated with errors
F	Waiting for FD
I	Incomplete job
L	Committing data (last despool)
M	Waiting for mount
R	Running
S	Waiting for SD
T	Completed successfully
W	Completed with warnings
a	SD despooling attributes
c	Waiting for client
d	Waiting for max jobs
e	Non-fatal error
f	Fatal error
i	Doing batch insert file records
j	Waiting for job
m	Waiting for media
p	Waiting for priority
q	Queued
s	Waiting for storage
t	Waiting for start time
```

### Query a single status
```
* .jobstatus =T
T	Completed successfully
```

### Query an error status
```
* .jobstatus =f
f	Fatal error
```

## Notes

- Output is tab-separated: `<code>\t<description>`, one per line.
- Output is terminated by the EOD signal.
- Status codes are case-sensitive: uppercase letters are terminal states (A, E, T, W), lowercase letters are transient/waiting states.
- Used by WebUI and automation scripts to decode the JobStatus column from `list jobs`.

## Related Commands

- [list](list.md) -- List jobs from catalog (includes JobStatus column)
- [.jobs](dot-jobs.md) -- List job names
- [.types](dot-types.md) -- List job types
- [.levels](dot-levels.md) -- List backup levels
