# exit — Sitzung beenden

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L339) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Terminate Bconsole session

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text, Konsolen parity-limited.

**Paritätsfälle (2):** `c004-exit-limited`, `exit`.

**Lokaler Handler:** [crates/odir/src/commands/basic.rs:62](../../../crates/odir/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`exit` beendet die Konsolensitzung und ist gleichwertig zu [`quit`](quit.md): Der Director schließt die Verbindung, ohne eine Antwort zu schicken, und oconsole beendet sich. `exit` ist für jede Konsole erlaubt, auch wenn ihre `CommandACL` den Befehl nicht nennt.

## Syntax

```text
exit
```

Abgekürzt bedeutet `ex` ebenfalls `exit` (der Director nimmt den ersten passenden Befehl seiner Tabelle; `exit` steht vor `export`). Für `export` muss man daher mindestens `exp` schreiben. oconsole erkennt nur das ausgeschriebene `exit` als Abmeldung und meldet bei der Abkürzung `Connection closed`.

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `exit` werden ignoriert.

## API-Modi

Wie bei [`quit`](quit.md): Unter `.api 0` und `.api 1` kommt keine Antwort, unter `.api 2` vor dem Schließen ein leeres Ergebnis `{"jsonrpc": "2.0", "id": null, "result": {}}` (mehrzeilig eingerückt).

## Beispiele

### Sitzung beenden

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> exit
```

### Eingeschränkte Konsole beendet die Sitzung

Auch die Konsole `parity-limited`, deren Befehlsliste `exit` nicht enthält, kann sich abmelden (Paritätsfall `c004-exit-limited`).

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> exit
```

### Abkürzung `ex`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> ex
Connection closed
```

## Fehlermeldungen

`exit` gibt keine Fehlermeldungen aus.

## Siehe auch

- [quit](quit.md) — gleichwertiger Befehl, mit JSON-Beispiel
- [.quit](dot-quit.md) — Punktbefehl mit derselben Wirkung
