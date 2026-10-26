# .filesets — Namen der FileSets auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L241) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all filesets

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 10 von 10 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (10):** `c003-dot_filesets-api2`, `c025-filesets-limited`, `c025-filesets-limited-api2`, `c025-filesets-operator`, `c025-filesets-operator-api2`, `c025-filesets-bvfs`, `c025-filesets-bvfs-api2`, `c025-filesets-acl`, `c025-filesets-acl-api2`, `dot_filesets`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:123](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.filesets` gibt die Namen aller FileSet-Ressourcen der Director-Konfiguration aus, einen pro Zeile. Programme nutzen ihn für die Vervollständigung von `fileset=` (etwa bei `run` oder `restore`) und für Auswahllisten. Der Befehl liest nur die Konfiguration und verändert nichts.

Es erscheinen nur FileSets, die die Konsole laut `FileSet ACL` sehen darf.

## Syntax

```text
.filesets
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.filesets`, ein Array von Objekten mit `name`.

## Beispiele

### Alle FileSets

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .filesets
Windows All Drives
SelfTest
LinuxAll
Catalog
c019r
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .filesets
Windows All Drives
SelfTest
LinuxAll
Catalog
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .filesets
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "filesets": [
      {
        "name": "Windows All Drives"
      },
      {
        "name": "SelfTest"
      },
      {
        "name": "LinuxAll"
      },
      {
        "name": "Catalog"
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit eingeschränkter Konsole

Die `FileSet ACL` der Konsole `parity-acl` schließt das FileSet `Catalog` aus.

```text
parity-acl@bareos-dir[25.1.2+onesimus.0.2.0]> .filesets
Windows All Drives
SelfTest
LinuxAll
c019r
parity-acl@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.filesets: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.filesets` nicht. |

## Siehe auch

- [show](show.md) — `show fileset=<name>` zeigt Inhalt und Optionen eines FileSets
- [list](list.md) — `list filesets` zeigt die FileSets im Katalog
- [.jobs](dot-jobs.md) — Namen der Jobs
