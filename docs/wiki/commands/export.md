# export -- Export Volumes to Autochanger Magazine

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L341) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Export volumes from normal slots to import/export slots

**Originalsyntax:**

```text
export storage=<storage-name> srcslots=<slot-selection> [ dstslots=<slot-selection> volume=<volume-name> scan ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text.
- Kein Autochanger im Fixture: verglichen sind nur Guards ohne Argumente; Slotbewegungen sind nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-045` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (2):** `c012-export-bad-jobid`, `export-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:988](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Export volumes from storage slots to the autochanger's import/export (mail) slots for physical removal.

## Syntax

```
export storage=<name> srcslots=<slot-selection> [dstslots=<slot-selection>] [scan]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `storage=<name>` | Storage daemon with autochanger (required) |
| `srcslots=<slot-selection>` | Source storage slot numbers (e.g. `1-5` or `1,3,5`) (required) |
| `dstslots=<slot-selection>` | Destination import/export slot numbers |
| `scan` | Scan barcodes before export |

## Examples

### Export volumes from specific slots
```
* export storage=Tape srcslots=1-5
Export from storage Tape
```

### Export to specific mail slots
```
* export storage=Tape srcslots=1-3 dstslots=101-103
Export from storage Tape
```

### Export with barcode scan
```
* export storage=Tape srcslots=1-5 scan
Export from storage Tape
Scanning volumes before export...
```

## Behavior

- The Director instructs the SD to move volumes from regular storage slots to the import/export (mail) slots.
- Once in the mail slots, the tapes can be physically removed from the autochanger for offsite storage or disposal.
- If `dstslots` is not specified, the SD uses the configured import/export slot range.
- The `scan` option triggers a barcode scan before the export.
- This command is only relevant for tape autochangers.
- This is an audited command.

## Related Commands

- [import](import.md) -- Import volumes from autochanger mail slots
- [move](move.md) -- Move volumes between slots
- [list](list.md) -- List volumes to identify which slots to export
