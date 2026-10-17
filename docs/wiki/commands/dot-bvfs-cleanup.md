# .bvfs_cleanup — Restore-Tabelle löschen (BVFS)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L304) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Cleanup BVFS cache for a certain handle

**Originalsyntax:**

```text
.bvfs_cleanup path=<handle>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 1 von 1 Fällen byte-identisch; Modi: Text.
- Nur `.bvfs_cleanup` ohne Argumente verglichen; das Löschen einer Handle-Tabelle ist nur durch Einheitentests belegt.

**Paritätsfälle (1):** `c002-bvfs-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:877](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_cleanup` löscht die Tabelle, die [.bvfs_restore](dot-bvfs-restore.md) für eine Restore-Auswahl angelegt hat. Man ruft es auf, nachdem der Restore mit [restore](restore.md) `file=?<tabelle>` übergeben (oder verworfen) wurde. Der BVFS-Cache bleibt erhalten.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Mit OVFS entstehen keine Zwischentabellen.

## Syntax

```text
.bvfs_cleanup path=b2<zahl>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `path=b2<zahl>` | Tabellenname aus `.bvfs_restore` | Zu löschende Tabelle. Namen, die nicht mit `b2` und einer Zahl gebildet sind, werden stillschweigend übergangen; eine nicht vorhandene Tabelle ist kein Fehler. |

## API-Modi

`.api 0` und `.api 1` geben bei Erfolg nichts aus. `.api 2` antwortet mit leerem `result` (siehe Beispiel auf [.bvfs_restore](dot-bvfs-restore.md)).

## Beispiele

### Tabelle löschen, ungültiger Name, fehlendes Argument

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup
Can't find path argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup path=b9999
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup path=nosuch
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Can't find path argument` | `path=` fehlt. |

## Siehe auch

- [.bvfs_restore](dot-bvfs-restore.md) — Tabelle anlegen
- [restore](restore.md) — Restore mit `file=?<tabelle>`
- [.bvfs_clear_cache](dot-bvfs-clear-cache.md) — BVFS-Cache leeren
