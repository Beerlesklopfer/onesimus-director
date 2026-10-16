# time -- Show Current Date and Time

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L484) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print current time

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/basic.rs:45](../../../crates/onesimus-dird/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display the Director's current local date and time.

## Syntax

```
time
```

## Arguments

None.

## Examples

### Show current time
```
* time
2026-03-29 14:35:22
```

### Use in a script to timestamp output
```
* time
2026-03-29 01:00:00
* run job=backup-bareos-fd level=Full yes
Job queued. JobId=15
```

### Verify timezone after Director restart
```
* time
2026-03-29 22:10:05
```

## Behavior

- Returns the Director's local system time in `YYYY-MM-DD HH:MM:SS` format.
- The timezone is the Director host's configured timezone (not UTC).
- No arguments are accepted; any extra input is ignored.

## Related Commands

- [version](version.md) -- Show Director version
- [status](status.md) -- Show Director status including uptime
