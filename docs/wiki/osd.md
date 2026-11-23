# osd – der Onesimus Storage Daemon

osd ersetzt bareos-sd. Er liest dieselbe Konfiguration (`bareos-sd.d`),
spricht mit odir und ofd über **gRPC mit TLS** (Hauptweg), mit bareos-dir
und bareos-fd über BNET (Rückfallweg, wie bareos-sd), und schreibt Volumes
im Bareos-Format, die `bls` und bareos-sd lesen können. Plan und Phasen:
`docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`.

Stand 20.11.2026: **P1 abgeschlossen**. Backup, Restore und Verify
(VolumeToCatalog) auf File-Devices, auch über mehrere Volumes, mit aktivem
und passivem Client, über gRPC und über BNET.

## Starten

```sh
cargo build -p osd-daemon                 # Programm target/debug/osd
target/debug/osd -t -c /etc/bareos        # Konfiguration prüfen
target/debug/osd -f -c /etc/bareos -d 50  # im Vordergrund laufen
```

| Argument | Bedeutung |
|---|---|
| `-c <Pfad>` | Datei, Verzeichnis mit `bareos-sd.d` oder `bareos-sd.d` selbst (Vorgabe `/etc/bareos`) |
| `-f` | im Vordergrund (osd geht in Stufe 1 noch nicht selbst in den Hintergrund) |
| `-d <n>` | Debug-Level; ab 1 Logging auf `debug` (`>dird`/`<dird` jeder Katalog-Anfrage) |
| `-t` | Konfiguration prüfen, Exit 0 oder 1 |
| `--disable-grpc` | keinen gRPC-Port öffnen, nur BNET (wie `ofd --disable-grpc`) |
| `--grpc-listen <Adresse:Port>` | gRPC-Port statt `osd.d` (`Grpc Address`/`Grpc Port`); scheitert er, endet osd mit 1 |
| `--grpc-cert <PEM>`, `--grpc-key <PEM>` | eigenes TLS-Zertifikat; sonst erzeugt osd beim ersten Start `onesimus-sd-grpc.crt`/`.key` im Working Directory |
| `--device-writer <Ring>` | intern, startet osd selbst für jeden Backup-Job |
| `--device-reader <Ring>` | intern, startet osd selbst für jeden Restore- und Verify-Job |

`SIGTERM`/`SIGINT` beenden osd mit Exit 0. Das Debian-Paket `osd` installiert
den Dienst, aktiviert ihn aber nicht; einen Rechner auf osd umzustellen
entscheidet der Maintainer.

## Konfiguration

- `bareos-sd.d` bleibt unverändert; ein bareos-sd kann denselben Baum lesen.
  Gelesen werden Storage, Director, Device, Messages, Autochanger; neu in
  Stufe 1 auch `Maximum Block Size` (Vorgabe 1 MiB), `Label Block Size`,
  `Maximum Volume Size`, `Just In Time Reservation` (Vorgabe yes) und
  `Client Connect Wait` (Vorgabe 1800 s).
- **`osd.d/`** (Plan D-8) liegt neben `bareos-sd.d` bzw. neben der
  Konfigurationsdatei, mit demselben Aufbau `osd.d/<Typ>/<Name>.conf`. Ein
  Eintrag erweitert die gleichnamige Ressource aus `bareos-sd.d` um
  Onesimus-Direktiven. Stufe 1 kennt nur `Ring Slots` (Fächer des Job-Rings,
  an `Storage` als Vorgabe, an `Device` je Gerät, mindestens 2, Vorgabe 8):

  ```
  # osd.d/device/FileStorage.conf
  Device {
    Name = FileStorage
    Ring Slots = 16
  }
  ```

  Fehler, die `osd -t` meldet: eine Bareos-Direktive in `osd.d`, ein Eintrag
  ohne gleichnamige Ressource in `bareos-sd.d`, eine neue Ressourcenart
  (`Distributor` kommt in P2). Ohne `osd.d/` verhält sich osd wie bareos-sd,
  bis auf den gRPC-Port (Vorgabe `SD Port` + 1000, siehe unten).
- **gRPC-Port** (Stufe 3) an der Storage in `osd.d`: `Grpc Enable`
  (Vorgabe yes), `Grpc Port` (Vorgabe `SD Port` + 1000, wie ofd mit FD-Port
  + 1000), `Grpc Address` (Vorgabe `SD Address`, sonst alle):

  ```
  # osd.d/storage/PDC-sd.conf
  Storage {
    Name = PDC-sd
    Grpc Port = 10103
  }
  ```

