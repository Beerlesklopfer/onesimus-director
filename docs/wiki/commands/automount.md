# automount -- Toggle Automatic Device Mounting

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L313) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Automount after label

**Originalsyntax:**

```text
automount on | off
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `stub`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler bestätigt on/off per Text; im BNET-Sessionloop wird kein automount-Zustand gesetzt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:31](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Enable or disable automatic mounting of storage devices after a `label` command.

## Syntax

```
automount [on|off]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `on` | Enable automatic mounting (default) |
| `off` | Disable automatic mounting |

## Examples

### Enable automount
```
* automount on
automount on
```

### Disable automount
```
* automount off
automount off
```

### Default (enable)
```
* automount
automount on
```

## Behavior

- When `on`, the Storage Daemon automatically mounts a device after a successful `label` command, making the volume immediately available for writing.
- When `off`, a manual `mount` command is required after labeling.
- The default state is `on`.
- This setting affects the current console session.
- This is not an audited command.

## Related Commands

- [mount](mount.md) -- Manually mount a device
- [label](label.md) -- Label a volume (auto-mounts if enabled)
- [unmount](unmount.md) -- Unmount a device
