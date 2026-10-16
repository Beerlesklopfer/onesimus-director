# quit -- Disconnect from Director

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L399) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Terminate Bconsole session

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/basic.rs:62](../../../crates/onesimus-dird/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Terminate the current bconsole session and close the connection to the Director.

## Syntax

```
quit
exit
.quit
```

## Arguments

None.

## Examples

### Normal disconnect
```
* quit
bye
```

### Using exit alias
```
* exit
bye
```

### Using dot-command variant
```
* .quit
bye
```

## Behavior

- All three forms (`quit`, `exit`, `.quit`) are equivalent and handled by the same code path.
- The Director responds with `bye` and closes the connection.
- Any running jobs are **not** affected -- they continue in the background.
- The session is removed from the Director's session manager.
- This is not an audited command.

## Related Commands

- [whoami](whoami.md) -- Show current console identity before disconnecting
- [status](status.md) -- Check running jobs before quitting
