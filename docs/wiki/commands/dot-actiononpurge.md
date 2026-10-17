# .actiononpurge — mögliche Aktionen nach dem Purge auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L220) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List possible actions on purge

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (2):** `c029-actiononpurge-api2`, `dot_actiononpurge`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:398](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.actiononpurge` gibt die Werte aus, die für `actiononpurge=` (bei `update volume`, `update pool` und in der Pool-Ressource) erlaubt sind: `None` (nichts tun) und `Truncate` (die Volume-Datei nach dem Purge kürzen, um Platz freizugeben). Programme füllen damit Auswahllisten. Die Liste ist fest im Director hinterlegt; der Befehl liest weder Konfiguration noch Katalog und verändert nichts.

## Syntax

```text
.actiononpurge
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommen dieselben zwei Zeilen. Unter `.api 2` steht die Liste als `result.actiononpurge`, ein Array von Objekten mit `name`.

## Beispiele

### Werte auflisten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .actiononpurge
None
Truncate
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .actiononpurge
None
Truncate
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .actiononpurge
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "actiononpurge": [
      {
        "name": "None"
      },
      {
        "name": "Truncate"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.actiononpurge: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.actiononpurge` nicht. Abkürzungen wie `.aop` gibt es nicht; sie werden ebenfalls als ungültig gemeldet. |

## Siehe auch

- [update](update.md) — `actiononpurge=` eines Volumes oder Pools setzen
- [truncate](truncate.md) — gepurgte Volumes kürzen
- [.volstatus](dot-volstatus.md) — mögliche Volume-Zustände
