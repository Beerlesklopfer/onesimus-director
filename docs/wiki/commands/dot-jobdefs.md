# .jobdefs — Namen der JobDefs auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L245) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all job defaults resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c025-jobdefs-limited`, `c025-jobdefs-limited-api2`, `c025-jobdefs-operator`, `c025-jobdefs-operator-api2`, `c025-jobdefs-bvfs`, `c025-jobdefs-bvfs-api2`, `c025-jobdefs-acl`, `c025-jobdefs-acl-api2`, `dot_jobdefs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:256](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.jobdefs` gibt die Namen aller JobDefs-Ressourcen der Director-Konfiguration aus, einen pro Zeile. JobDefs sind Vorlagen, von denen Jobs ihre Vorgabewerte erben; Konfigurationswerkzeuge brauchen die Liste etwa beim Anlegen neuer Jobs. Der Befehl liest nur die Konfiguration und verändert nichts.

Es erscheinen nur JobDefs, die die Konsole laut `Job ACL` sehen darf.

## Syntax

```text
.jobdefs
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.jobdefs`, ein Array von Objekten mit `name`.

## Beispiele

### Alle JobDefs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobdefs
DefaultJob
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobdefs
DefaultJob
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobdefs
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobdefs": [
      {
        "name": "DefaultJob"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Berechtigung

Die Konsole `parity-limited` hat `.jobdefs` nicht in ihrer `Command ACL`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .jobdefs
.jobdefs: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.jobdefs: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.jobdefs` nicht. |

## Siehe auch

- [.jobs](dot-jobs.md) — Namen der Jobs
- [show](show.md) — `show jobdefs=<name>` zeigt die Vorlage
- [configure](configure.md) — Job anlegen, z. B. mit `jobdefs=`
