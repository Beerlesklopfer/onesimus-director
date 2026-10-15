# mount -- Mount a Storage Device

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L375) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Mount storage

**Originalsyntax:**

```text
mount storage=<storage-name> slot=<num> drive=<drivenum> jobid=<jobid> | job=<job-name> | ujobid=<complete_name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:101](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Instruct the Storage Daemon to mount a device, making it available for reading or writing.

## Syntax

```
mount [storage=<name>] [slot=<num>] [drive=<num>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `storage=<name>` | Storage daemon resource name |
| `slot=<num>` | Autochanger slot number to mount |
| `drive=<num>` | Drive number for autochangers with multiple drives |

## Examples

### Mount a file storage device
```
* mount storage=File
Sending "mount" to Storage daemon File ...
Device "File" mounted.
```

### Mount a specific autochanger slot
```
* mount storage=Tape slot=5 drive=0
Sending "mount" to Storage daemon Tape ...
Device "Tape" mounted (slot 5, drive 0).
```

### Mount after label
```
* label volume=Full-0002 storage=File pool=Full
...
* mount storage=File
Sending "mount" to Storage daemon File ...
Device "File" mounted.
```

## Behavior

- The Director sends a `mount` command to the Storage Daemon via TLS-PSK.
- For file-based storage, this makes the device directory accessible.
- For tape/autochanger devices, this loads a tape from the specified slot into the drive.
- The SD confirms the mount with a status response.
- Mounting is typically needed after a manual `unmount` or when a tape needs to be loaded.
- This is an audited command.

## Related Commands

- [unmount](unmount.md) -- Unmount a storage device
- [release](release.md) -- Release a storage device
- [automount](automount.md) -- Toggle automatic mounting
- [label](label.md) -- Label and auto-mount a volume
