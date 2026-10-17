# .bvfs_lsfiles — Dateien eines Verzeichnisses auflisten (BVFS)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L286) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List files using BVFS

**Originalsyntax:**

```text
.bvfs_lsfiles jobid=<jobid> path=<path> | pathid=<pathid> [limit=<limit>] [offset=<offset>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-bvfs.

**Paritätsfälle (9):** `c022-lsfiles`, `c022-lsfiles-all`, `c022-lsfiles-other`, `c022-lsfiles-errors`, `c022-lsfiles-api1`, `c022-lsfiles-api2`, `c022-acl-limited`, `c022-negative-limit`, `bvfs-lsfiles`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:914](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_lsfiles` listet die Dateien (ohne Unterverzeichnisse) eines Verzeichnisses im Stand einer Job-Kette. Je Datei kommt die Version aus dem neuesten Job der Kette. Die FileIds der Ausgabe sind die Eingabe für [.bvfs_restore](dot-bvfs-restore.md). Vorher muss der BVFS-Cache mit [.bvfs_update](dot-bvfs-update.md) gefüllt sein. Der Befehl liest nur aus dem Katalog.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Der Onesimus-Weg ist [.ovfs_ls](dot-ovfs-ls.md), das Dateien und Verzeichnisse in einem Aufruf liefert.

## Syntax

```text
.bvfs_lsfiles jobid=<id>[,<id>…] path=<verzeichnis> [pattern=<text>] [limit=<n>] [offset=<n>]
.bvfs_lsfiles jobid=<id>[,<id>…] pathid=<pathid> [pattern=<text>] [limit=<n>] [offset=<n>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<liste>` | JobIds, durch Komma getrennt | Job-Kette, meist aus [.bvfs_get_jobids](dot-bvfs-get-jobids.md). |
| `path=<verzeichnis>` | Pfad mit `/` am Ende | Verzeichnis, dessen Dateien gelistet werden. |
| `pathid=<pathid>` | Zahl aus [.bvfs_lsdirs](dot-bvfs-lsdirs.md) | Statt `path=`. |
| `pattern=<text>` | regulärer Ausdruck (PostgreSQL) | Nur Dateinamen, die passen. |
| `limit=<n>` | Zahl, Standard 2000 | Höchstzahl der Zeilen. |
| `offset=<n>` | Zahl, Standard 0 | So viele Zeilen überspringen. |

Jede Textzeile hat fünf durch Tabulator getrennte Spalten: PathId, FileId, JobId, LStat (Bareos-kodierte Dateiattribute) und Dateiname.

## API-Modi

`.api 0` und `.api 1` liefern dieselben Tabulator-Zeilen. `.api 2` liefert `result.files` mit Objekten aus `type` (`F`), `pathid`, `fileid`, `jobid`, `lstat`, `name`, `stat` (entschlüsselte Attribute) und `linkfileindex`.

## Beispiele

### Erste Dateien eines Verzeichnisses

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/usr/sbin/ limit=5
1	35	1	d FPe7 IHt B A A A wD BAA I BqZNg7 BqVJOy BqZNhJ A A C	addgnupghome
1	91	1	d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C	agetty
1	156	1	d HHiH IHt B A A A z8wQ BAA Gfo Bqv8O4 Bqu6nE Bqu8bV A A C	anvil-server
1	2	1	d FD8A IHt B A A A C1Ig BAA Ww BqUv/L BqKv5b BqUv/W A A C	apache2
1	36	1	d FPe8 IHt B A A A ip BAA I BqZNg7 BqVJOy BqZNhJ A A C	applygnupgdefaults
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 pathid=1 limit=3 offset=5
1	8	1	d FNkK IHt B A A A IkA BAA BI BqZNg7 BqU9jI BqZNg7 A A C	badblocks
1	172	1	d HY27 IHt B A A A CTPo BAA Sg BqzjrV BqyJP+ BqytUE A A C	bareos-dbcheck
1	174	1	d HY3h IHt B A A A Gk0I BAA 0o Bqzik/ BqyJP+ BqytUE A A C	bareos-dir
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1,5 path=/usr/sbin/ limit=4
1	35	1	d FPe7 IHt B A A A wD BAA I BqZNg7 BqVJOy BqZNhJ A A C	addgnupghome
1	91	1	d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C	agetty
1	156	1	d HHiH IHt B A A A z8wQ BAA Gfo Bqv8O4 Bqu6nE Bqu8bV A A C	anvil-server
1	2	1	d FD8A IHt B A A A C1Ig BAA Ww BqUv/L BqKv5b BqUv/W A A C	apache2
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alle Dateien eines Verzeichnisses

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/usr/sbin/
1	35	1	d FPe7 IHt B A A A wD BAA I BqZNg7 BqVJOy BqZNhJ A A C	addgnupghome
[…]
1	6	1	d FNjg IHt B A A A PlI BAA CA BqZNgv BqQuWw BqZNg3 A A C	zic
1	114	1	d FvxA IHt B A A A hoI BAA EQ BqlAu8 BqbMCM BqlAu8 A A C	zramctl
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Leeres Verzeichnis, unbekannter Pfad und Muster ohne Treffer

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/no/such/dir/
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/usr/sbin/ pattern=ssh
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Negatives limit

