# .ovfs_tree — Restore-Baum einer Job-Kette (OVFS)

## Zweck

`.ovfs_tree` liefert den Verzeichnisbaum einer Job-Kette unterhalb eines Verzeichnisses, so wie ein Restore ihn zurückbringen würde: je Pfad gilt die Version aus dem neuesten Job, gelöschte Dateien fehlen, Hardlinks und Delta-Dateien sind aufgelöst. Jeder Knoten mit Katalogeintrag trägt seine Restore-Identität (`jobid`, `fileindex`). Der Befehl liest nur aus dem Katalog und braucht keinen Cache.

OVFS ist eine Onesimus-Erweiterung ohne Gegenstück in Bareos. Mit der PostgreSQL-Extension `ovfs` ruft der Director `ovfs.tree` auf, ohne sie rechnet er dasselbe mit SQL ([.ovfs_status](dot-ovfs-status.md)). BVFS ([.bvfs_lsdirs](dot-bvfs-lsdirs.md) …) bleibt unverändert und nutzt OVFS nicht.

## Syntax

```text
.ovfs_tree jobid=<id>[,<id>…] [root=<verzeichnis>] [depth=<n>] [max_entries=<n>] [json]
.ovfs_tree client=<client> [fileset=<fileset>] [root=<verzeichnis>] [depth=<n>] [max_entries=<n>] [json]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `jobid=<liste>` | positive JobIds, durch Komma getrennt; auch `jobids=` | Die Job-Kette (Full, Differential, Incrementals); die Reihenfolge ist egal. |
| `client=<client>` | Client-Name | Statt `jobid=`: die Kette des neuesten Stands wie [.ovfs_latest](dot-ovfs-latest.md). |
| `fileset=<fileset>` | FileSet-Name | Mit `client=`: nötig bei Backups mehrerer FileSets. |
| `root=<verzeichnis>` | Pfad, auch `path=`; Standard `/` | Wurzel des Baums. Ein fehlendes `/` am Ende wird ergänzt. |
| `depth=<n>` | Zahl ab 0, Standard 1 | 0 = nur die Wurzel, 1 = Wurzel und direkte Einträge usw. Verzeichnisse an der Tiefengrenze tragen `entries` (Zahl der Einträge darunter) statt `children`. |
| `max_entries=<n>` | Zahl, Standard 100000 | Höchstzahl der Knoten; bei mehr bricht der Befehl mit einer Meldung ab. |
| `json` | – | Das Ergebnis als eine JSON-Zeile ausgeben, in jedem API-Modus. Bei großen Bäumen günstiger als `.api 2`. |

Für große Sicherungen lädt man oben mit kleiner Tiefe und klappt Verzeichnisse mit [.ovfs_ls](dot-ovfs-ls.md) auf.

## API-Modi

`.api 0` und `.api 1` zeigen den Baum lesbar, ein Knoten pro Zeile, zwei Leerzeichen Einrückung je Ebene, mit `size`, `mtime`, `jobid`, `fileindex` und `entries`; davor die Zeile `jobids: …`. `.api 2` und `json` liefern `{"path", "jobids", "root", "depth", "tree"}`; `tree` ist ein Knoten mit `name`, `path`, `type` (`directory`, `file`, `symlink` …) und, soweit vorhanden, `jobid`, `fileid`, `fileindex`, `delta_seq`, `delta_parts`, `mode`, `uid`, `gid`, `size`, `mtime`, `link_fileindex`, `entries` und `children`. Ist unter `root` nichts wiederherstellbar, ist `tree` `null` und der Text lautet `<root>: nothing to restore`.

## Beispiele

### Zwei Ebenen des neuesten Stands

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree client=bareos-fd fileset=SelfTest root=/ depth=2
Using Catalog "MyCatalog"
jobids: 600,615,616,620,622,624,626,1,5
/
  usr/
    sbin/  mtime=2026-10-15 17:24:44 jobid=5 fileindex=12 entries=180
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ein Verzeichnis mit allen Dateien

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=600,615,616,620,622,624,626,1,5 root=/usr/sbin depth=1 max_entries=1000
jobids: 600,615,616,620,622,624,626,1,5
/usr/sbin/  mtime=2026-10-15 17:24:44 jobid=5 fileindex=12
  addgnupghome  file size=3075 mtime=2026-07-13 07:28:50 jobid=1 fileindex=35
[…]
  zic  file size=63816 mtime=2026-06-29 21:37:52 jobid=1 fileindex=6
  zramctl  file size=137736 mtime=2026-07-31 15:34:36 jobid=1 fileindex=114
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Bestimmte Jobs, Tiefe 1 und 0

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1,5 root=/usr depth=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
jobids: 1,5
/usr/
  sbin/  mtime=2026-10-15 17:24:44 jobid=5 fileindex=12 entries=180
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1 depth=0
jobids: 1
/  entries=180
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON-Zeile

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1,5 root=/usr depth=1 json
{"path":"fallback","jobids":[1,5],"root":"/usr","depth":1,"tree":{"name":"/usr/","path":"/usr/","type":"directory","children":[{"name":"sbin","path":"/usr/sbin/","type":"directory","jobid":5,"fileid":192,"fileindex":12,"mode":16877,"uid":0,"gid":0,"size":12382,"mtime":1792085084,"entries":180}]}}
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1,5 root=/usr depth=1
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "path": "fallback",
    "jobids": [
      1,
      5
    ],
    "root": "/usr",
    "depth": 1,
    "tree": {
      "name": "/usr/",
      "path": "/usr/",
      "type": "directory",
      "children": [
        {
          "name": "sbin",
          "path": "/usr/sbin/",
          "type": "directory",
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
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree
Need jobid=<list> or client=<name> [fileset=<name>].
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=99999
job 99999 does not exist
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=abc
Invalid jobid list "abc": expected positive integers separated by commas.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1 depth=-1
depth must not be negative, got -1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1 depth=x
Invalid depth="x": expected an integer.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1 root=/ depth=5 max_entries=10
the tree has more than 10 entries; choose a smaller depth or a deeper root
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .ovfs_tree jobid=1 root=/nosuch/
jobids: 1
/nosuch/: nothing to restore
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beispielausgabe nicht verfügbar: Antworten über die Extension (`"path":"ovfs"`), weil sie auf dem Testsystem nicht installiert ist; sie sind bis auf `path` gleich.

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Need jobid=<list> or client=<name> [fileset=<name>].` | Weder `jobid=` noch `client=` angegeben. |
| `Invalid jobid list "<text>": expected positive integers separated by commas.` | `jobid=` ist keine Liste positiver Zahlen. |
| `Invalid depth="<text>": expected an integer.` | `depth=` (oder `max_entries=`) ist keine Zahl. |
| `depth must not be negative, got <n>` | `depth=` ist negativ. |
| `job <n> does not exist` | Eine JobId der Liste gibt es nicht. |
| `the tree has more than <n> entries; choose a smaller depth or a deeper root` | `max_entries` überschritten; kleinere Tiefe oder tiefere Wurzel wählen. |
| `client "<client>" has backups of several FileSets [...]; choose one` | Mit `client=` fehlt `fileset=`. |
| `Unauthorized command from this console.` | Die Konsole darf einen Job, Client oder ein FileSet der Kette nicht sehen (wird auditiert). |
| `.ovfs_tree: is an invalid command.` | Der Befehl steht nicht in der Command ACL der Konsole. |

## Siehe auch

- [.ovfs_ls](dot-ovfs-ls.md) — ein Verzeichnis nachladen
- [.ovfs_latest](dot-ovfs-latest.md) — Job-Kette des neuesten Stands
- [.ovfs_versions](dot-ovfs-versions.md) — Versionen einer Datei
- [.ovfs_status](dot-ovfs-status.md) — Extension oder Fallback
- [restore](restore.md) — Restore im Dialog
