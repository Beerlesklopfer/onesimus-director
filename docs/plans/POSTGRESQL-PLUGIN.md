# Planung: PostgreSQL-Plugin für ofd

Stand: 18.11.2026. Status: **Plan freigegeben; Umsetzung beginnt mit der gemeinsamen Phase 1.**
Freigabe durch den Nutzer am 18.11.2026 mit den Entscheidungen unten.

Das Plugin sichert und stellt PostgreSQL-Cluster wieder her: globale Objekte
(Rollen, Tablespaces) und jede Datenbank einzeln, **ohne Passwort im Director**
und ohne Vor-Skript im Job. Es ersetzt die heutigen `RunScript`-Dumps
(`dump-database.sh --password=…` in `BackupNextcloud`, `BackupPassbolt`,
`BackupEleLa`) und schließt die Lücke, dass neue Datenbanken (z. B. `multigpt`
auf PDC) von keinem Job erfasst werden.

Es gelten [Plugin-Architektur](../wiki/plugin-architektur.md) und
[Protobuf-Vertrag](../../proto/filed_plugin.proto): signiertes Rust/WASM, kein
Python-/Shell-Plugin, eigene Instanz je Job, begrenzte Queues, außerbandiges
Cancel. DIR↔FD über gRPC mit BNET-Fallback, FD↔SD bleibt BNET.

## Anlass (Bestandsaufnahme 18.11.2026)

| Befund | Wirkung |
| --- | --- |
| Datenbank-Passwörter im Klartext in `RunScript`-Kommandos | Sichtbar über `show job` und in jedem Joblog im Katalog. |
| Ein Login je Datenbank-Eigentümer (`EleLa`, `www-data`, `passbolt`, `multi-gpt`, `system`) | Jede Datenbank braucht eigene Zugangsdaten; neue Datenbanken fallen heraus. |
| Dump-Verzeichnis nicht im FileSet (`/var/backups/nextcloud`, behoben am 18.11.) | Der Job lief „OK“, der Dump wurde nie gesichert. |
| `pg_dumpall --globals-only` als Nicht-Superuser | `permission denied for table pg_authid`; Rollen fehlen in der Sicherung. |

Daraus folgen drei Anforderungen: **kein Geheimnis in der Konfiguration**,
**alle Datenbanken automatisch**, **der Dump ist Teil des Datenstroms**, nicht
eine Datei, deren Aufnahme ins FileSet man vergessen kann.

## Übergang bis zur Abnahme

Bis Phase 2 abgenommen ist, sichert ein Vor-Skript je Host alle Datenbanken als
`postgres` über den lokalen Socket (peer, ohne Passwort) nach
`/var/backups/postgres` (`globals.sql`, `<db>.dump` im Custom-Format); dieser
Ordner steht im FileSet. Das Skript ist eine Übergangshilfe wie das
[Container-Dump-Skript](../wiki/container-dump.md) und wird weder eingebettet
noch zum Plugin-Ausführungspfad.

## Stand der Grundlage

Vorhanden: Verifier, Wasmtime-Host (`wasm::load_signed`), Executor, Vertrag
`FileDaemonPlugin`, `ofd --check-plugin`, Maskierung von Plugin-Optionen
(`ofd/src/secrets.rs`). Es fehlen dieselben Voraussetzungen wie beim
[OpenLDAP-Plugin](OPENLDAP-PLUGIN.md), Phase 0 und 1 sind **gemeinsam**:

| Lücke | Wirkung |
| --- | --- |
| `ofd` führt `Plugin = …` im `Include` nicht aus | Ohne Anschluss an die Job-Engine läuft kein Plugin. |
| Der Gast hat keine Netz-, Datei- oder Prozess-Importe | Der Zugriff auf PostgreSQL braucht einen Broker auf dem Host. |
| `FileEntry` kennt keine Attribute und keine virtuelle Art | DB-Name, Größe, Serverversion nur im Inhalt, nicht in der Oberfläche. |

## Plugin-Zeile

```
Include {
  Plugin = "postgresql:instance=main:databases=*:exclude=test_*"
}
```

