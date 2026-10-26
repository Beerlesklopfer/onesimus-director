# messages — Gesammelte Meldungen anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L371) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Display pending messages

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 13 von 13 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-limited, parity-operator.
- Jobmeldungen selbst (Texte aus Jobs) sind Gegenstand von C-051, nicht dieses Befehls.

**Paritätsfälle (13):** `c027-gui-messages`, `c028-messages`, `c036-messages-twice`, `c036-messages-abbrev`, `c036-messages-args`, `c036-autodisplay-on-messages`, `c036-gui-on-messages`, `c036-messages-api1`, `c036-messages-api2`, `c036-autodisplay-on-api1`, `c036-messages-limited`, `c036-messages-operator`, `messages`.

**Lokaler Handler:** [crates/odir/src/commands/extra.rs:87](../../../crates/odir/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`messages` gibt die Meldungen aus, die der Director für die Konsole gesammelt hat, zum Beispiel Job-Berichte, Warnungen und Fehler. Welche Meldungen auf der Konsole landen, legt die `console`-Zeile der Messages-Ressource fest. Ausgegebene Meldungen werden aus der Warteschlange entfernt und erscheinen kein zweites Mal. Der Befehl verändert sonst nichts.

## Syntax

```text
messages
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `messages` werden ignoriert (`messages all` wirkt wie `messages`). Der Befehl kann abgekürzt werden, etwa `mes`.

## API-Modi

Unter `.api 0` steht bei leerer Warteschlange `You have no messages.`. Unter `.api 1` kommt bei leerer Warteschlange gar keine Ausgabe, unter `.api 2` ein leeres `result`. Gleiches gilt für den Punktbefehl `.messages`, der auch im Standardmodus bei leerer Warteschlange nichts ausgibt.

## Beispiele

### Keine Meldungen vorhanden

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
You have no messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vorhandene Meldungen abholen

Beispielausgabe nicht verfügbar: Meldungen entstehen vor allem durch Jobs, und in der Testumgebung werden keine Jobs gestartet.

### Mit autodisplay

Ist `autodisplay on` gesetzt, gibt der Director Meldungen nach jedem Befehl von selbst aus; `messages` bleibt trotzdem nutzbar.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay on
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
You have no messages.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .messages
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .messages
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> messages
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eingeschränkte Konsole

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> messages
messages: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .messages
.messages: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> autodisplay on
autodisplay: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `You have no messages.` | Die Warteschlange ist leer; kein Fehler. |
| `messages: is an invalid command.` | Die Konsole darf `messages` nicht ausführen (Command ACL). |

## Siehe auch

- [autodisplay](autodisplay.md) — Meldungen automatisch nach jedem Befehl ausgeben
- [.messages](dot-messages.md) — Meldungen für Programme abholen
- [status](status.md) — Status von Director, Client und Storage
- [show](show.md) — mit `show messages` die Messages-Ressourcen anzeigen
