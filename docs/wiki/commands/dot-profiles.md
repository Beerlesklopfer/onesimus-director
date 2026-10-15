# .profiles — List Profile Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L263) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all profile resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:379](../../../crates/onesimus-dird/src/commands/dot.rs).

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