| Option | Vorgabe | Bedeutung |
| --- | --- | --- |
| `instance` | `main` | Name im virtuellen Baum; mehrere Cluster je Host möglich. |
| `socket` | `/var/run/postgresql` | Lokaler Unix-Socket; Anmeldung als OS-Eigentümer der Instanz (peer). |
| `port` | `5432` | Bestimmt den Socket-Namen `.s.PGSQL.<port>`. |
| `databases` | `*` | Liste oder Muster; `*` = alle mit `datallowconn` und nicht Template. |
| `exclude` | leer | Muster, die ausgelassen werden (z. B. `test_*`). Die Bareos-Katalogdatenbank ist immer ausgeschlossen, siehe unten. |
| `globals` | `yes` | Rollen, Mitgliedschaften, Tablespaces, Datenbank-Rechte. |
| `class` | `logical` | Sicherungsklasse, siehe unten. |
| `uri`, `password` | – | Nur für entfernte Server, siehe Broker. Passwort wird maskiert. |

Ein ausgewähltes, aber nicht erreichbares Objekt (Datenbank aus der Liste
existiert nicht, Socket fehlt) ist ein **Planfehler**, kein stilles Weglassen.
Ein Muster, das nichts trifft, erzeugt eine Warnung.

## Virtueller Baum

```
/@postgresql/main/_info.json                 Cluster-Metadaten
/@postgresql/main/globals.sql                Rollen, Tablespaces (pg_dumpall -g)
/@postgresql/main/db/EleLa/_info.json        Datenbank-Metadaten
/@postgresql/main/db/EleLa/EleLa.dump        pg_dump -Fc
/@postgresql/main/db/multigpt/multigpt.dump
```

- **Pfad:** Datenbanknamen werden für `/`, Steuerzeichen und Nicht-UTF-8
  escaped; Pfadbytes bleiben Bytes. Namen mit `-` (`multi-gpt`) bleiben lesbar.
- **`_info.json` (Cluster):** Serverversion, `pg_dump`-Version, Datenverzeichnis,
  Kodierung, Liste der Datenbanken mit Größe, Zeitpunkt, Klasse, `pg_current_wal_lsn()`
  zu Beginn und Ende. Die Oberfläche liest dies, ohne Dumps zu öffnen.
- **`_info.json` (Datenbank):** Eigentümer, Kodierung, Collation, Größe,
  Erweiterungen (`pg_extension`), Schemas, Tabellen mit geschätzter Zeilenzahl
  und Größe. Damit ist der Inhalt **durchsuchbar**, obwohl der Dump ein Stück ist.
- **Zeitstempel:** Beginn des Dump-Snapshots; Größe = Dump-Länge.

## Sicherungsklassen

| Klasse | Inhalt | Konsistenz | Restore-Einheit | Phase |
| --- | --- | --- | --- | --- |
| `logical` | `pg_dump -Fc` je Datenbank, `globals.sql` | je Datenbank ein Snapshot; **zwischen** Datenbanken nicht | Datenbank, mit `pg_restore -t` auch einzelne Tabellen | 2 (MVP) |
| `logical-tables` | je Tabelle eine Datei über `COPY` in einer `REPEATABLE READ`-Transaktion mit exportiertem Snapshot, plus Schema | je Datenbank ein Snapshot | Tabelle über die normale Dateiauswahl | 4 |
| `physical` | `pg_basebackup` plus WAL-Archiv | Cluster, Point-in-Time | ganzer Cluster | später, eigene Freigabe |

Der MVP ist `logical`: einfach, versionsunabhängig wiederherstellbar und genau
das, was die heutigen Skripte tun. Inkrementale gibt es in dieser Klasse nicht;
jede Sicherung schreibt jeden Dump vollständig. Unveränderte Datenbanken zu
überspringen (gleicher Stand von `pg_stat_database`) ist **nicht** zuverlässig
und wird nicht gemacht. Echte Inkrementale liefert erst `physical`.

## Klasse `physical`: WAL (zu klären)

Point-in-Time-Recovery braucht zwei Dinge: eine Basissicherung des Clusters
und **jedes** WAL-Segment danach, lückenlos. WAL entsteht laufend, nicht nur
während eines Jobs. Das ist der Unterschied zu allen anderen Plugins und der
Grund für eine eigene Freigabe.

