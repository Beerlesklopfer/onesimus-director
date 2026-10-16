# add -- Add Volume to a Pool

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L309) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Add media to a pool

**Originalsyntax:**

```text
add pool=<pool-name> storage=<storage-name> jobid=<jobid>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, Konsolen parity-c030-client, parity-c030-job, parity-c030-pool, parity-c030-storage.
- Verglichen sind Dialog, ACL-Warnung und Abbruch; das tatsächliche Anlegen von Volumes und der Katalogzustand danach sind nicht verglichen.

**Paritätsfälle (9):** `c011-add-cancel`, `c011-add-pool-cancel`, `c011-add-pool-badnumber-cancel`, `c011-add-api1-cancel`, `c030-add-job-cancel`, `c030-add-storage-cancel`, `c030-add-client-cancel`, `c030-add-pool-cancel`, `add-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:855](../../../crates/onesimus-dird/src/commands/extra.rs).

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
