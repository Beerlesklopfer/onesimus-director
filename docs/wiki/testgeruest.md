# Testgerüst Director + FD + SD

Das Testgerüst startet einen vollständigen Satz Daemons auf dem eigenen
Rechner und prüft damit Backup, Restore, Verify, Estimate und Abbruch von
Ende zu Ende. Es ist das Sicherheitsnetz für den Umbau von ofd und für den
Neubau von osd (Gutachten `docs/audits/ofd-struktur-2026-11-19.md` §3 und §6,
Schritt 0).

Code: Crate `crates/onesimus-rig` (Bibliothek) und die Tests in
`crates/onesimus-rig/tests/rig.rs`. Die Rustdoc-Seite des Crates erklärt jeden
Typ, enthält ein Glossar (`onesimus_rig::glossary`), ein Klassendiagramm
(`Rig`) und den beim Bau erzeugten Modulbaum.

## Was läuft

| Teil | Programm | Port |
|---|---|---|
| Director | `target/debug/odir` aus dem Workspace (`--disable-grpc`: ohne eigenen gRPC-API-Server) | 39391 |
| File Daemon | `target/debug/ofd` aus dem Workspace (`--disable-grpc`, außer mit `RigOptions::fd_grpc`) | 39392 |
| Storage Daemon | `/usr/sbin/bareos-sd` 25 (austauschbar, siehe unten) | 39393 |
| Katalog | eigener PostgreSQL-Cluster aus `initdb`, nur TCP auf `127.0.0.2` | 39394 |

Jeder Test hat einen eigenen Satz Ports (39391, 39401, 39411, 39421, 39431,
je vier aufeinanderfolgende) und ein eigenes Verzeichnis
`/tmp/ofdrig-<Port>-<PID>`. Alles läuft als normaler Benutzer; kein
Systemdienst und keine Systemdatenbank wird angefasst.

Warum `127.0.0.2`: Der Cluster hat keine Unix-Sockets, weil Socket-Pfade unter
`/tmp/…` die Grenze von 107 Zeichen leicht überschreiten. odir verbindet sich
bei `127.0.0.1` und `localhost` aber über den Socket in `/var/run/postgresql`
(`crates/odir/src/check_catalog.rs` `uses_socket`). Jede andere Adresse aus
`127.0.0.0/8` geht über TCP.

## Starten

```sh
cargo test -p onesimus-rig -- --ignored
```

Der Test baut vorher `odir` und `ofd` (`cargo build -p odir -p ofd`), damit
nie ein veraltetes Binary geprüft wird. Ohne `--ignored` laufen nur die
Unit-Tests des Crates; die Gerüst-Tests stehen dann als „ignored, needs
bareos-sd and initdb: run with --ignored“ in der Ausgabe.

Dauer: rund 35 s für alle fünf Tests (sie laufen parallel).

Voraussetzungen (fehlt eine, scheitert der Test mit einer Liste der fehlenden
Teile und je einem Hinweis, er besteht nie still):

- `bareos-sd` 25 (`apt install bareos-storage`)
- PostgreSQL-Server-Programme `initdb`, `pg_ctl` und `psql`
  (`apt install postgresql`; gesucht unter `/usr/lib/postgresql/<Version>/bin`
  und im `PATH`)
- Katalogschema `/usr/lib/bareos/scripts/ddl/creates/postgresql.sql`
  (`apt install bareos-database-postgresql`)
- `diff`

Umgebungsvariablen:

| Variable | Wirkung |
|---|---|
| `ONESIMUS_RIG_SD` | Pfad des Storage Daemons (Vorgabe `/usr/sbin/bareos-sd`) |
| `ONESIMUS_RIG_ODIR`, `ONESIMUS_RIG_OFD` | andere Binaries für Director und FD |
| `ONESIMUS_RIG_PGBIN` | Verzeichnis mit `initdb`, `pg_ctl`, `psql` |
| `ONESIMUS_RIG_DDL` | anderes Katalogschema |
| `ONESIMUS_RIG_NO_BUILD=1` | nicht vorher bauen, vorhandene Binaries nehmen |
| `ONESIMUS_RIG_KEEP=1` | Gerüstverzeichnis (Konfiguration, Logs, Volumes) nach dem Lauf behalten |

