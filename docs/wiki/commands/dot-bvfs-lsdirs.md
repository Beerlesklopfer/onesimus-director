# .bvfs_lsdirs — Verzeichnisse einer Job-Kette auflisten (BVFS)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L282) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List directories using BVFS

**Originalsyntax:**

```text
.bvfs_lsdirs jobid=<jobid> path=<path> | pathid=<pathid> [limit=<limit>] [offset=<offset>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 11 von 11 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-bvfs.

**Paritätsfälle (11):** `c002-bvfs-noargs`, `c022-lsdirs-root`, `c022-lsdirs-deep`, `c022-lsdirs-pathid`, `c022-lsdirs-limit`, `c022-lsdirs-missing`, `c022-lsdirs-api1`, `c022-lsdirs-api2`, `c022-acl-limited`, `c022-negative-limit`, `bvfs-lsdirs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:912](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_lsdirs` listet die Unterverzeichnisse eines Verzeichnisses im Stand einer Job-Kette, so wie ein Restore-Dateibrowser sie braucht. Vorher muss der BVFS-Cache der Jobs mit [.bvfs_update](dot-bvfs-update.md) gefüllt sein. Der Befehl liest nur aus dem Katalog.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Der Onesimus-Weg ist OVFS mit [.ovfs_ls](dot-ovfs-ls.md) und [.ovfs_tree](dot-ovfs-tree.md).

## Syntax

```text
.bvfs_lsdirs jobid=<id>[,<id>…] path=<verzeichnis> [limit=<n>] [offset=<n>]
.bvfs_lsdirs jobid=<id>[,<id>…] pathid=<pathid> [limit=<n>] [offset=<n>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<liste>` | JobIds, durch Komma getrennt | Job-Kette, meist aus [.bvfs_get_jobids](dot-bvfs-get-jobids.md). |
| `path=<verzeichnis>` | Pfad mit `/` am Ende, `/` für die Wurzel, leer für die oberste Ebene | Verzeichnis, dessen Unterverzeichnisse gelistet werden. |
| `pathid=<pathid>` | Zahl aus einer früheren Ausgabe | Statt `path=`. |
| `limit=<n>` | Zahl, Standard 2000 | Höchstzahl der Zeilen. |
| `offset=<n>` | Zahl, Standard 0 | So viele Zeilen überspringen. |

Jede Textzeile hat fünf durch Tabulator getrennte Spalten: PathId, FileId, JobId, LStat (Bareos-kodierte Dateiattribute) und Name. Die ersten beiden Zeilen sind `.` (das Verzeichnis selbst) und `..`. Verzeichnisse ohne eigenen Katalogeintrag haben FileId und JobId 0.

## API-Modi

`.api 0` und `.api 1` liefern dieselben Tabulator-Zeilen. `.api 2` liefert `result.directories` mit Objekten aus `type` (`D`), `pathid`, `fileid`, `jobid`, `lstat`, `name`, `fullpath`, `stat` (entschlüsselte Attribute mit `dev`, `ino`, `mode`, `nlink`, `uid`, `gid`, `user`, `group`, `rdev`, `size`, `atime`, `mtime`, `ctime`) und `linkfileindex`. Ein unbekannter Pfad ergibt unter `.api 2` ein Fehlerobjekt ohne Meldung.

## Beispiele

### Oberste Ebene, Wurzel und Unterverzeichnis

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=
14904	0	0	A A A A A A A A A A A A A A	.
14903	0	0	A A A A A A A A A A A A A A	/
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/
14903	0	0	A A A A A A A A A A A A A A	.
14904	0	0	A A A A A A A A A A A A A A	..
14902	0	0	A A A A A A A A A A A A A A	usr/
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/usr/
14902	0	0	A A A A A A A A A A A A A A	.
14903	0	0	A A A A A A A A A A A A A A	..
1	180	1	d Tn EHt B A A A DA6 BAA A BqzjrV BqytVM BqytVM A A C	sbin/
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Jobs

Mit JobId 1 und 5 kommt der Verzeichniseintrag aus dem neueren Job 5.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/usr/sbin/
1	180	1	d Tn EHt B A A A DA6 BAA A BqzjrV BqytVM BqytVM A A C	.
14902	0	0	A A A A A A A A A A A A A A	..
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1,5 path=/usr/sbin/
1	192	5	d Tn EHt B A A A DBe BAA A Bq0Qxk Bq0Qxc Bq0Qxc A A C	.
14902	0	0	A A A A A A A A A A A A A A	..
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Über die PathId

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 pathid=1
1	180	1	d Tn EHt B A A A DA6 BAA A BqzjrV BqytVM BqytVM A A C	.
14902	0	0	A A A A A A A A A A A A A A	..
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Seitenweise mit limit und offset

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/ limit=1
14903	0	0	A A A A A A A A A A A A A A	.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/ limit=1 offset=1
14904	0	0	A A A A A A A A A A A A A A	..
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/ offset=5
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/usr/
14902	0	0	A A A A A A A A A A A A A A	.
14903	0	0	A A A A A A A A A A A A A A	..
1	180	1	d Tn EHt B A A A DA6 BAA A BqzjrV BqytVM BqytVM A A C	sbin/
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/usr/
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "directories": [
      {
        "type": "D",
        "pathid": 14902,
        "fileid": 0,
        "jobid": 0,
        "lstat": "A A A A A A A A A A A A A A",
        "name": ".",
        "fullpath": ".",
        "stat": {
          "dev": 0,
          "ino": 0,
          "mode": 0,
          "nlink": 0,
          "uid": 0,
          "gid": 0,
          "user": "root",
          "group": "root",
          "rdev": 0,
          "size": 0,
          "atime": 0,
          "mtime": 0,
          "ctime": 0
        },
        "linkfileindex": 0
      },
      {
        "type": "D",
        "pathid": 14903,
        "fileid": 0,
        "jobid": 0,
        "lstat": "A A A A A A A A A A A A A A",
        "name": "..",
        "fullpath": "..",
        "stat": {
          "dev": 0,
          "ino": 0,
          "mode": 0,
          "nlink": 0,
          "uid": 0,
          "gid": 0,
          "user": "root",
          "group": "root",
          "rdev": 0,
          "size": 0,
          "atime": 0,
          "mtime": 0,
          "ctime": 0
        },
        "linkfileindex": 0
      },
      {
        "type": "D",
        "pathid": 1,
        "fileid": 180,
        "jobid": 1,
        "lstat": "d Tn EHt B A A A DA6 BAA A BqzjrV BqytVM BqytVM A A C",
        "name": "sbin/",
        "fullpath": "/usr/sbin/",
        "stat": {
          "dev": 29,
          "ino": 1255,
          "mode": 16877,
          "nlink": 1,
          "uid": 0,
          "gid": 0,
          "user": "root",
          "group": "root",
          "rdev": 0,
          "size": 12346,
          "atime": 1791900373,
          "mtime": 1791677772,
          "ctime": 1791677772
        },
        "linkfileindex": 0
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/no/such/dir/
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=99999 path=/
Unauthorized command from this console.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1
Can't find jobid, pathid or path argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=x path=/
Can't find jobid, pathid or path argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Ohne Argumente:

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs
Can't find jobid, pathid or path argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Can't find jobid, pathid or path argument` | `jobid=` fehlt oder ist keine Zahlenliste, oder weder `path=` noch `pathid=` ist angegeben. |
| `Unauthorized command from this console.` | Eine JobId gibt es nicht oder die Konsole darf sie nach Job-, Client- oder FileSet-ACL nicht sehen. |
| (keine Ausgabe) | Den Pfad gibt es in dieser Job-Kette nicht. |

## Siehe auch

- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) — Dateien eines Verzeichnisses
- [.bvfs_update](dot-bvfs-update.md) — Cache füllen (Voraussetzung)
- [.bvfs_get_jobids](dot-bvfs-get-jobids.md) — Job-Kette ermitteln
- [.bvfs_restore](dot-bvfs-restore.md) — Auswahl für den Restore erzeugen
- [.ovfs_ls](dot-ovfs-ls.md) — ein Verzeichnis der Job-Kette (Onesimus)
