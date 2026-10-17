# .defaults — Vorgabewerte einer Ressource anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L237) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Get default settings

**Originalsyntax:**

```text
.defaults job=<job-name> | client=<client-name> | storage=<storage-name> | pool=<pool-name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (9):** `c041-defaults-backupcatalog`, `c041-defaults-backupcatalog-api2`, `c041-defaults-jobdefs`, `dot-defaults-job`, `dot-defaults-client`, `dot-defaults-storage`, `dot-defaults-pool`, `dot-defaults-restore`, `dot-defaults-job-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:142](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.defaults` zeigt die wichtigsten Einstellungen eines Jobs, Clients, Storages oder Pools als `schlüssel=wert`-Zeilen. Die WebUI und Skripte nutzen den Befehl, um ein Formular (etwa „Job starten“ oder „Restore“) mit den Werten aus der Konfiguration vorzubelegen. Er liest nur die Konfiguration und verändert nichts.

## Syntax

```text
.defaults job=<jobname>
.defaults client=<clientname>
.defaults storage=<storagename>
.defaults pool=<poolname>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `job=<jobname>` | Name eines Jobs | Job, Pool, Messages, Client, Storage, Zielverzeichnis (`where`), bei Restore-Jobs `replace`, Level, Typ, FileSet, aktiviert, Katalog |
| `client=<clientname>` | Name eines Clients | Adresse, Port, Datei- und Job-Aufbewahrung in Sekunden, Autoprune, aktiviert, Katalog |
| `storage=<storagename>` | Name eines Storages | Adresse, Port, aktiviert, Medientyp, Device |
| `pool=<poolname>` | Name eines Pools | Pooltyp, Label-Format, Recycling- und Aufbewahrungswerte, Grenzen für Volumes |

Pro Aufruf gibt man genau eine Ressource an. Der Name muss genau stimmen. Eine JobDefs-Vorlage (`job=DefaultJob`) ist kein Job und liefert nichts. Ressourcen, die die Konsole laut ACL nicht sehen darf, liefern ebenfalls nichts.

Bei `pool=` heißt eine Zeile `purge_oldest_volume==0` mit doppeltem Gleichheitszeichen; so gibt es auch das Bareos-Original aus.

## API-Modi

Unter `.api 0` und `.api 1` kommen dieselben `schlüssel=wert`-Zeilen. Unter `.api 2` steht dasselbe als Objekt `result.defaults`; Zahlen wie `port` und `enabled` sind dort JSON-Zahlen, Aufbewahrungszeiten und Größen Zeichenketten. Ein unbekannter Name ergibt `"defaults": {}`.

## Beispiele

### Vorgaben eines Backup-Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults job=backup-bareos-fd
job=backup-bareos-fd
pool=Incremental
messages=Standard
client=bareos-fd
storage=File
where=
level=Incremental
type=Backup
fileset=SelfTest
enabled=1
catalog=MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vorgaben eines Restore-Jobs

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults job=RestoreFiles
job=RestoreFiles
pool=Incremental
messages=Standard
client=bareos-fd
storage=File
where=/tmp/bareos-restores
replace=always
level=
type=Restore
fileset=LinuxAll
enabled=1
catalog=MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vorgaben eines Clients

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults client=bareos-fd
client=bareos-fd
address=localhost
port=9102
file_retention=5184000
job_retention=15552000
autoprune=0
enabled=1
catalog=MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vorgaben eines Storages im Skriptmodus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults storage=File
storage=File
address=WS-JOERG.intranet.bernau.family
port=9103
enabled=1
media_type=File
device=FileStorage
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vorgaben eines Pools

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults pool=Full
pool=Full
pool_type=Backup
label_format=Full-
purge_oldest_volume==0
recycle_oldest_volume=0
max_volumes=100
vol_retention=31536000
vol_use_duration=0
max_vol_jobs=0
max_vol_files=0
max_vol_bytes=53687091200
auto_prune=1
recycle=1
file_retention=0
job_retention=0
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults job=backup-bareos-fd
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "defaults": {
      "job": "backup-bareos-fd",
      "pool": "Incremental",
      "messages": "Standard",
      "client": "bareos-fd",
      "storage": "File",
      "where": "",
      "level": "Incremental",
      "type": "Backup",
      "fileset": "SelfTest",
      "enabled": 1,
      "catalog": "MyCatalog"
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Name

Für einen unbekannten Job, eine JobDefs-Vorlage oder eine nicht freigegebene Ressource kommt eine leere Antwort.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults job=nosuch
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Unter `.api 2` ist das Objekt leer:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults job=nosuch
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "defaults": {}
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nicht freigegebener Job

Die Konsole `parity-limited` darf den Job `BackupCatalog` nicht sehen.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults job=BackupCatalog
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Argument

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults
.defaults command requires a parameter.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Unter `.api 2` kommt ein Fehlerobjekt ohne Text:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .defaults
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {}
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original setzt in `error.data.result` ein leeres Objekt `"defaults": {}` ein; Onesimus lässt `result` leer (C-073).

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.defaults command requires a parameter.` | Es fehlt `job=`, `client=`, `storage=` oder `pool=` (auch bei einem anderen Schlüssel wie `bogus=x`). Unter `.api 2` kommt stattdessen ein Fehlerobjekt ohne Text. |
| `.defaults: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.defaults` nicht. |

## Siehe auch

- [.jobs](dot-jobs.md), [.clients](dot-clients.md), [.storages](dot-storages.md), [.pools](dot-pools.md) — die gültigen Namen
- [show](show.md) — vollständige Ressource anzeigen
- [run](run.md) — Job starten, Werte mit `mod` ändern
