# .jobdefs — List JobDefs Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L245) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all job defaults resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c025-jobdefs-limited`, `c025-jobdefs-limited-api2`, `c025-jobdefs-operator`, `c025-jobdefs-operator-api2`, `c025-jobdefs-bvfs`, `c025-jobdefs-bvfs-api2`, `c025-jobdefs-acl`, `c025-jobdefs-acl-api2`, `dot_jobdefs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:253](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one JobDefs resource name per line from the Director configuration.

## Syntax

```
.jobdefs
```

## Arguments

None.

## Examples

### List all jobdefs
```
* .jobdefs
DefaultJob
DefaultRestore
LinuxBackup
```

### No jobdefs configured
```
* .jobdefs
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- JobDefs are template resources from which Job resources inherit directives.
- Used by console programs to populate the `jobdefs=` argument completion list.

## Related Commands

- [.jobs](dot-jobs.md) -- List job names
- [.defaults](dot-defaults.md) -- Show default values for a job
- [show](show.md) -- Show full resource configuration