## Transporte: gRPC und BNET

Grundsatz des Plans: „gRPC ist unser Pfad, auch analog zum ofd.“ BNET
bleibt für bareos-dir und bareos-fd.

```
odir ──gRPC StorageDaemon/Control──► osd ◄──gRPC StorageDaemon/Data── ofd (aktiver Client)
odir ──BNET SD-Port (Rückfall)─────► osd ──gRPC FileDaemon/Data─────► ofd (passiver Client)
bareos-dir ──BNET──► osd ◄──BNET── bareos-fd
```

- **Proto:** `proto/stored_service.proto` (onesimus-proto, Zweig
  `stored`): `service StorageDaemon { rpc Control(stream StoredFrame)
  returns (stream StoredFrame); rpc Data(…) }`. Ein `StoredFrame` ist genau
  eine BNET-Nachricht (Nutzdaten oder Signal) ohne Längenkopf, mit
  denselben Feldern wie `filed.ControlFrame`. Hello, CRAM-MD5 und alle
  Befehle laufen im TLS-Strom unverändert, wie über BNET. `FileDaemon` hat
  dazu `Data` für die Verbindung eines osd zum passiven Client bekommen.
- **Transport:** das Crate `onesimus-bnet-grpc` (gemeinsam für odir, ofd,
  osd; Herkunft `bareos-bnet` `fd_grpc`). In osd ist ein gRPC-Strom die
  zweite Umsetzung von `transport::Channel`; Befehlsschleife, Schreiben und
  Lesen sind dieselben.
- **Anmeldung:** wie odir ↔ ofd. TLS mit dem Zertifikat des osd (beim
  ersten Start selbstsigniert erzeugt, der Fingerabdruck bleibt über
  Neustarts gleich); der Director hält es mit `Grpc CA Certificate File`
  fest und prüft `Grpc Server Name`. Danach CRAM-MD5 mit dem
  Storage-Passwort (Director) bzw. dem Job-Schlüssel (FD) im Strom. Kein
  zweites Passwortsystem. Ein Strom nimmt nur die Art seiner RPC an
  (`Control`: Director, `Data`: FD eines Jobs).
- **Director** (Storage-Ressource, dieselben Onesimus-Direktiven wie am
  Client):

  ```
  Storage {
    Name = PDC-sd
    Address = sd.example.org
    SD Port = 9103
    Password = "…"
    Device = FileStorage
    Media Type = File
    Grpc Enable = yes
    Grpc Port = 10103                        # Vorgabe SD Port + 1000
    Grpc CA Certificate File = /etc/bareos/tls/osd.crt   # Kopie von onesimus-sd-grpc.crt
    Grpc Server Name = sd.example.org        # Vorgabe Address
  }
  ```

  Ohne `Grpc Enable = yes` versucht odir kein gRPC (bareos-sd). Mit ihm
  versucht odir jeden Weg zum SD zuerst über gRPC: Job-Pfade (Backup,
  Restore, Verify) mit dem Ziel, das der Job vor dem `fork` mitbekommt,
  Befehlswege (`status storage`, `label`, `mount`, `setdebug` …) über den
  gRPC-Wähler, den odir in `osd::connect` anmeldet. Scheitern Verbindung,
  TLS, Dienst oder Anmeldung, schreibt odir die Ursache ins Log („SD
  <Adresse>:<gRPC-Port>: …; falling back to BNET on port <SD Port>“) und
  nimmt BNET mit den bisherigen Fehlertexten. Ein laufender Job wechselt
  den Transport nie.
- **Datenverbindung FD ↔ SD:** Laufen die Steuerverbindungen zu SD *und*
  FD über gRPC (beide sind also Onesimus-Daemons), hängt odir die
  gRPC-Angaben der Gegenseite an den Befehl: an `storage address=…` für
  ofd die des osd (`grpcport=… grpcname=… grpcca=<PEM base64>`), an
  `passive client address=…` für osd die des ofd. ofd bzw. osd versuchen
  dann `StorageDaemon/Data` bzw. `FileDaemon/Data` und fallen bei jedem
  Fehler auf BNET mit Adresse und Port des Befehls zurück. bareos-fd und
  bareos-sd sehen die Angaben nie.
