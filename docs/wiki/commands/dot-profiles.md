# .profiles — List Profile Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L263) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all profile resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_profiles-api2`, `c025-profiles-limited`, `c025-profiles-limited-api2`, `c025-profiles-operator`, `c025-profiles-operator-api2`, `c025-profiles-bvfs`, `c025-profiles-bvfs-api2`, `c025-profiles-acl`, `c025-profiles-acl-api2`, `dot_profiles`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:372](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one profile resource name per line from the Director configuration.

## Syntax

```
.profiles
```

## Arguments

None.

## Examples

### List all profiles
```
* .profiles
operator
webui-admin
webui-readonly
```

### No profiles configured
```
* .profiles
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Profile resources define reusable ACL sets that can be assigned to Console resources.
- Used by console programs to populate the `profile=` argument completion list.

## Related Commands

- [.consoles](dot-consoles.md) -- List console names
- [.acl](dot-acl.md) -- Manage console ACLs
