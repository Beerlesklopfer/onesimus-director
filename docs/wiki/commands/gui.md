# gui -- Enable/Disable GUI Mode

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L346) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Switch between interactive (gui off) and non-interactive (gui on) mode

**Originalsyntax:**

```text
gui on | off
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI.

**Paritätsfälle (30):** `c005-gui-noarg`, `c005-gui-foo`, `c005-gui-upper`, `c005-gui-on-off`, `c005-gui-foo-api2`, `c005-gui-on-api2`, `c005-automount-prompt-then-command`, `c027-gui-list-pools`, `c027-gui-llist-pools`, `c027-gui-list-jobs`, `c027-gui-llist-jobs`, `c027-gui-list-media`, `c027-gui-llist-media`, `c027-gui-list-clients`, `c027-gui-list-jobtotals`, `c027-gui-list-jobmedia`, `c027-gui-list-filesets`, `c027-gui-list-log`, `c027-gui-sql`, `c027-gui-sqlquery-keyword`, `c027-gui-status-dir`, `c027-gui-show`, `c027-gui-messages`, `c027-gui-off-again`, `c027-gui-noarg`, `c027-gui-api1`, `c027-gui-api2`, `c036-gui-on-messages`, `gui-on`, `gui-off`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:66](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Switch the console session to GUI mode, which modifies output formatting for use by graphical user interfaces.

## Syntax

```
gui [on|off]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `on` | Enable GUI mode |
| `off` | Disable GUI mode (return to interactive mode) |

## Examples

### Enable GUI mode
```
* gui on
gui mode on
```

### Disable GUI mode
```
* gui off
gui mode off
```

### Default (enable)
```
* gui
gui mode on
```

## Behavior

- In GUI mode, the Director modifies output to be more suitable for parsing by graphical frontends:
  - Suppresses interactive prompts
  - Disables ANSI color codes
  - Sends messages automatically (implicitly enables `autodisplay`)
- Used by tools like `bat` (Bareos Administration Tool) and web-based interfaces.
- This setting is per-session.
- This is not an audited command.

## Related Commands

- [autodisplay](autodisplay.md) -- Control automatic message display
- [.api](dot-api.md) -- Switch between API output modes (0/1/2)
