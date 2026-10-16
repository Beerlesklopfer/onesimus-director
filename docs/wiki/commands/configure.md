# configure — Manage Resources

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L319) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Configure director resources

**Dynamische Syntax:** `add <resource-type> <resource-directives> | export client=<client>`. Der [Originalgenerator](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/dird_conf.cc#L918) leitet Pflichtfelder, optionale Felder und Typen aus Ressourcendirektiven ab.

Ressourcentypen für `add`: `client`, `jobdefs`, `job`, `storage`, `catalog`, `schedule`, `fileset`, `pool`, `messages`, `counter`, `profile`, `console`, `user`.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 31 von 31 Fällen byte-identisch; Modi: Text.
- `configure add` ist für Ressourcen ohne Katalogbezug verglichen, die Wirkung über `show` im selben Fall; `reload` danach ist nicht abgedeckt.

**Paritätsfälle (31):** `c019-configure-bare`, `c019-configure-add-only`, `c019-configure-export-only`, `c019-configure-invalid-sub`, `c019-export-no-client-arg`, `c019-export-unknown-client`, `c019-export-twice`, `c019-add-invalid-type`, `c019-add-director`, `c019-add-device`, `c019-add-unknown-directive`, `c019-add-forbidden-char`, `c019-add-missing-name`, `c019-add-missing-value`, `c019-add-missing-required`, `c019-add-console`, `c019-add-console-type-name`, `c019-add-duplicate-existing`, `c019-add-profile`, `c019-add-schedule`, `c019-add-schedule-invalid`, `c019-add-messages`, `c019-add-invalid-name`, `c019-add-unknown-profile`, `c019-add-empty-password`, `c019-add-counter-invalid`, `c019-add-user`, `c019-add-fileset`, `c019-add-case-keys`, `configure-export`, `configure-bad`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/configure.rs:79](../../../crates/onesimus-dird/src/commands/configure.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Add, modify, delete, or read Director resources at runtime.

## Syntax

```
configure add <resource-type> name=<name> [key=value ...]
configure read <resource-type>=<name>
configure delete <resource-type>=<name>
```

## Sub-Commands

| Sub-Command | Description |
|-------------|-------------|
| `add` | Add a new resource (client, job, pool, storage, schedule, fileset) |
| `read` | Show current resource configuration |
| `delete` | Remove a resource |

## Examples

### Add a new client
```
* configure add client name=webserver address=10.0.0.5 password="secret123"
Client 'webserver' added.
```

### Add a new job
```
* configure add job name=backup-webserver client=webserver pool=Full fileset=LinuxAll schedule=WeeklyCycle
Job 'backup-webserver' added.
```

### Read a resource
```
* configure read client=bareos-fd
Client {
  Name = bareos-fd
  Address = localhost
  Port = 9102
  Password = "SlC3mJTFz3DnQX9ZEJwWJgAIoxe5GbD/OmyQADlFrN9A"
  Enabled = yes
}
```

### Delete a resource
```
* configure delete client=webserver
Client 'webserver' deleted.
```

## Notes

- Changes are applied immediately (no restart needed)
- A `reload` is triggered automatically after `configure add/delete`
- Resources are written to the config directory (`bareos-dir.d/`)

## Related Commands

- [reload](reload.md) — Reload configuration manually
- [show](show.md) — Show resource details
- [list](list.md) — List catalog records
