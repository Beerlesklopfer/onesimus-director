# .help — List Commands (Machine Format)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L243) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print parsable information about a command

**Originalsyntax:**

```text
.help [ all ] [ item=cmd ] [ full ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/basic.rs:92](../../../crates/onesimus-dird/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion and command discovery. Returns command metadata in a format determined by the current API mode.

## Syntax

```
.help [all] [item=<command>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List dot-commands only |
| `all` | List all commands (regular + dot-commands) |
| `item=<command>` | Show detail for a single command |

## Output Format

Depends on the session API mode set via [.api](dot-api.md):

- **Mode 0 (human):** Colored table with Command and Description columns
- **Mode 1 (api):** Tab-separated fields: `name\tdescription\tusage\targuments\taudit`
- **Mode 2 (json):** JSON array of objects with `command`, `description`, `usage`, `arguments`, `audit`

## Examples

### List dot-commands (API mode 1)
```
* .api 1
* .help
.help	List dot commands	.help [all]	all	false
.clients	List clients	.clients [enabled|disabled]	enabled disabled	false
.jobs	List jobs	.jobs [type=<T>] [enabled|disabled]	type enabled disabled	false
.pools	List pools	.pools [type=<T>]	type	false
.storage	List storages	.storage [enabled|disabled]	enabled disabled	false
...
```

### Show detail for one command
```
* .help item=.clients
  Command:     .clients
  Description: List clients
  Usage:       .clients [enabled|disabled]
  Arguments:   enabled disabled
  Audit:       no
```

### List all commands (JSON mode)
```
* .api 2
* .help all
[
  {"command":"help","description":"Print help","usage":"help [all|<command>]","arguments":"all","audit":false},
  {"command":"status","description":"Show status","usage":"status [dir|client=<name>|storage=<name>]","arguments":"dir client storage","audit":false},
  ...
]
```

## Notes

- Default (no arguments) returns only dot-commands, which is what bconsole needs for tab-completion.
- `all` adds regular commands (run, status, list, etc.) to the output.
- Output is terminated by the EOD signal.

## Related Commands

- [help](help.md) -- Human-readable help listing
- [.api](dot-api.md) -- Switch output mode
- [.authorized](dot-authorized.md) -- Check command authorization
