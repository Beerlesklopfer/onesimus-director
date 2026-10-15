# setdevice -- Set Device Parameters

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L463) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets device parameter

**Originalsyntax:**

```text
setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- storage/device/autoselect werden jetzt validiert und als echter setdevice-Befehl an den SD gesendet; dessen Antwort wird weitergereicht. Protokoll- und Fehlerpfadtests bestehen. Storage-ACL-Abgleich und Live-SHA-256-Parität bleiben offen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:219](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Modify runtime parameters of a storage device on the Storage Daemon.

## Syntax

```
setdevice storage=<name> device=<name> autoselect=<bool>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `storage=<name>` | Storage daemon resource name (required) |
| `device=<name>` | Device name on the SD (required) |
| `autoselect=<bool>` | Enable/disable automatic device selection (`yes`/`no`) |

## Examples

### Disable autoselect for a device
```
* setdevice storage=Tape device=Drive-0 autoselect=no
Device Drive-0 selected
```

### Enable autoselect
```
* setdevice storage=Tape device=Drive-0 autoselect=yes
Device Drive-0 selected
```

### Set device on file storage
```
* setdevice storage=File device=FileStorage autoselect=yes
Device FileStorage selected
```

## Behavior

- The Director sends the device parameter change to the Storage Daemon via TLS-PSK.
- `autoselect=no` prevents the SD from automatically choosing this device for new jobs, useful for reserving a drive for restores.
- `autoselect=yes` re-enables automatic selection.
- The change is effective immediately but does not persist across SD restarts.
- This is an audited command.

## Related Commands

- [status](status.md) -- Check storage device status
- [mount](mount.md) -- Mount a device
- [unmount](unmount.md) -- Unmount a device
