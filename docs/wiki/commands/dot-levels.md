# .levels — Level der Jobtypen auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L251) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all backup levels

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 6 von 6 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (6):** `c029-levels-api2-type`, `c029-levels-backup`, `c029-levels-two`, `dot_levels`, `dot_levels-type-Backup`, `dot_levels-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:260](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.levels` gibt die Level aus, die für Jobs erlaubt sind, etwa `Full`, `Incremental` oder `VirtualFull` für Backups und `Catalog` oder `VolumeToCatalog` für Verify-Jobs. Programme füllen damit die Auswahl für `level=` bei `run` oder `estimate`. Die Tabelle ist fest im Director hinterlegt; der Befehl liest weder Konfiguration noch Katalog und verändert nichts.

Ohne Argument kommt die ganze Tabelle. Dabei erscheinen `Full`, `Incremental` und `Differential` mehrfach, weil sie für Backup, Copy und Migrate je einen eigenen Eintrag haben.

## Syntax

```text
.levels [ <jobtyp> ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `<jobtyp>` | `Backup`, `Verify`, `Copy`, `Migrate`, `Restore`, `Admin`, `Consolidate`, `Archive` | nur die Level dieses Jobtyps |

Der Jobtyp steht als einzelnes Wort ohne `=`, Groß- und Kleinschreibung ist egal (`BACKUP` geht). Er muss vollständig ausgeschrieben sein; ein Anfang wie `back` oder die Form `type=Backup` liefern eine leere Liste. Jobtypen ohne Level (`Restore`, `Admin`, …) liefern ebenfalls nichts. Mehr als ein Wort ergibt immer eine leere Liste.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Liste. Unter `.api 2` steht sie als `result.levels`, ein Array von Objekten mit `name`, `level` (Level-Kennbuchstabe als Zahl, z. B. 70 = `F`) und `jobtype` (Jobtyp-Kennbuchstabe als Zahl, z. B. 66 = `B`).

## Beispiele

### Alle Level

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .levels
Full
Base
Incremental
Differential
Since
VirtualFull
Catalog
InitCatalog
VolumeToCatalog
DiskToCatalog
Data
Full
Incremental
Differential
Full
Incremental
Differential
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Level für Backups

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .levels Backup
Full
Base
Incremental
Differential
Since
VirtualFull
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Level für Verify-Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .levels Verify
Catalog
InitCatalog
VolumeToCatalog
DiskToCatalog
Data
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobtyp ohne Level

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .levels Restore
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Form `type=` wird nicht verstanden

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .levels type=Backup
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .levels Backup
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "levels": [
      {
        "name": "Full",
        "level": 70,
        "jobtype": 66
      },
      {
        "name": "Base",
        "level": 66,
        "jobtype": 66
      },
      {
        "name": "Incremental",
[…]
        "jobtype": 66
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.levels: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.levels` nicht. |

Ein unbekannter Jobtyp erzeugt keine Meldung, nur eine leere Antwort.

## Siehe auch

- [.types](dot-types.md) — alle Jobtypen
- [run](run.md) — Job mit `level=` starten
- [estimate](estimate.md) — Umfang eines Backups für ein Level schätzen
