# unmount -- Unmount a Storage Device

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L492) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Unmount storage

**Originalsyntax:**

```text
unmount storage=<storage-name> [ drive=<drivenum> ] jobid=<jobid> | job=<job-name> | ujobid=<complete_name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Paritätsfall nachgewiesen.

- Kein Paritätsfall: Unmount am echten SD ist laut Testregeln nicht live erlaubt; Verhalten nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:175](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Instruct the Storage Daemon to unmount a device, making it safe to remove media or perform maintenance.

## Syntax

```
unmount [storage=<name>] [drive=<num>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `storage=<name>` | Storage daemon resource name |
| `drive=<num>` | Drive number for autochangers with multiple drives |

## Examples

### Unmount a file storage device
```
* unmount storage=File
Sending "unmount" to Storage daemon File ...
Device "File" unmounted.
```

### Unmount a specific drive
```
* unmount storage=Tape drive=0
Sending "unmount" to Storage daemon Tape ...
Device "Tape" unmounted.
```

### Unmount before physical tape swap
```
* unmount storage=Tape
Sending "unmount" to Storage daemon Tape ...
Device "Tape" unmounted.
(swap tape, then mount again)
* mount storage=Tape slot=3
```

## Behavior

- The Director sends an `unmount` command to the Storage Daemon via TLS-PSK.
- The SD closes any open volume on the device and makes the medium removable.
- For tape devices, the tape is rewound and ejected from the drive.
- If a job is actively writing to the device, the unmount will fail.
- The `umount` alias is also accepted.
- This is an audited command.

## Related Commands

- [mount](mount.md) -- Mount a storage device
- [release](release.md) -- Release a storage device
- [status](status.md) -- Check if a device is in use before unmounting
