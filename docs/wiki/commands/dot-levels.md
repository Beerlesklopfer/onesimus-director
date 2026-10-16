# .levels — List Backup Levels

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L251) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all backup levels

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:260](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one backup level name per line.

## Syntax

```
.levels
```

## Arguments

None.

## Examples

### List all levels
```
* .levels
Full
Incremental
Differential
VirtualFull
InitCatalog
Catalog
VolumeToCatalog
DiskToCatalog
```

## Notes

- Output is one level name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Returns a static list of all supported backup levels.
- Used by console programs to populate the `level=` argument completion list (e.g., for `run job=... level=`).

## Related Commands

- [run](run.md) -- Run a job at a specified level
- [.jobs](dot-jobs.md) -- List job names
- [.types](dot-types.md) -- List job types
- [.defaults](dot-defaults.md) -- Show default level for a job
