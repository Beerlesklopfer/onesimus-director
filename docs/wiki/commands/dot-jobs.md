# .jobs — Namen der Jobs auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L247) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all job resources

**Originalsyntax:**

```text
.jobs [type=<jobtype>] | [enabled | disabled]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 31 von 31 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (31):** `c003-dot_jobs-enabled`, `c020-compact-jobs`, `c025-jobs-limited`, `c025-jobs-limited-api2`, `c025-jobs-enabled-limited`, `c025-jobs-enabled-limited-api2`, `c025-jobs-type-B-limited`, `c025-jobs-type-B-limited-api2`, `c025-jobs-operator`, `c025-jobs-operator-api2`, `c025-jobs-enabled-operator`, `c025-jobs-enabled-operator-api2`, `c025-jobs-type-B-operator`, `c025-jobs-type-B-operator-api2`, `c025-jobs-bvfs`, `c025-jobs-bvfs-api2`, `c025-jobs-enabled-bvfs`, `c025-jobs-enabled-bvfs-api2`, `c025-jobs-type-B-bvfs`, `c025-jobs-type-B-bvfs-api2`, `c025-jobs-acl`, `c025-jobs-acl-api2`, `c025-jobs-enabled-acl`, `c025-jobs-enabled-acl-api2`, `c025-jobs-type-B-acl`, `c025-jobs-type-B-acl-api2`, `c028-dot-jobs`, `dot_jobs`, `dot_jobs-type-B`, `dot_jobs-limited`, `dot_jobs-api2`.

**Lokaler Handler:** [crates/odir/src/commands/dot.rs:83](../../../crates/odir/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.jobs` gibt die Namen aller Job-Ressourcen der Director-Konfiguration aus, einen pro Zeile. Programme nutzen ihn für die Vervollständigung von `job=` und für Auswahllisten, etwa in der WebUI. Der Befehl liest nur die Konfiguration und verändert nichts.

Es erscheinen nur Jobs, die die Konsole laut `Job ACL` sehen darf. Die Reihenfolge ist die der Konfiguration.

## Syntax

```text
.jobs [ type=<jobtyp> ] [ enabled | disabled ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `type=<jobtyp>` | `Backup`, `Restore`, `Verify`, `Admin`, `Copy`, `Migrate`, `Consolidate`, `Archive` oder der Kennbuchstabe | nur Jobs dieses Typs |
| `enabled` | | nur aktivierte Jobs |
| `disabled` | | nur deaktivierte Jobs |

Bei `type=` zählt nur der erste Buchstabe des Wertes, und zwar mit Groß- und Kleinschreibung: Er wird mit dem Typ-Kennbuchstaben verglichen (`B` Backup, `R` Restore, `V` Verify, `D` Admin, `c` Copy, `g` Migrate, `O` Consolidate, `A` Archive). Deshalb liefert `type=Backup` und `type=Bogus` die Backup-Jobs, `type=backup` aber nichts. Ein leerer Wert (`type=`) filtert nicht. Der Schlüssel `type` selbst darf groß geschrieben werden.

`enabled` und `disabled` wirken wie bei [.clients](dot-clients.md): ein Wert dahinter wird ignoriert, beide zusammen ergeben eine leere Liste.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Namensliste. Unter `.api 2` steht sie als `result.jobs`, ein Array von Objekten mit `name` und `enabled`.

## Beispiele

### Alle Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobs
parity-admin
parity-verify
parity-copy
parity-migrate
parity-consolidate
parity-restore-regex
backup-bareos-fd
RestoreFiles
BackupCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur Backup-Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobs type=B
backup-bareos-fd
BackupCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Typ mit Kleinbuchstaben

`backup` beginnt mit `b`, und diesen Kennbuchstaben gibt es nicht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobs type=backup
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur Restore-Jobs im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobs type=R
parity-restore-regex
RestoreFiles
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .jobs type=B
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "jobs": [
      {
        "name": "backup-bareos-fd",
        "enabled": true
      },
      {
        "name": "BackupCatalog",
        "enabled": true
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit eingeschränkter Konsole

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .jobs
backup-bareos-fd
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.jobs: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.jobs` nicht. |

## Siehe auch

- [.types](dot-types.md) — die möglichen Jobtypen
- [.defaults](dot-defaults.md) — Vorgabewerte eines Jobs
- [.jobdefs](dot-jobdefs.md) — Namen der JobDefs
- [run](run.md) — Job starten
- [show](show.md) — vollständige Job-Ressource anzeigen
