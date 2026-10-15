# .acl — Manage Console ACLs

Machine-interface command to manage Access Control Lists for Console resources. Supports listing, setting, deleting, clearing, and copying ACL entries.

## Syntax

```
.acl list [console=<name>]
.acl set console=<name> jobacl=<job1>,<job2> [clientacl=<c1>,<c2>] ...
.acl delete console=<name> jobacl=<job>
.acl clear console=<name>
.acl copy from=<console1> to=<console2>
```

## Arguments

| Sub-Command | Arguments | Description |
|-------------|-----------|-------------|
| `list` | `console=<name>` (optional) | List ACLs for one or all consoles |
| `set` | `console=<name>`, `<type>acl=<values>` | Set ACL entries for a console |
| `delete` | `console=<name>`, `<type>acl=<value>` | Remove specific ACL entries |
| `clear` | `console=<name>` | Remove all ACL entries for a console |
| `copy` | `from=<console>`, `to=<console>` | Copy all ACLs from one console to another |

ACL types include: `jobacl`, `clientacl`, `storageacl`, `scheduleacl`, `poolacl`, `filesetacl`, `catalogacl`, `commandacl`, `whereacl`, `pluginoptionsacl`.

## Examples

### List ACLs for all consoles
```
* .acl list
Console: bareos-mon
  (ACL configuration pending)

Console: webui-console
  (ACL configuration pending)
```

### Set ACLs for a console
```
* .acl set console=webui-console jobacl=backup-bareos-fd,RestoreFiles clientacl=bareos-fd
ACL set for console 'webui-console':
  jobacl = backup-bareos-fd,RestoreFiles
  clientacl = bareos-fd
```

### Copy ACLs between consoles
```
* .acl copy from=webui-console to=admin-console
ACLs copied from 'webui-console' to 'admin-console'
```

## Notes

- Output is terminated by the EOD signal.
- The `.profile` command is an alias for `.acl` (same handler).
- ACL changes affect the running Director state; use `reload` to persist changes or modify the configuration files directly.
- Without a sub-command, prints usage help listing all available sub-commands.

## Related Commands

- [.consoles](dot-consoles.md) -- List console names
- [.profiles](dot-profiles.md) -- List profile names
- [.authorized](dot-authorized.md) -- Check resource authorization
- [reload](reload.md) -- Reload Director configuration
