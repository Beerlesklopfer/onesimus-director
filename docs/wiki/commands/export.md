# export -- Export Volumes to Autochanger Magazine

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L341) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Export volumes from normal slots to import/export slots

**Originalsyntax:**

```text
export storage=<storage-name> srcslots=<slot-selection> [ dstslots=<slot-selection> volume=<volume-name> scan ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Leitet storage und srcslots an den SD weiter; dstslots, volume und scan aus der Originalhilfe werden im Handler nicht ausgewertet.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:406](../../../crates/onesimus-dird/src/commands/extra.rs).

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