### Wie WAL vom Server zum FD kommt

| Weg | Wie | Bewertung |
| --- | --- | --- |
| `archive_command` | Server ruft je Segment ein Shell-Kommando auf | **Abgelehnt:** unsignierter Shellcode im Datenbankserver, genau das, was die Signaturpflicht verhindern soll. Schlägt das Kommando fehl, staut sich WAL im Server. |
| `pg_receivewal` mit Replikations-Slot | Ein dauerhafter Empfänger holt WAL über das Replikationsprotokoll in ein Spool-Verzeichnis | **Vorschlag.** Kein Eingriff in `postgresql.conf` außer dem Slot; Werkzeug aus PostgreSQL selbst; Broker startet es als Instanzbenutzer über den lokalen Socket (peer, `local replication`). |

Der Empfänger läuft **dauerhaft**, nicht nur im Job. Er gehört deshalb zum
Broker der Instanz und läuft unter deren Benutzer; `ofd` startet und überwacht
ihn. Der Plugin-Gast steuert ihn nicht, er liest nur das Spool.

### Gefahren und Gegenmaßnahmen

- **Slot hält WAL fest:** Steht der Empfänger, behält der Server WAL bis die
  Platte voll ist und die Datenbank stoppt. Pflicht: `max_slot_wal_keep_size`
  setzen; Überschreitung macht den Slot ungültig, die Kette ist dann
  unterbrochen und der nächste Job wird automatisch zur Basissicherung
  hochgestuft, mit Fehlermeldung im Joblog.
- **Lücke in der Kette:** Jeder Job prüft die Segmentfolge (Timeline, LSN) vom
  letzten gesicherten Segment an. Eine Lücke ist ein Fehler, keine Warnung,
  und erzwingt eine neue Basissicherung.
- **Spool-Bereinigung:** Ein Segment wird erst gelöscht, wenn ein Job es
  erfolgreich (`T`) an den SD übergeben hat. Ein fehlgeschlagener Job löscht nichts.
- **Vertraulichkeit:** WAL enthält jede Änderung im Klartext, auch Passwörter
  in Anwendungstabellen. Spool `0700`, Instanzbenutzer; Pool verschlüsselt.

### Abbildung auf Bareos-Level

| Level | Inhalt |
| --- | --- |
| Full | `pg_basebackup` (tar-Strom, `-X none`) plus die WAL-Segmente bis zum Ende der Basissicherung |
| Incremental | alle WAL-Segmente seit dem letzten Job aus dem Spool |
| Differential | alle WAL-Segmente seit dem letzten Full |
| ab PostgreSQL 17 | optional inkrementelle Basissicherung (`summarize_wal`, `pg_basebackup --incremental`), Restore mit `pg_combinebackup`; verkürzt die Recovery-Zeit bei langen WAL-Ketten |

Virtueller Baum: `/@postgresql/main/base/<lsn>/base.tar`,
`/@postgresql/main/wal/<timeline>/<segment>`, `backup_manifest` aus
`pg_basebackup` zur Prüfung beim Restore (`pg_verifybackup`).

### Wiederherstellung

Immer in ein **neues** Datenverzeichnis, nie über den laufenden Cluster:
Basissicherung entpacken, WAL-Segmente in ein Verzeichnis, `restore_command`
liest nur dort, `recovery_target_time`/`_lsn` aus dem Restore-Job. Das
Starten des wiederhergestellten Clusters bleibt ein Schritt des Administrators.

### Offene Fragen zu WAL

1. Welche Instanzen brauchen Point-in-Time-Recovery? Für die meisten reicht
   `logical` täglich; `physical` kostet einen dauerhaften Empfänger je Instanz.
2. Wert für `max_slot_wal_keep_size` je Instanz (Platz auf dem Server).
3. Spool-Ort: auf dem Datenbankhost (Vorschlag `/var/lib/onesimus/wal/<instanz>`)
   oder auf einem anderen Datenträger, damit ein Plattenausfall nicht Datenbank
   und WAL zugleich trifft.
4. Wie lange darf die letzte Sicherung zurückliegen, bevor `ofd` warnt
   (Recovery Point Objective)? Der Empfänger hält zwar alles, aber erst der
   Job bringt es vom Host weg.

