# reload -- Reload Director Configuration

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L423) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Reload conf file

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:566](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Reload the Director's configuration files without restarting the daemon. Equivalent to sending SIGHUP.

## Syntax

```
reload
```

## Arguments

None.

## Examples

### Reload after config change
```
* reload
reloaded
Director 'bareos-dir': 1 clients, 3 jobs, 1 storages, 4 pools, 1 schedules
```

### Verify reload with show
```
* reload
reloaded
Director 'bareos-dir': 1 clients, 3 jobs, 1 storages, 4 pools, 1 schedules
* show all
Director: bareos-dir
Clients: 1
Jobs: 3
Storages: 1
Pools: 4
FileSets: 2
Schedules: 1
Consoles: 1
```

### Reload after adding a new job
```
* reload
reloaded
Director 'bareos-dir': 2 clients, 4 jobs, 1 storages, 4 pools, 1 schedules
```

## Behavior

- Re-reads all Director configuration files from disk.
- Running jobs are not affected -- they continue with the configuration they were started with.
- New or modified resources become available immediately.
- Deleted resources are removed from the in-memory config (but running jobs using them continue).
- If the configuration has errors, the reload fails and the previous configuration remains active.
- Reports a summary of loaded resources after a successful reload.
- This is an audited command.

## Related Commands

- [show](show.md) -- Show resource configuration after reload
- [create](create.md) -- Create pool records for newly added pools
- [update](update.md) -- Update catalog pool records after reload
