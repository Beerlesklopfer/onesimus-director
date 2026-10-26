# .consoles — Namen der Konsolen auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L233) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all console resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_consoles-api2`, `c025-consoles-limited`, `c025-consoles-limited-api2`, `c025-consoles-operator`, `c025-consoles-operator-api2`, `c025-consoles-bvfs`, `c025-consoles-bvfs-api2`, `c025-consoles-acl`, `c025-consoles-acl-api2`, `dot_consoles`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:243](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.consoles` listet die Namen aller Console-Ressourcen des Directors, einen pro Zeile. Oberflächen und die Tab-Vervollständigung nutzen den Befehl. Die Liste enthält auch die eingebaute Standardkonsole `*UserAgent*` und Konsolen, die mit [configure](configure.md) zur Laufzeit angelegt wurden. Der Befehl liest nur.

## Syntax

```text
.consoles
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert (Paritätsfall `dcfg-consoles-arg`).

Die Reihenfolge ist die interne Reihenfolge der Konfiguration, nicht alphabetisch. Die Liste wird nicht nach ACLs gefiltert: wer `.consoles` ausführen darf, sieht alle Konsolen.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Liste. Unter `.api 2` steht sie in `result.consoles` als Array von Objekten mit dem Feld `name`.

## Beispiele

### Konsolen auflisten

Paritätsfall `dot_consoles`. `c019e` und `c019f` wurden vorher mit `configure add console` angelegt und stehen deshalb am Ende.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .consoles
parity-operator
parity-listacl
parity-limited
parity-device
parity-c030-job
parity-c030-storage
parity-c030-client
parity-c030-pool
parity-bvfs
parity-acl
bareos-mon
*UserAgent*
c019e
c019f
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsolen als JSON

Paritätsfall `c003-dot_consoles-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .consoles
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "consoles": [
      {
        "name": "parity-operator"
      },
      {
        "name": "parity-listacl"
      },
[…]
      {
        "name": "bareos-mon"
      },
      {
        "name": "*UserAgent*"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Berechtigung

`parity-limited` hat `.consoles` nicht in der `CommandACL` (Paritätsfall `c025-consoles-limited`).

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .consoles
.consoles: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.consoles: is an invalid command.` | Die Konsole darf `.consoles` nicht ausführen (`CommandACL`). |

## Siehe auch

- [.profiles](dot-profiles.md) — Namen aller Profile
- [.users](dot-users.md) — Namen aller User-Ressourcen
- [show](show.md) — `show consoles` zeigt die Konsolen mit allen Direktiven
- [configure](configure.md) — Konsole zur Laufzeit anlegen
- [whoami](whoami.md) — Namen der eigenen Konsole anzeigen