## Zugriff: Broker mit Capability `postgresql.read` / `postgresql.write`

Der Gast bleibt ohne Netz und ohne Prozesse. Ein Broker auf dem Host (Teil von
`ofd`) bietet typisierte, jobgebundene Operationen:

| Operation | Zweck |
| --- | --- |
| `pg.info` | Serverversion, Datenbankliste mit Eigentümer, Größe, `datallowconn` |
| `pg.db_info` | Metadaten einer Datenbank für `_info.json` |
| `pg.dump_globals` | Strom von `pg_dumpall --globals-only` |
| `pg.dump` | Strom von `pg_dump -Fc` einer Datenbank |
| `pg.restore` | Nur Restore, nur nach Plan-Freigabe, siehe unten |

- **Der Broker startet `pg_dump`/`pg_dumpall`/`pg_restore` selbst**, mit festem,
  administriertem Binärpfad und fester Argumentliste. Der Gast bestimmt nur die
  Datenbank aus der freigegebenen Liste. Es gibt keine generische Exec-Fähigkeit.
- **Version:** `pg_dump` muss mindestens die Hauptversion des Servers haben.
  Der Broker wählt `/usr/lib/postgresql/<major>/bin/pg_dump` passend zum Server
  und bricht mit Planfehler ab, wenn er fehlt.
- **Lokal (bevorzugt): ein Broker-Prozess je Instanz, als der Benutzer, unter
  dem die Instanz läuft.** `ofd` ermittelt den Benutzer aus der Instanz selbst:
  Eigentümer des Sockets `.s.PGSQL.<port>`, gegengeprüft mit dem Eigentümer
  von `postmaster.pid` im Datenverzeichnis und dem laufenden `postmaster`-Prozess.
  Stimmen die drei nicht überein, ist das ein Planfehler. Der Broker meldet sich
  über den Socket per peer an. Der Benutzer ist **nicht** vom Plugin wählbar;
  root ist als Instanzbenutzer abgelehnt. Mehrere Instanzen unter verschiedenen
  Benutzern (Debian-Cluster als `postgres`, Container- oder Anwendungsinstanzen
  als eigener Benutzer) bekommen getrennte Broker-Prozesse, wie der
  per-UID-Broker im [Podman-Plan](PODMAN-PLUGIN.md).
- **Entfernt:** `uri=postgresql://host:port` mit `sslmode=verify-full`, Benutzer
  und Passwort aus den Plugin-Optionen (maskiert), nie im Katalog oder in
  `_info.json`. Kein Fallback auf unverschlüsselt.
- **Abbruch:** Cancel beendet den `pg_dump`-Prozess (SIGTERM, nach Frist
  SIGKILL) und den Strom; die Verbindung hinterlässt keine Sperren.

## Sicherung

1. `Initialize`; `pg.info` prüft Socket, Serverversion, passende Werkzeuge.
2. Datenbankliste aus `databases`/`exclude` bilden; jede fehlende ausdrücklich
   genannte Datenbank ist ein Planfehler.
3. `globals.sql` zuerst (wenn `globals=yes`).
4. Je Datenbank `_info.json`, dann `<db>.dump` als Strom in den SD-Datenstrom.
   Kein Zwischenspeicher auf der Platte: der Dump fließt direkt vom Broker
   über `ReadFile` zum SD.
5. Cluster-`_info.json` zuletzt, mit den tatsächlich gesicherten Datenbanken
   und den WAL-Positionen.
6. Schlägt ein einzelner Dump fehl, laufen die anderen weiter; der Job endet
   mit Fehler, nicht mit Warnung.

## Wiederherstellung

1. **Standard: in Dateien** (`where=/pfad`): `globals.sql` und `.dump`-Dateien
   werden geschrieben, am laufenden Server ändert sich nichts. Weiter von Hand
   mit `pg_restore`.
2. **Online** nur ausdrücklich (`restore_mode=online`): `pg.restore` legt eine
   **neue** Datenbank an (`target_db=`, Vorgabe `<db>_restore_<jobid>`) und
   spielt den Dump ein. Eine vorhandene Datenbank wird nie überschrieben
   (`replace=never`); Umbenennen auf den Originalnamen bleibt ein bewusster
   Schritt des Administrators.
