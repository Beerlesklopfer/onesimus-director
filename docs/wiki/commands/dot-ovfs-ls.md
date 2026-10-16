# .ovfs_ls — Ein Verzeichnis einer Job-Kette auflisten (OVFS)

## Zweck

`.ovfs_ls` listet die direkten Einträge eines Verzeichnisses im Stand einer Job-Kette: erst die Dateien, dann die Unterverzeichnisse mit der Zahl ihrer Einträge. Damit lädt ein Restore-Browser den Baum Verzeichnis für Verzeichnis nach. Verzeichnisse, unter denen nichts wiederherstellbar ist, fehlen. Der Befehl liest nur aus dem Katalog.

OVFS ist eine Onesimus-Erweiterung ohne Gegenstück in Bareos (vergleichbar: [.bvfs_lsdirs](dot-bvfs-lsdirs.md) und [.bvfs_lsfiles](dot-bvfs-lsfiles.md) zusammen). Extension oder Fallback: siehe [.ovfs_status](dot-ovfs-status.md).

## Syntax

```text
.ovfs_ls jobid=<id>[,<id>…] path=<verzeichnis> [json]
.ovfs_ls client=<client> [fileset=<fileset>] path=<verzeichnis> [json]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<liste>` | positive JobIds, durch Komma getrennt; auch `jobids=` | Die Job-Kette. |
| `client=<client>` | Client-Name | Statt `jobid=`: Kette des neuesten Stands wie [.ovfs_latest](dot-ovfs-latest.md). |
| `fileset=<fileset>` | FileSet-Name | Mit `client=`: nötig bei Backups mehrerer FileSets. |
| `path=<verzeichnis>` | Pfad mit `/` am Ende; auch `dir=` | Das Verzeichnis. Für das nächste `.ovfs_ls` nimmt man den `path` eines Verzeichniseintrags. |
| `json` | – | Das Ergebnis als JSON-Zeile ausgeben, in jedem API-Modus. |

## API-Modi

`.api 0` und `.api 1` zeigen nach `jobids: …` eine Zeile pro Eintrag: Art, Größe, Änderungszeit, Name, `jobid`/`fileindex` und bei Verzeichnissen `entries`. `.api 2` und `json` liefern `{"path", "jobids", "dir", "entries"}`; jeder Eintrag hat die Spalten von `ovfs.ls` (`name`, `name_b64`, `path`, `path_b64`, `kind`, `jobid`, `fileid`, `fileindex`, `delta_seq`, `delta_parts`, `mode`, `uid`, `gid`, `size`, `mtime`, `link_fileindex`, `entries`), leere Spalten fallen weg.

## Beispiele

### Verzeichnis einer Job-Kette

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls jobid=1,5 path=/usr/
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
jobids: 1,5
directory         12382  2026-10-15 17:24:44  sbin/  jobid=5 fileindex=12  entries=180
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls jobid=1 path=/
jobids: 1
directory                                     usr/  entries=180
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Verzeichnis des neuesten Stands

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls client=bareos-fd fileset=SelfTest path=/usr/sbin/
Using Catalog "MyCatalog"
jobids: 600,615,616,620,622,624,626,1,5
file               3075  2026-07-13 07:28:50  addgnupghome  jobid=1 fileindex=35
[…]
file              63816  2026-06-29 21:37:52  zic  jobid=1 fileindex=6
file             137736  2026-07-31 15:34:36  zramctl  jobid=1 fileindex=114
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON-Zeile

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls jobid=1,5 path=/usr/ json
{"path":"fallback","jobids":[1,5],"dir":"/usr/","entries":[{"name":"sbin","path":"/usr/sbin/","kind":"directory","jobid":5,"fileid":192,"fileindex":12,"mode":16877,"uid":0,"gid":0,"size":12382,"mtime":1792085084,"entries":180}]}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls jobid=1,5 path=/usr/
jobids: 1,5
directory         12382  2026-10-15 17:24:44  sbin/  jobid=5 fileindex=12  entries=180
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls jobid=1,5 path=/usr/
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "path": "fallback",
    "jobids": [
      1,
      5
    ],
    "dir": "/usr/",
    "entries": [
      {
        "name": "sbin",
        "path": "/usr/sbin/",
        "kind": "directory",
        "jobid": 5,
        "fileid": 192,
        "fileindex": 12,
        "mode": 16877,
        "uid": 0,
        "gid": 0,
        "size": 12382,
        "mtime": 1792085084,
        "entries": 180
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler

Ein unbekannter Pfad ergibt nur die Zeile `jobids:`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls jobid=1
Need path=<directory>.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls path=/
Need jobid=<list> or client=<name> [fileset=<name>].
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_ls jobid=1 path=/nosuch/
jobids: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Need jobid=<list> or client=<name> [fileset=<name>].` | Weder `jobid=` noch `client=` angegeben. |
| `Need path=<directory>.` | `path=` fehlt. |
| `Invalid jobid list "<text>": expected positive integers separated by commas.` | `jobid=` ist keine Liste positiver Zahlen. |
| `job <n> does not exist` | Eine JobId der Liste gibt es nicht. |
| `client "<client>" has backups of several FileSets [...]; choose one` | Mit `client=` fehlt `fileset=`. |
| `Unauthorized command from this console.` | Die Konsole darf einen Job, Client oder ein FileSet der Kette nicht sehen. |

## Siehe auch

- [.ovfs_tree](dot-ovfs-tree.md) — mehrere Ebenen auf einmal
- [.ovfs_versions](dot-ovfs-versions.md) — Versionen einer Datei
- [.ovfs_latest](dot-ovfs-latest.md) — Job-Kette des neuesten Stands
