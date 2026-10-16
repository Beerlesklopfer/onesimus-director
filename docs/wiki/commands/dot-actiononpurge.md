# .actiononpurge — List Action-on-Purge Options

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L220) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List possible actions on purge

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:398](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one action-on-purge option per line.

## Syntax

```
.actiononpurge
```

## Arguments

None.

## Examples

### List all options
```
* .actiononpurge
Truncate
None
```

## Notes

- Output is one option per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Returns a static list: `Truncate` and `None`.
- `Truncate` causes the volume file to be truncated after all jobs on it are purged, reclaiming disk space.
- `None` disables the automatic action (default behavior).
- Used by console programs to populate the `actiononpurge=` argument completion list.

## Related Commands

- [.volstatus](dot-volstatus.md) -- List volume status values
- [.pools](dot-pools.md) -- List pool names
- [purge](purge.md) -- Purge jobs/volumes from catalog
- [truncate](truncate.md) -- Truncate purged volumes