3. **Globale Objekte** werden online nicht automatisch angewendet. Der Plan
   listet fehlende Rollen, die der Dump als Eigentümer oder in Rechten braucht.
4. Einzelne Tabellen: Restore in Dateien, dann `pg_restore -t`. In der Klasse
   `logical-tables` direkt über die Dateiauswahl.

## Sicherheit und Betrieb

- Kein Passwort im Director, im Joblog, im Katalog oder in `_info.json`
  (`mask_plugin_secrets`; Testfall mit `password=`).
- `globals.sql` enthält Passwort-Hashes der Rollen, die Dumps enthalten alle
  Daten. Pool und Volumes sind entsprechend zu schützen (Verschlüsselung).
- Der Broker hält während des Dumps nur eine lesende Verbindung; `pg_dump`
  setzt `ACCESS SHARE`-Sperren. DDL auf gesicherten Tabellen wartet bis zum
  Ende des jeweiligen Dumps; das steht in der Dokumentation.
- Trap, Timeout und Cancel betreffen nur diese Instanz (Executor-Vertrag).

## Phasen und Abnahme

| Phase | Liefergegenstand | Exitkriterium |
| --- | --- | --- |
| 0 | gemeinsam mit OpenLDAP: `FileEntry.attributes`, `FILE_KIND_VIRTUAL` | Roundtrip in Protobuf und Katalog getestet |
| 1 | gemeinsam mit OpenLDAP: `Plugin =` in `ofd` laden, `NextFile`/`ReadFile` in den SD-Strom, `WriteFile` beim Restore, Cancel | Testplugin sichert und stellt wieder her |
| 2 | Signierte Binärteile (Manifest v2, Hash je Triplet, `fexecve`, siehe [Phase 1](PLUGIN-PHASE1.md)), Broker `pg.*` (lokal, peer), Plugin `postgresql`, Klasse `logical`, Restore in Dateien | Wegwerf-Cluster: alle Datenbanken, `globals.sql`, `_info.json`; `pg_restore` des Ergebnisses ergibt identische Daten |
| 3 | Online-Restore in neue Datenbank, entfernter Server mit TLS | Roundtrip: sichern, löschen, wiederherstellen, Vergleich |
| 4 | Klasse `logical-tables`, Durchsuchen in der Oberfläche | Tabelle einzeln wiederhergestellt; DB-/Tabellenanzeige statt Pfad |
| 5 | Ablösung der `RunScript`-Dumps auf PDC und CLOUD | Alle Jobs ohne Passwort; Übergangsskript entfernt |

Pflichttests: Datenbanknamen mit `-`, Leerzeichen, Großbuchstaben und
Nicht-UTF-8; Kodierung `SQL_ASCII` (vorhanden: `EleLa`, `bareos`); große
Datenbank (Strom ohne Zwischendatei, Speicherobergrenze); `pg_dump` älter als
Server; Server fällt während des Dumps aus; Cancel bei laufendem `pg_dump`
(kein Zombie, keine Sperre); neue Datenbank zwischen zwei Läufen wird
automatisch erfasst; ausgeschlossene Datenbank erscheint nicht; Plugin A hält
Device B nicht an.

## Entscheidungen (18.11.2026)

1. **Bareos-Katalog ausgeschlossen.** Die Katalogdatenbank des Directors wird
   vom Plugin nie gesichert, auch nicht bei `databases=*`; zuständig bleibt
   `BackupCatalog`. Erkannt wird sie über den Catalog-Eintrag der Konfiguration
   (Datenbankname und Host); ohne diese Angabe gilt der Name `bareos`. Der
   Ausschluss steht in `_info.json`, damit er in der Oberfläche sichtbar ist.
2. **Broker je Instanz als deren Benutzer** (siehe Zugriff). Der Broker braucht
   für den Datenbankzugriff kein root; `ofd` startet ihn und reicht nur den
   Socket der Instanz durch.
3. **Reihenfolge:** zuerst Phase 1 gemeinsam mit OpenLDAP (Plugins in `ofd`
   ausführen), dann Phase 2 PostgreSQL als erstes produktives Plugin.
