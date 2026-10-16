# setip -- Update Client IP Address

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L466) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets new client address -- if authorized

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text, Konsolen parity-limited.
- Verglichen ist nur die Ablehnung für Konsolen ohne gleichnamigen Client; das Setzen der Client-Adresse ist nur durch Einheitentests belegt.

**Paritätsfälle (2):** `c012-setip-limited`, `setip`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:304](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Update the client's IP address in the Director's in-memory configuration to the address of the currently connected console.

## Syntax

```
setip
```

## Arguments

None.

## Examples

### Update client IP
```
* setip
Client address updated
```

### Typical use for roaming clients
```
* whoami
*UserAgent*
* setip
Client address updated
```

### Verify with show
```
* setip
Client address updated
* show clients
Client: name=bareos-fd address=192.168.1.100 port=9102 enabled=yes
```

## Behavior

- Updates the client address in the Director's configuration to the IP address of the current console connection.
- Designed for roaming clients (e.g. laptops) that do not have a fixed IP address.
- The client must connect to the Director first (client-initiated connection), then the Director records the source IP.
- The change is in-memory only and does not persist across Director restarts.
- This is not an audited command.

## Related Commands

- [show](show.md) -- Show client configuration to verify the address
- [status](status.md) -- Check client connectivity
