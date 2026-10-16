# .schedule — List Schedule Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L268) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all schedule resources

**Originalsyntax:**

```text
.schedule [enabled | disabled]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 18 von 18 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (18):** `c003-dot_schedule-api2`, `c025-schedule-limited`, `c025-schedule-limited-api2`, `c025-schedule-enabled-limited`, `c025-schedule-enabled-limited-api2`, `c025-schedule-operator`, `c025-schedule-operator-api2`, `c025-schedule-enabled-operator`, `c025-schedule-enabled-operator-api2`, `c025-schedule-bvfs`, `c025-schedule-bvfs-api2`, `c025-schedule-enabled-bvfs`, `c025-schedule-enabled-bvfs-api2`, `c025-schedule-acl`, `c025-schedule-acl-api2`, `c025-schedule-enabled-acl`, `c025-schedule-enabled-acl-api2`, `dot_schedule`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:127](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one schedule resource name per line from the Director configuration.

## Syntax

```
.schedule [enabled|disabled]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List all schedules |
| `enabled` | List only enabled schedules |
| `disabled` | List only disabled schedules |

## Examples

### List all schedules
```
* .schedule
WeeklyCycle
WeeklyCycleAfterBackup
MonthlyCycle
```

### List only enabled schedules
```
* .schedule enabled
WeeklyCycle
WeeklyCycleAfterBackup
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Used by console programs to populate the `schedule=` argument completion list.

## Related Commands

- [.jobs](dot-jobs.md) -- List job names
- [.defaults](dot-defaults.md) -- Show default values for a job (includes schedule)
- [status](status.md) -- Show scheduled jobs
