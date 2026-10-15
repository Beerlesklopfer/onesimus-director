# configure — Manage Resources

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L319) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Configure director resources

**Dynamische Syntax:** `add <resource-type> <resource-directives> | export client=<client>`. Der [Originalgenerator](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/dird_conf.cc#L918) leitet Pflichtfelder, optionale Felder und Typen aus Ressourcendirektiven ab.

Ressourcentypen für `add`: `client`, `jobdefs`, `job`, `storage`, `catalog`, `schedule`, `fileset`, `pool`, `messages`, `counter`, `profile`, `console`, `user`.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Lokales CRUD ist eine Erweiterung, kein vollständiger Port aller Bareos-Ressourcendirektiven. Der Persistenzpfad ist auf /etc/bareos festgelegt; Originalsyntax wird dynamisch aus deutlich umfangreicheren Direktiventabellen erzeugt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/configure.rs:42](../../../crates/onesimus-dird/src/commands/configure.rs).

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
