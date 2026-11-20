# osd – der Onesimus Storage Daemon

osd ersetzt bareos-sd. Er liest dieselbe Konfiguration (`bareos-sd.d`),
spricht mit bareos-dir/odir und bareos-fd/ofd über BNET und schreibt
Volumes im Bareos-Format, die `bls` und bareos-sd lesen können. Plan und
Phasen: `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`.

Stand 20.11.2026: **P1 Stufe 1**, Backup auf ein File-Device über BNET.

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
| `--device-writer <Ring>` | intern, startet osd selbst für jeden Job |

`SIGTERM`/`SIGINT` beenden osd mit Exit 0. Das Debian-Paket `osd` installiert
den Dienst, aktiviert ihn aber nicht, solange osd nicht wiederherstellen kann.

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
  (`Distributor` kommt in P2). Ohne `osd.d/` verhält sich osd wie bareos-sd.

## Was geht

- **Director-Seite:** Anmeldung mit TLS-PSK und CRAM-MD5 (`3000 OK Hello`),
  `JobId=…` (Sitzung `SDid`/`SDtime`, Schlüssel für den FD),
  `getSecureEraseCmd`, `use storage=`/`use device=` (Just-In-Time-Reservierung
  wie bareos-sd 25: `JustInTime Device`), `run`, `passive client …`,
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
  (Kopf `<FileIndex> <Stream> 0`, Daten, EOD), `append end`, `append close`.
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
Director ─BNET─► server ─► director (Befehle) ── run ──► fd_session (Erzeuger)
FD ─BNET─► server ─► Job (aktiv)        ▲                     │ Blöcke
osd ─BNET─► FD (passiv, passive client) ┘                     ▼
                                      /dev/shm/osd-<Port>-<JobId>.ring
                                                              │ Fächer, Postfach (CatReq, Jmsg)
                                                              ▼
                                  osd --device-writer (Kindprozess) ─► Volume-Datei
```

- `crates/osd/daemon`: Programm (`server`, `director`, `jobs`, `fd_session`,
  `writer`, `device`, `ringproto`, `transport`, `status`).
- `crates/osd/ring` (`osd-ring`): der Ring. Ein Erzeuger, N Verbraucher,
  Freigabe nach dem kleinsten Lesezeiger, Warten/Wecken über `futex`,
  Postfach je Verbraucher. Linux umgesetzt, macOS/Windows übersetzbare
  Platzhalter.
- `crates/osd` (`osd`): Schnittstelle, darin `askdir` (CatReq, UpdCat, Jmsg,
  Status).
- Der Transport ist ein Trait (`transport::Channel`); gRPC kommt in Stufe 3
  als zweite Umsetzung.

## Prüfen

```sh
cargo test -p osd -p osd-daemon -p osd-ring -p bareos-format
cargo test -p onesimus-rig --test osd -- --ignored --nocapture
ONESIMUS_RIG_SD=/usr/sbin/bareos-sd cargo test -p onesimus-rig --test osd -- --ignored   # Gegenprobe
```

Die Gerüst-Tests (`osd_full_backup_active_client`, Ports ab 39471;
`osd_full_backup_passive_client`, ab 39481) bauen odir, ofd und osd und
prüfen: Job `T` mit 13 Dateien und „Backup OK“, je ein Media-, JobMedia- und
13 File-Sätze, `VolBytes` gleich Dateigröße, das Volume mit dem eigenen Leser
(Label, SOS/EOS, Attribute, Prüfsummen) und mit `bls`, falls installiert.

## Abweichungen von bareos-sd

| Thema | bareos-sd | osd (Stufe 1) |
|---|---|---|
| Lesen, Restore, Verify, `bootstrap`, `read open/data/close` | ja | nein (Stufe 2): `use storage … append=0` wird mit 3924 abgelehnt, `bootstrap` mit `3904 Error bootstrap` |
| gRPC zu odir/ofd | – | kommt in Stufe 3 |
| Gerätearten | File, Tape, Fifo, Droplet … | nur `File` |
| Kein Volume verfügbar | wartet auf `label`/`mount` des Operators (`JS_WaitMedia`) | Meldung wie bareos-sd, dann endet der Job mit Fehler |
| Recycling, `relabel`, `Purged`-Volumes | ja | nein; `relabel` antwortet mit 3912 |
| `autochanger`, `listen`, `nextrun`, `finish`, `readlabel`, `replicate`, `resolve`, `setdevice`, `stats` | ja | unbekannt (3900) |
| Spooling (Daten, Attribute), Checkpoints, Quota | ja | nein (die Angaben im Job-Kommando werden gelesen und ignoriert) |
| Bandbreite (`setbandwidth`) | begrenzt die Verbindung | nur mit `2000 OK Bandwidth` beantwortet |
| `Maximum Concurrent Jobs`, Device-Belegung | Reservierungslogik mit Warteschlange | ein Job je Device; ist es belegt, scheitert die Reservierung |
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
| Hello eines Directors mit Leerzeichen im Namen | `0x01` | wie bareos-sd (Server-Seite in `onesimus-daemon`) |
