# create -- Create Pool from Resource Definition

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L321) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Create DB Pool from resource

**Originalsyntax:**

```text
create pool=<pool-name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 4 von 4 Fällen byte-identisch; Modi: Text.
- Verglichen sind Dialog und unbekannter Pool; das Anlegen eines Pools im Katalog ist nicht verglichen.

**Paritätsfälle (4):** `c011-create-cancel`, `c011-create-unknown-cancel`, `c012-create-unknown-pool`, `create-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:813](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Create a pool record in the catalog database based on the Pool resource definition in the Director configuration.

## Syntax

```
create [pool=<pool-name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `pool=<pool-name>` | Pool resource name to create in the catalog |

## Examples

### Create a specific pool
```
* create pool=Full
Pool created from resource
```

### Create default pool
```
* create
Pool created from resource
```

### Create after adding a new pool resource
```
* reload
reloaded
* create pool=Offsite
Pool created from resource
```

## Behavior

- Reads the Pool resource definition from the Director configuration and creates a matching record in the catalog database.
- If the pool record already exists in the catalog, it is not modified -- use `update pool` to synchronize changes.
- Pool creation is typically needed after adding a new Pool resource to the configuration and reloading.
- The Director automatically creates pool records for configured pools at startup, so this command is mainly needed for pools added at runtime.
- This is an audited command.

## Related Commands

- [update](update.md) -- Update an existing pool record from resource
- [show](show.md) -- Show pool resource definitions
- [list](list.md) -- List pool records in catalog
- [reload](reload.md) -- Reload configuration before creating
