# label -- Label a New Volume

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L363) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Label a tape

**Originalsyntax:**

```text
label storage=<storage-name> volume=<volume-name> pool=<pool-name> slot=<slot> [ drive = <drivenum>] [ barcodes ] [ encrypt ] [ yes ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 13 von 13 Fällen byte-identisch; Modi: Text, API 1.
- Verglichen sind Dialog-, Namens- und Abbruchpfade; das Labeln selbst (SD-Wirkung, Katalogeintrag) ist laut Testregeln nicht live erlaubt und nur durch Einheitentests belegt.

**Paritätsfälle (13):** `c011-label-cancel`, `c011-label-name-cancel`, `c011-label-pool-cancel`, `c011-label-illegal-cancel`, `c011-label-used-cancel`, `c011-label-barcodes`, `c011-label-unknown-storage-cancel`, `c011-label-api1-cancel`, `c012-label-no-pool`, `label-nostorage`, `t001-label-existing-cancel`, `t001-label-illegal-arg-cancel`, `t001-label-illegal-char-cancel`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:26](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Write a Bareos label to a new volume on a Storage Daemon device and register it in the catalog.

## Syntax

```
label [volume=<name>] [storage=<name>] [pool=<name>] [barcodes] [encrypt]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `volume=<name>` | Name for the new volume |
| `storage=<name>` | Storage daemon to label on (required) |
| `pool=<name>` | Pool to assign the volume to |
| `barcodes` | Use barcode reader to label all unregistered slots |
| `encrypt` | Enable volume encryption |

## Examples

### Label a single volume
```
* label volume=Full-0001 storage=File pool=Full
Connecting to Storage daemon File ...
Sending label command for Volume "Full-0001" Pool "Full" Slot 0 drive 0 ...
Label command sent to SD. Waiting for response...
3000 OK label. VolBytes=237 Volume="Full-0001" Device="FileStorage" (/srv/BACKUP/bareos-storage)
```

### Label using barcodes (autochanger)
```
* label storage=Tape barcodes
Sending barcode label command to Storage Tape...
Labeled 12 volumes from barcode reader.
```

### Without storage (interactive selection)
```
* label
label: specify storage=<name>
Available: File, Tape
```

## Behavior

- The Director connects to the specified Storage Daemon via TLS-PSK.
- The SD writes a Bareos volume label to the physical medium (file, tape, etc.).
- A new media record is created in the catalog with status `Append`.
- With `barcodes`, the SD reads barcodes from all autochanger slots and labels each unregistered volume.
- The volume name must be unique within the catalog.
- If `automount` is enabled, the device is automatically mounted after labeling.
- This is an audited command.

## Related Commands

- [relabel](relabel.md) -- Re-label an existing volume
- [list](list.md) -- List volumes in catalog (`list volumes`)
- [mount](mount.md) -- Mount a device after labeling
- [add](add.md) -- Add a volume to a pool without labeling
