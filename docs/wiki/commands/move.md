# move -- Move Volume Between Autochanger Slots

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L379) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Move slots in an autochanger

**Originalsyntax:**

```text
move storage=<storage-name> srcslots=<slot-selection> dstslots=<slot-selection>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `semantic_mismatch`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Bareos verwendet srcslots/dstslots für Slot-Auswahlen. Der lokale Handler erwartet srcslot/dstslot (Singular), standardmäßig 0; Originalsyntax wird damit nicht korrekt umgesetzt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:462](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Move a volume from one autochanger slot to another within the same autochanger.

## Syntax

```
move [storage=<name>] srcslot=<num> dstslot=<num>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `storage=<name>` | Storage daemon with autochanger |
| `srcslot=<num>` | Source slot number (required) |
| `dstslot=<num>` | Destination slot number (required) |

## Examples

### Move a volume between slots
```
* move storage=Tape srcslot=3 dstslot=7
Moving media from slot 3 to 7
```

### Move within default storage
```
* move srcslot=1 dstslot=10
Moving media from slot 1 to 10
```

### Reorganize after import
```
* import storage=Tape srcslots=101-103
* move storage=Tape srcslot=1 dstslot=20
Moving media from slot 1 to 20
```

## Behavior

- The Director instructs the SD to use the autochanger robot to physically move a volume between slots.
- The destination slot must be empty.
- The catalog slot assignment is updated automatically.
- This command is only relevant for tape autochangers.
- This is an audited command.

## Related Commands

- [import](import.md) -- Import volumes from mail slots
- [export](export.md) -- Export volumes to mail slots
- [update](update.md) -- Update slot information (`update slots`)
