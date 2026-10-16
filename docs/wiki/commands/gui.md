# gui — Modus für Programme ein- oder ausschalten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L346) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Switch between interactive (gui off) and non-interactive (gui on) mode

**Originalsyntax:**

```text
gui on | off
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI.

**Paritätsfälle (30):** `c005-gui-noarg`, `c005-gui-foo`, `c005-gui-upper`, `c005-gui-on-off`, `c005-gui-foo-api2`, `c005-gui-on-api2`, `c005-automount-prompt-then-command`, `c027-gui-list-pools`, `c027-gui-llist-pools`, `c027-gui-list-jobs`, `c027-gui-llist-jobs`, `c027-gui-list-media`, `c027-gui-llist-media`, `c027-gui-list-clients`, `c027-gui-list-jobtotals`, `c027-gui-list-jobmedia`, `c027-gui-list-filesets`, `c027-gui-list-log`, `c027-gui-sql`, `c027-gui-sqlquery-keyword`, `c027-gui-status-dir`, `c027-gui-show`, `c027-gui-messages`, `c027-gui-off-again`, `c027-gui-noarg`, `c027-gui-api1`, `c027-gui-api2`, `c036-gui-on-messages`, `gui-on`, `gui-off`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:66](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`gui on` schaltet die Sitzung in einen Modus für grafische Oberflächen und Skripte, `gui off` zurück in den interaktiven Normalmodus (Standard). Die Einstellung gilt nur für die eigene Sitzung bis zu deren Ende. Im GUI-Modus ändert sich:

- **Katalogauswahl:** Ohne `catalog=` nimmt der Director stillschweigend den ersten Katalog der Konfiguration. Die Zeile `Automatically selected Catalog: …` und die Auswahlliste bei mehreren Katalogen entfallen.
- **Zahlen in Tabellen:** Zahlen aus dem Katalog erscheinen ohne Tausenderpunkte und linksbündig (`824021543` statt `824,021,543`).
- **Nachrichtenhinweis:** Der Hinweis `You have messages.` nach einem Befehl entfällt.

## Syntax

```text
gui on | off
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `on` | — | GUI-Modus einschalten |
| `off` | — | GUI-Modus ausschalten (Standard) |

Groß- und Kleinschreibung ist egal (`gui OFF`). Der Director sucht unter allen Wörtern hinter `gui` das erste, das `on` oder `off` heißt; andere Wörter davor werden übersprungen. Ein Wert wie `on=1` zählt als `on`.

## API-Modi

Erfolgreich kommt in keinem Modus Text zurück; unter `.api 2` ist es ein leeres Ergebnis (`"result": {}`). Fehlt `on`/`off`, kommt unter `.api 0` und `.api 1` die Zeile `ON or OFF keyword missing.`, unter `.api 2` ein JSON-Fehler mit dieser Meldung (siehe Beispiel).

## Beispiele

### GUI-Modus einschalten und eine Liste abrufen

Nach `gui on` erscheinen die Zahlen ohne Formatierung und linksbündig, und der Katalog wird ohne Hinweis gewählt (Paritätsfall `c027-gui-list-pools`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
Using Catalog "MyCatalog"
+--------+--------------+---------+---------+----------+---------------+
| poolid | name         | numvols | maxvols | pooltype | labelformat   |
+--------+--------------+---------+---------+----------+---------------+
| 1      | Scratch      | 0       | 0       | Scratch  | *             |
| 2      | Incremental  | 1       | 100     | Backup   | Incremental-  |
| 3      | Full         | 3       | 100     | Backup   | Full-         |
| 4      | Differential | 1       | 100     | Backup   | Differential- |
+--------+--------------+---------+---------+----------+---------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### GUI-Modus ausschalten

Nach `gui off` sind die Zahlen wieder rechtsbündig.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> list pools
+--------+--------------+---------+---------+----------+---------------+
| poolid | name         | numvols | maxvols | pooltype | labelformat   |
+--------+--------------+---------+---------+----------+---------------+
|      1 | Scratch      |       0 |       0 | Scratch  | *             |
|      2 | Incremental  |       1 |     100 | Backup   | Incremental-  |
|      3 | Full         |       3 |     100 | Backup   | Full-         |
|      4 | Differential |       1 |     100 | Backup   | Differential- |
+--------+--------------+---------+---------+----------+---------------+
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Schlüsselwort fehlt

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui
ON or OFF keyword missing.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Schlüsselwort fehlt, JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui foo
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
          "ON or OFF keyword missing.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> gui on
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
| `ON or OFF keyword missing.` | Hinter `gui` steht weder `on` noch `off`. Befehl mit `on` oder `off` wiederholen. |

## Siehe auch

- [autodisplay](autodisplay.md) — Nachrichten automatisch anzeigen
- [.api](dot-api.md) — Maschinenformate für Programme (`.api 1`, `.api 2`)
- [use](use.md) — Katalog ausdrücklich wählen
- [list](list.md) — Katalogeinträge auflisten
