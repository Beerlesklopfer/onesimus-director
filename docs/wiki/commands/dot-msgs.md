# .msgs — List Pending Message Types

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L259) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all message resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_msgs-api2`, `c025-msgs-limited`, `c025-msgs-limited-api2`, `c025-msgs-operator`, `c025-msgs-operator-api2`, `c025-msgs-bvfs`, `c025-msgs-bvfs-api2`, `c025-msgs-acl`, `c025-msgs-acl-api2`, `dot_msgs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:366](../../../crates/onesimus-dird/src/commands/dot.rs).

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
