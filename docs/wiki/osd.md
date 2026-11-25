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
  ohne gleichnamige Ressource in `bareos-sd.d`, eine unbekannte
  Ressourcenart, die Fehler des [Verteilers](#verteiler-distributor-p2).
  Ohne `osd.d/` verhält sich osd wie bareos-sd,
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

## Verteiler (Distributor, P2)

Ein Verteiler ist für den Director ein Device; osd wählt für jeden
schreibenden Job anhand von Regeln (Routen) ein **Ziel-Device** aus
`bareos-sd.d`. So gehen z. B. Fulls auf eine Wechselplatte und Incrementals
auf die Platte im Rechner, ohne dass der Director mehr als einen Storage
kennt (Plan: Abschnitte „Routen“, D-1, D-2).

**Syntax.** Die Ressource gibt es nur im Onesimus-Verzeichnis,
`<Overlay>/osd.d/distributor/<Name>.conf` (Vorgabe `/etc/onesimus`,
`--overlay`); in `bareos-sd.d` wäre sie eine unbekannte Ressource, ein
bareos-sd liest die Datei nie. Die Ziele nennt sie mit den Namen ihrer
`Device`-Ressourcen aus `bareos-sd.d`. Eine Route ist eine Unterressource;
die Direktiven stehen in `{ }` mit `;` oder Zeilenwechsel getrennt, wie im
Plan:

```
# /etc/onesimus/osd.d/distributor/Verteiler.conf
Distributor {
  Name = Verteiler
  Media Type = Onesimus                 # was der Director sieht
  Route { Level = Full;          Device = USB-Wechsel }
  Route { Client = "CLOUD-fd";   Device = S3-Archiv }
  Route { Pool = Monatlich;      Device = Band-1 }
  Route { Job = Etc; Type = Backup; FileSet = Etc; Device = Platte-2 }
  Route {                        Device = FileStorage }   # ohne Merkmal: der Rest
}
```

Im Director bleibt alles wie bei einem Device: `Storage { … Device =
Verteiler; Media Type = Onesimus }`.

| Direktive | Bedeutung |
|---|---|
| `Name`, `Media Type` | Pflicht. `Name` darf kein Device und kein Autochanger sein. `Media Type` ist der des Director-Storage |
| `Description` | Text |
| `Route { … }` | mindestens eine; die **erste** passende entscheidet |
| Merkmale `Job`, `Client`, `Pool`, `FileSet` | Name der Job-Ressource, des Clients, des Pools (`use storage`), des FileSets; genau gleich geschrieben |
| Merkmal `Level` | `Full`, `Incremental`, `Differential`, `VirtualFull`, `Base`, `Since` |
| Merkmal `Type` | `Backup`, `Copy`, `Migrate`, `Archive`, `Consolidate`, `Restore`, `Verify` |
| `Device` | das Ziel, genau eins (Pflicht je Route) |
| `Stripe`, mehrere `Devices` | Striping gibt es erst in P5: `Stripe = yes` und eine Liste von Devices lehnt osd ab (`Stripe = no` ist erlaubt) |

Mehrere Merkmale einer Route gelten zusammen (und). Eine Route ohne Merkmal
passt immer; sie muss die letzte sein, und es darf nur eine geben (sonst wären
die folgenden Routen nie erreichbar). Fehlt sie und passt keine Route, schlägt
die Reservierung wie bei einem Device ohne passendes Medium fehl.

**Schreiben.** Mit `use storage` (oder, mit `Just In Time Reservation`, wenn
die ersten Daten eintreffen) prüft osd, dass der MediaType des Director-Storage
der des Verteilers ist, wählt die erste passende Route aus Level, Client
(`client_name` des `JobCommand`), Pool (`use storage`), Job, Typ und FileSet
und reserviert das **Ziel-Device**. Jedes Ziel behält seinen eigenen
MediaType (D-2 Variante a): der Katalogverkehr (`CatReq`: nächstes Volume,
Label, Update) und das Volume-Label tragen den MediaType des **Ziels**; im
Katalog stehen die Volumes also mit `VFile`, `VFile2` usw., nicht mit dem
MediaType des Verteilers. Ist das Ziel belegt, weicht osd nicht auf ein
anderes Ziel aus; es verhält sich wie bei einem normalen Device (belegt: die
Reservierung scheitert; ohne Volume: Mount-Anforderung und Warten). Das Job-Log
nennt das Ziel („Using Device "VDev2" … to write.“), das Daemon-Log die Route
(„Distributor VDist: job … goes to device VDev2 (Level = Full)“).

**Lesen.** Restore, Verify und der lesende Teil von Copy/Migration: Welches
Ziel gelesen wird, steht erst im Bootstrap fest. Der Job hält deshalb alle
freien Ziele des Verteilers (belegte stehen nicht zur Verfügung), das Ziel mit
dem MediaType des Director-Storage zuerst, sonst das erste. Der Geräteleser
nimmt für **jedes Volume** das Ziel, dessen MediaType zum Volume im Bootstrap
passt („Changing read device. Want Media Type=… have=…“, „Media Type change.
New read device … chosen.“ im Job-Log). So liest ein Restore einer Kette
(Full auf Ziel A, Incremental auf Ziel B) in einem Job von beiden Devices.
Passt kein Ziel zum MediaType, endet der Job wie bei einem Device
(„No suitable device found to read Volume …“).

**Copy und Migrate im selben SD.** Der Kopierjob liest über den Verteiler und
schreibt in das Ziel seiner Route (z. B. `Route { Type = Copy; Device = VDev2 }`).
`Type = Copy` und `Type = Migrate` gelten für den Copy- bzw. Migrate-Job, den
der SD sieht (Bareos-Typ `c`/`g`), und nicht nur für den entstehenden Datensatz
(`C`/`M`). Hält der Job das Ziel seiner Schreibroute selbst als Lesegerät,
übernimmt es die Schreibseite. Ein anderes gehaltenes Ziel wird dann
Lesegerät; gibt es keines, scheitert die Reservierung. Der Director verlangt
für Lesen und Schreiben **verschiedene Storage-Ressourcen** („cannot … using
the same read and write storage“, wie bareos-dir). Die Pool-Ressource mit
`Next Pool` braucht also eine zweite Director-Storage-Ressource, die ebenfalls
`Device = <Verteiler>` nennt. Geprüft in `copy_and_migrate_routes` (VT-3),
dort auch ein Restore der Kopie über einen Director-Storage mit
`Device = VDev, VDev2`.

**Andere Befehle.** `label` mit dem Verteiler als Device nimmt das Ziel, dessen
MediaType der des `label`-Befehls ist (der Director gibt den des Storage
mit, also nur der eines Ziels, z. B. `label … mediatype=VFile2`), sonst
Fehler 3999 mit den Zielen. `mount`/`unmount`/`release` gelten für alle Ziele.
`status storage` (und `.status devices`) nennen nach den Devices jeden
Verteiler mit seinen Routen:

```
Distributor "VDist" (Media Type "VDistMT") routes jobs to its devices:
    Route 1: Level = Full -> Device "VDev" (free)
    Route 2: always -> Device "VDev2" (in use by job Backup.2026-11-23_10.00.00_02)
```

**Prüfung (`osd -t`).** Fehler beim Laden: Ziel-Device nicht in `bareos-sd.d`
(„Route target Device … not found in bareos-sd.d“), Ziel ist ein Verteiler,
Ziel ist kein File-Device (P2 kennt nur File-Devices), Route ohne `Device`,
Verteiler ohne Route, mehr als eine Route ohne Merkmal oder eine Route nach
der ohne Merkmal, doppelter Name oder Name eines Devices/Autochangers, unbekannte
Direktive oder unbekannter Level/Typ, `Stripe = yes`/mehrere `Devices`,
fehlender `Media Type`. Ob der Director-Storage den `Media Type` des
Verteilers trägt, kann `osd -t` nicht prüfen, weil osd den Director nicht kennt
(VT-4). Ein schreibender Job mit falschem Media Type endet mit „Distributor
"…" has Media Type "…", but the Director's Storage names Media Type "…"; set
Media Type = … in the Director's Storage resource for this Distributor.“

**Grenzen.** Nur File-Devices als Ziel (S3, Wechselplatte, Band folgen mit
P3, P4, P6). Kein Striping (P5), keine Zweige (P2b). Beim Lesen wartet ein Job
nicht auf ein belegtes Ziel, er hält die freien und liest von ihnen (VT-1).

Test: `cargo test -p onesimus-rig --test osd_distributor -- --ignored --nocapture`
(Full auf `VDev`, Incremental auf `VDev2`, Restore der Kette aus beiden);
Auswahl und Konfigurationsfehler als Unit-Tests in `bareos-config`
(`sd.rs`) und `osd-daemon` (`daemon.rs`, `reader.rs`).

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
| Lesen: falsches, fehlendes oder unlesbares Volume | Warnung, dann Mount-Anforderung an den Operator (`DirAskSysopToMountVolume`) | Warnung wie bareos-sd, dann Mount-Anforderung an den Operator (M_MOUNT „Please mount read Volume …“, Status JS_WaitMount, geweckt von `mount`/`label` auf dem Device, Wartezeit eine Stunde, verdoppelt bis ein Tag, neun Runden; seit 23.11.2026, `tests/osd_read_wait.rs` gegen bareos-sd als Gegenprobe) |
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
| Copy/Migration | SD-zu-SD (`listen` am schreibenden, `replicate` am lesenden SD) | osd kann beide Seiten (seit 23.11.2026). **Schreibende Seite**: `listen` wartet ohne Zeitgrenze auf den lesenden SD (`Hello Start Storage Job <Job>`, TLS-PSK `R_JOB`, CRAM-MD5 mit dem Auth-Schlüssel des Jobs, nur BNET), dann `start replicate` / `replicate data` / `end replicate` mit denselben Datensätzen wie ein FD. Gemessen mit bareos-sd als Leser: `tests/osd_copy.rs` (eine Kopie, zwei Kopien durch einen Steuerjob; ob sie überlappen, ist nicht geprüft; `ssl=0`: Klartext zwischen den SDs gemessen, `TLS Enable = no` im Storage lässt osd `ssl=0` melden; mit bareos-sd als Leser muss auch dessen `TLS Enable`/`TLS Require` auf `no` stehen, weil Bareos TLS sonst verlangt). **Lesende Seite** (`replicate`): osd verbindet sich zum schreibenden SD (`Hello Start Storage Job`, TLS-PSK `R_JOB`, CRAM-MD5 mit `Authorization`), antwortet `3000 OK replicate` und schickt auf `run` die Datensätze wie ein FD (`start replicate`, `replicate data`, Kopf `<FileIndex> <Stream> 0`, zwei EOD, `end replicate`). Gemessen: osd zu osd und osd zu bareos-sd, jeweils mit Restore der Kopie und `diff` gegen den Quellbaum (`tests/osd_copy.rs`) |
| Copy und VirtualFull innerhalb eines SD | zwei Reservierungen in einem Job (`use storage append=0`, dann `append=1`), `run` kopiert vom Lese- zum Schreib-Device | gleich (seit 23.11.2026): Lese- und Schreibseite des Kopierprotokolls laufen in einem Prozess gegeneinander (`MemChannel`), die Datei-Indizes werden bei VirtualFull neu nummeriert wie in `mac.cc`. Gemessen: Copy auf ein zweites Device und VirtualFull aus Full und Incremental, jeweils mit Restore gegen den Quellbaum (`tests/osd_copy.rs`, auch gegen bareos-sd als Gegenprobe). Die beiden Ringe heißen `osd-<Port>-<JobId>.ring` (schreiben) und `…-<JobId + 2^31>.ring` (lesen) |
| Gerätearten | File, Tape, Fifo, Droplet … | nur `File` |
| Kein Volume verfügbar (Schreiben) | wartet auf `label`/`mount` des Operators (`JS_WaitMedia`) | gleich (seit 21.11.2026, C-089: M_MOUNT, JS_WaitMedia, geweckt von `label`/`mount`, Poll, Wartezeiten; auf dem H340 geprüft) |
| Recycling, `relabel`, `Purged`-Volumes | ja | Recycling seit 25.11.2026 wie bareos-sd (das Volume wird neu beschrieben, „Recycled volume … all previous data lost.“, Katalog mit `relabel=1`; Rig-Vergleich `sd_recycle.rs`); `relabel` seit 25.11.2026: Antwort und Katalog wie bareos-sd, die Volume-Datei wird aber umbenannt (bareos-sd 25.1.3 behält den alten Dateinamen, das umbenannte Volume ist dann nicht mehr ladbar, C-096; Rig-Test `sd_relabel.rs`) |
| `listen`, `replicate` | ja | ja (schreibende und lesende Seite einer Kopie) |
| `autochanger`, `nextrun`, `finish`, `readlabel`, `resolve`, `setdevice`, `stats` | ja | unbekannt (3900) |
| Spooling (Daten, Attribute), Checkpoints, Quota | ja | nein (die Angaben im Job-Kommando werden gelesen und ignoriert) |
| Bandbreite (`setbandwidth`) | begrenzt die Verbindung | nur mit `2000 OK Bandwidth` beantwortet |
| `Maximum Concurrent Jobs`, Device-Belegung | Reservierungslogik mit Warteschlange, Lesen und Schreiben getrennt gezählt; bei `Maximum Concurrent Jobs` > 1 am Device schreiben mehrere Jobs verschachtelt in dasselbe Volume | ein Job je Device; ein schreibender Job wartet, bis das Device frei ist (seit 25.11.2026, wie bareos-sd mit der Vorgabe 1, ohne Meldung; Rig-Test `sd_busy_device.rs` gegen bareos-sd); Verschachteln bei > 1 folgt (Schritt 2); ein lesender Job wartet ebenso (seit 25.11.2026, Heartbeat an den Director, `sd_busy_read.rs`) |
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


## Gesundheit der Platten (SMART, seit 23.11.2026)

osd liest zu jedem File-Device die SMART-Werte der Platte, auf der das `Archive Device` liegt (Crate `smartread`, ATA/SATA über SG_IO und NVMe, ohne `smartctl`), und bewertet sie mit `ok`, `warning` oder `critical`. Die Platte wird über den Einhängepunkt gefunden; md, LVM und Netz-Dateisysteme bleiben `unknown`. Die Abfrage läuft höchstens alle zehn Minuten je Platte.

| Wo | Was |
| --- | --- |
| `status storage` | Aufbau wie bareos-sd 25.1.3 (Rig-Test `status_sd.rs`); je Device zusätzlich eine Zeile `Health: Disk /dev/sdb (<Modell>): ok, 32 °C, 54879 h, pending 0, reallocated 0, uncorrectable 0, CRC 0` |
| `.status health` | je Device eine Zeile `Device=… Disk=… Level=… Temperature=… PowerOnHours=… Pending=… Reallocated=… OfflineUncorrectable=… CrcErrors=… SelfTest="…" Model="…" Reasons="…"` |

**Bewertung:** `critical` bei SMART FAILED, fehlgeschlagenem Selbsttest, einem Pre-Fail-Attribut an seiner Schwelle oder ab 60 °C (NVMe: kritische Warnung, Reserve unter der Schwelle); `warning` bei Pending, Reallocated oder Offline-Uncorrectable Sectors, CRC-Fehlern, ab 55 °C (NVMe: Medienfehler, 90 % Haltbarkeit verbraucht). Fehlerprotokoll und Selbsttestprotokoll der Platte liest `smartread` noch nicht.

**Rechte:** SMART braucht `CAP_SYS_RAWIO` (NVMe `CAP_SYS_ADMIN`) und Zugriff auf `/dev/sdX`; ohne beides meldet osd `unknown` mit dem Grund. Auf dem H340: Gruppe `disk` und `capabilities="^cap_sys_rawio"` im OpenRC-Dienst (siehe den Plan, D-21).

### Der Director wertet es aus

`Health Check Interval` (Onesimus-Direktive, ohne Angabe aus: der Director fragt einen SD sonst nie von sich aus ab). Bareos kennt sie nicht; damit `bareos-dir.d` Standard bleibt, steht sie im Overlay `/etc/onesimus/odir.d/` (Option `--overlay <Verzeichnis>` von odir und osd, Vorgabe `/etc/onesimus`), in `/etc/onesimus/odir.d/storage/H340.conf`:

```
Storage {
  Name = H340
  Health Check Interval = 1 hour
}
```

Statt die Datei von Hand anzulegen: `configure update storage=H340 healthcheckinterval="1 hour" overlay` schreibt die Datei (`/etc/onesimus/odir.d/storage/H340.conf`), lädt die Konfiguration neu und stellt die alte Datei wieder her, wenn der Director die Änderung nicht annimmt. Der Schalter `overlay` nimmt nur Onesimus-Direktiven; eine Bareos-Direktive weist er ab. `healthcheckinterval=` (leer) entfernt die Direktive.

Jede Ressource in `odir.d/` erweitert die gleichnamige Ressource aus `bareos-dir.d`; unbekannte Ressourcen und andere Direktiven lehnt der Director beim Laden ab (`odir.d: Storage "H340": "Password" is not an Onesimus extension directive`). Erlaubt sind alle Onesimus-Direktiven des Directors:

| Ressource | Direktiven |
| --- | --- |
| `Director` | `Grpc Port`, `Grpc TLS Certificate`, `Grpc TLS Key`, `mDNS Advertise` |
| `Catalog` | `DB Driver` |
| `Client` | `Grpc Enable`, `Grpc Port`, `Grpc CA Certificate File`, `Grpc Server Name` |
| `Storage` | `Grpc Enable`, `Grpc Port`, `Grpc CA Certificate File`, `Grpc Server Name`, `Health Check Interval` |

**osd und ofd:** `osd.d/` und `ofd.d/` liegen im selben Verzeichnis (`--overlay <Verzeichnis>`, Vorgabe `/etc/onesimus`), also `/etc/onesimus/osd.d/storage/<Name>.conf` und `/etc/onesimus/ofd.d/client/<Name>.conf`. Für ofd gelten dort `Grpc Enable`, `Grpc Port` und `Grpc Address` (die Kommandozeile `--grpc-listen` und `--disable-grpc` geht vor).

Die gRPC-Direktiven standen früher in `bareos-dir.d`. Dort nimmt sie der Director noch an (damit eine bestehende Konfiguration läuft), warnt aber beim Laden und nennt den Zielort; was in `odir.d/` steht, gilt zuletzt und gewinnt. Bareos selbst lehnt sie in `bareos-dir.d` ab: wer zu Bareos zurückwill, nimmt sie dort heraus.

Der Director fragt dann `.status health` im Abstand (er schaut alle 30 Sekunden nach fälligen Abfragen) und

* schreibt je Device und Abfrage eine Zeile in `onesimus.medium_health` (Schema-Schritt v2256: `storage`, `device`, `checked_at`, `disk`, `model`, `level`, `reasons`, Temperatur, Betriebsstunden, Pending/Reallocated/Uncorrectable/CRC, Selbsttest) für Verläufe,
* meldet über die Messages-Ressource des Directors (`Daemon`: Mail, Operator) jeden Wechsel des Urteils und eine erste Messung, die nicht `ok` ist: Warnung bei `warning` und `unknown`, Fehler bei `critical`, Info, wenn das Device wieder `ok` ist.

Beispiel einer Auswertung des Verlaufs: `SELECT DISTINCT ON (storage, device) * FROM onesimus.medium_health ORDER BY storage, device, checked_at DESC`.

Getestet: `onesimus-rig/tests/osd_health.rs` (Zeile in der Tabelle und Meldung im Daemon-Log), Unit-Tests in `storage_health.rs`. Gemessen auf dem H340: `status storage` zeigt `sdb` und `sdc` als `ok`.
