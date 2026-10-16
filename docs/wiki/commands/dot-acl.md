# .acl — Konsolen-ACLs verwalten (Onesimus, Platzhalter)

## Zweck

`.acl` ist eine Onesimus-Erweiterung zur Verwaltung der ACLs von Console-Ressourcen; das Original kennt den Befehl nicht und antwortet `.acl: is an invalid command.`. Der Befehl ist ein Platzhalter: `list` zeigt nur die Namen der Konsolen ohne ihre ACLs, `set`, `delete`, `clear` und `copy` bestätigen den Aufruf, ändern aber nichts an der laufenden Konfiguration oder an den Dateien. ACLs ändert man in den Dateien unter `bareos-dir.d/console/` und `bareos-dir.d/profile/` und lädt sie mit [reload](reload.md); ansehen kann man sie mit `show console=<name>`.

## Syntax

```text
.acl list [console=<name>]
.acl set console=<name> <typ>acl=<wert>[,<wert> …] …
.acl delete console=<name> <typ>acl=<wert>
.acl clear console=<name>
.acl copy from=<console> to=<console>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `list` | – | Konsolen auflisten, je Konsole mit dem Hinweis `(ACL configuration pending)`. Die Standardkonsole `*UserAgent*` fehlt in der Liste. |
| `set` | – | Gibt die übergebenen ACL-Werte als Bestätigung aus; keine Wirkung. |
| `delete` | – | Bestätigt das Löschen; keine Wirkung. |
| `clear` | – | Bestätigt das Leeren aller ACLs; keine Wirkung. |
| `copy` | – | Bestätigt das Kopieren; keine Wirkung. |
| `console=<name>` | Konsolenname | Bei `list` optional (Filter), bei `set`, `delete` und `clear` Pflicht. Ein unbekannter Name wird nicht geprüft. |
| `<typ>acl=<wert>` | z. B. `jobacl`, `clientacl`, `storageacl`, `poolacl`, `filesetacl`, `catalogacl`, `scheduleacl`, `commandacl`, `whereacl`, `pluginoptionsacl` | Bei `set` wird jedes Argument, dessen Schlüssel auf `acl` endet, in der Bestätigung wiederholt. |
| `from=<console>`, `to=<console>` | Konsolennamen | Bei `copy` beide Pflicht. |

Ohne oder mit unbekanntem Unterbefehl kommt die Kurzhilfe. `.profile` ist kein Kurzname für `.acl`: wie im Original wird `.profile` zu [.profiles](dot-profiles.md) ergänzt und listet die Profile.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Textausgabe. Unter `.api 2` antwortet der Befehl nur mit einem leeren Ergebnis `"result": {}`; der Text geht verloren.

## Beispiele

### Kurzhilfe

Paritätsfall `dcfg-acl-acl`; ebenso bei `.acl foo` (`dcfg2-acl-unknown-sub`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl
.acl: specify one of:
  .acl list [console=<name>]
  .acl set console=<name> jobacl=<job1>,<job2>
  .acl delete console=<name> jobacl=<job>
  .acl clear console=<name>
  .acl copy from=<console1> to=<console2>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original kennt `.acl` nicht und antwortet `.acl: is an invalid command.` (Onesimus-Erweiterung, kein Kompatibilitätseintrag).

### Alle Konsolen auflisten

Paritätsfall `dcfg-acl-acl_list`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl list
Console: parity-operator
  (ACL configuration pending)

Console: parity-listacl
  (ACL configuration pending)

[…]
Console: c019e
  (ACL configuration pending)

Console: bareos-mon
  (ACL configuration pending)

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eine Konsole auflisten

Paritätsfälle `dcfg-acl-acl_list_console-parity-limited` und `dcfg-acl-acl_list_console-nosuch`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl list console=parity-limited
Console: parity-limited
  (ACL configuration pending)

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl list console=nosuch
No consoles configured
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### ACLs setzen

Die Antwort wiederholt nur die Eingabe; die ACLs der Konsole bleiben unverändert (Paritätsfall `dcfg-acl-acl_set_console-parity-limited_jobacl-a,b_clientacl-c`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl set console=parity-limited jobacl=a,b clientacl=c
ACL set for console 'parity-limited':
  jobacl = a,b
  clientacl = c
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### ACL-Einträge löschen und leeren

Paritätsfälle `dcfg-acl-acl_delete_console-parity-limited_jobacl-a` und `dcfg-acl-acl_clear_console-parity-limited`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl delete console=parity-limited jobacl=a
ACL entries deleted for console 'parity-limited'
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl clear console=parity-limited
All ACLs cleared for console 'parity-limited'
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### ACLs kopieren

Paritätsfall `dcfg-acl-acl_copy_from-parity-limited_to-parity-acl`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl copy from=parity-limited to=parity-acl
ACLs copied from 'parity-limited' to 'parity-acl'
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlende Argumente

Paritätsfälle `dcfg-acl-acl_set` und `dcfg-acl-acl_copy_from-x`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl set
.acl set: console=<name> required
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl copy from=x
.acl copy: from=<console> to=<console> required
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Aufruf unter `.api 2`

Paritätsfall `dcfg-acl-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl list console=parity-limited
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
| `.acl: specify one of:` mit der Kurzhilfe | Unterbefehl fehlt oder ist unbekannt. |
| `.acl set: console=<name> required` | `console=` fehlt (ebenso `.acl delete: …` und `.acl clear: …`). |
| `.acl copy: from=<console> to=<console> required` | `from=` oder `to=` fehlt. |
| `No consoles configured` | Zu `console=<name>` gibt es keine Console-Ressource. |

## Siehe auch

- [show](show.md) — `show console=<name>` und `show profile=<name>` zeigen die tatsächlichen ACLs
- [.authorized](dot-authorized.md) — eine einzelne Berechtigung prüfen
- [.consoles](dot-consoles.md) — Namen aller Console-Ressourcen
- [.profiles](dot-profiles.md) — Namen aller Profile
- [reload](reload.md) — geänderte ACL-Dateien laden
