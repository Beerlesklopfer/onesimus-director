# add -- Add Volume to a Pool

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L309) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Add media to a pool

**Originalsyntax:**

```text
add pool=<pool-name> storage=<storage-name> jobid=<jobid>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der lokale Handler verlangt volume=<name> und legt einen vereinfachten Katalogdatensatz an; storage/jobid aus der Originalhilfe werden nicht ausgewertet. Die dokumentierte Originalsyntax ohne volume führt lokal nur zur Aufforderung, volume anzugeben.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:398](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Add an existing volume record to a pool in the catalog without physically labeling the medium.

## Syntax

```
add [pool=<pool-name>] [storage=<storage-name>] [jobid=<id>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `pool=<pool-name>` | Pool to add the volume to |
| `storage=<storage-name>` | Storage daemon for the volume |
| `jobid=<id>` | Associate with a specific job |

## Examples

### Add a volume to a pool
```
* add pool=Full storage=File
Volume added to pool
```

### Add interactively
```
* add
Volume added to pool
```

### Add after manual volume creation
```
* add pool=Offsite storage=Tape
Volume added to pool
```

## Behavior

- Creates a volume catalog record and assigns it to the specified pool.
- Unlike `label`, this does not write a physical label to the medium -- it only updates the catalog.
- Useful for manually registering volumes that were created outside of Bareos (e.g. pre-labeled tapes).
- The volume name is prompted interactively if not provided.
- This is an audited command.

## Related Commands

- [label](label.md) -- Label and register a new volume (physical + catalog)
- [delete](delete.md) -- Delete a volume record from the catalog
- [list](list.md) -- List volumes in a pool
- [update](update.md) -- Update volume attributes