`limit=-1` hebt die Begrenzung auf.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/usr/sbin/ limit=-1 offset=178
1	114	1	d FvxA IHt B A A A hoI BAA EQ BqlAu8 BqbMCM BqlAu8 A A C	zramctl
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/usr/sbin/ limit=3
1	35	1	d FPe7 IHt B A A A wD BAA I BqZNg7 BqVJOy BqZNhJ A A C	addgnupghome
1	91	1	d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C	agetty
1	156	1	d HHiH IHt B A A A z8wQ BAA Gfo Bqv8O4 Bqu6nE Bqu8bV A A C	anvil-server
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/usr/sbin/ limit=3
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "files": [
      {
        "type": "F",
        "pathid": 1,
        "fileid": 35,
        "jobid": 1,
        "lstat": "d FPe7 IHt B A A A wD BAA I BqZNg7 BqVJOy BqZNhJ A A C",
        "name": "addgnupghome",
        "stat": {
          "dev": 29,
          "ino": 1374139,
          "mode": 33261,
          "nlink": 1,
          "uid": 0,
          "gid": 0,
          "user": "root",
          "group": "root",
          "rdev": 0,
          "size": 3075,
          "atime": 1784993851,
          "mtime": 1783927730,
          "ctime": 1784993865
        },
        "linkfileindex": 0
      },
      {
        "type": "F",
        "pathid": 1,
        "fileid": 91,
        "jobid": 1,
        "lstat": "d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C",
        "name": "agetty",
        "stat": {
          "dev": 29,
          "ino": 1506345,
          "mode": 33261,
          "nlink": 1,
          "uid": 0,
          "gid": 0,
          "user": "root",
          "group": "root",
          "rdev": 0,
          "size": 110152,
          "atime": 1791097158,
          "mtime": 1785512076,
          "ctime": 1788087228
        },
        "linkfileindex": 0
      },
      {
        "type": "F",
        "pathid": 1,
        "fileid": 156,
        "jobid": 1,
        "lstat": "d HHiH IHt B A A A z8wQ BAA Gfo Bqv8O4 Bqu6nE Bqu8bV A A C",
        "name": "anvil-server",
        "stat": {
          "dev": 29,
          "ino": 1865863,
          "mode": 33261,
          "nlink": 1,
          "uid": 0,
          "gid": 0,
          "user": "root",
          "group": "root",
          "rdev": 0,
          "size": 13618192,
          "atime": 1790952376,
          "mtime": 1790683588,
          "ctime": 1790691029
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles path=/
Can't find jobid, pathid or path argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=99999 path=/usr/sbin/
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Unauthorized command from this console.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Can't find jobid, pathid or path argument` | `jobid=` fehlt oder ist keine Zahlenliste, oder weder `path=` noch `pathid=` ist angegeben. |
| `Unauthorized command from this console.` | Eine JobId gibt es nicht oder die Konsole darf sie nach Job-, Client- oder FileSet-ACL nicht sehen. |
| (keine Ausgabe) | Das Verzeichnis enthält in dieser Job-Kette keine Dateien oder es gibt den Pfad nicht. |

## Siehe auch

- [.bvfs_lsdirs](dot-bvfs-lsdirs.md) — Unterverzeichnisse
- [.bvfs_versions](dot-bvfs-versions.md) — alle Versionen einer Datei
- [.bvfs_restore](dot-bvfs-restore.md) — Auswahl für den Restore erzeugen
- [.ovfs_ls](dot-ovfs-ls.md) — ein Verzeichnis der Job-Kette (Onesimus)
