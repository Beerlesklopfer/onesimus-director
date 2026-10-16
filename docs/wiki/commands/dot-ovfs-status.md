# .ovfs_status — OVFS-Extension und Restore-Baum-Weg anzeigen

## Zweck

`.ovfs_status` zeigt, ob die PostgreSQL-Extension `ovfs` in der Katalogdatenbank installiert und nutzbar ist und welcher Weg die `.ovfs_*`-Befehle beantwortet: die Extension (`ovfs`) oder die gleichwertige SQL-Umsetzung im Director (`fallback`). Beide Wege liefern dieselben Ergebnisse; die Extension ist bei großen Katalogen schneller. Der Befehl liest nur.

OVFS ist eine Onesimus-Erweiterung ohne Gegenstück in Bareos. Es ist der Onesimus-Weg für Restore-Bäume; BVFS ([.bvfs_lsdirs](dot-bvfs-lsdirs.md) …) bleibt daneben unverändert für Bareos-Werkzeuge erhalten.

## Syntax

```text
.ovfs_status [refresh] [json]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `refresh` | – | Die Extension jetzt neu prüfen, statt das gespeicherte Ergebnis zu zeigen. |
| `json` | – | Das Ergebnis als JSON-Zeile ausgeben, in jedem API-Modus. |

Der Director prüft die Extension bei jeder neuen Katalogverbindung, nach `reload` und wenn ein Aufruf der Extension scheitert. Nutzbar ist sie nur, wenn sie in Version 0.1.x installiert ist, die Katalogrolle USAGE auf ihr Schema hat und alle benötigten Funktionen (`ovfs.tree`, `ovfs.tree_latest`, `ovfs.jobids_latest`, `ovfs.ls`, `ovfs.versions`) mit den erwarteten Signaturen und Ergebnistypen existieren und ausführbar sind. Sonst antwortet der Fallback; die Textzeile nennt den Grund.

## API-Modi

`.api 0` und `.api 1` zeigen zwei Textzeilen: `OVFS extension: <zustand>` und `Restore trees: ovfs extension` oder `Restore trees: Director fallback (plain SQL)`. `.api 2` und das Schlüsselwort `json` liefern das Objekt `ovfs` mit den Feldern `state` (`unknown`, `check_failed`, `not_installed`, `unusable`, `available`), `installed`, `usable`, `version`, bei `available` zusätzlich `schema`, bei `unusable` und `check_failed` `reason`, und `path` (`ovfs` oder `fallback`).

Mögliche Zustände in der Textzeile:

| Text | Bedeutung |
|---|---|
| `version <v> available (schema <s>)` | Extension nutzbar, die `.ovfs_*`-Befehle verwenden sie. |
| `not installed` | Keine Extension `ovfs` in der Datenbank; der Fallback antwortet. |
| `version <v> installed but not usable: <grund>` | Installiert, aber falsche Version, fehlende Rechte oder Funktionen. |
| `check failed: <fehler>` | Die Prüfung selbst ist gescheitert. |
| `not checked yet` | Noch keine Katalogverbindung geprüft. |

## Beispiele

### Status ohne installierte Extension

Auf dem Testsystem ist die Extension nicht installiert; der Director rechnet selbst.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_status
OVFS extension: not installed
Restore trees: Director fallback (plain SQL)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_status refresh
OVFS extension: not installed
Restore trees: Director fallback (plain SQL)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_status json
{"ovfs":{"state":"not_installed","installed":false,"usable":false,"version":null,"path":"fallback"}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beispielausgabe nicht verfügbar: Mit installierter Extension (`version 0.1.x available (schema ovfs)`) gibt es auf dem Testsystem keine Katalogdatenbank.

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_status
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "ovfs": {
      "state": "not_installed",
      "installed": false,
      "usable": false,
      "version": null,
      "path": "fallback"
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Recht auf den Befehl

```text
parity-bvfs@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_status
.ovfs_status: is an invalid command.
parity-bvfs@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `No catalog configured.` | Der Director hat keinen Katalog. |
| `.ovfs_status: is an invalid command.` | Der Befehl steht nicht in der Command ACL der Konsole. |

## Siehe auch

- [.ovfs_tree](dot-ovfs-tree.md) — Restore-Baum einer Job-Kette
- [.ovfs_ls](dot-ovfs-ls.md) — ein Verzeichnis der Job-Kette
- [.ovfs_latest](dot-ovfs-latest.md) — Job-Kette des neuesten Stands
- [.ovfs_versions](dot-ovfs-versions.md) — Versionen einer Datei
