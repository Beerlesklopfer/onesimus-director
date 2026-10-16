# .users — List User Resource Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L235) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all user resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c025-users-limited`, `c025-users-limited-api2`, `c025-users-operator`, `c025-users-operator-api2`, `c025-users-bvfs`, `c025-users-bvfs-api2`, `c025-users-acl`, `c025-users-acl-api2`, `dot_users`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:383](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one User resource name per line from the Director configuration.

## Syntax

```
.users
```

## Arguments

None.

## Examples

### List users
```
* .users
admin
operator
```

### No user resources configured (typical)
```
* .users
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- User resources are optional and rarely configured in most Bareos installations. Access control is typically handled via Console and Profile resources instead.
- Returns empty output when no User resources exist.

## Related Commands

- [.consoles](dot-consoles.md) -- List console names
- [.profiles](dot-profiles.md) -- List profile names
- [whoami](whoami.md) -- Show current console identity
