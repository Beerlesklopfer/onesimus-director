# .storages

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L277) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all storage resources

**Originalsyntax:**

```text
.storages [enabled | disabled]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 18 von 18 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (18):** `c025-storages-limited`, `c025-storages-limited-api2`, `c025-storages-enabled-limited`, `c025-storages-enabled-limited-api2`, `c025-storages-operator`, `c025-storages-operator-api2`, `c025-storages-enabled-operator`, `c025-storages-enabled-operator-api2`, `c025-storages-bvfs`, `c025-storages-bvfs-api2`, `c025-storages-enabled-bvfs`, `c025-storages-enabled-bvfs-api2`, `c025-storages-acl`, `c025-storages-acl-api2`, `c025-storages-enabled-acl`, `c025-storages-enabled-acl-api2`, `dot_storages`, `dot_storages-limited`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:110](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
