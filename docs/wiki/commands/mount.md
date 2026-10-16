# mount -- Mount a Storage Device

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L375) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Mount storage

**Originalsyntax:**

```text
mount storage=<storage-name> slot=<num> drive=<drivenum> jobid=<jobid> | job=<job-name> | ujobid=<complete_name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 16 von 16 Fällen byte-identisch; Modi: Text.
- Autochanger-Slots und NDMP fehlen bzw. sind ungeprüft (C-045/C-050 Umfeld); verglichen sind Auswahl und Fehlerpfade am File-Storage.

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (16):** `c013-mount-auto`, `c013-mount-positional`, `c013-mount-storage`, `c013-mount-sd-drive`, `c013-mount-slot`, `c013-mount-twice`, `c013-mount-question`, `c013-mount-notfound`, `c013-mount-bare-storage`, `c013-mount-jobid-bad`, `c013-mount-jobid-notrunning`, `c013-mount-job-notrunning`, `c013-mount-job-novalue`, `c013-mount-ujobid-notrunning`, `c013-release-then-mount`, `mount-dialog`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:171](../../../crates/onesimus-dird/src/commands/admin.rs).

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
