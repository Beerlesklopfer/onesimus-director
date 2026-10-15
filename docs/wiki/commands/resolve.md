# resolve -- DNS Resolve a Hostname

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L428) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Resolve a hostname

**Originalsyntax:**

```text
resolve client=<client-name> | storage=<storage-name> <host-name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Löst Clientnamen oder einen Hostnamen auf; die originale storage=<name>-Auflösung fehlt im Handler.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:269](../../../crates/onesimus-dird/src/commands/extra.rs).

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
