# .bvfs_restore — Restore-Auswahl als Tabelle anlegen (BVFS)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L299) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Mark BVFS files/directories for restore. Stored in handle.

**Originalsyntax:**

```text
.bvfs_restore path=<handle> jobid=<jobid> [fileid=<file-id>] [dirid=<dirid>] [hardlink=<hardlink>]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Paritätsfall nachgewiesen.

- Kein Paritätsfall: ändert den Katalog (Handle-Tabelle); nur durch Einheitentests belegt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bvfs.rs:892](../../../crates/onesimus-dird/src/commands/bvfs.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`.bvfs_restore` legt im Katalog eine Tabelle mit der Restore-Auswahl an: einzelne Dateien (FileIds), ganze Verzeichnisse (PathIds) und Hardlinks. Diese Tabelle übergibt man danach an [restore](restore.md) mit `file=?<tabelle>`; der Restore-Dialog überspringt dann Menü und Dateibaum. Zum Schluss löscht [.bvfs_cleanup](dot-bvfs-cleanup.md) die Tabelle wieder. Der Befehl schreibt nur in den Katalog und startet keinen Job.

BVFS bleibt in Onesimus unverändert zur Abwärtskompatibilität erhalten (etwa für bareos-webui). Die OVFS-Befehle ([.ovfs_tree](dot-ovfs-tree.md), [.ovfs_ls](dot-ovfs-ls.md)) liefern zu jedem Eintrag die Restore-Identität (JobId, FileIndex) ohne Zwischentabelle.

## Syntax

```text
.bvfs_restore path=b2<zahl> jobid=<id>[,<id>…] fileid=<id>[,<id>…] [dirid=<id>[,<id>…]] [hardlink=<jobid>,<fileindex>[,…]]
.bvfs_restore path=b2<zahl> jobid=<id>[,<id>…] dirid=<id>[,<id>…]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `path=b2<zahl>` | `b2` gefolgt von einer Zahl, z. B. `b2101` | Name der Tabelle; frei wählbar, aber eindeutig pro laufender Auswahl. Eine vorhandene Tabelle gleichen Namens wird ersetzt. |
| `jobid=<liste>` | JobIds, durch Komma getrennt | Job-Kette, aus der restauriert wird. |
| `fileid=<liste>` | FileIds aus [.bvfs_lsfiles](dot-bvfs-lsfiles.md) oder [.bvfs_versions](dot-bvfs-versions.md) | Einzelne Dateien. |
| `dirid=<liste>` | PathIds aus [.bvfs_lsdirs](dot-bvfs-lsdirs.md) | Ganze Verzeichnisse mit Inhalt. |
| `hardlink=<liste>` | Paare aus JobId und FileIndex | Hardlink-Ziele. |

Mindestens eines von `fileid=`, `dirid=` und `hardlink=` muss angegeben sein.

## API-Modi

`.api 0` und `.api 1` antworten bei Erfolg mit `OK`. `.api 2` antwortet mit leerem `result`, bei Fehlern mit einem Fehlerobjekt.

## Beispiele

### Datei und Verzeichnis auswählen

FileIds und PathIds stammen aus `.bvfs_lsfiles` und `.bvfs_lsdirs`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsfiles jobid=1 path=/usr/sbin/ limit=2
1	35	1	d FPe7 IHt B A A A wD BAA I BqZNg7 BqVJOy BqZNhJ A A C	addgnupghome
1	91	1	d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C	agetty
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_lsdirs jobid=1 path=/usr/
14902	0	0	A A A A A A A A A A A A A A	.
14903	0	0	A A A A A A A A A A A A A A	..
1	180	1	d Tn EHt B A A A DA6 BAA A BqzjrV BqytVM BqytVM A A C	sbin/
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 fileid=1 path=b2101
OK
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 dirid=1 path=b2102
OK
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 fileid=1
Can't find jobid, pathid or path argument
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 fileid=1 path=x2101
Can't create restore list
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup path=b2101
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup path=b2102
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auswahl an restore übergeben

Mit `file=?b2105` übernimmt `restore` die Tabelle. Hier wird die Abschlussfrage mit `no` beantwortet.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 fileid=1 path=b2105
OK
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> restore file=?b2105 client=bareos-fd restorejob=RestoreFiles where=/tmp/bareos-restores replace=never
Bootstrap records written to /var/lib/bareos/bareos-dir.restore.11.bsr

The job will require the following
[…]
Using Catalog "MyCatalog"
Run Restore job
JobName:         RestoreFiles
Bootstrap:       /var/lib/bareos/bareos-dir.restore.11.bsr
Where:           /tmp/bareos-restores
Replace:         Never
FileSet:         LinuxAll
Backup Client:   bareos-fd
Restore Client:  bareos-fd
Format:          Native
Storage:         File
When:            2026-10-16 23:07:49
Catalog:         MyCatalog
Priority:        10
Plugin Options:  *None*
OK to run? (yes/mod/no): 
> no
Job not run.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup path=b2105
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_update jobid=1
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 fileid=1 path=b2104
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_restore jobid=1 fileid=1
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
          "Can't find jobid, pathid or path argument\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .bvfs_cleanup path=b2104
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Can't find jobid, pathid or path argument` | `path=` oder `jobid=` fehlt. |
| `Can't create restore list` | Der Tabellenname beginnt nicht mit `b2` und einer Zahl, keine von `fileid=`/`dirid=`/`hardlink=` ist angegeben, eine Liste enthält etwas anderes als Zahlen, oder die Auswahl ist leer. |
| `Unauthorized command from this console.` | Eine JobId gibt es nicht oder die Konsole darf sie nicht sehen. |

## Siehe auch

- [restore](restore.md) — Restore mit `file=?<tabelle>` starten
- [.bvfs_cleanup](dot-bvfs-cleanup.md) — Tabelle wieder löschen
- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) — FileIds ermitteln
- [.bvfs_lsdirs](dot-bvfs-lsdirs.md) — PathIds ermitteln
