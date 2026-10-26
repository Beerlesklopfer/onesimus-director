# version — Version des Directors anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L521) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print Director version

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (9):** `c004-quit-after-command`, `c009-version`, `c009-version-json`, `c009-version-api1`, `c020-compact-version`, `c020-json-version`, `c020-compact-persists`, `c020-compact-reset`, `version`.

**Lokaler Handler:** [crates/odir/src/commands/basic.rs:13](../../../crates/odir/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`version` zeigt Name, Version und Build-Datum des Directors sowie das Betriebssystem, auf dem er läuft. Der Befehl liest nur und verändert nichts. Man braucht ihn zum Beispiel für Fehlerberichte oder um nach einem Update zu prüfen, ob der neue Director läuft.

## Syntax

```text
version
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `version` werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Textzeile. Unter `.api 2` steht die Information als JSON-Objekt `result.version` mit den Feldern `name`, `type`, `version`, `bdate`, `operatingsystem`, `distname`, `distversion` und `customversionid`. Mit `.api json compact=yes` kommt dasselbe JSON in einer Zeile.

## Beispiele

### Version anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie) debian Debian GNU/Linux 13 (trixie)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Version als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": {
      "name": "bareos-dir",
      "type": "bareos-director",
      "version": "25.1.2+onesimus.0.2.0",
      "bdate": "15 October 2026",
      "operatingsystem": "Debian GNU/Linux 13 (trixie)",
      "distname": "debian",
      "distversion": "Debian GNU/Linux 13 (trixie)",
      "customversionid": ""
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Version als kompaktes JSON

Mit `compact=yes` steht die ganze Antwort in einer Zeile (Paritätsfall `c020-compact-version`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
{"jsonrpc":"2.0","id":null,"result":{"version":{"name":"bareos-dir","type":"bareos-director","version":"25.1.2+onesimus.0.2.0","bdate":"15 October 2026","operatingsystem":"Debian GNU/Linux 13 (trixie)","distname":"debian","distversion":"Debian GNU/Linux 13 (trixie)","customversionid":""}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`version` gibt keine eigenen Fehlermeldungen aus.

## Siehe auch

- [status](status.md) — Status des Directors mit Startzeit und laufenden Jobs
- [memory](memory.md) — Kopfzeilen des Director-Status
- [time](time.md) — aktuelle Uhrzeit des Directors
- [.api](dot-api.md) — API-Modus umschalten
