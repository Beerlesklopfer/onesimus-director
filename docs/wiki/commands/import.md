# import -- Import Volumes from Autochanger Magazine

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L358) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Import volumes from import/export slots to normal slots

**Originalsyntax:**

```text
import storage=<storage-name> [ srcslots=<slot-selection> dstslots=<slot-selection> volume=<volume-name> scan ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text.
- Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen und Katalogupdates sind nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-045` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (2):** `c012-import-unknown-storage`, `import-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:984](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Import volumes from the autochanger's import/export (mail) slots into storage slots.

## Syntax

```
import storage=<name> [srcslots=<slot-selection>] [dstslots=<slot-selection>] [scan]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `storage=<name>` | Storage daemon with autochanger (required) |
| `srcslots=<slot-selection>` | Source slot numbers (e.g. `1-5` or `1,3,5`) |
| `dstslots=<slot-selection>` | Destination slot numbers |
| `scan` | Scan barcodes after import |

## Examples

### Import from default mail slots
```
* import storage=Tape
Import from storage Tape
```

### Import specific slots with scan
```
* import storage=Tape srcslots=1-5 scan
Import from storage Tape
Scanning imported volumes...
```

### Import to specific destination slots
```
* import storage=Tape srcslots=101-105 dstslots=1-5
Import from storage Tape
```

## Behavior

- The Director instructs the SD to move volumes from the autochanger's import/export slots (mail slots) to regular storage slots.
- If `srcslots` is not specified, the SD uses the configured import/export slot range.
- The `scan` option triggers a barcode scan after the import, automatically labeling unregistered volumes.
- This command is only relevant for tape autochangers (libraries) -- file-based storage does not use import.
- This is an audited command.

## Related Commands

- [export](export.md) -- Export volumes to autochanger mail slots
- [label](label.md) -- Label imported volumes
- [update](update.md) -- Update slot information (`update slots`)
- [move](move.md) -- Move volumes between slots
