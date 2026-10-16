# resolve -- DNS Resolve a Hostname

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L428) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Resolve a hostname

**Originalsyntax:**

```text
resolve client=<client-name> | storage=<storage-name> <host-name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 19 von 19 Fällen byte-identisch; Modi: Text, API 2.
- Legacy-/Nicht-TLS-Daemons sind im Fixture nicht vorhanden.

**Paritätsfälle (19):** `c018-localhost`, `c018-ipv4-literal`, `c018-ipv6-literal`, `c018-multiple`, `c018-unknown`, `c018-unknown-after-known`, `c018-keyword-value`, `c018-no-args`, `c018-client-missing-name`, `c018-client-unknown`, `c018-storage-unknown`, `c018-client`, `c018-client-twice`, `c018-storage`, `c018-client-storage`, `c018-api2`, `c018-api2-client`, `resolve-local`, `resolve-client`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/resolve.rs:7](../../../crates/onesimus-dird/src/commands/resolve.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Resolve a hostname to its IP address, either on the Director or on a specific File Daemon.

## Syntax

```
resolve [client=<name>] <hostname>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `client=<name>` | Resolve on this File Daemon instead of the Director |
| `<hostname>` | Hostname to resolve |

## Examples

### Resolve on the Director
```
* resolve bareos-fd.example.com
bareos-fd.example.com resolves to 192.168.1.50
```

### Resolve on a File Daemon
```
* resolve client=bareos-fd storage-server.local
Resolving storage-server.local on client bareos-fd: 10.0.0.5
```

### Diagnose connectivity issues
```
* resolve client=bareos-fd bareos-sd.example.com
Resolving bareos-sd.example.com on client bareos-fd: 192.168.1.100
```

## Behavior

- Without `client`, the Director resolves the hostname locally using its own DNS.
- With `client`, the Director connects to the File Daemon via TLS-PSK and asks it to resolve the hostname.
- Useful for diagnosing DNS-related connectivity issues between daemons.
- For example, verifying that the FD can resolve the SD hostname before running a backup.
- This is not an audited command.

## Related Commands

- [status](status.md) -- Check daemon connectivity
- [setip](setip.md) -- Update client IP address