- **Log-Zeilen** (welcher Weg lief): osd „Director connects over gRPC
  TLS|BNET“, „File daemon of job … connects over gRPC TLS|BNET“,
  „connected to File daemon … over gRPC TLS|BNET (passive client)“; ofd
  „Connected Storage daemon at … over gRPC TLS|BNET“; odir „SD …: connected
  over gRPC TLS“; im Job-Log „Connected Storage daemon at …, encryption:
  gRPC TLS“.
- **Abschalten:** `osd --disable-grpc` oder `Grpc Enable = no` in `osd.d`;
  odir fällt dann auf BNET zurück, ofd bekommt keine gRPC-Angaben.

## Was geht

- **Director-Seite:** Anmeldung mit TLS-PSK und CRAM-MD5 (`3000 OK Hello`),
  `JobId=…` (Sitzung `SDid`/`SDtime`, Schlüssel für den FD),
  `getSecureEraseCmd`, `use storage=`/`use device=` (Schreiben mit
  Just-In-Time-Reservierung wie bareos-sd 25: `JustInTime Device`; Lesen mit
  `append=0`: das Device wird sofort reserviert), `bootstrap` (vor oder nach
  `use storage`, wie odir es bei Verify bzw. Restore schickt), `run`,
  `passive client …`,
  `status <arg>`, `.status current|last|header|running|devices`, `label`,
  `mount`, `unmount`, `release`, `cancel`, `setdebug=`, `setbandwidth=`,
  `pluginoptions`.
- **FD-Seite, aktiv:** Der FD ruft an (`Hello Start Job <Job>`, TLS-PSK mit
  Identität `R_JOB` und dem Job-Schlüssel, CRAM-MD5).
- **FD-Seite, passiv** (`Passive = yes` am Client): odir schickt
  `passive client address=… port=… ssl=4`, osd ruft den FD an
  (`Hello Storage calling Start Job <Job>`, TLS-PSK `R_JOB`, CRAM-MD5 als
  Client) und antwortet `2000 OK passive client`.
- **Datenphase** in beiden Fällen gleich: `append open`, `append data`
  (Kopf `<FileIndex> <Stream> 0`, Daten, EOD), `append end`, `append close`;
  beim Restore und Verify `read open`, `read data`, `read close` (je
  Datensatz `rechdr <VolSessionId> <VolSessionTime> <FileIndex> <Stream>
  <Länge>`, dann die Daten, am Ende EOD).
- **Bootstrap** (`bareos_format::bsr`): alle Schlüsselwörter von
  `parse_bsr` (Volume, MediaType, Device, Slot, VolSessionId/-Time, VolAddr,
  VolFile/VolBlock, FileIndex, Count, JobId, Job, Client, Stream,
  FileRegex). Die Volumes werden in Bootstrap-Reihenfolge gelesen, je
  Volume nur die `VolAddr`-Bereiche („Forward spacing Volume …“); jeder
  Datensatz wird wie in `stored/bsr.cc` geprüft (`MatchBsr` mit den
  Erledigt-Merkern, `IsThisBsrDone`). Labels gehen nicht an den FD.
- **Lesen über mehrere Volumes:** Am Ende eines Volumes („End of Volume at
  file … on device …“) geht es mit dem nächsten des Bootstraps weiter
  (`GetVolInfo … write=0`, Label-Prüfung, „Ready to read from volume …“);
  ein Datensatz, den das Backup am Volume-Ende zerteilt hat, kommt beim FD
  ganz an. Danach „End of all volumes.“, „Releasing device …“ und
  `UpdateMedia` für das zuletzt gelesene Volume, wie bareos-sd.
- **Falsches Volume:** „Read acquire: Wrong Volume mounted on device …:
  Wanted … have …“ (Warnung), dann „Too many errors trying to mount device
  … for reading.“ und der Job endet mit Fehler; ebenso bei fehlender Datei
  („Read open device … failed“) oder fehlendem Label.
- **Volume:** Volumes über CatReq (`FindMedia`, `GetVolInfo`, `UpdateMedia`,
  `CreateJobMedia`, `DeleteNullJobmediaRecords`) in derselben Reihenfolge wie
  bareos-sd; neue Volumes werden gelabelt (`Label Media = yes`), vorhandene
  geprüft und am Ende fortgeschrieben; bei `MaxVolBytes` bzw.
  `Maximum Volume Size` wird das Volume `Full` und der Job geht auf dem
  nächsten weiter. Attribute gehen an den Director, sobald ihr Block auf dem
  Volume steht.
