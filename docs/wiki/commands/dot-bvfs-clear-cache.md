# .bvfs_clear_cache — BVFS-Cache leeren

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L307) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Clear BVFS cache

**Originalsyntax:**

```text
.bvfs_clear_cache yes
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Verglichen sind Ausgaben ohne `yes`; `.bvfs_clear_cache yes` ändert den Katalog und ist nur durch Einheitentests belegt.

**Paritätsfälle (3):** `c022-clear-cache-noyes`, `c035-bvfs-clear-cache-api1`, `c035-bvfs-clear-cache-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:859](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_clear_cache yes` leert den gesamten BVFS-Cache im Katalog: Es setzt `Job.HasCache` aller Jobs auf 0 und leert die Tabellen `PathHierarchy` und `PathVisibility`. Danach muss der Cache mit [.bvfs_update](dot-bvfs-update.md) neu aufgebaut werden, bevor [.bvfs_lsdirs](dot-bvfs-lsdirs.md) wieder Verzeichnisse findet. Backup-Daten und Dateieinträge bleiben unberührt.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Die OVFS-Befehle nutzen diesen Cache nicht.

## Syntax

```text
.bvfs_clear_cache yes
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `yes` | – | Pflicht; ohne `yes` geschieht nichts. Eine Rückfrage gibt es nicht. |

## API-Modi

`.api 0` und `.api 1` antworten mit `OK`. `.api 2` antwortet mit leerem `result`.

## Beispiele

### Ohne und mit yes

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_clear_cache
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Can't find 'yes' argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_clear_cache yes
OK
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_clear_cache yes
OK
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_clear_cache yes
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
| `Can't find 'yes' argument` | `yes` fehlt; der Cache bleibt unverändert. |

## Siehe auch

- [.bvfs_update](dot-bvfs-update.md) — Cache neu aufbauen
- [.bvfs_cleanup](dot-bvfs-cleanup.md) — Restore-Tabelle löschen
