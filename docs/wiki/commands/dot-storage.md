# .storage — Kurzform von .storages

## Zweck

`.storage` ist kein eigener Befehl, sondern eine Abkürzung von [`.storages`](dot-storages.md): Der Director akzeptiert wie das Bareos-Original jeden eindeutigen Anfang eines Befehlsnamens und führt dann den ersten passenden Befehl der Befehlstabelle aus. Ausgabe, Argumente, API-Modi und Berechtigungen sind daher genau die von `.storages`. Der Befehl liest nur die Konfiguration.

## Syntax

```text
.storage [ enabled | disabled ]
```

## Argumente

Wie bei [.storages](dot-storages.md): `enabled` oder `disabled` filtern nach dem Aktivierungszustand.

## API-Modi

Wie bei `.storages`; unter `.api 2` heißt das Array ebenfalls `storages`.

## Beispiele

### Alle Storages

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .storage
ParityNdmpNative
ParityNdmp
File
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur aktivierte Storages

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .storage enabled
ParityNdmpNative
ParityNdmp
File
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .storage
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "storages": [
      {
        "name": "ParityNdmpNative",
        "enabled": true
      },
      {
        "name": "ParityNdmp",
        "enabled": true
      },
      {
        "name": "File",
        "enabled": true
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

Wie bei [.storages](dot-storages.md): Erlaubt die `Command ACL` der Konsole `.storages` nicht, meldet der Director, dass der Befehl ungültig ist (`… is an invalid command.`).

## Siehe auch

- [.storages](dot-storages.md) — der vollständige Befehl
- [.defaults](dot-defaults.md) — Vorgabewerte eines Storages