- **Format:** Blockkopf `BB02` mit CRC-32, Datensatzköpfe Version 2,
  Volume-Label, SOS- und EOS-Label (`bareos-format`, `volume` und `label`).
  Belege: Label-Block und letzter Block eines bareos-sd-Volumes werden
  byte-genau nachgebaut (`bareos_format::volume::tests`); im Gerüst sind die
  Datenblöcke 2–12 eines Backups mit osd byte-gleich mit denen von bareos-sd,
  die übrigen unterscheiden sich nur in Zeitstempeln, Versionstext und
  Rechnernamen. `bls` listet die 13 Dateien des Testbaums.

## Aufbau

```
Director ─gRPC/BNET─► server ─► director (Befehle) ── run ──► fd_session (Erzeuger)
FD ─gRPC/BNET─► server ─► Job (aktiv)        ▲                     │ Blöcke
osd ─gRPC/BNET─► FD (passiv, passive client) ┘                     ▼
                                      /dev/shm/osd-<Port>-<JobId>.ring
                                                              │ Fächer, Postfach (CatReq, Jmsg)
                                                              ▼
                                  osd --device-writer (Kindprozess) ─► Volume-Datei
```

Lesen (Restore, Verify) nutzt denselben Ring in Gegenrichtung:

```
FD ◄─gRPC/BNET─ fd_session ◄─ read_session (Verbraucher: Datensätze, Bootstrap-Filter, rechdr)
                                   ▲ Blöcke                      │ CatReq, Jmsg (Postfach, Leser fragt)
                     /dev/shm/osd-<Port>-<JobId>.ring             ▼
                                   ▲                          Director
                  osd --device-reader (Kindprozess, Erzeuger) ◄─ Volume-Dateien in Bootstrap-Reihenfolge
```

- `crates/osd/daemon`: Programm (`server`, `grpc`, `director`, `jobs`,
  `fd_session`, `writer`, `read_session`, `reader`, `device`, `ringproto`,
  `transport`, `status`).
- `crates/osd/ring` (`osd-ring`): der Ring. Ein Erzeuger, N Verbraucher,
  Freigabe nach dem kleinsten Lesezeiger, Warten/Wecken über `futex`,
  Postfach je Verbraucher; zum Lesen übernimmt der Geräteleser die
  Erzeugerrolle (`take_producer`) und fragt die Netzsitzung über das
  Postfach (`ask_consumer`). Linux umgesetzt, macOS/Windows übersetzbare
  Platzhalter.
- `crates/bareos-format`: Volume-Format, dazu `bsr` (Bootstrap) und
  `volume::RecordAssembler` (Datensätze je Sitzung über Blöcke und Volumes).
- `crates/osd` (`osd`): Schnittstelle, darin `askdir` (CatReq, UpdCat, Jmsg,
  Status).
- Der Transport ist ein Trait (`transport::Channel`) mit zwei Umsetzungen
  über `BnetStream`: BNET-Socket und gRPC-Strom (`onesimus-bnet-grpc`
  `ControlIo`).
- `crates/onesimus-bnet-grpc`: der gRPC-Transport von odir, ofd und osd.

## Prüfen

```sh
cargo test -p osd -p osd-daemon -p osd-ring -p bareos-format -p onesimus-bnet-grpc
cargo test -p onesimus-rig --test osd -- --ignored --nocapture
cargo test -p onesimus-rig --test osd_grpc -- --ignored --nocapture --test-threads=1   # Interop-Matrix
ONESIMUS_RIG_SD=/usr/sbin/bareos-sd cargo test -p onesimus-rig --test osd -- --ignored   # Gegenprobe
ONESIMUS_RIG_SD=/usr/sbin/bareos-sd cargo test -p onesimus-rig --test osd_grpc -- --ignored --test-threads=1   # odir fällt auf BNET zurück
cargo build -p osd-daemon && ONESIMUS_RIG_SD=$PWD/target/debug/osd cargo test -p onesimus-rig --test rig -- --ignored
```

Die Gerüst-Tests in `crates/onesimus-rig/tests/osd.rs` bauen odir, ofd
und osd und laufen seit Stufe 3 auf gRPC (ofd mit gRPC-Port, Client und
Storage mit `Grpc Enable = yes`); gegen bareos-sd fällt odir für den SD auf
BNET zurück. Sie prüfen:

