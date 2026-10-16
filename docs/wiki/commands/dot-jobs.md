# .jobs — List Job Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L247) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all job resources

**Originalsyntax:**

```text
.jobs [type=<jobtype>] | [enabled | disabled]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:83](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one job resource name per line from the Director configuration.

## Syntax

```
.jobs [type=<jobtype>] [enabled|disabled]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List all jobs |
| `type=<jobtype>` | Filter by job type (Backup, Restore, Verify, Admin, etc.) |
| `enabled` | List only enabled jobs |
| `disabled` | List only disabled jobs |

## Examples

### List all jobs
```
* .jobs
backup-bareos-fd
backup-webserver
RestoreFiles
```

### Filter by type
```
* .jobs type=Backup
backup-bareos-fd
backup-webserver
```

### List only enabled jobs
```
* .jobs enabled
backup-bareos-fd
RestoreFiles
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- The `type=` filter is case-insensitive.
- Used by console programs to populate the `job=` argument completion list.

## Related Commands

- [run](run.md) -- Run a job
- [.defaults](dot-defaults.md) -- Show default values for a job
- [.types](dot-types.md) -- List all job types
- [.jobdefs](dot-jobdefs.md) -- List JobDefs names
- [.clients](dot-clients.md) -- List client names
