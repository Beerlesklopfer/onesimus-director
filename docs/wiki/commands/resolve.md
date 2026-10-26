# resolve — Hostnamen auflösen

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

**Lokaler Handler:** [crates/odir/src/commands/resolve.rs:7](../../../crates/odir/src/commands/resolve.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`resolve` zeigt, zu welchen IP-Adressen ein Hostname aufgelöst wird. Ohne `client=` oder `storage=` löst der Director selbst auf; mit `client=` bzw. `storage=` lässt er den File Daemon bzw. Storage Daemon auflösen und zeigt dessen Ergebnis. So lassen sich Namensauflösungsprobleme auf jeder Seite einer Verbindung finden. Der Befehl verändert nichts.

## Syntax

```text
resolve <hostname> …
resolve client=<client> <hostname> …
resolve storage=<storage> <hostname> …
resolve client=<client> storage=<storage> <hostname> …
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `<hostname>` | Name, IPv4- oder IPv6-Adresse | Aufzulösender Name; mehrere möglich. |
| `client=<client>`, `fd=<client>` | Name einer Client-Ressource | Der File Daemon dieses Clients löst auf. |
| `storage=<storage>` | Name einer Storage-Ressource | Der Storage Daemon löst auf. |

Ohne `client=`/`storage=` gilt jedes Wort als Hostname; bei `foo=bar` nur der Teil vor dem `=`. Löst der Director selbst auf und scheitert ein Name, bricht der Befehl dort ab; folgende Namen werden nicht mehr versucht. Ohne jedes Argument kommt keine Ausgabe.

Die Antwortzeile lautet `<daemon> resolves <hostname> to <adressen>`, jede Adresse als `host[ipv4;…]` oder `host[ipv6;…]`. `<daemon>` ist der Name des auflösenden Daemons (`bareos-dir`, der Name des File Daemons oder `bareos-sd`). Bei `client=` und `storage=` erscheinen davor die Verbindungszeilen zum Daemon.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Textausgabe. Unter `.api 2` ist das Ergebnis leer (`"result": {}`); die Adressen werden dort nicht ausgegeben.

## Beispiele

### Namen auf dem Director auflösen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve localhost
bareos-dir resolves localhost to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv4;127.0.0.1]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Namen und Adressen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve localhost 127.0.0.1 ::1
bareos-dir resolves localhost to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv4;127.0.0.1]
bareos-dir resolves 127.0.0.1 to host[ipv4;127.0.0.1]
bareos-dir resolves ::1 to host[ipv6;::1]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Name nicht auflösbar

Der dritte Name wird nicht mehr versucht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve localhost nonexistent.invalid localhost
bareos-dir resolves localhost to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv4;127.0.0.1]
bareos-dir Failed to resolve nonexistent.invalid
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auf dem File Daemon auflösen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve client=bareos-fd localhost
Connecting to Client bareos-fd at localhost:9102
 Handshake: Immediate TLS, Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
WS-JOERG.intranet.bernau.family-fd resolves localhost to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv4;127.0.0.1]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auf dem Storage Daemon auflösen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve storage=File localhost
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
bareos-sd resolves localhost to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv4;127.0.0.1]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### File Daemon und Storage Daemon zugleich

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve client=bareos-fd storage=File localhost
Connecting to Client bareos-fd at localhost:9102
 Handshake: Immediate TLS, Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3
WS-JOERG.intranet.bernau.family-fd resolves localhost to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv4;127.0.0.1]
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103 ...
bareos-sd resolves localhost to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv4;127.0.0.1]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 2`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve localhost
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `bareos-dir Failed to resolve <hostname>` | Der Director kann den Namen nicht auflösen. DNS und `/etc/hosts` des Director-Rechners prüfen. |
| `Client "<name>" not found.` | Es gibt keine Client-Ressource dieses Namens. Namen mit [`.clients`](dot-clients.md) prüfen. |
| `Client name missing.` | `client` oder `fd` ohne `=<name>`. |
| `Storage "<name>" not found.` | Es gibt keine Storage-Ressource dieses Namens. Namen mit [`.storage`](dot-storage.md) prüfen. |
| `Storage name missing.` | `storage` ohne `=<name>`. |

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve client=nosuch localhost
Client "nosuch" not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> resolve storage
Storage name missing.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Kann der File Daemon oder Storage Daemon nicht erreicht werden, erscheinen die üblichen Verbindungsfehler; Beispielausgabe nicht verfügbar, weil im Testaufbau beide Daemons laufen.

## Siehe auch

- [status](status.md) — Verbindung zu Client und Storage prüfen
- [setip](setip.md) — Adresse eines Clients setzen
- [.clients](dot-clients.md) — Namen der Clients
- [.storage](dot-storage.md) — Namen der Storages
