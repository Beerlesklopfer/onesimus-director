# .users — Namen der Benutzer auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L235) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all user resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c025-users-limited`, `c025-users-limited-api2`, `c025-users-operator`, `c025-users-operator-api2`, `c025-users-bvfs`, `c025-users-bvfs-api2`, `c025-users-acl`, `c025-users-acl-api2`, `dot_users`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:386](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.users` listet die Namen aller User-Ressourcen, einen pro Zeile. User-Ressourcen beschreiben Benutzer, die sich über eine Konsole mit PAM-Anmeldung einloggen; ihre Rechte kommen aus Profilen. Gibt es keine User-Ressource, ist die Antwort leer. Der Befehl liest nur.

## Syntax

```text
.users
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert (Paritätsfall `dcfg-users-arg`).

Die Reihenfolge ist die interne Reihenfolge der Konfiguration, nicht alphabetisch. Die Liste wird nicht nach ACLs gefiltert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Liste. Unter `.api 2` steht sie in `result.users` als Array von Objekten mit dem Feld `name`.

## Beispiele

### Keine User-Ressourcen

In der Standardkonfiguration gibt es keine User-Ressource; die Antwort ist leer (Paritätsfall `dot_users`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .users
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Benutzer auflisten

`dcfg-user` wurde mit `configure add user` angelegt, `dcfg-u` als Datei `bareos-dir.d/user/dcfg-u.conf` und mit `reload` geladen (Paritätsfall `dcfg-users-added`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
reloaded
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .users
dcfg-user
dcfg-u
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Benutzer als JSON

Paritätsfall `dcfg-users-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .users
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "users": [
      {
        "name": "dcfg-user"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Berechtigung

Paritätsfall `c025-users-limited`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .users
.users: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.users: is an invalid command.` | Die Konsole darf `.users` nicht ausführen (`CommandACL`). |

## Siehe auch

- [.consoles](dot-consoles.md) — Namen aller Console-Ressourcen
- [.profiles](dot-profiles.md) — Namen aller Profile
- [show](show.md) — `show users` zeigt die User-Ressourcen mit allen Direktiven
- [configure](configure.md) — User-Ressource zur Laufzeit anlegen
