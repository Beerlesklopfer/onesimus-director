# .bvfs_update — BVFS-Cache aufbauen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L290) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Update BVFS cache

**Originalsyntax:**

```text
.bvfs_update [jobid=<jobid>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (3):** `c022-update-jobid`, `c022-update-unknown-job`, `c022-update-api2`.

**Lokaler Handler:** [crates/odir/src/commands/bvfs.rs:837](../../../crates/odir/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_update` füllt den BVFS-Cache im Katalog (Tabellen `PathHierarchy` und `PathVisibility`, Markierung `Job.HasCache`). Ohne diesen Cache finden [.bvfs_lsdirs](dot-bvfs-lsdirs.md) und [.bvfs_lsfiles](dot-bvfs-lsfiles.md) keine Verzeichnisse. Der Befehl schreibt in den Katalog, ändert aber keine Backup-Daten; für bereits aufbereitete Jobs tut er nichts.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Die OVFS-Befehle ([.ovfs_tree](dot-ovfs-tree.md), [.ovfs_ls](dot-ovfs-ls.md)) brauchen keinen Cache.

## Syntax

```text
.bvfs_update
.bvfs_update jobid=<id>[,<id>…]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<liste>` | JobIds, durch Komma getrennt | Nur diese Jobs aufbereiten. Ohne `jobid=` werden alle Jobs ohne Cache aufbereitet; das kann bei großen Katalogen dauern. |

## API-Modi

Ohne Fehler gibt der Befehl unter `.api 0` und `.api 1` nichts aus (außer den Katalogzeilen beim ersten Katalogzugriff der Sitzung); `.api 2` antwortet mit leerem `result`. Ein Fehler kommt als Textzeile beziehungsweise in `error.data.messages.error`.

## Beispiele

### Cache für einzelne Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=1,5
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Cache für alle Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannte JobId

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=99999
ERROR: BVFS reported a problem for 99999
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=1,99999
ERROR: BVFS reported a problem for 1,99999
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=1
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=99999
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
          "ERROR: BVFS reported a problem for 99999\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `ERROR: BVFS reported a problem for <liste>` | Mindestens eine JobId der Liste gibt es nicht im Katalog. JobIds mit [list jobs](list.md) prüfen. |

## Siehe auch

- [.bvfs_get_jobids](dot-bvfs-get-jobids.md) — Job-Kette ermitteln
- [.bvfs_lsdirs](dot-bvfs-lsdirs.md) — Verzeichnisse auflisten
- [.bvfs_clear_cache](dot-bvfs-clear-cache.md) — Cache vollständig leeren
- [.ovfs_tree](dot-ovfs-tree.md) — Restore-Baum ohne Cache (Onesimus)
