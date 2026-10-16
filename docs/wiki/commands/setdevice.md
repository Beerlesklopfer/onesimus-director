# setdevice -- Set Device Parameters

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L463) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets device parameter

**Originalsyntax:**

```text
setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 23 von 23 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-device, parity-limited.
- NDMP-Storages werden nicht übersprungen (C-050). Verglichen sind ACL, Argumentprüfung und SD-Antwort auf dem Fixture-Storage.

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (23):** `c006-setdevice-noargs-api2`, `c006-setdevice-wrongarg`, `c006-setdevice-nostorage`, `c006-setdevice-badbool`, `c031-setdevice-forbidden`, `c031-setdevice-forbidden-unknown`, `c031-setdevice-forbidden-api1`, `c031-setdevice-forbidden-api2`, `c031-setdevice-permitted-nodev`, `c031-setdevice-permitted-nodev-api2`, `c031-setdevice-limited-nocmd`, `c031-setdevice-nodev`, `c031-setdevice-nodev-api1`, `c031-setdevice-nodev-api2`, `c031-setdevice-nostorage-api2`, `c031-setdevice-autoselect-upper`, `c031-setdevice-autoselect-on`, `c031-setdevice-key-upper`, `c031-setdevice-key-novalue`, `c031-setdevice-empty-value`, `c031-setdevice-missing-two`, `c031-setdevice-wrongarg-api2`, `setdevice-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:321](../../../crates/onesimus-dird/src/commands/extra.rs).

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