| Test | Ports | Inhalt |
|---|---|---|
| `osd_backup_restore_active_client` | ab 39471 | Backup (13 Dateien, Media, JobMedia, File-Sätze, `VolBytes` = Dateigröße, eigener Leser, `bls`), dann Restore in ein anderes Verzeichnis: `diff -r`, Attribute (Rechte, Eigentümer, mtime, Symlink-Ziele), Hardlinks, Löcher |
| `osd_backup_restore_passive_client` | ab 39481 | dasselbe mit `Passive = yes`: osd ruft den FD an, auch beim Restore |
| `osd_backup_verify_restore_over_several_volumes` | ab 39491 | `Maximum Volume Bytes = 5M`: Backup über drei Volumes, Verify VolumeToCatalog und Restore lesen alle |
| `osd_and_bareos_sd_read_each_others_volumes` | ab 39501 | Backup mit bareos-sd → Restore mit osd; osd hängt ein Backup an dasselbe Volume → Restore mit bareos-sd; `bls` liest beide Sitzungen |

Die Interop-Matrix `crates/onesimus-rig/tests/osd_grpc.rs` macht je Fall
Backup, Verify und Restore und belegt aus den Logs, welcher Transport lief
(`RigOptions::fd_grpc`/`sd_grpc`: `On`, `Disabled` = konfiguriert, aber
`--disable-grpc`, `Off`):

| Test | Ports | Director ↔ SD | Director ↔ FD | FD ↔ SD |
|---|---|---|---|---|
| `a_all_grpc_active` | ab 39511 | gRPC | gRPC | gRPC (`StorageDaemon/Data`) |
| `a_all_grpc_passive` | ab 39521 | gRPC | gRPC | gRPC (`FileDaemon/Data`) |
| `b_osd_disable_grpc_active` | ab 39531 | BNET (Rückfall) | gRPC | BNET |
| `b_osd_disable_grpc_passive` | ab 39541 | BNET (Rückfall) | gRPC | BNET |
| `c_ofd_disable_grpc_active` | ab 39551 | gRPC | BNET (Rückfall) | BNET |
| `c_ofd_disable_grpc_passive` | ab 39561 | gRPC | BNET (Rückfall) | BNET |

Dazu besteht `crates/onesimus-rig/tests/rig.rs` mit
`ONESIMUS_RIG_SD=$PWD/target/debug/osd` vollständig (Backup, Estimate,
Verify, Restore, Abbruch und die vier erwarteten Defekte).

## Abweichungen von bareos-sd

