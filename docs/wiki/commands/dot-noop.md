# . — Leerbefehl

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L219) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** no op

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text.

**Paritätsfälle (3):** `c007-lone-dot`, `c007-dot-with-args`, `dot-noop`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:68](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

Ein einzelner Punkt `.` als Befehl tut nichts und antwortet ohne Text. Programme schicken ihn, um zu prüfen, ob die Verbindung noch steht, oder um nach einem Fehler sicher wieder am Hauptprompt zu sein. Der Befehl verändert nichts.

Innerhalb eines Dialogs hat der Punkt eine andere Aufgabe: Als Antwort auf eine Frage des Directors (Auswahlliste, „Turn on or off?“ usw.) bricht `.` den Dialog ab. Das ist dann kein Befehl, sondern eine Antwort.

## Syntax

```text
.
. <beliebige wörter>
```

## Argumente

Der Befehl hat keine Argumente. Wörter hinter dem Punkt werden ignoriert, Leerzeichen davor und danach ebenfalls. Zwei Punkte (`..`) sind dagegen ein unbekannter Befehl.

## API-Modi

Unter `.api 0` kommt keine Antwort, nur das Ende-Signal `EOD`. Unter `.api 1` kommt kein Text; die Signalfolge ist `CMD_BEGIN`, `CMD_BEGIN`, `CMD_OK`, `CMD_OK`, `MAIN_PROMPT` (-16, -16, -15, -15, -18), also ein doppelter Rahmen, weil der Befehl selbst noch einmal Anfang und Ende meldet. Unter `.api 2` gilt dieselbe Signalfolge, dazu kommt das leere Ergebnis `{"jsonrpc": "2.0", "id": null, "result": {}}`. Programme, die Antworten an den Signalen zerlegen, müssen den doppelten Rahmen berücksichtigen (C-061).

Zum Vergleich: Eine ganz leere Zeile beantwortet der Director unter `.api 1/2` nur mit `MAIN_PROMPT`, unter `.api 0` mit `EOD`. oconsole schickt leere Zeilen gar nicht erst ab.

## Beispiele

### Leerbefehl im Textmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Wörter hinter dem Punkt

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> . foo bar=1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

Es kommt kein Text; nur die Signale zeigen das Ende der Antwort.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im JSON-Modus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Dialog abbrechen

`automount` ohne Argument fragt nach; `.` bricht die Frage ab, ohne etwas zu ändern.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount
Turn on or off?
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Zwei Punkte

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> ..
..: is an invalid command.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Berechtigung

Auch der Leerbefehl unterliegt der `Command ACL`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .
.: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.` nicht. |
| `..: is an invalid command.` | Mehr als ein Punkt ist kein gültiger Befehl. |

## Siehe auch

- [.api](dot-api.md) — Ausgabemodi und Rahmensignale
- [.quit](dot-quit.md) — Verbindung beenden
- [automount](automount.md) — Beispiel eines Befehls mit Rückfrage
