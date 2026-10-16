# relabel -- Re-label an Existing Volume

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L416) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Relabel a tape

**Originalsyntax:**

```text
relabel storage=<storage-name> oldvolume=<old-volume-name>  volume=<new-volume-name> pool=<pool-name> [ encrypt ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 8 von 8 Fällen byte-identisch; Modi: Text.
- Verglichen sind Dialog- und Abbruchpfade; Relabel über den SD ist nur durch Einheitentests belegt (Antwortauswertung siehe C-053).

**Offene Logeinträge:** `C-053` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (8):** `c011-relabel-cancel`, `c011-relabel-pool-cancel`, `c011-relabel-status`, `c011-relabel-unknown-cancel`, `c012-relabel-unknown-old`, `relabel-noargs`, `t001-relabel-not-purged`, `t001-relabel-old-unknown-cancel`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:631](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Write a new label to an existing volume, effectively recycling it for new data. The old volume data is overwritten.

## Syntax

```
relabel [oldvolume=<name>] [volume=<name>] [storage=<name>] [pool=<name>] [encrypt]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `oldvolume=<name>` | Name of the existing volume to relabel |
| `volume=<name>` | New name for the volume |
| `storage=<name>` | Storage daemon where the volume resides |
| `pool=<name>` | Pool to assign the relabeled volume to |
| `encrypt` | Enable volume encryption on the relabeled volume |

## Examples

### Relabel a volume with a new name
```
* relabel oldvolume=Full-0001 volume=Full-0001-recycled storage=File pool=Full
Connecting to Storage daemon File ...
Sending relabel command ...
3000 OK relabel. VolBytes=237 Volume="Full-0001-recycled"
```

### Relabel keeping the same name (recycle)
```
* relabel oldvolume=Full-0001 volume=Full-0001 storage=File
Relabeling volume Full-0001
```

### Relabel with pool change
```
* relabel oldvolume=Inc-0005 volume=Full-0010 storage=File pool=Full
Relabeling volume Inc-0005
```

## Behavior

- The old volume must be in `Purged` status before it can be relabeled.
- The Director connects to the SD, which overwrites the volume header with a new label.
- The catalog record is updated with the new volume name, pool, and reset byte/file counters.
- All previous job references to the old volume name are invalidated.
- This is an audited command.

## Related Commands

- [label](label.md) -- Label a brand new volume
- [purge](purge.md) -- Purge a volume before relabeling
- [update](update.md) -- Update volume attributes without relabeling
