# .ovfs_versions — Versionen einer Datei auflisten (OVFS)

## Zweck

`.ovfs_versions` listet alle gesicherten Versionen einer Datei (oder eines Verzeichnisses) eines Clients, die neueste zuerst. Gezählt werden nur Versionen, die auf einem Volume liegen, aus Jobs der Typen Backup und Archiv (mit `copies` auch Copy-Jobs). Versionen aus Jobs, die die Konsole nicht sehen darf, fallen weg. Der Befehl liest nur aus dem Katalog.

OVFS ist eine Onesimus-Erweiterung ohne Gegenstück in Bareos; es ersetzt [.bvfs_versions](dot-bvfs-versions.md) ohne PathIds. Extension oder Fallback: siehe [.ovfs_status](dot-ovfs-status.md).

## Syntax

```text
.ovfs_versions client=<client> path=<verzeichnis> [name=<datei>] [copies] [json]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `client=<client>` | Client-Name | Client der Backups. |
| `path=<verzeichnis>` | Pfad mit `/` am Ende; auch `dir=` | Verzeichnis der Datei. |
| `name=<datei>` | Dateiname; auch `fname=`, `filename=` | Datei. Leer oder fehlend: Versionen des Verzeichnisses selbst. |
| `copies` | – | Auch Versionen aus Copy-Jobs. |
| `json` | – | Das Ergebnis als JSON-Zeile ausgeben, in jedem API-Modus. |

## API-Modi

`.api 0` und `.api 1` liefern je Version eine Zeile mit zehn durch Tabulator getrennten Spalten: JobId, Level, Jobtyp, Startzeit, Größe, Änderungszeit, FileIndex, Volumes, InChanger (0/1) und MD5. `.api 2` und `json` liefern `{"path", "client", "dir", "name", "versions"}` mit Objekten aus `jobid`, `level`, `job_type`, `jobtdate`, `starttime`, `fileid`, `fileindex`, `delta_seq`, `md5`, `mode`, `uid`, `gid`, `size`, `mtime`, `volumes` und `inchanger`.

## Beispiele

### Versionen einer Datei

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_versions client=bareos-fd path=/usr/sbin/ name=zic
Using Catalog "MyCatalog"
1	I	B	2026-10-13 21:00:02	63816	2026-06-29 21:37:52	6	Incremental-47322	0	Iey5/m9WEm8ywJxB7yysgQ
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_versions client=bareos-fd path=/usr/sbin/ name=zic copies
1	I	B	2026-10-13 21:00:02	63816	2026-06-29 21:37:52	6	Incremental-47322	0	Iey5/m9WEm8ywJxB7yysgQ
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Versionen des Verzeichnisses selbst

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_versions client=bareos-fd path=/usr/sbin/
5	I	B	2026-10-15 21:00:02	12382	2026-10-15 17:24:44	12	Incremental-47322	0	0
1	I	B	2026-10-13 21:00:02	12346	2026-10-11 00:16:12	180	Incremental-47322	0	0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON-Zeile

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_versions client=bareos-fd path=/usr/sbin/ name=zic json
{"path":"fallback","client":"bareos-fd","dir":"/usr/sbin/","name":"zic","versions":[{"jobid":1,"level":"I","job_type":"B","jobtdate":1791918003,"starttime":"2026-10-13 21:00:02","fileid":6,"fileindex":6,"delta_seq":0,"md5":"Iey5/m9WEm8ywJxB7yysgQ","mode":33261,"uid":0,"gid":0,"size":63816,"mtime":1782769072,"volumes":["Incremental-47322"],"inchanger":false}]}
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_versions client=bareos-fd path=/usr/sbin/ name=zic
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "path": "fallback",
    "client": "bareos-fd",
    "dir": "/usr/sbin/",
    "name": "zic",
    "versions": [
      {
        "jobid": 1,
        "level": "I",
        "job_type": "B",
        "jobtdate": 1791918003,
        "starttime": "2026-10-13 21:00:02",
        "fileid": 6,
        "fileindex": 6,
        "delta_seq": 0,
        "md5": "Iey5/m9WEm8ywJxB7yysgQ",
        "mode": 33261,
        "uid": 0,
        "gid": 0,
        "size": 63816,
        "mtime": 1782769072,
        "volumes": [
          "Incremental-47322"
        ],
        "inchanger": false
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler

Ein unbekannter Client ergibt keine Ausgabe.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_versions client=bareos-fd
Need client=<name> path=<directory> name=<file>.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_versions client=nosuch path=/usr/sbin/ name=zic
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Need client=<name> path=<directory> name=<file>.` | `client=` oder `path=` fehlt. |
| `Unauthorized command from this console.` | Die Konsole darf den Client nicht sehen (wird auditiert). |
| `.ovfs_versions: is an invalid command.` | Der Befehl steht nicht in der Command ACL der Konsole. |

## Siehe auch

- [.ovfs_ls](dot-ovfs-ls.md) — Dateien eines Verzeichnisses
- [.ovfs_tree](dot-ovfs-tree.md) — Restore-Baum einer Job-Kette
- [.bvfs_versions](dot-bvfs-versions.md) — Versionen über PathId (BVFS)
