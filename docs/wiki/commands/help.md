# help -- List Available Commands

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L350) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print help on specific command

**Originalsyntax:**

```text
help add autodisplay automount cancel configure create delete disable enable estimate exit gui label list llist messages memory mount prune purge quit query restore relabel release reload run status setbandwidth setdebug setip show sqlquery time trace truncate unmount umount update use var version wait
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/basic.rs:60](../../../crates/onesimus-dird/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display a list of all regular (non-dot) console commands with descriptions, or show detailed help for a specific command.

## Syntax

```
help [<command>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `<command>` | Optional. Show detailed help for a specific command. |

## Examples

### List all regular commands
```
* help
  Available commands:

  add                  Add media to a pool
  autodisplay          Autodisplay console messages
  automount            Automount after label
  cancel               Cancel a job
  configure            Configure director resources
  create               Create DB pool from resource
  delete               Delete catalog records
  disable              Disable a job/client/schedule
  enable               Enable a job/client/schedule
  estimate             Estimate backup size
  ...
  whoami               Print the currently logged in user

  Use ".help" for dot-commands. "help <cmd>" for details.
```

### Help for a specific command
```
* help run
run: Run a job
  Usage: run [job=<name>] [level=<level>] [client=<name>] [storage=<name>] [pool=<name>] [when=<time>] [yes]
  Arguments: job,level,client,storage,pool,when,yes
```

### Help for an unknown command
```
* help foobar
Unknown command: foobar
Use ".help" for complete command list.
```

## Behavior

- Without arguments, lists all regular commands (non-dot) with short descriptions.
- With a command name argument, shows usage and argument details for that command.
- For machine-readable output or dot-command help, use [.help](dot-help.md) instead.
- The output includes ANSI color codes for terminal readability.
- This is not an audited command.

## Related Commands

- [.help](dot-help.md) -- Machine-readable help, includes dot-commands
- [whoami](whoami.md) -- Show current console identity
