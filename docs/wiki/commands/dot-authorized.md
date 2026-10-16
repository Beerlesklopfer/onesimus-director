# .authorized — Check Command Authorization

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L224) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Check for authorization

**Originalsyntax:**

```text
.authorized job=<job-name> | client=<client-name> | storage=<storage-name> | schedule=<schedule-name> | pool=<pool-name> | cmd=<command> | fileset=<fileset-name> | catalog=<catalog>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, Konsolen parity-limited.

**Paritätsfälle (3):** `dot-authorized-job`, `dot-authorized-job-denied`, `dot-authorized-cmd`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:217](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole to check whether the current console session has access to a specific resource. Returns `authorized` or `not authorized`.

## Syntax

```
.authorized job=<name> | client=<name> | storage=<name> | pool=<name> | fileset=<name> | schedule=<name> | catalog=<name> | cmd=<command>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `job=<name>` | Check access to a Job resource |
| `client=<name>` | Check access to a Client resource |
| `storage=<name>` | Check access to a Storage resource |
| `pool=<name>` | Check access to a Pool resource |
| `fileset=<name>` | Check access to a FileSet resource |
| `schedule=<name>` | Check access to a Schedule resource |
| `catalog=<name>` | Check access to a Catalog resource |
| `cmd=<command>` | Check access to a console command |

Multiple arguments can be combined; all must be authorized for the result to be `authorized`.

## Examples

### Check job authorization
```
* .authorized job=backup-bareos-fd
authorized
```

### Check a non-existent resource
```
* .authorized client=nonexistent-fd
not authorized
```

### Check multiple resources at once
```
* .authorized job=backup-bareos-fd client=bareos-fd storage=File
authorized
```

## Notes

- Returns a single line: either `authorized` or `not authorized`.
- Output is terminated by the EOD signal.
- Currently checks whether the named resource exists in the configuration. Full ACL-based checks (per Console resource) are planned.
- Used by the WebUI to enable/disable UI elements based on the connected console's permissions.

## Related Commands

- [.consoles](dot-consoles.md) -- List console names
- [.acl](dot-acl.md) -- Manage console ACLs
- [.profiles](dot-profiles.md) -- List profile names
- [whoami](whoami.md) -- Show current console identity
