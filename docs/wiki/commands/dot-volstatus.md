# .volstatus — mögliche Volume-Zustände auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L280) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all volume status

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (2):** `c029-volstatus-api2`, `dot_volstatus`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:393](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.volstatus` gibt alle Zustände aus, die ein Volume im Katalog haben kann (`Append`, `Full`, `Used`, `Purged` und so weiter). Programme füllen damit die Auswahl für `volstatus=` bei `update volume` oder `truncate`. Die Liste ist fest im Director hinterlegt; der Befehl liest weder Konfiguration noch Katalog und verändert nichts.

## Syntax

```text
.volstatus
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Liste. Unter `.api 2` steht sie als `result.volstatus`, ein Array von Objekten mit `name`.

## Beispiele

### Zustände auflisten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .volstatus
Unlabeled
Append
Full
Used
Recycle
Purged
Cleaning
Error
Archive
Disabled
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .volstatus
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "volstatus": [
      {
        "name": "Unlabeled"
      },
      {
        "name": "Append"
      },
      {
[…]
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Berechtigung

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .volstatus
.volstatus: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.volstatus: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.volstatus` nicht. |

## Siehe auch

- [update](update.md) — `update volume=<name> volstatus=<zustand>`
- [truncate](truncate.md) — Volumes im Zustand `Purged` kürzen
- [.actiononpurge](dot-actiononpurge.md) — Aktionen nach dem Purge
- [.media](dot-media.md) — Volume-Namen im Katalog
