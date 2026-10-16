# .clients — List Client Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L231) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all client resources

**Originalsyntax:**

```text
.clients [enabled | disabled]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:73](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one client resource name per line from the Director configuration.

## Syntax

```
.clients [enabled|disabled]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List all clients |
| `enabled` | List only enabled clients |
| `disabled` | List only disabled clients |

## Examples

### List all clients
```
* .clients
bareos-fd
webserver-fd
dbserver-fd
```

### List only enabled clients
```
* .clients enabled
bareos-fd
webserver-fd
```

### List only disabled clients
```
* .clients disabled
dbserver-fd
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Used by console programs to populate the `client=` argument completion list.

## Related Commands

- [status](status.md) -- Show client status
- [.defaults](dot-defaults.md) -- Show default values for a client
- [.authorized](dot-authorized.md) -- Check access to a client resource
- [.jobs](dot-jobs.md) -- List job names
