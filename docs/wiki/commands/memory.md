# memory — Kopfzeilen des Director-Status anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L373) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print current memory usage

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (3):** `c009-memory`, `c009-memory-json`, `memory`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:97](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`memory` heißt laut Beschreibung „Print current memory usage“, gibt aber wie das Original nur die Kopfzeilen von `status director` aus: Name, Version, Build-Datum und Betriebssystem des Directors, Startzeit, Zahl der gelaufenen und laufenden Jobs, Datenbanktyp und Art des Binärprogramms. Angaben zum Speicherverbrauch gibt es nicht. Der Befehl liest nur.

## Syntax

```text
memory
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `memory` werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommen dieselben zwei Zeilen. Unter `.api 2` steht die Information im Objekt `result.header` mit den Feldern `director`, `version`, `release_date`, `os`, `binary_info`, `daemon_started`, `jobs_run`, `jobs_running` und `config_warnings`.

## Beispiele

### Kopfzeilen anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> memory
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie)
Daemon started 16-Oct-26 16:41. Jobs: run=0, running=0 db:postgresql, Onesimus binary
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beim Original steht am Ende `Bareos community binary` statt `Onesimus binary`; das ist die Kennung des jeweiligen Programms, keine Abweichung im Verhalten.

### Kopfzeilen als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> memory
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "header": {
      "director": "bareos-dir",
      "version": "25.1.2+onesimus.0.2.0",
      "release_date": "15 October 2026",
      "os": "Debian GNU/Linux 13 (trixie)",
      "binary_info": "Onesimus",
      "daemon_started": "16-Oct-26 16:41",
      "jobs_run": 0,
      "jobs_running": 0,
      "config_warnings": false
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`memory` gibt keine eigenen Fehlermeldungen aus.

## Siehe auch

- [status](status.md) — vollständiger Status des Directors (`status director`)
- [version](version.md) — nur die Versionszeile
