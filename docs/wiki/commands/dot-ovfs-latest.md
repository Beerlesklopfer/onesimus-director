# .ovfs_latest — Job-Kette des neuesten Stands ermitteln (OVFS)

## Zweck

`.ovfs_latest` ermittelt die Job-Kette für den neuesten Stand eines Clients, so wie Menüpunkt 5 von [restore](restore.md) sie wählt: die letzte Full-Sicherung des FileSets, die letzte Differential danach und alle Incrementals nach Full oder Differential. Berücksichtigt werden nur Backups mit Status `T` oder `W`; Differentials und Incrementals nur, wenn sie auf einem aktivierten Volume liegen. Der Befehl liest nur aus dem Katalog.

Die Liste übergibt man als `jobid=` an [.ovfs_tree](dot-ovfs-tree.md) und [.ovfs_ls](dot-ovfs-ls.md). So bleibt der Baum während des Blätterns gleich, auch wenn inzwischen ein neues Backup fertig wird. OVFS ist eine Onesimus-Erweiterung ohne Gegenstück in Bareos (vergleichbar: [.bvfs_get_jobids](dot-bvfs-get-jobids.md)).

## Syntax

```text
.ovfs_latest client=<client> [fileset=<fileset>] [json]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `client=<client>` | Client-Name | Client, dessen neuester Stand gesucht wird. |
| `fileset=<fileset>` | FileSet-Name | Nötig, wenn der Client Backups mehrerer FileSets hat. |
| `json` | – | Das Ergebnis als JSON-Zeile ausgeben, in jedem API-Modus. |

## API-Modi

`.api 0` und `.api 1` liefern die JobIds in einer Zeile, durch Kommas getrennt und nach Startzeit geordnet. `.api 2` und `json` liefern `{"path": …, "client": …, "jobids": [...]}`; `path` nennt den Weg, der geantwortet hat (`ovfs` oder `fallback`, siehe [.ovfs_status](dot-ovfs-status.md)). Fehler stehen unter `.api 2` in `error.data.messages.error`.

## Beispiele

### Neuester Stand eines FileSets

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=bareos-fd fileset=SelfTest
Using Catalog "MyCatalog"
600,615,616,620,622,624,626,1,5
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=bareos-fd fileset=SelfTest json
{"path":"fallback","client":"bareos-fd","jobids":[600,615,616,620,622,624,626,1,5]}
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=bareos-fd fileset=SelfTest
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "path": "fallback",
    "client": "bareos-fd",
    "jobids": [
      600,
      615,
      616,
      620,
      622,
      624,
      626,
      1,
      5
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=bareos-fd
Using Catalog "MyCatalog"
client "bareos-fd" has backups of several FileSets ["BenchmarkSmall", "Catalog", "SelfTest"]; choose one
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=bareos-fd fileset=LinuxAll
no Full backup of client "bareos-fd" with FileSet "LinuxAll" found
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=bareos-fd json
client "bareos-fd" has backups of several FileSets ["BenchmarkSmall", "Catalog", "SelfTest"]; choose one
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest
Need client=<name> [fileset=<name>].
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=nosuch
client "nosuch" does not exist
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_latest client=bareos-fd fileset=nosuch
no Full backup of client "bareos-fd" with FileSet "nosuch" found
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Need client=<name> [fileset=<name>].` | `client=` fehlt. |
| `client "<client>" does not exist` | Den Client gibt es nicht im Katalog. |
| `client "<client>" has backups of several FileSets [...]; choose one` | `fileset=` angeben. |
| `no Full backup of client "<client>" with FileSet "<fileset>" found` | Für diese Kombination gibt es keine Full-Sicherung. |
| `Warning: Full backup job <n> of client "<client>" is on no enabled volume; the chain [...] lacks it and a restore would be incomplete` | Die Full-Sicherung liegt auf keinem aktivierten Volume; die Kette ist unvollständig (wie in Bareos). |
| `Unauthorized command from this console.` | Die Konsole darf Client, FileSet oder einen Job der Kette nicht sehen. |
| `.ovfs_latest: is an invalid command.` | Der Befehl steht nicht in der Command ACL der Konsole. |

## Siehe auch

- [.ovfs_tree](dot-ovfs-tree.md) — Restore-Baum der Kette
- [.ovfs_ls](dot-ovfs-ls.md) — ein Verzeichnis der Kette
- [restore](restore.md) — Menüpunkt 5 wählt dieselbe Kette
- [.bvfs_get_jobids](dot-bvfs-get-jobids.md) — Job-Kette zu einer JobId (BVFS)
