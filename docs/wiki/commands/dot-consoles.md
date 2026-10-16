# .consoles — List Console Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L233) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all console resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_consoles-api2`, `c025-consoles-limited`, `c025-consoles-limited-api2`, `c025-consoles-operator`, `c025-consoles-operator-api2`, `c025-consoles-bvfs`, `c025-consoles-bvfs-api2`, `c025-consoles-acl`, `c025-consoles-acl-api2`, `dot_consoles`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:240](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one console resource name per line from the Director configuration.

## Syntax

```
.consoles
```

## Arguments

None.

## Examples

### List all consoles
```
* .consoles
bareos-mon
admin-console
webui-console
```

### Single console setup
```
* .consoles
bareos-mon
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Console resources define named console connections with optional ACL restrictions.
- The default console (no Console resource match) connects as `*UserAgent*` with full access.

## Related Commands

- [.acl](dot-acl.md) -- Manage console ACLs
- [.profiles](dot-profiles.md) -- List profile names
- [whoami](whoami.md) -- Show current console identity
