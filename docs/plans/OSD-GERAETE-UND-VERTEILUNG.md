# osd: ein Netzwerk-Stream, mehrere Ausgabegeräte

Stand: 20.11.2026. Status: **Entschieden** (D-1 bis D-13, Maintainer 20.11.2026), Umsetzung in Phasen.

## Vorgabe (Maintainer, 20.11.2026)

> Der SD hat einen Netzwerk-Stream, der auf diverse Out-Devices verteilt wird.

Präzisiert per Rückfrage:

- **Verteilen** heißt **nach Regeln routen**, **aufteilen (Striping)** und **verzweigen (Branch)**: ein Stream speist mehrere Jobs, siehe [Zweige](#zweige-branch-3-2-1-im-laufenden-betrieb).
- **Spiegeln** ist nicht gemeint, **Ausweichen** auf ein anderes Device auch nicht.
- **Gerätearten:** Datei/Verzeichnis, Wechselplatten (USB), Objektspeicher (S3), Band.

## Ausgangslage (gemessen 20.11.2026)

- `crates/osd` ist eine **Protokoll-Bibliothek** und noch kein Daemon (621 Zeilen: `connect`, `command`, `job`, `catrec`, `status`). odir benutzt sie an 8 Stellen ([ofd-Gutachten §4](../audits/ofd-struktur-2026-11-19.md)).
- Auf PDC läuft **bareos-sd 25** mit einem einzigen File-Device. Im Director ist es so konfiguriert: `Storage PDC-sd`, `Device = FileStorage`, `MediaType = File`.
- Laut ofd-Gutachten §5 sollen vor dem osd-Bau `bareos-format` (Stromformat, Record-Header, später Volume-/Blockformat) und `onesimus-daemon` (Accept-Schleife, Handshake, TLS, Bandbreite) wenigstens als Hülle vorliegen. Sonst entsteht eine dritte Kopie dieser Teile neben odir und ofd.

## Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Device** | Wie in Bareos: ein Ausgabegerät des SD mit `ArchiveDevice`, `MediaType` und `DeviceType`. Ein Device schreibt immer genau ein Volume zur Zeit. |
| **Verteiler** (neu) | Onesimus-Ressource im SD. Sie nimmt den Stream eines Jobs an und gibt ihn an ein oder mehrere Devices weiter. Der Director sieht den Verteiler wie ein Device. |
| **Route** (neu) | Eine Regel im Verteiler: Passen die Merkmale, gehört der Job zu diesem Ziel. Merkmale sind Job, Client, Level, Pool, Typ und FileSet. Die erste passende Regel entscheidet. |
| **Zweig** (Branch, neu) | Ein eigener Job, der seine Daten nicht selbst beim FD liest, sondern aus dem Stream eines Haupt-Jobs bekommt. Er hat eigenes Ziel, eigene Volumes und eigene Katalogsätze. |
| **Stripe-Satz** (neu) | N Volumes auf N Devices, auf die ein Job gleichzeitig schreibt. Die Blöcke gehen reihum an die Devices. Erst alle N zusammen ergeben die Sicherung. |

## Grundsatz: gRPC ist der Weg, BNET bleibt für Bareos

Vorgabe (Maintainer, 20.11.2026): „gRPC ist unser Pfad, auch analog zum ofd.“

- **Zu odir und zu ofd:** osd spricht **gRPC mit TLS**. Das ist derselbe Weg, den odir und ofd heute nehmen (`fd_grpc` in bareos-bnet bzw. dessen Nachfolger in der ofd-Schnittstelle, ofd-Gutachten §5).
  - Über die Steuerverbindung laufen Job-Setup, `use storage`, CatReq, `status`, `label`, `mount` und `release`.
  - Über die Datenverbindung vom FD laufen die Records.
  - Die osd-Schnittstelle (`crates/osd`) bekommt dafür ein eigenes Proto `onesimus-proto-stored` (ofd-Gutachten §5, Punkt 5).
- **Zu bareos-dir und bareos-fd:** BNET wie bareos-sd, als Rückfallweg, so wie ofd ihn mit `--disable-grpc` anbietet. Damit bleibt osd ein Ersatz für bareos-sd in einer reinen Bareos-Umgebung.
- **Onesimus-Erweiterungen** wie Zweige (Branch), Medien-Ereignisse und Stripe-Angaben gibt es **nur über gRPC**. Über BNET verhält sich osd wie bareos-sd.
- **Ohne Striping:** Ein Volume, das ein Device ohne Striping schreibt, hat das Bareos-Volume-Format. `bls`, `bextract` und ein bareos-sd können es lesen. Ziel ist, dass ein Wechsel zurück auf bareos-sd möglich bleibt.
- **Mit Striping:** Das ist eine **Abweichung**. Ein Stripe-Satz ist nur für osd lesbar (D-4).

## Routen

### Wo entschieden wird

In Bareos wählt der Director den Storage. Pro Job ist das `Storage` in Job, Pool oder Schedule-`Run`. Der SD wählt in diesem Storage ein freies Device mit passendem MediaType. Das nächste Volume holt der SD per CatReq beim Director, nach Pool und MediaType.

Routen im SD heißt also: Der Verteiler wählt für den Job **ein Device unter seinen Zielen**. Mit dessen MediaType fragt er den Director nach einem Volume.

### Vorschlag D-1: Verteiler als Device-Ersatz

```
# bareos-sd.d/device/Verteiler.conf (osd)
Distributor {
  Name = Verteiler
  Media Type = Onesimus            # was der Director sieht (D-2)
  Route { Level = Full;        Device = USB-Wechsel }
  Route { Client = "CLOUD-fd"; Device = S3-Archiv }
  Route { Pool = Monatlich;    Devices = Band-1, Band-2; Stripe = yes }
  Route {                      Device = FileStorage }   # ohne Merkmal: alles Übrige
}
```

Im Director bleibt der Storage, wie er ist: `Storage PDC-sd`, `Device = Verteiler`, `MediaType = Onesimus`.

- Ohne Bedingung entspricht eine Route dem heutigen Verhalten.
- Ist das Ziel-Device nicht bereit (Platte nicht eingesteckt, Band fehlt), wartet osd wie bareos-sd mit einer Mount-Anforderung. Es weicht nicht aus, so ist es vorgegeben.

### D-2: MediaType bei mehreren Gerätearten

- **Problem:** Der Katalog speichert je Volume einen MediaType. Beim Restore wählt der Director danach Storage und Device. Hat der Verteiler Ziele mit verschiedenen MediaTypes (File, USB, S3, Tape), gibt es zwei Möglichkeiten.
- **Variante a (empfohlen):** Jedes Ziel-Device behält seinen eigenen MediaType (`File`, `USB`, `S3`, `LTO`). Der Director-Storage führt alle Devices des Verteilers, mit Bareos' `Device = a, b, c` und je eigenem MediaType. Volumes sind dann wie in Bareos eindeutig einem Gerätetyp zugeordnet. Pools trennen sich nach MediaType, wie Bareos es verlangt.
- **Variante b:** Ein gemeinsamer MediaType `Onesimus` für alle Ziele. osd merkt sich, welches Volume auf welchem Device liegt. Das ist einfacher im Director, aber ein bareos-sd könnte keines dieser Volumes zuordnen.

## Aufteilen (Striping)

- **Schreiben:** Der Verteiler öffnet für den Job N Volumes auf N Devices. Die Bareos-Blöcke gehen reihum an die Devices, jeder Block trägt seine laufende Nummer. Jedes Stripe-Volume bekommt im Label die Kennung des Stripe-Satzes, seine Stellung (i von N) und die JobId.
- **Katalog:** Je Stripe-Volume entsteht ein JobMedia-Satz, mit denselben VolSessionId/-Time und FileIndex-Bereichen. Der Bootstrap enthält alle N Volumes.
- **Lesen:** osd erkennt den Stripe-Satz am Label, öffnet alle N Volumes **gleichzeitig** und setzt die Blöcke nach Nummer wieder zusammen.
- **D-4 (Abweichung):** bareos-sd würde die Volumes nacheinander lesen und Unsinn liefern. Stripe-Volumes bekommen deshalb einen eigenen MediaType (z. B. `LTO-Stripe`), den nur osd bedient. Ein Restore solcher Jobs geht nur mit osd.
- **D-5:** Stripe nur innerhalb gleicher Gerätearten (z. B. 2× Band, 2× Datei auf verschiedenen Platten) oder auch gemischt? Empfehlung: nur gleiche Art. Das langsamste Device bestimmt den Takt.
- **Fehler:** Fällt ein Device im Stripe-Satz aus, scheitert der Job. Teil-Sätze sind wertlos.
- **D-18 (Bedingung):** Striping vervielfacht das Ausfallrisiko: reißt ein Band oder fällt eine Platte eines Stripe-Satzes aus, fehlt jeder N-te Block und der ganze Job ist verloren. Ein Verteiler mit Striping ist deshalb nur gültig, wenn der Job einen Zweig (Branch, D-9) auf ein anderes Medium hat oder (später) Parität; `osd -t` und odir `-t` lehnen Striping ohne eins von beiden ab.

## Zweige (Branch): 3-2-1 im laufenden Betrieb

Vorgabe (Maintainer, 20.11.2026): „Somit lässt sich die 3-2-1-Regel on the fly umsetzen, indem ein I und AI parallel laufen.“

**Idee:** Der FD liest die Daten **einmal**, osd gibt den Stream an **mehrere Zweige**. Jeder Zweig ist ein **eigener Job** mit eigener JobId, eigenem Level, Pool, eigenen Fristen und Volumes. Es ist also kein Spiegeln von Blöcken, sondern ein Abzweig auf Job-Ebene.

```
                         ┌─ Zweig 1: Incremental        → FileStorage (lokal)      Pool Inc
FD ─ ein Stream ─ osd ───┤
                         └─ Zweig 2: Always Incremental → USB-Wechsel / S3 (außer Haus)  Pool AI
```

So ergibt sich 3-2-1 ohne Copy-Job und ohne zweites Lesen des Clients: 3 ist das Original plus zwei Zweige, 2 sind zwei Medienarten, 1 ist ein Zweig außer Haus.

### Was sich gegenüber Bareos ändert

| Ebene | Bareos | mit Zweigen |
| --- | --- | --- |
| Director | ein Job = eine FD-Sitzung = ein SD-Job | **ein Haupt-Job** (liest beim FD) und **Zweig-Jobs** (nur Katalog und SD), die sich die FD-Sitzung teilen. Nur odir kann das (Onesimus-Erweiterung) |
| Job-Ressource | — | neue Direktive, z. B. `Branch = <Job-Name>` (mehrfach). Der Zweig-Job ist ein normaler Job mit eigenem Pool/Storage/Level-Schema; er liest nicht selbst beim FD, sondern hängt sich an den Haupt-Job |
| SD | ein Job schreibt einen Datenstrom | osd verteilt jeden Record an alle Zweige; jeder Zweig hat sein eigenes Ziel (Route) und eigene Volumes und JobMedia-Sätze |
| Katalog | Files je Job | **jeder Zweig-Job bekommt seine eigenen File-Sätze**. Der Haupt-Job liefert die Attribute einmal, odir schreibt sie für jeden Zweig. Restore, Prune und Purge bleiben je Job unabhängig |
| Level | ein Level je Job | Ein Zweig hat **sein eigenes Level-Schema**. Damit ein gemeinsamer Stream reicht, muss das Haupt-Level die Zweige abdecken. Ein Full im Haupt-Job ist Full für alle Zweige. Inc und AI können denselben Inc-Stream nutzen, wenn beide dieselbe Basis (`since`-Zeit) haben (D-10) |
| Always Incremental | Konsolidierung über VirtualFull aus Inc-Jobs eines Pools | unverändert im AI-Zweig: Seine Inc-Jobs werden wie in Bareos konsolidiert. Der Inc-Zweig wird nie konsolidiert |

### Grenzen

- **Basis:** Inc und AI teilen sich einen Stream nur, wenn ihre Basis gleich ist, also die letzte Sicherung, gegen die verglichen wird. Weichen sie ab, etwa weil ein Zweig einen Lauf verpasst hat, muss der Haupt-Job die **ältere** Basis lesen, damit beide Zweige alles bekommen. Der Zweig mit der jüngeren Basis speichert dann etwas mehr als nötig. Das ist harmlos.
- **Fehler:** Scheitert ein Zweig, zum Beispiel weil das Volume voll ist oder die USB-Platte fehlt, darf das den anderen Zweig nicht abbrechen (D-11). Der gescheiterte Zweig-Job endet mit Fehler, sein Partner mit OK.
- **Nur über gRPC zwischen odir und osd.** Ein bareos-dir kennt keine Zweige.

## Datenpfad: Ringpuffer in `/dev/shm`

Vorgabe (Maintainer, 20.11.2026): „Die Aufteilung der Devices erfolgt über einen Ringpuffer in /dev/shm. Die Synchronisation der Puffer darüber ebenfalls.“

```
FD ─ Netz ─► Empfänger (schreibt) ─► /dev/shm/osd-<JobId>.ring ─┬─► Device-Schreiber 1 (liest)
                                     Kopf: Zeiger, Zustände      ├─► Device-Schreiber 2
                                                                └─► …
```

- **Ein Ring je Job.** Er ist eine Datei in `/dev/shm`, die alle Beteiligten mit `mmap` einbinden. Sie besteht aus einem **Kopf** und N gleich großen **Fächern** (Slots).
  - Ein Fach nimmt einen Bareos-Block auf (Blockgröße des Devices, Vorgabe 64 KiB), mit Folgenummer und Länge.
  - Größe und Anzahl der Fächer sind einstellbar (D-12).
- **Erzeuger:** die Netzsitzung, aktiv oder passiv, BNET oder gRPC. Sie schreibt die empfangenen Records in Blöcke und die Blöcke in freie Fächer.
- **Verbraucher:** je Ziel-Device ein Schreiber. Er liest Fächer und schreibt sie aufs Volume. Er kann ein eigener Prozess sein, wie die Job-Worker von odir. Das ist der Grund für gemeinsamen Speicher statt Kanälen im Prozess: Fällt ein Geräteprozess aus, bleiben Empfänger und die anderen Geräte stehen.
- **Verteilen über den Ring:**
  - **Route:** ein Verbraucher, er liest jedes Fach.
  - **Striping:** Fach `n` gehört dem Verbraucher `n mod N`. Jedes Fach wird genau einmal gelesen; die Folgenummer erlaubt das Zusammensetzen beim Lesen.
  - **Zweige:** Jeder Zweig-Verbraucher liest **jedes** Fach. Ein Fach wird erst frei, wenn alle Zweige es gelesen haben: Freigabe = kleinster Lesezeiger. Der langsamste Zweig bestimmt den Takt, solange der Ring voll ist (D-13).
- **Synchronisation über den Ring selbst:**
  - Im Kopf stehen der Schreibzeiger, je Verbraucher ein Lesezeiger und ein Zustand (läuft / fertig / Fehler / PID).
  - Alle Zeiger sind atomare 64-Bit-Zähler.
  - Gewartet wird auf voll oder leer mit **futex** auf Kopfwörtern. Das funktioniert prozessübergreifend im gemeinsamen Speicher, ohne zusätzlichen Kanal.
- **Fehler:**
  - Stirbt ein Verbraucher, sieht der Erzeuger das am Zustand bzw. an der PID. Der zugehörige Zweig wird als gescheitert markiert, sein Lesezeiger zählt nicht mehr für die Freigabe (D-11). Die anderen laufen weiter.
  - Stirbt der Erzeuger, beenden die Verbraucher den Job mit Fehler.
  - Am Jobende wird der Ring gelöscht. Beim Start räumt osd verwaiste Ringe weg.
- **Lesen (Restore)** nutzt denselben Ring in Gegenrichtung: Geräteleser füllen ihn, die Netzsitzung leert ihn. Beim Striping setzt die Netzsitzung die Blöcke nach Folgenummer zusammen.
- **Stufe 1 (ein File-Device)** baut den Ring schon mit genau einem Verbraucher, damit Routen, Striping und Zweige später nur Verbraucher hinzufügen.

## Platte als emuliertes Band (D-14, Maintainer 21.11.2026)

> Es macht Sinn, ein eigenes Filesystem für Backups zu etablieren, um auf die Platten zu schreiben. Dann haben wir die doppelte Organisation gespart. (20.11.2026)
>
> Für uns wäre das die Emulation eines Tape-Devices, also ein Treiber, der eine HDD wie ein Tape anspricht. Linear Tape File System sollte ebenfalls funktionieren. (21.11.2026)

- **Heute** ist die Organisation doppelt: Ein allgemeines Dateisystem verwaltet Dateien und Extents, und in den Volume-Dateien verwaltet das Bareos-Volume-Format noch einmal Blöcke, Labels und Sitzungen.
- **Beschluss (D-14):** osd spricht eine Platte oder Partition **wie ein Band** an, am Betriebssystem vorbei (Linux `O_DIRECT|O_EXCL`, die Gegenstücke auf macOS und Windows später). Statt eines Dateisystems mit Extents und Belegung gilt Band-Semantik: Kassetten, Partitionen, Sätze, Dateimarken, Datenende (EOD), Bandende-Vorwarnung, MAM. Die Platte ist ein **Wechsler mit Fächern**, jede Kassette ein fester Bereich mit Barcode.
- **Ebene:** der **Bandkern ist eine Rust-Schicht in osd**, ohne Kernelmodul. Frontends darauf:
  1. **Bareos:** osd schreibt bytegleich das Bareos-Bandformat (Volume-Label als erster Satz, Bareos-Blöcke als Sätze, Dateimarken). JobMedia adressiert Datei/Block wie bei einem echten Band. Eine Kassette lässt sich 1:1 auf ein echtes LTO kopieren.
  2. **LTFS:** ein Treiber-Plugin `libtape-osd.so` für die LTFS-Referenzimplementierung über deren Schnittstelle `tape_ops` (`src/libltfs/tape_ops.h`; mitgeliefert sind dort schon die Nicht-SCSI-Treiber `file` und `itdtimg`). `mkltfs`/`ltfs` arbeiten damit auf einer emulierten Kassette wie auf einem LTO-Band.
  3. **Optional später:** ein SCSI-Frontend (`/dev/nstX`, Wechsler über `/dev/sgX`) über LIO/TCMU oder mhVTL, damit auch bareos-sd, `mt`, `mtx` und `btape` die Platte direkt ansprechen. Nur Linux, root.
- **Folgen für das Modell** (aus `tape_ops`): zwei Partitionen je Kassette (`format`), je Partition eigenes EOD; Sätze variabler Länge (LTFS 512 KiB, Bareos 64 KiB); `locate`/`space`/`readpos` über ein Satzverzeichnis je Partition; ein kleiner MAM-Bereich je Kassette (`read_attribute`/`write_attribute`: Barcode, Volume-Kohärenz); `remaining_capacity`, `allow_overwrite`, `load`/`unload`. Log-/Mode-Pages, Verschlüsselung und WORM liefern Festwerte oder sind abgeschaltet.
- **Absturzsicherheit:** wie bei einem Band. Nach einem Absturz liest osd je Partition vom letzten bestätigten Zeiger bis zum Datenende vorwärts, solange crc32c und Satzfolge stimmen. Maßgeblich bleibt der Katalog (JobMedia). Recyceln heißt zurückspulen und überschreiben; eine Belegungs-Bitmap entfällt.
- **Schutz fremder Platten** (IBM-Warnung, Recherche): Magic-Label mit Kopien verlangen, Platten mit Partitionstabelle oder Dateisystem-Signatur ohne `--force` ablehnen, `O_EXCL`.
- **Striping (D-4/D-5) und Zweige:** über N emulierte Laufwerke wie bei echten Bändern; I- und AI-Zweig schreiben auf verschiedene Kassetten oder Platten.
- **Wechselplatten (D-6):** eine eingesteckte Platte ist ein eingelegtes Magazin; osd erkennt sie am Platten-Label (UUID), nicht am Gerätepfad.
- **Platten im SAN über iSCSI (D-20, beschlossen 21.11.2026):** „Gerade bei Platten ist es sinnvoll, SCSI über IP zu nehmen und diese in einem SAN zu betreiben.“
  - **osd als iSCSI-Initiator:** die Plattenbänder liegen auf LUNs eines SAN; osd sieht sie als Blockgeräte (Linux: open-iscsi, ggf. dm-multipath) und schreibt den Bandkern darauf wie auf eine lokale Platte. Ein LUN ist ein Magazin.
  - **Kennung:** statt `ID_SERIAL` die LUN-Kennung aus SCSI-VPD-Seite 0x83 (WWN/NAA) im Magazin-Label; osd erkennt einen LUN daran, auch über mehrere Pfade.
  - **Alleiniger Zugriff über das Netz:** `O_EXCL` wirkt nur auf dem eigenen Host. Mehrere Hosts sehen denselben LUN, deshalb **SCSI Persistent Reservations** (SPC-3, „Write Exclusive – Registrants Only“) beim Laden des Magazins, dazu Eigentümer (osd-Instanz) und Lease-Zeit im Magazin-Label. Ein zweiter osd übernimmt nur nach Ablauf der Lease bzw. mit `--force` (Übernahme bei Ausfall, wie bei einer geteilten Bandbibliothek).
  - **Pfadausfall:** iSCSI-Zeitlimits (`replacement_timeout`) und multipath `no_path_retry` begrenzen; ein Schreibfehler nach Pfadverlust ist ein Gerätefehler des Laufwerks, kein Datenfehler. Bestätigt ist nur, was nach fsync im EOD-Zeiger steht; Wiederherstellung wie nach einem Absturz.
  - **Auslagern ohne Abziehen:** iSCSI-Logout bzw. Wegnehmen des LUN-Mappings im SAN = Magazin aus der Bibliothek genommen (Air-Gap gegen Ransomware, ohne Platten zu tragen); osd meldet es wie das Abziehen einer Wechselplatte (D-6/D-7).
  - **osd als iSCSI-Target (später, optional):** das emulierte Band und den Wechsler selbst als SCSI-Geräte (SSC/SMC) über iSCSI anbieten, damit andere Hosts (bareos-sd, LTFS mit dem `sg`-Treiber, fremde Backup-Software) sie als Bandbibliothek sehen. Ersetzt bzw. ergänzt das lokale SCSI-Frontend (Punkt 3 oben). Wege: LIO mit TCMU oder `tgt`, das Band- und Wechsler-Emulation mitbringen soll **[unbelegt]**.
  - **Rechte und Plattformen:** Initiator unter Linux mit open-iscsi; Windows und macOS haben eigene Initiatoren (macOS nur über Drittanbieter **[unbelegt]**). Persistent Reservations brauchen root bzw. `CAP_SYS_RAWIO` (SG_IO).
- **Labels (D-16)** auf drei Ebenen, wie Barcode-Aufkleber und Cartridge Memory bei LTO:
  1. **Magazin-Label, elektronisch** (Wechsler-Label an LBA 0, mit Kopien): Magazin-Barcode der Platte, Platten-UUID, **Seriennummer des Laufwerks** (lokal: udev `ID_SERIAL`, sonst ioctl; im SAN: LUN-Kennung aus VPD 0x83), Kassettenzahl und -größe, Generation. osd erkennt eine Platte an UUID und Barcode; eine unbekannte Platte oder eine abweichende Seriennummer (geklont, getauscht) wird gemeldet und nicht eingehängt.
  2. **Kassetten-Barcode** je Fach, in der Fachtabelle und im MAM der Kassette. Aufbau `<Präfix><laufende Nummer><Medienkennung>`, Medienkennung `D1` (Plattenband, Format 1; analog `L8` bei LTO-8). **Das Präfix ist frei wählbar** (je Magazin beim Einrichten); Vorgabe `OSM` für Magazine und `OS` für Kassetten, also z. B. `OSM001` und `OS0001D1`. Empfohlen sind 6 + 2 Zeichen wie bei LTO, damit eine Kassette mit gleichem Namen auf ein echtes LTO kopiert werden kann; längere Namen sind erlaubt. osd lehnt einen Barcode ab, den ein bekanntes Magazin schon trägt. Der emulierte Wechsler meldet die Barcodes an Bareos, `update slots`, `label barcodes` und `status slots` funktionieren unverändert, und **der Bareos-Volume-Name ist der Kassetten-Barcode**. Im Katalog verweist `Media.Location` auf den Magazin-Barcode: Volume → Fach → Platte → Seriennummer.
  3. **Physischer Aufkleber:** beim Einrichten erzeugt osd ein Etikett als PDF: Magazin-Barcode als Code 39 (wie LTO), QR-Code mit der UUID, Seriennummer, Kassetten-Barcodes mit Fach. Direktes Drucken auf einem Etikettendrucker ist offen; das PDF reicht für den Anfang. Für LUNs im SAN entfällt der Aufkleber; dort steht der Magazin-Barcode zusätzlich im LUN-Namen bzw. Alias des SAN.
- **Einrichten:**
  ```
  osd disk init device=/dev/sdX barcode=<Magazin-Barcode> prefix=<Kassetten-Präfix> cassettes=<N> [size=…] [--force]
     → lehnt Platten mit Partitionstabelle oder Dateisystem-Signatur ohne --force ab,
       schreibt das Magazin-Label, legt die Fächer an, erzeugt das Etikett (PDF)
  update slots storage=… ; label barcodes storage=… pool=…
  ```
- **Weiterlesen hinter Schadstellen (D-17):** Jeder Satz trägt Kopf, Satznummer und crc32c. Bei einem Lesefehler (Sektorfehler) springt der Bandkern zum nächsten gültigen Satz und liest dahinter weiter; verloren sind nur die betroffenen Sätze, nicht der Rest der Kassette. Die Lücke wird als Fehler mit Satzbereich gemeldet (Job-Log, Medien-Ereignis D-7). Das kann ein echtes LTO nach einem Bandriss nicht.
- **Ausfall einer ganzen Platte bzw. eines LUN** entspricht einem Bandriss: nur ein zweiter Zweig oder eine Kopie rettet (siehe [Beschädigte Volumes](#beschädigte-volumes-bandriss-plattenschaden)). Im SAN gehört der zweite Zweig auf ein anderes Speichersystem bzw. einen anderen Standort.
- **Status:** beschlossen, **eigene Phase nach P1** (PT, siehe [Phasen](#phasen)). Entwurf und Belege: [OSD-RECHERCHE-SPEICHER-UND-RING.md](OSD-RECHERCHE-SPEICHER-UND-RING.md), Nachtrag B. Die frühere osdfs-Skizze (Nachtrag A) ist damit ersetzt; Label mit Kopien, crc32c und das Vorwärtslesen nach Absturz bleiben übernommen.

## Beschädigte Volumes (Bandriss, Plattenschaden)

Gilt für echte Bänder, Plattenbänder und Datei-Volumes.

- **Bareos heute:** Ein Volume mit Lesefehler wird `VolStatus = Error`. Verloren sind alle Jobs mit JobMedia auf diesem Volume, bei Jobs über mehrere Volumes der ganze Job. Ein Copy-Job wird zum Backup hochgestuft, wenn der Original-Job gelöscht oder purged wird **[unbelegt, aus der Bareos-Doku aus dem Gedächtnis; vor der Umsetzung prüfen]**. Ohne Kopie bleiben `bls`/`bextract` bis zur Schadstelle und bei LTO eine Labor-Spleißung. Nach einem Bandriss das Laufwerk reinigen und prüfen.
- **Vorsorge:** 3-2-1 über Zweige (I und AI parallel auf verschiedene Kassetten bzw. Platten oder SAN-Systeme); kleinere Volumes (`Maximum Volume Jobs`/`Maximum Volume Bytes`) begrenzen den Schaden, als Empfehlung in die Doku; Striping nur mit Zweig oder Parität (D-18).
- **Ablauf „Volume beschädigt“ in odir (D-19)**, als Befehl (Name offen, z. B. `volume damaged volume=<v> [dryrun|yes]`), typisiert über gRPC und als GUI-Dialog:
  1. Volume auf `Error` setzen;
  2. betroffene Jobs aus JobMedia auflisten (bei Jobs über mehrere Volumes alle);
  3. je Job zeigen, ob es eine Kopie oder einen Zweig gibt;
  4. Kopien hochstufen (Original-Job purgen, damit die Kopie zum Backup wird);
  5. für Clients ohne Kopie einen neuen Full vorschlagen bzw. mit `yes` starten;
  6. bei Plattenbändern Magazin-Barcode und Seriennummer bzw. LUN nennen (welche Platte zu ziehen ist).
  Mit `dryrun` nur die Übersicht. Unabhängig von osd umsetzbar; Backlog-Punkt V-1.

## Gerätearten

| Art | Bareos-Gegenstück | osd-Umfang |
| --- | --- | --- |
| Datei/Verzeichnis | `DeviceType = File` | wie bareos-sd, Volume = Datei |
| Wechselplatte (USB) | `File` + `RemovableMedia = yes`, Mount-Befehle | dazu ein **Device-Watcher** (D-6): erkennt das Einstecken (udev oder Dateisystem-Label), hängt ein, prüft und liest das Volume-Label, meldet „bereit“ und löst wartende Mount-Anforderungen aus. Beim Abziehen gibt er das Device sauber frei |
| Objektspeicher (S3) | `DeviceType = droplet`/`dplcompat` (Chunks) | Volume in Stücken als Objekte, Chunk-Größe einstellbar, Zugangsdaten aus Datei |
| Band | `DeviceType = Tape`, Autochanger mit `mtx` | Bareos-Bandformat (Blockgröße, EOF-Marken), Autochanger-Befehle. Größter Aufwand, zuletzt |
| Plattenband (D-14) | `DeviceType = Tape` + Autochanger, aber auf Platte | Platte oder Partition als emulierter Wechsler mit Kassetten (Bandkern in osd); Bareos-Bandformat und LTFS-Plugin, siehe [Platte als emuliertes Band](#platte-als-emuliertes-band-d-14-maintainer-21112026) |

## Device-Watcher (D-6)

- **Auftrag:** Die Wechselplatten stehen auf dem SD-Rechner, also erkennt **osd selbst** sie. Ein eigener Dienst wäre ein zweiter Prozess mit eigener Fehlerquelle.
- **Gerätezuordnung:** Eine Wechselplatte gehört einem Device, wenn ihr Dateisystem-Label oder ihre UUID zu `Removable Label = …` am Device passt.
- **Rückmeldung an den Director:**
  - wie bareos-sd über `status storage` und die Mount-Anforderung in den Job-Meldungen;
  - optional als gRPC-Push-Event „Medium eingelegt/entfernt“ an odir für die GUI. Das braucht ein Proto-Feld in onesimus-proto (D-7).

## Konfiguration

- osd liest `bareos-sd.d` (Storage, Director, Device, Autochanger, Messages) mit `bareos-config`.
- Neue Ressourcen: `Distributor` mit `Route`-Blöcken, und am Device `Removable Label` sowie die S3-Angaben.
- **D-8 (entschieden: additiv, rückwärtskompatibel, Namensgleichheit):** osd liest `bareos-sd.d` unverändert, die neuen Ressourcen und Direktiven kommen zusätzlich aus `osd.d/`, das dieselbe Struktur hat; eine Ergänzung gilt der gleichnamigen Ressource gleichen Typs in `bareos-sd.d`. Ohne `osd.d/` verhält sich osd wie bareos-sd; ein bareos-sd läuft mit derselben `bareos-sd.d` weiter (Bareos würde `Distributor` dort als unbekannte Ressource ablehnen).

## Phasen

| # | Inhalt | Prüfung |
| --- | --- | --- |
| P0 ✅ (20.11.2026) | `bareos-format` und `onesimus-daemon` als gemeinsame Crates (ofd-Gutachten §5), osd-Schnittstelle (`crates/osd`) und Daemon (`crates/osd/daemon`) trennen | ofd und odir bauen unverändert. Erledigt, siehe [P0 umgesetzt](#p0-umgesetzt-20112026) |
| P1 ✅ (20.11.2026; Stufen 1, 2, 3) | osd als Ersatz für bareos-sd mit **einem File-Device**: Label, Mount, Append, Read, Status, CatReq, Bareos-Volume-Format. Steuer- und Datenweg über **gRPC** (odir, ofd) und BNET (bareos-dir, bareos-fd). Stufe 1: Backup über BNET, aktiver und passiver Client, Datenweg über den Ring; Stufe 2: Lesen/Restore; Stufe 3: gRPC | Testgerüst Director+FD+SD (`onesimus-rig`): Backup und Restore mit odir+ofd über gRPC und mit bareos-fd über BNET. `bls` liest das Volume. Erledigt, siehe [Stufe 1](#p1-stufe-1-umgesetzt-20112026), [Stufe 2](#p1-stufe-2-umgesetzt-20112026), [Stufe 3](#p1-stufe-3-umgesetzt-20112026); was bleibt: [Nach P1](#nach-p1-offen) |
| PT | Plattenband v1 (D-14): Bandkern auf einer vorbelegten Datei bzw. einem Loop-Device, dann `/dev/sdX` mit `O_DIRECT\|O_EXCL` (zunächst Linux); eine Platte, kein Striping; Kassetten mit zwei Partitionen, MAM, Wechsler; Labels (D-16) mit `osd disk init` und Etikett-PDF; Weiterlesen hinter Schadstellen (D-17); Bareos-Frontend und LTFS-Plugin `libtape-osd.so`. Danach PT2: LUNs im SAN über iSCSI mit Persistent Reservations (D-20) | ohne root testbar; Export einer Kassette in eine Bandabbild- bzw. Volume-Datei besteht `bls`; `mkltfs` + `ltfs` + Dateien schreiben/lesen auf einer emulierten Kassette; kill -9 während des Schreibens verliert keinen bestätigten Satz; `update slots`/`label barcodes` liefern die Kassetten-Barcodes; fremde Platte ohne `--force` abgelehnt; eingeschleuste Schadstelle: Lesen setzt dahinter fort und meldet den Satzbereich. PT2: iSCSI-Target im Rig (LIO mit `fileio`-Backstore, eigener Netz-Namespace), zwei osd-Instanzen auf demselben LUN, die zweite wird per Reservation abgewiesen; Pfadabbruch während des Schreibens. Teilt das Bareos-Bandformat mit P6 |
| P2 | Verteiler mit Routen über File-Devices | Routing-Fälle im Rig, Restore über den MediaType |
| P2b | Zweige (Branch): odir startet Haupt- und Zweig-Jobs, osd verteilt an beide, Katalogsätze je Zweig | Inc auf File, AI auf zweites File-Device; Restore aus jedem Zweig einzeln; ein gescheiterter Zweig bricht den anderen nicht ab; Konsolidierung des AI-Zweigs |
| P3 | Wechselplatten und Device-Watcher | Rig mit Loop-Device, Ein- und Abziehen simuliert |
| P4 | S3 | gegen einen lokalen S3 (z. B. MinIO im eigenen Podman-root, siehe Podman-Isolation) |
| P5 | Striping | Backup über 2–3 File-Devices, Restore nur mit osd, bareos-sd lehnt den MediaType ab |
| P6 | Band und Autochanger | `mhvtl` (virtuelles Band) im Rig |

## P0 umgesetzt (20.11.2026)

Commits auf `development` (lokal, nicht gepusht), in dieser Reihenfolge:

| Commit | Inhalt |
| --- | --- |
| `dac4237` | `bareos-format`: breg (aus ofd und odir zusammengeführt), Netz-Köpfe der Datensätze (`data_header`, `RecHdr`), `digest_name`; ofd und odir nutzen sie. Macht ofd und odir wieder baubar, nachdem `9ff7efc` die alten breg-Dateien versehentlich mitgelöscht hatte |
| `15b069a` | neun Inline-Kopien von BashSpaces auf `bareos_format::bash`; D13 am Bareos-Quelltext geprüft |
| `38339c7` | `onesimus-daemon`: Accept-Schleife, TLS-PSK-Wahl, Director-Handshake, gRPC-TLS, Bandbreitengrenze; ofd nutzt sie |
| `cec505f` | osd: ein `connect_sd` (D12), echtes FileSet im SD-Job-Kommando (D11), `use storage` mit BashSpaces, Modulkommentar richtiggestellt |
| `5e3366c` | `bareos-config`: SD-Ressourcen (Storage, Director, Device, Messages, Autochanger) |
| `be26325` | `crates/osd/daemon` (Paket `osd-daemon`, Programm `osd`): `-c -f -d -t -v`, `-t` prüft `bareos-sd.d`, sonst „osd: P1 pending“ (Exit 2); Paketierung `packaging/osd/` |
| `793be93` | CI testet und dokumentiert osd und osd-daemon, README |

Prüfung: `cargo test --workspace --exclude onesimus-rig` grün, 1255 Tests bestanden, 0 fehlgeschlagen, 34 ignoriert. `cargo doc --workspace --no-deps` ohne Fehler und ohne Namenskollision `osd` (M9).

Schon am 19.11.2026 vorhanden (`b912a92`, `771a98d`): `bareos-format` mit LStat, Base64, Stream-IDs, `M_*`, BashSpaces und dem einen SD-Job-Formatierer; `onesimus-daemon` mit Logging, Signalen, PID-Datei, Hintergrund, Rechteabgabe und `-t`.

Nach P1 verschoben, jeweils mit Grund:

- **Hello mit `0x01` statt `'_'` (D13):** In P0 ändert sich kein Verhalten. Die Umstellung trifft jede Director-Verbindung zu FD und SD und braucht einen Lauf gegen bareos-sd/-fd im Gerüst.
- **Teile von ofd, die zu eng verflochten sind:** Jobtabelle und passiver SD-Anschluss (`sd.rs`), der gRPC-Dienst mit Laufzeit (`grpc.rs` `start`), das Binden des Ports mit den FD-Fehlertexten. Dazu Signale, PID-Datei und `-k` in odir sowie dessen gRPC-TLS mit mDNS-Namen. Die Liste steht in der Crate-Doku von `onesimus-daemon`.
- **Verbindungsaufbau „Client zum SD“ (`R_JOB`)** in der osd-Schnittstelle: Den braucht erst ein osd, der Daten annimmt.
- **Storage-Name in `use storage`** aus `osd::job`: Die Job-Parameter tragen keinen Storage-Namen, es bleibt `File`. In `sd_copy` und `verify` ist der Name richtig.
- **Unbekannte Direktiven in `bareos-sd.d`** werden noch nicht abgelehnt. bareos-sd lehnt sie ab.
- **ofd nach `crates/ofd/daemon` und die Plugins verschieben** gehört nicht zu P0 (Vorgabe).
- **Client-Connects zum FD** (sechs Varianten in odir) gehören zur ofd-Schnittstelle (Gutachten §6 Schritt 4), nicht zu P0.

## P1 Stufe 1 umgesetzt (20.11.2026)

osd sichert über BNET auf ein File-Device und schreibt das Bareos-Volume-Format; der Datenweg läuft schon über den Ring mit genau einem Geräteschreiber als eigenem Prozess (D-12, D-13). Anleitung, Stand und alle Abweichungen von bareos-sd: [`docs/wiki/osd.md`](../wiki/osd.md).

Commits auf `development` (lokal, nicht gepusht), in dieser Reihenfolge:

| Commit | Inhalt |
| --- | --- |
| `41cc205` | `bareos-format`: Blockkopf `BB02` mit CRC-32, Datensätze wie `WriteRecordToBlock` (Fortsetzungsköpfe), Volume- und Sitzungslabels, Leser `read_volume`/`summarize`; Label-Block und letzter Block eines bareos-sd-Volumes werden byte-genau nachgebaut |
| `223d9fb`, `e497815` | `osd-ring` (`crates/osd/ring`): Ring je Job, ein Erzeuger, N Verbraucher, Freigabe = kleinster Lesezeiger, Postfach, Info-Bereich; `memmap2`, futex mit 100 ms Zeitgrenze, Cache-Zeilen, seitenweise Fächer, Lebendfrist (Recherche B, D-15); macOS/Windows übersetzbare Platzhalter |
| `783ef76` | `onesimus-rig`: SD wählbar (`RigOptions::sd`), Client mit `Passive = yes`, beliebige Pakete bauen |
| `f7dcabb` | `bareos-config`: `osd.d/` als Erweiterung von `bareos-sd.d` nach D-8 (Namensgleichheit, nur Onesimus-Direktiven, Waisen sind Fehler); `Maximum Block Size`, `Label Block Size`, `Maximum Volume Size`, `Just In Time Reservation`, `Client Connect Wait` |
| `cff310a` | `osd::askdir`: CatReq, UpdCat, Jmsg und Status des SD wie `stored/askdir.cc` |
| `21104b9` | `onesimus-daemon::version`: Versionszeile mit Baudatum und Betriebssystem |
| `cb0b31b` | `bareos-format`: Rest eines Datensatzes am Volume-Ende (Job über mehrere Volumes) |
| `036c15c` | `osd-daemon`: Director-Befehle, aktiver und passiver FD, Datenphase über `transport::Channel`, Erzeuger und Geräteschreiber (`osd --device-writer`), Volume-Wahl, Label, Volume-Wechsel, EOS; Gerüst-Tests `tests/osd.rs` |
| `e964f77` | Glossar und Klassendiagramm von `osd-ring`, CI, README |
| dieser Commit | Wiki-Seite `osd.md`, Testgerüst, dieser Abschnitt |

Belege (Gerüst, 20.11.2026, odir + ofd `--disable-grpc` + osd; `bareos-fd` ist auf dem Rechner nicht installiert):

- Aktiver und passiver Client: „Termination: Backup OK“, 13 Dateien, Katalog `Media 1|VVol-0001|VFile|Append|1|0|14|12592459`, `JobMedia FirstIndex=1 LastIndex=13 StartBlock=226 EndBlock=12592458`, 13 File-Sätze; `bls` listet 13 Dateien und Verzeichnisse.
- Die Katalog-Anfragen kommen in derselben Reihenfolge wie bei bareos-sd 25.1.3 (FindMedia, GetVolInfo, UpdateMedia relabel=1, GetVolInfo, UpdateMedia VolMounts, UpdateMedia VolJobs, CreateJobMedia 0, CreateJobMedia, UpdateMedia, DeleteNullJobmediaRecords).
- Volume von osd gegen Volume von bareos-sd aus demselben Gerüst: gleiche Blockzahl (14), gleiche Blocklängen und -nummern, gleiche Datensatzköpfe; die Nutzdaten der Blöcke 2–12 sind byte-gleich, Block 0 (Label: Version, Zeit), 1 und 13 (SOS/EOS-Zeit, Attribute mit anderen Zeiten) unterscheiden sich nur in diesen Feldern.
- Passiver Client: odir schickt `passive client address=127.0.0.1 port=… ssl=4` (war schon vorhanden), osd ruft ofd an.

Prüfung: `cargo test --workspace --exclude onesimus-rig` grün, 1298 Tests bestanden, 0 fehlgeschlagen, 35 ignoriert (davon neu: bareos-format 32, osd-ring 10, osd 13, osd-daemon 13 + 5, bareos-config 61, onesimus-daemon 21). `cargo test -p onesimus-rig --test osd -- --ignored`: 2 von 2 mit osd, ebenso 2 von 2 mit `ONESIMUS_RIG_SD=/usr/sbin/bareos-sd`. `cargo doc --no-deps --document-private-items` der neuen Crates ohne Warnung.

Stufe 2 (Lesen) ist erledigt, siehe [P1 Stufe 2 umgesetzt](#p1-stufe-2-umgesetzt-20112026).

## P1 Stufe 2 umgesetzt (20.11.2026)

osd liest wie bareos-sd für Restore und Verify (VolumeToCatalog) über
BNET, mit aktivem und passivem Client, auch über mehrere Volumes. Der
Lesepfad nutzt den Ring in Gegenrichtung (siehe
[Datenpfad](#datenpfad-ringpuffer-in-devshm)): Ein Geräteleser (`osd
--device-reader`, eigener Prozess) übernimmt die Erzeugerrolle und füllt
die Fächer mit den Blöcken der Volumes, die Netzsitzung leert sie, setzt
die Datensätze zusammen, filtert sie nach dem Bootstrap und schickt sie mit
`rechdr` an den FD. Anleitung, Stand und Abweichungen:
[`docs/wiki/osd.md`](../wiki/osd.md).

Commits auf `development` (lokal, nicht gepusht), in dieser Reihenfolge:

| Commit | Inhalt |
| --- | --- |
| `b98998a` | `bareos-format`: Bootstrap (`bsr`: `parse_bsr`, `CreateRestoreVolumeList`, Lesebereiche je Volume, `BsrMatcher` mit den Erledigt-Merkern von `stored/bsr.cc`), `volume::RecordAssembler` (Datensätze je Sitzung über Blöcke und Volumes) |
| `8d14b91` | `osd-ring`: Gegenrichtung – `take_producer`, Postfach vom Erzeuger zum Verbraucher (`ask_consumer`, `serve_producer`) |
| `4805160` | `osd-daemon`: `use storage … append=0`, `bootstrap`, `read open/data/close`, Geräteleser, Netzsitzung als Verbraucher, Label-Prüfung mit den bareos-sd-Meldungen, Lesen über mehrere Volumes, `UpdateMedia` beim Freigeben |
| `0a8241b` | `onesimus-rig`: `tree::compare_trees`, `Rig::replace_sd`; Gerüst-Tests für Restore (aktiv, passiv), drei Volumes, bareos-sd ↔ osd |
| dieser Commit | Wiki `osd.md`, Testgerüst, dieser Abschnitt |

Belege (Gerüst, 20.11.2026, odir + ofd `--disable-grpc` + osd; `bareos-fd` ist nicht installiert):

- `tests/rig.rs` mit `ONESIMUS_RIG_SD=$PWD/target/debug/osd`: 5 von 5 (Backup, Estimate, Verify VolumeToCatalog „Verify OK“, Restore „Restore OK“ mit `diff -r` ohne Unterschied, Abbruch; D1, D14, D15, D16 bestehen wie erwartet). Gegenprobe mit bareos-sd: 5 von 5 (D16 im zweiten Lauf; im ersten war Port 39432 von einem kurzlebigen Client-Socket eines parallelen Gerüsts belegt).
- `tests/osd.rs` mit osd: 4 von 4. Restore aktiv und passiv: „Restore OK“, 13 Dateien, `diff -r` rc=0, Rechte, Eigentümer, mtime und Symlink-Ziele gleich, Hardlinks und Löcher der Sparse-Datei erhalten. Beim passiven Client meldet der FD keine eigene SD-Verbindung, osd hat ihn angerufen.
- Drei Volumes (`Maximum Volume Bytes = 5M`): `VVol-0001` und `VVol-0002` Full mit je 4 194 530 Bytes, `VVol-0003` Append; JobMedia `1–5`, `5–5`, `5–13`; die ersten beiden Volumes enden mitten in einem Datensatz (3 505 bzw. 5 409 Bytes fehlen dort). Verify und Restore melden „Ready to read from volume“ für alle drei, „End of all volumes.“, „Verify OK“ bzw. „Restore OK“.
- bareos-sd ↔ osd auf einem Volume: Backup mit bareos-sd → Restore mit osd OK; osd hängt ein zweites Backup an dasselbe Volume → Restore mit bareos-sd OK; `bls` meldet „26 files and directories found.“. Die Job-Meldungen von osd beim Lesen gleichen denen von bareos-sd 25.1.3 Zeile für Zeile („Ready to read from volume …“, „Forward spacing Volume … to file:block 0:…“, „End of Volume at file 0 on device …“, „End of all volumes.“, „Releasing device …“); die Katalog-Anfragen auch (`GetVolInfo … write=0` je Volume, `UpdateMedia` am Ende).
- `tests/osd.rs` mit `ONESIMUS_RIG_SD=/usr/sbin/bareos-sd`: 4 von 4 (Gegenprobe; bei bareos-sd sind die Volumes 4 194 534 Bytes groß, das Label ist 4 Bytes länger).

Prüfung: `cargo test --workspace --exclude onesimus-rig` grün, 1357 Tests bestanden, 0 fehlgeschlagen, 35 ignoriert; neu sind bareos-format 12 (Bootstrap, Zusammensetzen über Volumes, Sitzungen), osd-ring 1 (Gegenrichtung), osd-daemon 4 (Filter, Job über zwei Volumes, falsches Volume), onesimus-rig 1 (`compare_trees`) und 2 neue Gerüst-Tests (aus den beiden Backup-Tests wurden Backup-und-Restore-Tests).

Stufe 3 (gRPC) ist erledigt, siehe [P1 Stufe 3 umgesetzt](#p1-stufe-3-umgesetzt-20112026); was aus den Stufen 1–3 offen bleibt, steht in [Nach P1](#nach-p1-offen).

## P1 Stufe 3 umgesetzt (20.11.2026)

gRPC mit TLS ist der Hauptweg zwischen odir/ofd und osd, BNET der
Rückfall, genau wie odir ↔ ofd (Grundsatz). Anleitung:
[`docs/wiki/osd.md`, Transporte](../wiki/osd.md#transporte-grpc-und-bnet).

- **Proto** (onesimus-proto, nicht gepusht): Zweig `stored` von `origin/main`
  (`aad8995`; das lokale `main` stand einen Commit dahinter), Commit
  `52e108b`: `stored_service.proto` mit `service StorageDaemon { rpc
  Control(stream StoredFrame) returns (stream StoredFrame); rpc Data(…) }`;
  `StoredFrame` hat die Felder von `filed.ControlFrame` (`bytes payload = 1`,
  `int32 signal = 2`), eine BNET-Nachricht je Rahmen. `FileDaemon` bekommt
  `Data` für die Verbindung eines osd zum passiven Client (FileDaemon hat
  sonst nur `Control`, also keine weiteren RPCs zu spiegeln). Zusammengeführt
  in `development` als `c23483b` (`--no-ff`).
- **Transport:** neues Crate `onesimus-bnet-grpc` statt einer Kopie von
  `fd_grpc`: `ControlIo` (Strom als `Read`/`Write` von BNET-Rahmen),
  Client für die vier Ströme, Serverseite für ofd und osd, `Target` mit den
  gRPC-Angaben als Befehls-Token. `bareos-bnet` ist dadurch frei von tonic,
  tokio und Protos (Gutachten §5 Punkt 1, Amendment §6.B). Ein neutrales
  Crate statt der ofd-Schnittstelle, weil FD- und SD-Wege es teilen.
- **Anmeldung** wie odir ↔ ofd: TLS mit dem beim ersten Start erzeugten
  selbstsignierten Zertifikat des Daemons (`onesimus-sd-grpc.crt`, stabil
  über Neustarts), im Director festgehalten mit `Grpc CA Certificate File`
  und geprüft mit `Grpc Server Name`; danach Hello und CRAM-MD5 im Strom
  (Storage-Passwort bzw. Job-Schlüssel).
- **osd:** gRPC-Port neben dem SD-Port (`osd.d` `Grpc Enable`/`Grpc Port`/
  `Grpc Address`, Vorgabe `SD Port` + 1000; `--disable-grpc`,
  `--grpc-listen`, `--grpc-cert`/`--grpc-key` wie ofd). Ein Strom ist die
  zweite Umsetzung von `transport::Channel`; Steuer- und Datenweg, Schreiben
  und Lesen, aktiv und passiv laufen durch denselben Code.
- **odir:** Storage mit `Grpc Enable = yes` (dieselben Direktiven wie am
  Client) wird zuerst über `StorageDaemon/Control` angesprochen, in
  Job-Pfaden mit dem vor dem `fork` festgehaltenen Ziel, in Befehlswegen
  über einen gRPC-Wähler in `osd::connect`. Jeder Fehler wird protokolliert
  und fällt auf BNET mit den bisherigen SD-Fehlertexten zurück; ohne
  `Grpc Enable` kein Versuch. Job-Kommando, CatReq-Weiterleitung und
  `status storage` laufen unverändert im Strom.
- **ofd/osd Datenweg:** Laufen beide Steuerverbindungen über gRPC, gibt
  odir die gRPC-Angaben der Gegenseite im Befehl mit (`storage address=…`
  an ofd, `passive client …` an osd); ofd ruft `StorageDaemon/Data`, osd
  ruft `FileDaemon/Data`, jeweils mit BNET-Rückfall.
- **Nebenbefund, behoben:** Job-Worker von odir schließen nach `fork()` die
  geerbten Deskriptoren, darunter tokios Signal-Pipe; jeder gRPC-Aufbau im
  Worker scheiterte deshalb („Bad file descriptor“), auch zu ofd, und fiel
  still auf BNET zurück. Ebenso drehte sich ein abgebrochener gRPC-Strom
  endlos in `read_exact` (Fehlerart `Interrupted`).

Commits auf `development` (lokal, nicht gepusht), in dieser Reihenfolge:

| Commit | Inhalt |
| --- | --- |
| `5c33957` | Submodul `proto` auf `c23483b` |
| `9649ac7` | `bareos-config`: `osd.d` `Grpc Enable`, `Grpc Port`, `Grpc Address` |
| `a891380` | `onesimus-bnet-grpc`; `onesimus-proto-rs` baut `stored_service.proto`; `bareos-bnet` ohne tonic; ofd: `FileDaemon/Data`, Client zu `StorageDaemon/Data` |
| `ccd9460` | `osd-daemon`: `StorageDaemon`-Dienst, passiver Client über gRPC, `--disable-grpc` |
| `1983776` | odir und `osd::connect`: Storage über gRPC zuerst, gRPC-Angaben für die Datenverbindung |
| `4d78e62` | odir: Worker behalten tokios Signal-Pipe |
| `44b6a38` | `onesimus-bnet-grpc`: Fehlerursache vollständig im Log |
| `98e7edc` | `onesimus-rig`: `RigOptions::fd_grpc`/`sd_grpc`, Interop-Matrix `tests/osd_grpc.rs`, `tests/osd.rs` auf gRPC |
| dieser Commit | Wiki, Testgerüst, dieser Abschnitt, Gutachten-Amendment |

Belege (Gerüst, 20.11.2026; `bareos-fd` ist nicht installiert):

- `tests/osd_grpc.rs` mit osd: 6 von 6, je Backup, Verify („Verify OK“) und Restore (`diff -r` rc=0). Transporte laut Log:
  - (a) aktiv: odir „SD 127.0.0.1:39516: connected over gRPC TLS“, osd „Director connects over gRPC TLS“, odir „FD 127.0.0.1:39515: connected over gRPC TLS“, ofd „Connected Storage daemon at 127.0.0.1:39516 over gRPC TLS“, osd „File daemon of job VBackup… connects over gRPC TLS“; Job-Log „Connected Storage daemon at 127.0.0.1:39513, encryption: gRPC TLS“.
  - (a) passiv: osd „VBackup…: connected to File daemon 127.0.0.1:39525 over gRPC TLS (passive client)“, ofd „Storage daemon connects over gRPC (passive client)“.
  - (b) osd `--disable-grpc`: odir „SD 127.0.0.1:39536: transport error: tcp connect error: Connection refused (os error 111); falling back to BNET on port 39533“, osd „Director connects over BNET“, ofd „Connected Storage daemon at 127.0.0.1:39533 over BNET“; passiv osd „connected to File daemon 127.0.0.1:39542 over BNET (passive client)“. Director ↔ FD bleibt gRPC.
  - (c) ofd `--disable-grpc`: odir „FD 127.0.0.1:39555: … Connection refused …; falling back to BNET on port 39552“, Director ↔ SD gRPC, FD ↔ SD BNET (aktiv und passiv).
- (d) `ONESIMUS_RIG_SD=/usr/sbin/bareos-sd`: `tests/osd_grpc.rs` 6 von 6 (odir „SD …: … Connection refused …; falling back to BNET“, ofd über BNET), `tests/osd.rs` 4 von 4, `tests/rig.rs` 5 von 5.
- `tests/osd.rs` mit osd (jetzt auf gRPC): 4 von 4; im gemischten Test laufen die bareos-sd-Abschnitte über TLS-PSK, der osd-Abschnitt über gRPC („Connected Storage daemon at 127.0.0.1:39506, encryption: gRPC TLS“ vom FD). `tests/rig.rs` mit `ONESIMUS_RIG_SD=$PWD/target/debug/osd`: 5 von 5.

Prüfung: `cargo test --workspace --exclude onesimus-rig` grün, 1364 Tests bestanden, 0 fehlgeschlagen, 35 ignoriert; neu sind onesimus-bnet-grpc 6, bareos-config 1 (`osd.d` gRPC), osd-daemon 1 (gRPC-Port, Pinning, `--disable-grpc`), odir 1 (`sd_grpc`); 6 neue Gerüst-Tests. `cargo doc --no-deps --document-private-items` der neuen Crates ohne Warnung.

## Nach P1 (offen)

Aus den Stufen 1–3 zusammengeführt, mit Grund:

- **gRPC-Erweiterungen von Onesimus:** Zweige (Branch), Medien-Ereignisse
  (D-7) und Stripe-Angaben als typisierte Nachrichten in
  `stored_service.proto` (Grundsatz: nur über gRPC). Nicht Teil von P1.
- **Datensatz als eine gRPC-Nachricht:** Der Lesepfad schickt je
  Datensatz `rechdr` und Daten als zwei Nachrichten, über gRPC wie über
  BNET; ein Datensatz je Nachricht mit dem Kopf als Feldern spart eine
  Nachricht und das Parsen.
- **Copy/Migration und VirtualFull** (SD-zu-SD, `replicate`): osd kann sie
  nicht; odir spricht den SD dafür nur über BNET an (`StorageUse::grpc`
  bleibt dort leer).
- **Zertifikat in `osd.d`:** ofd und osd nehmen ein eigenes Zertifikat nur
  über `--grpc-cert`/`--grpc-key`; eine Direktive fehlt.
- **Verbindungsaufbau „Client zum SD“** (ofd `sd.rs`, BNET und gRPC) liegt
  noch in ofd; nach Gutachten §5 gehört er in die osd-Schnittstelle.
- **Warten auf ein Volume:** beim Schreiben (`JS_WaitMedia`, `mount`) und
  beim Lesen (Mount-Anforderung statt Abbruch).
- **Anderes Device beim Lesen**, wenn der MediaType des Bootstraps nicht
  passt.
- **Recycling und `relabel`**, Spooling, Bandbreite (`setbandwidth` wird
  nur beantwortet), mehrere Jobs je Device (getrennte Lese-/Schreibzählung).
- **Hintergrundbetrieb, PID-Datei**, `-u`/`-g` für osd.
- **Übrige bareos-sd-Befehle:** `autochanger`, `readlabel`, `resolve`,
  `listen`, `nextrun`, `finish`, `setdevice`, `stats`.
- **Ablauf „Volume beschädigt“** (D-19, odir, Backlog V-1), siehe
  [Beschädigte Volumes](#beschädigte-volumes-bandriss-plattenschaden).
- **Ring:** macOS- und Windows-Teile; Striping im Ring (Verbraucher k liest
  Fach ≡ k mod N, beim Lesen setzt die Netzsitzung nach Folgenummer
  zusammen).
- **Unbekannte Direktiven in `bareos-sd.d`** werden noch nicht abgelehnt
  (aus P0); dazu die weiteren Punkte unter „Nach P1 verschoben“ im
  Abschnitt P0 (Hello mit `0x01`, ofd-Verschiebung, Client-Connects zum
  FD).

## Entscheidungen (Maintainer, 20.11.2026)

Alle wie empfohlen, D-8 präzisiert: „additiv, rückwärtskompatibel“.

| # | Frage | Entscheidung |
| --- | --- | --- |
| D-1 | Verteiler als Device-Ersatz mit `Route`-Blöcken | ja |
| D-2 | MediaType: je Ziel eigener (a) oder gemeinsamer (b) | a |
| D-3 | Merkmale einer Route: Job, Client, Level, Pool, Typ, FileSet; weitere? | diese sechs |
| D-4 | Stripe-Volumes nur für osd lesbar, eigener MediaType | ja (unvermeidbar) |
| D-5 | Stripe nur innerhalb einer Geräteart | ja |
| D-6 | Device-Watcher im osd-Prozess | ja |
| D-7 | gRPC-Push „Medium eingelegt/entfernt“ für die GUI | später, mit GUI-Sitzung abstimmen |
| D-8 | Neue Ressourcen in `osd.d/` statt `bareos-sd.d/` | **additiv, rückwärtskompatibel:** osd liest `bareos-sd.d` unverändert; Onesimus-Erweiterungen (Verteiler, Routen, Zweig-, Ring- und S3-Angaben, `Removable Label`) kommen zusätzlich aus `osd.d/`. Ohne `osd.d/` verhält sich osd wie bareos-sd, und bareos-sd läuft mit derselben `bareos-sd.d` weiter. **Namensgleichheit ist Bedingung:** `osd.d/` hat dieselbe Struktur wie `bareos-sd.d` (`osd.d/<typ>/<Name>.conf`); eine Ergänzung gilt der Ressource gleichen Typs und gleichen `Name` in `bareos-sd.d` und fügt nur Onesimus-Direktiven hinzu (überschreibt keine Bareos-Direktive). Eine Ergänzung ohne gleichnamige Ressource in `bareos-sd.d` ist ein Konfigurationsfehler (`-t` meldet ihn). Neue Ressourcentypen (`Distributor`) stehen nur in `osd.d/` und verweisen auf Devices **mit deren Namen aus `bareos-sd.d`** |
| D-9 | Zweige: Direktive `Branch = <Job>` am Haupt-Job; der Zweig-Job ist eine normale Job-Ressource | ja |
| D-10 | Unterschiedliche Basis der Zweige: der Haupt-Job liest ab der älteren Basis | ja |
| D-11 | Ein gescheiterter Zweig bricht die anderen nicht ab | ja |
| D-12 | Ring: ein Fach = ein Bareos-Block; Fachgröße = Blockgröße des Devices, Fachzahl einstellbar (Vorgabe z. B. 64 Fächer je Job) | ja |
| D-13 | Geräte-Schreiber als eigene Prozesse (wie die Job-Worker von odir) statt Threads | ja (Ausfall eines Geräts reißt den Job nicht mit) |
| D-14 | Platte als **emuliertes Band** (Wechsler mit Kassetten, Bandkern in osd, ohne Kernelmodul); Frontends Bareos-Bandformat und LTFS (`tape_ops`-Plugin), SCSI (TCMU/mhVTL) optional später; eigene Phase PT nach P1. Ersetzt die osdfs-Skizze | **ja** (21.11.2026) |
| D-16 | Labels für Plattenbänder: Magazin-Label elektronisch (Barcode, UUID, Seriennummer bzw. LUN-Kennung), Kassetten-Barcodes `<Präfix><Nr><D1>` mit **frei wählbarem Präfix**, physisches Etikett als PDF | **ja** (21.11.2026) |
| D-17 | Plattenband liest hinter Schadstellen weiter (Satzkopf + crc32c) und meldet die Lücke | **ja** (21.11.2026) |
| D-18 | Striping nur mit Zweig auf anderes Medium oder Parität; `-t` lehnt sonst ab | **ja** (21.11.2026) |
| D-19 | Ablauf „Volume beschädigt“ in odir (Übersicht, Kopien hochstufen, neuen Full anstoßen), unabhängig von osd | **ja** (21.11.2026) |
| D-20 | Plattenbänder auf SAN-LUNs über iSCSI: osd als Initiator, LUN = Magazin, Kennung aus VPD 0x83, Persistent Reservations + Lease für den alleinigen Zugriff, Logout als Auslagern; osd als iSCSI-Target für fremde Hosts später optional | **ja** (21.11.2026) |
| D-15 | Ring: `memmap2` + eigene Plattformschicht, Logik nach Disruptor/Aeron, Warten immer mit Zeitlimit (Recherche B) | **offen** (Empfehlung: ja) |
