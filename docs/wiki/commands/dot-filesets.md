# .filesets — List FileSet Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L241) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all filesets

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_filesets-api2`, `c025-filesets-limited`, `c025-filesets-limited-api2`, `c025-filesets-operator`, `c025-filesets-operator-api2`, `c025-filesets-bvfs`, `c025-filesets-bvfs-api2`, `c025-filesets-acl`, `c025-filesets-acl-api2`, `dot_filesets`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:120](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one FileSet resource name per line from the Director configuration.

## Syntax

```
.filesets
```

## Arguments

None.

## Examples

### List all filesets
```
* .filesets
LinuxAll
SelfTest
WindowsAll
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- No filtering options are available for this command.
- Used by console programs to populate the `fileset=` argument completion list.

## Related Commands

- [.jobs](dot-jobs.md) -- List job names
- [.defaults](dot-defaults.md) -- Show default values for a job (includes fileset)
- [show](show.md) -- Show full resource configuration
