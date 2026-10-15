# .msgs — List Pending Message Types

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L259) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all message resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:373](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole. Returns pending messages queued for the current console session.

## Syntax

```
.msgs
```

## Arguments

None.

## Examples

### No pending messages
```
* .msgs
```

### Pending messages available
```
* .msgs
28-Mar 21:20 bareos-dir JobId 10: Backup OK
28-Mar 21:20 bareos-dir JobId 10: Sending spooled attrs to catalog
```

## Notes

- Output is one message per line (if any), no headers.
- Output is terminated by the EOD signal.
- Returns the messages queued for the console, then clears them.
- Functionally similar to `messages` but in dot-command (machine) format.

## Related Commands

- [messages](messages.md) -- Display pending console messages (human format)
- [.messages](dot-messages.md) -- List Messages resource names
- [autodisplay](autodisplay.md) -- Toggle automatic message display
