# setdevice — Geräteeinstellung im Storage Daemon ändern

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L463) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets device parameter

**Originalsyntax:**

```text
setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 23 von 23 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-device, parity-limited.
- NDMP-Storages werden nicht übersprungen (C-050). Verglichen sind ACL, Argumentprüfung und SD-Antwort auf dem Fixture-Storage.

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (23):** `c006-setdevice-noargs-api2`, `c006-setdevice-wrongarg`, `c006-setdevice-nostorage`, `c006-setdevice-badbool`, `c031-setdevice-forbidden`, `c031-setdevice-forbidden-unknown`, `c031-setdevice-forbidden-api1`, `c031-setdevice-forbidden-api2`, `c031-setdevice-permitted-nodev`, `c031-setdevice-permitted-nodev-api2`, `c031-setdevice-limited-nocmd`, `c031-setdevice-nodev`, `c031-setdevice-nodev-api1`, `c031-setdevice-nodev-api2`, `c031-setdevice-nostorage-api2`, `c031-setdevice-autoselect-upper`, `c031-setdevice-autoselect-on`, `c031-setdevice-key-upper`, `c031-setdevice-key-novalue`, `c031-setdevice-empty-value`, `c031-setdevice-missing-two`, `c031-setdevice-wrongarg-api2`, `setdevice-noargs`.

**Lokaler Handler:** [crates/odir/src/commands/extra.rs:248](../../../crates/odir/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`setdevice` ändert zur Laufzeit die Einstellung `Auto Select` eines Geräts im Storage Daemon, ohne dessen Konfiguration anzufassen. Mit `autoselect=no` wählt der Storage Daemon das Gerät nicht mehr selbst für Jobs aus, etwa während einer Wartung; mit `autoselect=yes` ist es wieder verfügbar. Der Befehl verändert den Zustand des Storage Daemons bis zu dessen nächstem Neustart, nicht den Katalog. Ist die Konsole auf bestimmte Storages beschränkt, darf sie nur deren Geräte ändern.

## Syntax

```text
setdevice storage=<storage> device=<gerät> autoselect=<bool>
```

## Argumente

Alle drei Argumente sind Pflicht; die Reihenfolge ist egal. Die Schlüsselwörter müssen klein geschrieben sein.

| Argument | Werte | Bedeutung |
|---|---|---|
| `storage=<storage>` | Name einer Storage-Ressource | Storage Daemon, an den der Befehl geht. |
| `device=<gerät>` | Name einer Device-Ressource im Storage Daemon | Gerät, dessen Einstellung geändert wird. |
| `autoselect=<bool>` | `yes`, `no`, `true`, `false`, `1`, `0` (klein geschrieben) | Neuer Wert für `Auto Select`. |

Andere Argumente sind nicht erlaubt.

## API-Modi

Unter `.api 0` und `.api 1` kommen Meldungen und die Antwort des Storage Daemons als Text. Unter `.api 2` stehen Fehler des Directors im JSON-Fehlerobjekt; die Antwort des Storage Daemons wird dabei nicht weitergegeben, auch eine Ablehnung durch den Storage Daemon erscheint als leeres Ergebnis:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "error": [
          "Argument missing: autoselect\n",
          "Argument missing: device\n",
          "Argument missing: storage\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=File device=nosuchdev autoselect=yes
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Unter `.api 1` sieht die Antwort aus wie unter `.api 0`:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=File device=nosuchdev autoselect=yes
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103
3991 Bad setdevice command: Device not found: nosuchdev
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Beispiele

### Gerät für die automatische Auswahl sperren

```text
Beispielausgabe nicht verfügbar: eine erfolgreiche Änderung verändert den laufenden Storage Daemon und darf im Testbetrieb nicht ausgeführt werden.
```

### Unbekanntes Gerät

Der Director verbindet sich mit dem Storage Daemon; der lehnt den unbekannten Gerätenamen ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=File device=nosuchdev autoselect=yes
Connecting to Storage daemon File at WS-JOERG.intranet.bernau.family:9103
3991 Bad setdevice command: Device not found: nosuchdev
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### NDMP-Storage

Für NDMP-Storages schickt der Director nichts an einen Storage Daemon und antwortet ohne Ausgabe:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=ParityNdmp device=FileStorage autoselect=yes
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlende Argumente

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice
Argument missing: autoselect
Argument missing: device
Argument missing: storage

USAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=File
Argument missing: autoselect
Argument missing: device

USAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage= device=nosuchdev autoselect=yes
Argument missing: storage

USAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Falsche Argumente und Werte

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice foo=bar
Wrong argument: foo

USAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice Storage=File device=nosuchdev autoselect=yes
Wrong argument: Storage

USAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=File device=x autoselect=maybe
Wrong argument: maybe

USAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=File device=nosuchdev autoselect=Yes
Wrong argument: Yes

USAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=nosuch device=x autoselect=yes
Storage "nosuch" not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Rechte

Eine Konsole, deren Storage-ACL den Storage nicht enthält, bekommt eine Ablehnung (Konsole `parity-device`):

```text
parity-device@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=ParityPreview device=nosuchdev autoselect=yes
Access to storage "ParityPreview" forbidden
parity-device@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Eine Konsole ohne `setdevice` in der Command-ACL kennt den Befehl nicht (Konsole `parity-limited`):

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> setdevice storage=File device=nosuchdev autoselect=no
setdevice: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Argument missing: <name>` | Ein Pflichtargument fehlt oder hat keinen Wert. Danach folgt die Kurzhilfe `USAGE: setdevice …`. |
| `Wrong argument: <wort>` | Unbekanntes Schlüsselwort (auch bei Großschreibung wie `Storage=`) oder ungültiger `autoselect`-Wert wie `maybe`, `Yes` oder `on`. |
| `Storage "<name>" not found.` | Es gibt keine Storage-Ressource dieses Namens. |
| `Access to storage "<name>" forbidden` | Die Storage-ACL der Konsole erlaubt diesen Storage nicht. |
| `setdevice: is an invalid command.` | Die Command-ACL der Konsole erlaubt `setdevice` nicht. |
| `Failed to connect to Storage daemon.` | Der Storage Daemon ist nicht erreichbar. |
| `3991 Bad setdevice command: Device not found: <gerät>` | Antwort des Storage Daemons: das Gerät gibt es dort nicht. Gerätenamen mit `status storage=<storage>` prüfen. |

## Siehe auch

- [status](status.md) — Geräte und ihren Zustand anzeigen (`status storage=<storage>`)
- [mount](mount.md) — Gerät einbinden
- [unmount](unmount.md) — Gerät aushängen
- [setdebug](setdebug.md) — Debug-Stufe des Storage Daemons ändern