gRPC (P1 Stufe 3, `RigOptions::fd_grpc` und `sd_grpc`, Typ `Grpc`): `Off`
(Vorgabe, wie bisher: der Daemon läuft mit `--disable-grpc`, die
Director-Ressource ohne `Grpc Enable`), `On` (ofd mit `--grpc-listen
127.0.0.1:<Basis+4>`, osd mit `Grpc Port = <Basis+5>` aus
`sd/osd.d/storage/vsd.conf`, Client bzw. Storage im Director mit `Grpc
Enable = yes`, Port, festgehaltenem Zertifikat aus dem Working Directory des
Daemons und `Grpc Server Name = localhost`) und `Disabled` (Director wie
`On`, Daemon mit `--disable-grpc`: der Director muss zurückfallen). Wo kein
Daemon ein Zertifikat erzeugt (`Disabled`, bareos-sd), legt das Gerüst ein
Ersatzzertifikat hin, damit der Director wirklich den gRPC-Port versucht.
bareos-sd bekommt nie gRPC-Argumente. Die Interop-Matrix steht in
`tests/osd_grpc.rs` (siehe [osd.md](osd.md#prüfen)).

Logs eines Laufs (mit `ONESIMUS_RIG_KEEP=1`): `dir.log`, `fd.log`, `sd.log`,
`pg.log` im Gerüstverzeichnis, dazu `dir-work/job.log`.

## Was geprüft wird

`backup_verify_restore_estimate_cancel` (Ports ab 39391):

1. Backup des Testbaums (Full, MD5, GZIP, Sparse, HardLinks = yes) → `T`, 13 Einträge.
   Der Baum enthält: gewöhnliche Dateien, ein Unterverzeichnis, eine leere
   Datei, eine 12-MiB-Datei, einen Symlink, zwei Hardlinks auf dieselbe Inode,
   einen Namen, der kein UTF-8 ist (`caf\xe9.txt`), und eine 8-MiB-Sparse-Datei.
2. Estimate mit Listing → nennt die Dateien des Baums.
3. Verify VolumeToCatalog → `T`.
4. Restore in das Zielverzeichnis → `T`; `diff -r --no-dereference` Quelle
   gegen Ziel ohne Unterschied; die Hardlinks teilen wieder eine Inode; die
   Sparse-Datei belegt weniger als ein Viertel ihrer Größe.
5. Abbruch eines laufenden Backups (48 MiB bei 4 MB/s) → der Job endet früh
   und nicht mit `T`.

Erwartete Defekte stehen als eigene Tests mit `#[should_panic]`. Sie bestehen,
solange der Defekt besteht. Wird er behoben, schlägt der Test fehl („panic did
not contain expected string“ bzw. „test did not panic“); dann wird aus ihm
eine gewöhnliche Prüfung.

| Test | Defekt | Fundstelle |
|---|---|---|
| `expected_defect_d1_fd_reports_cancel_as_error` | D1: ofd meldet nach `cancel` `TermCode=69` (E) statt A | Gutachten §7 |
| `expected_defect_d14_job_bandwidth_is_overwritten` | D14 (neu): Job-Direktive `Maximum Bandwidth` erreicht den FD als `setbandwidth`, `storage_cmd` überschreibt sie mit dem Client-Wert | `ofd/src/job/backup_cmds.rs` `setbandwidth_cmd`, `ofd/src/job.rs` `storage_cmd`; Bareos `filed/dir_cmd.cc` 751, 1412–1420 |
| `expected_defect_d15_estimate_before_first_backup_is_silent` | D15 (neu): `estimate` vor dem ersten Backup antwortet nur „Using Catalog“, weil odir den FileSet-Satz nur sucht statt ihn anzulegen. Ursache des Gutachten-Befunds „Estimate-Pfad nicht erreicht“ | `odir/src/commands/estimate.rs` `estimate_job` |
| `expected_defect_d16_canceled_backup_is_fatal_in_catalog` | D16 (neu): abgebrochenes Backup steht im Katalog als `f` statt `A` (Worker endet mit Code 1). Der Fachmann maß bei anderem Zeitverlauf `A` | `odir/src/job_manager.rs` |

Nicht geprüft und bewusst ausgelassen: Verify DiskToCatalog. Mit `Sparse = yes`
meldet es bei Nulldateien „MD5 digest differs“; das ist geerbtes
Bareos-Verhalten und kein ofd-Defekt (Gutachten §3).

Bekannte Konfigurationsfalle: Bareos 25 setzt in FileSet-Optionen
`HardLinks = no` als Vorgabe (odir sendet dann `H`). Ohne `HardLinks = yes`
werden zwei Hardlinks als zwei Dateien gesichert und wiederhergestellt.

## Fremder File Daemon (Container) und Audit-Log

`RigOptions` hat drei Schalter für Tests, die ihren FD selbst mitbringen:

| Feld | Vorgabe | Wirkung |
|---|---|---|
| `start_fd` | `true` | `false`: kein eigener ofd (Client `vfd`), der FD-Port wird nicht geprüft |
| `extra_director` | leer | zusätzliche oder ersetzende Director-Ressourcen als (relativer Pfad, Text), z. B. `client/x.conf`, `fileset/X.conf`, `job/X.conf` |
| `auditing` | `false` | `Auditing = yes` im Director, Audit-Meldungen nach `dir-work/audit.log` (`Rig::daemon_log("audit")`) |

Dazu `Cluster::dump()` (ganzer Katalog als `pg_dump --data-only`, für
Lecktests) und `Rig::daemon_log("job")` (`dir-work/job.log`).

Nutzer: `crates/onesimus-rig/tests/ldap_o7.rs` (O-7 des OpenLDAP-Plans,
`docs/plans/OPENLDAP-PLUGIN.md`). Der Test installiert ofd und
`ofd-plugin-openldap` als Debian-Pakete in zwei slapd-Containern
(`--network host`, FD auf `127.0.0.2:39461/39462`), das Gerüst auf den Ports
ab 39451 startet nur odir, bareos-sd und den Katalog. Podman läuft isoliert
über `tests/openldap/o7-podman.sh`. Start:

```sh
(cd plugins/openldap && CARGO_TARGET_DIR=../../target/plugins cargo +1.99.1 build --release --target wasm32-wasip2)
cargo test -p onesimus-rig --test ldap_o7 -- --ignored --nocapture
```

Der Test baut ofd mit einer Wegwerf-Plugin-CA (`ONESIMUS_PLUGIN_CA`) in
einem eigenen Zielverzeichnis `target/ldap-o7`, nie in `target/debug` oder
`target/release`; private Schlüssel bleiben im Speicher.

## bareos-sd durch osd ersetzen

Der Storage Daemon ist eine Variable (`ONESIMUS_RIG_SD` oder
`RigOptions::sd`). Das Gerüst startet ihn mit `-f -c <Datei> -d 50`; osd
liest `<Gerüst>/sd/bareos-sd.conf` wie bareos-sd (Seite `docs/wiki/osd.md`).

Eigene Tests für osd (P1 Stufe 1 und 2) stehen in
`crates/onesimus-rig/tests/osd.rs`:

```sh
cargo test -p onesimus-rig --test osd -- --ignored --nocapture
```

| Test | Ports | Inhalt |
|---|---|---|
| `osd_backup_restore_active_client` | ab 39471 | Backup und Restore, der FD ruft osd an |
| `osd_backup_restore_passive_client` | ab 39481 | dasselbe mit `Passive = yes` (`RigOptions::passive_client`): osd ruft den FD an |
| `osd_backup_verify_restore_over_several_volumes` | ab 39491 | Pool mit `Maximum Volume Bytes = 5M`: Backup über drei Volumes, Verify VolumeToCatalog, Restore |
| `osd_and_bareos_sd_read_each_others_volumes` | ab 39501 | Backup mit bareos-sd, Restore mit osd; Backup mit osd auf dasselbe Volume, Restore mit bareos-sd (`Rig::replace_sd`); `bls` liest das gemischte Volume. Ohne `/usr/sbin/bareos-sd` übersprungen |

Die Backup-Prüfungen: `T` mit 13 Dateien und „Backup OK“, je ein Media-,
JobMedia- und 13 File-Sätze, `VolBytes` gleich der Dateigröße, das Volume
mit dem Leser aus `bareos-format` (Volume-Label, SOS/EOS, Attribute,
Prüfsummen) und mit `/usr/sbin/bls`, falls installiert. Die
Restore-Prüfungen: „Restore OK“ mit 13 Dateien, `diff -r` ohne Unterschied
und `tree::compare_trees` (Typ, Inhalt, Größe, Rechte, Eigentümer, mtime,
Symlink-Ziele), Hardlinks auf einer Inode, Löcher der Sparse-Datei. Mit
`ONESIMUS_RIG_SD=/usr/sbin/bareos-sd` laufen die ersten drei als Gegenprobe
gegen bareos-sd.

Die Tests in `tests/rig.rs` mit `ONESIMUS_RIG_SD=$PWD/target/debug/osd`
(vorher `cargo build -p osd-daemon`; gemessen 20.11.2026): alle fünf
bestehen, Backup, Estimate, Verify VolumeToCatalog, Restore und Abbruch
sowie die vier erwarteten Defekte D1, D14, D15, D16 (sie liegen in ofd bzw.
odir).

Bekannte Falle: Die Gerüst-Ports liegen im Bereich, aus dem Linux
kurzlebige Client-Ports vergibt. Ein Socket eines eben beendeten Gerüsts
(`TIME-WAIT`) kann einen Port bis zu 60 s belegen; der Test meldet dann
„Port … ist belegt“. Ein zweiter Lauf besteht (`ss -tan | grep <Port>`).

Ebenso lassen sich Director und FD tauschen (`ONESIMUS_RIG_ODIR`,
`ONESIMUS_RIG_OFD`), etwa gegen `/usr/sbin/bareos-dir` als Gegenprobe. Die
Kommandozeile `-f --disable-grpc -c <Verzeichnis> -d 100` ist dabei auf odir
und ofd zugeschnitten; für die Bareos-Programme muss `--disable-grpc` entfallen
(noch nicht umgesetzt).

## Aufräumen

`Rig` räumt in `Drop` ab, auch wenn ein Test vorher scheitert: Director, FD
und SD bekommen `SIGTERM` an ihre Prozessgruppe, nach 10 s `SIGKILL`; der
Cluster wird mit `pg_ctl stop -m immediate` beendet; das Verzeichnis wird
gelöscht (außer mit `ONESIMUS_RIG_KEEP=1`). Bricht der Testprozess selbst hart
ab (`kill -9`), bleibt das Verzeichnis liegen: `rm -rf /tmp/ofdrig-*` und
`pkill -f /tmp/ofdrig-` räumen von Hand auf.
