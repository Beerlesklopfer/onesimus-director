# quit — Sitzung beenden

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L399) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Terminate Bconsole session

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 5 von 5 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (5):** `c004-quit-abbrev`, `c004-quit-after-command`, `c004-quit-api1`, `c004-quit-json`, `quit`.

**Lokaler Handler:** [crates/odir/src/commands/basic.rs:62](../../../crates/odir/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`quit` beendet die Konsolensitzung. Der Director schließt die Verbindung, ohne vorher eine Antwort zu schicken; oconsole beendet sich danach. Laufende Jobs, Einstellungen anderer Sitzungen und die Konfiguration bleiben unberührt. [`exit`](exit.md) und der Punktbefehl [`.quit`](dot-quit.md) wirken genauso.

## Syntax

```text
quit
```

Wie jeder Befehl darf `quit` abgekürzt werden; der Director nimmt den ersten Befehl seiner Tabelle, der mit dem Eingegebenen beginnt. `q` und `qu` bedeuten `quit` (nicht `query`). oconsole erkennt nur die ausgeschriebenen Formen `quit`, `exit` und `.quit` als Abmeldung; bei einer Abkürzung schließt der Director die Verbindung trotzdem, und oconsole meldet dann `Connection closed`.

`quit` und `exit` sind immer erlaubt, auch wenn die `CommandACL` einer Konsole sie nicht nennt.

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `quit` werden ignoriert.

## Dialoge

`quit` fragt nicht nach. Steht man gerade in einem Dialog eines anderen Befehls (Prompt `> `), wird `quit` dort als Antwort gelesen und nicht als Befehl; zuerst mit `.` abbrechen.

## API-Modi

Unter `.api 0` und `.api 1` schickt der Director nichts und schließt die Verbindung. Unter `.api 2` schickt er vorher noch ein leeres Ergebnis:

```text
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
```

## Beispiele

### Sitzung beenden

Nach `quit` kehrt oconsole zur Shell zurück.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie) debian Debian GNU/Linux 13 (trixie)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> quit
```

### Mit Abkürzung beenden

Der Director beendet die Sitzung auch bei `q` (Paritätsfall `c004-quit-abbrev`); die letzte Zeile stammt von oconsole.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> q
Connection closed
```

### Sitzung im JSON-Modus beenden

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> quit
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
```

## Fehlermeldungen

`quit` gibt keine Fehlermeldungen aus. Meldungen nach dem Beenden kommen von oconsole selbst, etwa `Connection closed`, wenn die Verbindung unerwartet abbricht.

## Siehe auch

- [exit](exit.md) — gleichwertiger Befehl
- [.quit](dot-quit.md) — Punktbefehl mit derselben Wirkung
- [wait](wait.md) — vor dem Beenden auf laufende Jobs warten
