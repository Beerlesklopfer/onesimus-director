# .profiles — Namen der Profile auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L263) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all profile resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_profiles-api2`, `c025-profiles-limited`, `c025-profiles-limited-api2`, `c025-profiles-operator`, `c025-profiles-operator-api2`, `c025-profiles-bvfs`, `c025-profiles-bvfs-api2`, `c025-profiles-acl`, `c025-profiles-acl-api2`, `dot_profiles`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:375](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.profiles` listet die Namen aller Profile-Ressourcen, einen pro Zeile. Profile bündeln ACLs, die Console- und User-Ressourcen über `Profile = <name>` übernehmen. Oberflächen und die Tab-Vervollständigung nutzen den Befehl. Der Befehl liest nur.

## Syntax

```text
.profiles
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert (Paritätsfall `dcfg-profiles-arg`).

Die Reihenfolge ist die interne Reihenfolge der Konfiguration, nicht alphabetisch. Die Liste wird nicht nach ACLs gefiltert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Liste. Unter `.api 2` steht sie in `result.profiles` als Array von Objekten mit dem Feld `name`.

## Beispiele

### Profile auflisten

Paritätsfall `dot_profiles`. `c019g` und `c019s` wurden vorher mit `configure add profile` angelegt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .profiles
webui-readonly
webui-admin
parity-acl
operator
c019g
c019s
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Profile als JSON

Paritätsfall `dcfg-profiles-api2`; hier wurden zuvor `c019s`, `c019g` und `dcfg-prof` angelegt und die Konfiguration neu geladen.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .profiles
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "profiles": [
      {
        "name": "webui-readonly"
      },
      {
        "name": "webui-admin"
      },
      {
        "name": "parity-acl"
      },
      {
        "name": "operator"
      },
      {
        "name": "dcfg-prof"
      },
      {
        "name": "c019s"
      },
      {
        "name": "c019g"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole mit Profil

`parity-operator` erhält seine Rechte über das Profil `operator` und darf `.profiles` ausführen (Paritätsfall `c025-profiles-operator`).

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .profiles
webui-readonly
webui-admin
parity-acl
operator
c019g
c019s
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Berechtigung

Paritätsfall `c025-profiles-limited`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .profiles
.profiles: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.profiles: is an invalid command.` | Die Konsole darf `.profiles` nicht ausführen (`CommandACL`). |

## Siehe auch

- [.consoles](dot-consoles.md) — Namen aller Console-Ressourcen
- [.users](dot-users.md) — Namen aller User-Ressourcen
- [show](show.md) — `show profiles` zeigt die Profile mit ihren ACLs
- [configure](configure.md) — Profil zur Laufzeit anlegen
- [.authorized](dot-authorized.md) — eine einzelne Berechtigung prüfen
