# whoami — Name der eigenen Konsole anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L527) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print the user name associated with this console

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-limited, parity-operator.

**Paritätsfälle (9):** `c010-whoami-default`, `c010-whoami-named`, `c010-whoami-limited`, `c010-whoami-json`, `c010-whoami-json-named`, `c010-whoami-api1`, `c036-messages-operator`, `whoami`, `whoami-limited`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:287](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`whoami` zeigt, unter welchem Namen die aktuelle Sitzung beim Director angemeldet ist. Bei einer benannten Konsole (`Console`-Ressource, auch über ein `User`/Profil) ist das deren Name. Bei der Standardkonsole, die sich mit dem Director-Passwort anmeldet und keine Rechtebeschränkung hat, lautet die Antwort `root`. Der Befehl liest nur.

## Syntax

```text
whoami
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `whoami` werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt der Name als eine Zeile. Unter `.api 2` steht er als Zeichenkette in `result.whoami`.

## Beispiele

### Standardkonsole

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> whoami
root
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Benannte Konsole

Angemeldet als Konsole `parity-operator` (Paritätsfall `c010-whoami-named`):

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> whoami
parity-operator
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Name als JSON

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> whoami
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "whoami": "parity-operator"
  }
}
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Kompakt (`.api json compact=yes`, Standardkonsole):

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> whoami
{"jsonrpc":"2.0","id":null,"result":{"whoami":"root"}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`whoami` gibt keine Fehlermeldungen aus. Der Befehl ist für jede Konsole erlaubt, auch wenn ihre `CommandACL` ihn nicht nennt (wie `quit` und `exit`).

## Siehe auch

- [help](help.md) — welche Befehle die eigene Konsole benutzen darf
- [.authorized](dot-authorized.md) — Berechtigung für einen Befehl prüfen
- [.consoles](dot-consoles.md) — Namen der Konsolen-Ressourcen
