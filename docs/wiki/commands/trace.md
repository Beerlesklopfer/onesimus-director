# trace — Debug-Ausgaben in eine Datei schreiben

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L485) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Turn on/off trace to file

**Originalsyntax:**

```text
trace on | off
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 1.
- Verglichen sind Dialog und Abbruch; das Schreiben der Trace-Datei ist nur durch Einheitentests belegt.

**Paritätsfälle (3):** `c011-trace-cancel`, `c011-trace-api1-cancel`, `trace-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:104](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`trace on` lässt den Director seine Debug-Ausgaben in die Datei `bareos-dir.trace` im Arbeitsverzeichnis des Directors (`WorkingDirectory`, ohne Angabe `/var/lib/bareos`) schreiben; `trace off` beendet das und schließt die Datei. Wie viel geschrieben wird, bestimmt der Debug-Level, der mit [`setdebug`](setdebug.md) eingestellt wird. Die Einstellung gilt für den ganzen Director, nicht nur für die eigene Sitzung, und bleibt bis zum Ausschalten oder Neustart bestehen.

## Syntax

```text
trace on | off
trace
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `off` | — | Trace ausschalten (Groß- und Kleinschreibung egal) |
| `on` | — | Trace einschalten |

Nur `off` schaltet aus; **jedes andere Wort schaltet ein**, auch Tippfehler wie `trace of`. Steht nicht genau ein Wort hinter `trace` (kein Wort oder mehrere), fragt der Director nach (siehe Dialoge).

## Dialoge

Ohne Argument (oder mit mehr als einem) fragt der Director `Turn on or off?`. Die Antwort `off` schaltet aus, jede andere Antwort ein. Ein einzelner `.` bricht ab, ohne etwas zu ändern.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> trace
Turn on or off?
> off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## API-Modi

Erfolgreich kommt in keinem Modus Text zurück. Unter `.api 1` und `.api 2` kommt die Frage als Texteingabe (oconsole zeigt `input> `). Wie im Original schickt der Director den Fragetext dabei erst nach der Antwort; im Terminal erscheint `Turn on or off?` also hinter der Eingabe (Paritätsfall `c011-trace-api1-cancel`).

## Beispiele

### Trace ein- und ausschalten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> trace on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> trace off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nachfrage abbrechen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> trace
Turn on or off?
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nachfrage unter `.api 1`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> trace
input> .
Turn on or off?
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `trace: <pfad>: <Fehler>` | Nur Onesimus: Die Trace-Datei lässt sich nicht öffnen, etwa weil das Arbeitsverzeichnis fehlt oder nicht beschreibbar ist. Rechte von `WorkingDirectory` prüfen. Das Original meldet hier nichts. Beispielausgabe nicht verfügbar: Der Testaufbau hat ein beschreibbares Arbeitsverzeichnis. |

## Siehe auch

- [setdebug](setdebug.md) — Debug-Level einstellen, auch `trace=1` für Client und Storage
- [status](status.md) — Status des Directors
