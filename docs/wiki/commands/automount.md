# automount — Automatisches Mounten nach label schalten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L313) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Automount after label

**Originalsyntax:**

```text
automount on | off
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 14 von 14 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI.

**Paritätsfälle (14):** `c005-automount-off`, `c005-automount-foo`, `c005-automount-upper`, `c005-automount-noarg`, `c005-automount-noarg-dot`, `c005-automount-twoargs`, `c005-automount-on-api1`, `c005-automount-noarg-api1`, `c005-automount-on-api2`, `c005-automount-noarg-api2`, `c005-automount-prompt-then-command`, `c036-getcmd-dot-messages`, `c036-getcmd-dot-messages-autodisplay`, `automount`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:58](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`automount` legt für die laufende Konsolensitzung fest, ob ein Volume nach `label` automatisch eingehängt (gemountet) wird. Die Einstellung gilt nur für diese Sitzung und ist ab Sitzungsbeginn eingeschaltet. Der Befehl verändert weder Katalog noch Konfiguration noch Daemons und gibt keine Bestätigung aus.

## Syntax

```text
automount on | off
automount
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `on` | – | Nach `label` automatisch mounten. |
| `off` | – | Nicht automatisch mounten. Groß- und Kleinschreibung spielt keine Rolle (`OFF` wirkt wie `off`). |

Jedes andere Wort als `off` schaltet automount ein, ohne Fehlermeldung (`automount foo` wirkt wie `automount on`). Steht nicht genau ein Wort hinter `automount`, fragt der Director nach.

## Dialoge

Ohne Argument (oder mit mehr als einem) fragt der Director `Turn on or off?`. Die Antwort wird wie das Argument ausgewertet: `off` schaltet aus, jede andere Antwort ein. Auch `.` schaltet daher ein; die Antwort bestätigt der Director nicht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount
Turn on or off? 
> off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount on off
Turn on or off? 
> on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## API-Modi

Unter `.api 0` und `.api 1` gibt `automount on` bzw. `automount off` nichts aus. Unter `.api 2` kommt ein leeres `result`. Ohne Argument fragt der Director unter `.api 1` mit einer freien Texteingabe; den Fragetext schickt er erst mit der Antwort. Unter `.api 2` fragt er gar nicht: `automount` ohne Argument endet sofort mit leerem `result`, und ein danach eingegebenes `off` gilt als neuer, unbekannter Befehl.

## Beispiele

### Automount ausschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Automount einschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekanntes Wort

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount foo
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Punkt als Antwort

Ein `.` bricht hier nicht ab, sondern schaltet automount ein.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount
Turn on or off? 
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1 mit Rückfrage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount
input> off
Turn on or off? 
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> off
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
          "off: is an invalid command.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`automount` gibt keine eigenen Fehlermeldungen aus. Die Meldung `off: is an invalid command.` unter `.api 2` entsteht, weil dort keine Rückfrage kommt und `off` als eigener Befehl gesendet wird.

## Siehe auch

- [label](label.md) — Volume beschriften; danach wird bei eingeschaltetem automount gemountet
- [mount](mount.md) — Volume von Hand einhängen
- [unmount](unmount.md) — Volume aushängen
