# .media — Volume-Namen aus dem Katalog auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L256) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all medias

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 5 von 5 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (5):** `c002-shared-then-private`, `c029-media`, `c029-media-api1`, `c029-media-api2`, `dot_media`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:325](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.media` gibt die Namen aller Volumes aus, die im Katalog stehen, alphabetisch sortiert und ohne Doppelte. Programme nutzen die Liste für die Vervollständigung von `volume=` und für Auswahllisten. Der Befehl liest die Tabelle `Media` des Katalogs und verändert nichts.

## Syntax

```text
.media
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `catalog=<name>` | Name eines Katalogs | diesen Katalog verwenden statt des automatisch gewählten |

Andere Wörter werden ignoriert.

## API-Modi

- `.api 0`: beim ersten Katalogzugriff der Sitzung stehen die Hinweiszeilen `Automatically selected Catalog: …` und `Using Catalog "…"` vor der Liste.
- `.api 1`: nur die Namen, ohne Hinweiszeilen.
- `.api 2`: `result.media`, ein Array von Objekten mit `name`.

## Beispiele

### Alle Volumes

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .media
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Differential-47323
Full-0001
Full-0002
Full-47321
Incremental-47322
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .media
Differential-47323
Full-0001
Full-0002
Full-47321
Incremental-47322
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .media
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "media": [
      {
        "name": "Differential-47323"
      },
      {
        "name": "Full-0001"
      },
      {
        "name": "Full-0002"
      },
      {
        "name": "Full-47321"
      },
      {
        "name": "Incremental-47322"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Berechtigung

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .media
.media: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.media: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.media` nicht. |
| `List Media failed: ERR=…` | Die Abfrage im Katalog ist fehlgeschlagen; der Text hinter `ERR=` kommt von der Datenbank. |

## Siehe auch

- [list](list.md) — `list volumes` mit allen Details
- [.volstatus](dot-volstatus.md) — mögliche Volume-Zustände
- [.mediatypes](dot-mediatypes.md) — Medientypen im Katalog
- [.pools](dot-pools.md) — Namen der Pools
