# autodisplay -- Toggle Automatic Message Display

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L311) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Autodisplay console messages

**Originalsyntax:**

```text
autodisplay on | off
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:26](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Enable or disable automatic display of console messages after each command.

## Syntax

```
autodisplay [on|off]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `on` | Enable automatic message display (default) |
| `off` | Disable automatic message display |

## Examples

### Enable autodisplay
```
* autodisplay on
autodisplay on
```

### Disable autodisplay
```
* autodisplay off
autodisplay off
```

### Default (enable)
```
* autodisplay
autodisplay on
```

## Behavior

- When `on`, the Director automatically sends any pending messages to the console after each command completes, without requiring an explicit `messages` command.
- When `off`, messages accumulate and must be retrieved manually with `messages`.
- The default state is `on`.
- This setting is per-session -- it does not affect other connected consoles.
- This is not an audited command.

## Related Commands

- [messages](messages.md) -- Manually display pending messages
- [gui](gui.md) -- Switch to GUI mode (also affects message display)
