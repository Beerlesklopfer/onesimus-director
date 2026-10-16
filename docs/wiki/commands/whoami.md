# whoami -- Show Current Console Identity

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L527) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print the user name associated with this console

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:287](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display the name of the console resource that is currently logged in.

## Syntax

```
whoami
```

## Arguments

None.

## Examples

### Default console
```
* whoami
*UserAgent*
```

### Named console
```
* whoami
admin-console
```

### Restricted console
```
* whoami
readonly-console
```

## Behavior

- Returns the name of the Console resource used to authenticate the current session.
- `*UserAgent*` indicates the default console (connected with the Director's password, not a named Console resource).
- Named consoles have ACLs that restrict which commands and resources they can access.
- Useful for verifying which identity is in use, especially when multiple Console resources are configured.
- This is not an audited command.

## Related Commands

- [help](help.md) -- List available commands for the current console
- [quit](quit.md) -- Disconnect the current session
- [.authorized](dot-authorized.md) -- Check ACL authorization for specific resources