| Thema | bareos-sd | osd (P1) |
|---|---|---|
| Lesen: Gerät | im Job-Thread | Geräteleser als eigener Prozess füllt den Ring, die Netzsitzung leert ihn (D-13) |
| Lesen: falsches, fehlendes oder unlesbares Volume | Warnung, dann Mount-Anforderung an den Operator (`DirAskSysopToMountVolume`) | Warnung wie bareos-sd, dann „Too many errors trying to mount device … for reading.“, der Job endet |
| Lesen: Volume mit anderem MediaType im Bootstrap | sucht ein anderes Device („Changing read device …“) | Meldung „Changing read device …“, dann „No suitable device found to read Volume …“ (ein Device je Job) |
| Lesen: Positionieren | springt erst, wenn ein Bootstrap-Abschnitt erledigt ist (`TryDeviceRepositioning`) | liest je Volume genau die vereinigten `VolAddr`-Bereiche; welche Datensätze an den FD gehen, entscheidet derselbe Filter wie bei bareos-sd |
| Lesen: `MatchBsrBlock` (Blöcke fremder Sitzungen überspringen) | nur bei Labels ausgewertet | nicht benutzt |
| Lesen: Block größer als ein Fach | Puffer wird vergrößert | Fehler; Fach = größte von `Maximum Block Size`, `Label Block Size` und 1 MiB |
| Lesen: Prüfsummenfehler im Block | Fehler, Job endet | gleich (Meldung „Volume data error at 0:<Adresse>! …“) |
| Lesen: „Releasing device“ | nach dem EOD an den FD | sobald der Leser das letzte Volume gelesen hat (der FD kann dann noch Daten verarbeiten); `UpdateMedia` mit denselben Feldern |
| Bootstrap-Datei | `<Working Directory>/<Storage>.<Job>.<n>.bootstrap`, gleich nach dem Lesen gelöscht | gleicher Name, bleibt bis zum Jobende (der Geräteleser liest sie erneut) |
| Bootstrap: `JobType`, `JobLevel` | lesen den Wert nicht (er wird dann als unbekanntes Schlüsselwort gemeldet) | Wert wird übersprungen |
| gRPC zu odir/ofd | – | `StorageDaemon` Control/Data mit TLS, BNET als Rückfall (Stufe 3) |
| Datensatz über gRPC | – | wie über BNET zwei Nachrichten (`rechdr`, Daten); ein Datensatz je Nachricht mit Kopf als Feldern folgt nach P1 |
| Copy/Migration | SD-zu-SD (`listen` am schreibenden, `replicate` am lesenden SD) | osd kann beide Seiten (seit 23.11.2026). **Schreibende Seite**: `listen` wartet ohne Zeitgrenze auf den lesenden SD (`Hello Start Storage Job <Job>`, TLS-PSK `R_JOB`, CRAM-MD5 mit dem Auth-Schlüssel des Jobs, nur BNET), dann `start replicate` / `replicate data` / `end replicate` mit denselben Datensätzen wie ein FD. Gemessen mit bareos-sd als Leser: `tests/osd_copy.rs` (eine Kopie, zwei Kopien durch einen Steuerjob; ob sie überlappen, ist nicht geprüft; `ssl=0` nicht gemessen). **Lesende Seite** (`replicate`): osd verbindet sich zum schreibenden SD (`Hello Start Storage Job`, TLS-PSK `R_JOB`, CRAM-MD5 mit `Authorization`), antwortet `3000 OK replicate` und schickt auf `run` die Datensätze wie ein FD (`start replicate`, `replicate data`, Kopf `<FileIndex> <Stream> 0`, zwei EOD, `end replicate`). Gemessen: osd zu osd und osd zu bareos-sd, jeweils mit Restore der Kopie und `diff` gegen den Quellbaum (`tests/osd_copy.rs`) |
| VirtualFull | SD-zu-SD | wie Copy; nicht gemessen |
| Gerätearten | File, Tape, Fifo, Droplet … | nur `File` |
| Kein Volume verfügbar (Schreiben) | wartet auf `label`/`mount` des Operators (`JS_WaitMedia`) | Meldung wie bareos-sd, dann endet der Job mit Fehler |
| Recycling, `relabel`, `Purged`-Volumes | ja | nein; `relabel` antwortet mit 3912 |
| `listen`, `replicate` | ja | ja (schreibende und lesende Seite einer Kopie) |
| `autochanger`, `nextrun`, `finish`, `readlabel`, `resolve`, `setdevice`, `stats` | ja | unbekannt (3900) |
| Spooling (Daten, Attribute), Checkpoints, Quota | ja | nein (die Angaben im Job-Kommando werden gelesen und ignoriert) |
| Bandbreite (`setbandwidth`) | begrenzt die Verbindung | nur mit `2000 OK Bandwidth` beantwortet |
| `Maximum Concurrent Jobs`, Device-Belegung | Reservierungslogik mit Warteschlange, Lesen und Schreiben getrennt gezählt | ein Job je Device, ob lesend oder schreibend; ist es belegt, scheitert die Reservierung |
| `status` | ausführlich (Spool, Volumes, beendete Jobs) | Kopf, laufende Jobs, Devices |
| `.status volumes/spooling/terminated/waitreservation` | Listen | leer |
| Attribute an den Director | wenn der Block geschrieben ist (Blockwechsel) | wenn der Geräteschreiber das Fach freigegeben hat (gleiche Reihenfolge) |
| `JobBytes` im 3099 | Summe der Datensätze | gleich |
| Volume-Label `LabelProg`/`ProgVersion` | Storage-Name, `Ver. 25.1.3 …` | Storage-Name, `Ver. <osd-Version> <Baudatum>` |
| Schreiben | im Job-Thread | Netzsitzung → Ring → Geräteschreiber als eigener Prozess (D-13) |
| Fächer des Rings | – | Vorgabe 8 Fächer zu je `Maximum Block Size` (1 MiB), also 8 MiB je Job in `/dev/shm`; D-12 nennt 64 als Beispiel, `Ring Slots` in `osd.d` stellt es ein |
| Ring-Name | – | `/dev/shm/osd-<SD-Port>-<JobId>.ring`: der Port trennt mehrere osd auf einem Rechner (der Plan nennt `osd-<JobId>.ring`) |
| Hintergrund (`-f` weglassen), PID-Datei, `-u`/`-g` | ja | osd bleibt im Vordergrund |
| Unbekannte Direktiven in `bareos-sd.d` | Fehler | werden noch nicht abgelehnt (aus P0) |
| Job-Kommando fehlerhaft | `3915 … stat=<Zahl der Felder>` | `stat=0` |
| Hello eines Directors mit Leerzeichen im Namen | `0x01` | wie bareos-sd (Server-Seite in `daemon-crate`) |
