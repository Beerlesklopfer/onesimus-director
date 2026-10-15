# release -- Release a Storage Device

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L420) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Release storage

**Originalsyntax:**

```text
release storage=<storage-name> [ drive=<drivenum> ] [ alldrives ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:124](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Release a storage device from the current job or session, freeing it for use by other jobs.

## Syntax

```
release [storage=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `storage=<name>` | Storage daemon resource name |

## Examples

### Release a file storage device
```
* release storage=File
Sending "release" to Storage daemon File ...
Device "File" released.
```

### Release after a manual operation
```
* mount storage=Tape
Sending "mount" to Storage daemon Tape ...
Device "Tape" mounted.
* release storage=Tape
Sending "release" to Storage daemon Tape ...
Device "Tape" released.
```

### Release the default storage
```
* release storage=File
Sending "release" to Storage daemon File ...
Device "File" released.
```

## Behavior

- The Director sends a `release` command to the Storage Daemon via TLS-PSK.
- The SD marks the device as available for other jobs.
- Unlike `unmount`, release does not eject or physically disconnect the medium.
- Typically used after manual tape operations or when a device is held by a console session.
- This is an audited command.

## Related Commands

- [mount](mount.md) -- Mount a storage device
- [unmount](unmount.md) -- Unmount a storage device
- [status](status.md) -- Check device status
