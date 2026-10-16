# .defaults — Show Default Values for a Resource

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L237) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Get default settings

**Originalsyntax:**

```text
.defaults job=<job-name> | client=<client-name> | storage=<storage-name> | pool=<pool-name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (9):** `c041-defaults-backupcatalog`, `c041-defaults-backupcatalog-api2`, `c041-defaults-jobdefs`, `dot-defaults-job`, `dot-defaults-client`, `dot-defaults-storage`, `dot-defaults-pool`, `dot-defaults-restore`, `dot-defaults-job-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:139](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole and the WebUI to retrieve default directive values for a named resource. Returns key=value pairs, one per line.

## Syntax

```
.defaults [job=<name>] [client=<name>] [storage=<name>] [pool=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List summary defaults for all jobs |
| `job=<name>` | Show defaults for the named Job resource |
| `client=<name>` | Show defaults for the named Client resource |
| `storage=<name>` | Show defaults for the named Storage resource |
| `pool=<name>` | Show defaults for the named Pool resource |

## Examples

### Show job defaults
```
* .defaults job=backup-bareos-fd
job=backup-bareos-fd
pool=Full
client=bareos-fd
storage=File
fileset=SelfTest
level=Incremental
type=Backup
enabled=1
schedule=WeeklyCycle
```

### Show client defaults
```
* .defaults client=bareos-fd
client=bareos-fd
address=127.0.0.1
port=9102
enabled=1
```

### Show storage defaults
```
* .defaults storage=File
storage=File
address=127.0.0.1
port=9103
enabled=1
mediatype=File
device=FileStorage
```

## Notes

- Output is key=value pairs, one per line, no headers.
- Output is terminated by the EOD signal.
- If the named resource does not exist, returns an error message like `Job 'xyz' not found`.
- Without arguments, returns a summary line per job: `job=<name> level=<level> client=<client> storage=<storage>`.
- Heavily used by the Bareos WebUI to pre-populate form fields when running jobs.

## Related Commands

- [.jobs](dot-jobs.md) -- List job names
- [.clients](dot-clients.md) -- List client names
- [.storage](dot-storage.md) -- List storage names
- [.pools](dot-pools.md) -- List pool names
- [show](show.md) -- Show full resource configuration
