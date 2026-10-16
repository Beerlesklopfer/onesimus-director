# memory -- Show Memory Usage Information

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L373) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print current memory usage

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (3):** `c009-memory`, `c009-memory-json`, `memory`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:97](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display memory usage statistics of the Director daemon.

## Syntax

```
memory
```

## Arguments

None.

## Examples

### Show memory usage
```
* memory
Memory usage information not available (Rust runtime)
```

### In C++ Bareos
```
* memory
Pool Bytes     Max Bytes  Max Used  Get         Free
Main  1234567  2345678    1890000   45678       44000
```

### Check after extended operation
```
* memory
Memory usage information not available (Rust runtime)
```

## Behavior

- In C++ Bareos, displays the internal memory pool allocator statistics including current bytes, maximum bytes, and allocation/free counts.
- In the Rust implementation, Rust uses its own memory allocator (jemalloc or system malloc) which does not expose the same pool-based statistics, so this command reports that detailed info is not available.
- Useful for diagnosing memory leaks or capacity issues in long-running Director instances.
- This is not an audited command.

## Related Commands

- [status](status.md) -- Show Director status
- [version](version.md) -- Show Director version
