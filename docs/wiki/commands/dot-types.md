# .types — List Job Types

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L279) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all job types

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:323](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one job type name per line.

## Syntax

```
.types
```

## Arguments

None.

## Examples

### List all job types
```
* .types
Backup
Restore
Verify
Admin
Archive
Copy
Migrate
Consolidate
```

## Notes

- Output is one type name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Returns a static list of all supported job types.
- Used by console programs to populate the `type=` argument completion list and to filter `.jobs type=<T>`.

## Related Commands

- [.jobs](dot-jobs.md) -- List job names (supports `type=` filter)
- [.levels](dot-levels.md) -- List backup levels
- [.jobstatus](dot-jobstatus.md) -- List job status codes
- [run](run.md) -- Run a job
