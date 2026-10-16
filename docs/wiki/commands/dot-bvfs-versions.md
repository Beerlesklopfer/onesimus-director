# .bvfs_versions — Versionen einer Datei auflisten (BVFS)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L295) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List versions of a file

**Originalsyntax:**

```text
.bvfs_versions jobid=0 client=<client-name> pathid=<path-id> filename=<file-name> [copies] [versions]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 5 von 5 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (5):** `c022-versions`, `c022-versions-errors`, `c022-versions-api2`, `c022-negative-limit`, `bvfs-versions`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:938](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_versions` listet alle gesicherten Versionen einer Datei eines Clients über alle Jobs hinweg, jeweils mit Volume. So lässt sich eine ältere Version gezielt für [.bvfs_restore](dot-bvfs-restore.md) auswählen (FileId der Zeile). Gezählt werden nur Versionen, die auf einem Volume liegen. Der Befehl liest nur aus dem Katalog.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Das Onesimus-Gegenstück ohne PathIds ist [.ovfs_versions](dot-ovfs-versions.md).

## Syntax

```text
.bvfs_versions jobid=<id> client=<client> path=<verzeichnis> fname=<datei> [copies] [versions] [limit=<n>] [offset=<n>]
.bvfs_versions jobid=<id> client=<client> pathid=<pathid> fname=<datei> [copies] [versions] [limit=<n>] [offset=<n>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<id>` | Zahl, üblich `0` | Pflicht, wird aber nicht ausgewertet (Bareos-Konvention). |
| `client=<client>` | Client-Name | Client, dessen Versionen gesucht werden. |
| `path=<verzeichnis>` | Pfad mit `/` am Ende | Verzeichnis der Datei. |
| `pathid=<pathid>` | Zahl | Statt `path=`. |
| `fname=<datei>` | Dateiname ohne Pfad; auch `filename=` oder `name=` | Datei, deren Versionen gelistet werden. |
| `copies` | – | Auch Versionen aus Copy-Jobs. |
| `versions` | – | Wird angenommen, hat aber (wie im Original) keine Wirkung. |
| `limit=<n>` | Zahl, Standard 2000 | Höchstzahl der Zeilen. |
| `offset=<n>` | Zahl, Standard 0 | So viele Zeilen überspringen. |

Jede Textzeile hat sieben durch Tabulator getrennte Spalten: PathId, FileId, JobId, LStat, MD5-Prüfsumme, Volume-Name und InChanger (1, wenn das Volume im Wechsler liegt).

## API-Modi

`.api 0` und `.api 1` liefern dieselben Tabulator-Zeilen (unter `.api 1` ohne Katalogzeilen). `.api 2` liefert `result.versions` mit Objekten aus `type` (`V`), `pathid`, `fileid`, `jobid`, `lstat`, `md5`, `volumename`, `volumeinchanger` und `stat` (entschlüsselte Attribute).

## Beispiele

### Versionen über Pfad und Dateinamen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=bareos-fd path=/usr/sbin/ fname=zic
Using Catalog "MyCatalog"
1	6	1	d FNjg IHt B A A A PlI BAA CA BqZNgv BqQuWw BqZNg3 A A C	Iey5/m9WEm8ywJxB7yysgQ	Incremental-47322	0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=bareos-fd path=/usr/sbin/ fname=zic copies versions
1	6	1	d FNjg IHt B A A A PlI BAA CA BqZNgv BqQuWw BqZNg3 A A C	Iey5/m9WEm8ywJxB7yysgQ	Incremental-47322	0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Versionen über PathId, mit limit und offset

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=bareos-fd pathid=1 fname=agetty
Using Catalog "MyCatalog"
1	91	1	d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C	X+cXL+LxLgLfXkChumL3zw	Incremental-47322	0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=bareos-fd path=/usr/sbin/ filename=agetty copies versions
1	91	1	d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C	X+cXL+LxLgLfXkChumL3zw	Incremental-47322	0
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=bareos-fd pathid=1 name=agetty limit=1 offset=1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter .api 1

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=bareos-fd path=/usr/sbin/ fname=zic
1	6	1	d FNjg IHt B A A A PlI BAA CA BqZNgv BqQuWw BqZNg3 A A C	Iey5/m9WEm8ywJxB7yysgQ	Incremental-47322	0
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=bareos-fd pathid=1 fname=agetty
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "versions": [
      {
        "type": "V",
        "pathid": 1,
        "fileid": 91,
        "jobid": 1,
        "lstat": "d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C",
        "md5": "X+cXL+LxLgLfXkChumL3zw",
        "volumename": "Incremental-47322",
        "volumeinchanger": 0,
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
        }
      }
    ]
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 pathid=1 fname=agetty
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "error": [
          "Can't find client or fname argument\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler

Ein unbekannter Client ergibt keine Ausgabe.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 pathid=1 fname=agetty
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Can't find client or fname argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions jobid=0 client=nosuch pathid=1 fname=agetty
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_versions client=bareos-fd pathid=1 fname=agetty
Can't find jobid, pathid or path argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Can't find client or fname argument` | `client=` oder `fname=` fehlt. |
| `Can't find jobid, pathid or path argument` | `jobid=` fehlt (auch wenn es nicht ausgewertet wird) oder weder `path=` noch `pathid=` ist angegeben. |
| `Unauthorized command from this console.` | Die Konsole darf den Client nicht sehen (Client ACL). |

## Siehe auch

- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) — Dateien eines Verzeichnisses
- [.bvfs_restore](dot-bvfs-restore.md) — eine Version für den Restore auswählen
- [.ovfs_versions](dot-ovfs-versions.md) — Versionen einer Datei (Onesimus)
