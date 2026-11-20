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

## Eigenes Backup-Dateisystem (Richtung, Maintainer 20.11.2026)

> Es macht Sinn, ein eigenes Filesystem für Backups zu etablieren, um auf die Platten zu schreiben. Dann haben wir die doppelte Organisation gespart.

- **Heute** ist die Organisation doppelt: Ein allgemeines Dateisystem verwaltet Dateien und Extents, und in den Volume-Dateien verwaltet das Bareos-Volume-Format noch einmal Blöcke, Labels und Sitzungen.
- **Ziel:** osd schreibt auf Rohgeräte oder Partitionen in einem **eigenen, auf Backups zugeschnittenen Layout**, am Betriebssystem vorbei, also ohne fremdes Dateisystem und ohne Seitencache (`O_DIRECT` bzw. die Gegenstücke auf macOS und Windows).
- **Skizze** (Grundlage: Recherche, siehe unten):
  - je Platte ein Superblock mit Label, das sich selbst beschreibt; Wechselplatten werden daran erkannt (D-6);
  - ein Volume ist eine benannte Menge von Extents. Darin stehen die Bareos-Blöcke unverändert, also Labels, Sitzungen und Records. Ein Volume lässt sich als gewöhnliche Bareos-Volume-Datei **exportieren**, für `bls` oder bareos-sd. Damit bleibt die Kompatibilität aus dem [Grundsatz](#grundsatz-grpc-ist-der-weg-bnet-bleibt-für-bareos) erhalten.
  - Belegt wird fast nur durch Anhängen. Freigegeben wird volumeweise beim Recyceln, wie bei Bareos: prune, purge, recycle.
  - Die Metadaten sind gegen Abstürze gesichert (Journal oder Copy-on-Write).
  - Ein Stripe-Satz besteht aus mehreren Platten mit gemeinsamem Superblock-Verweis (D-4/D-5). Prüfsummen werden je Block geführt.
- **Status:** Richtung festgelegt. Recherche: [OSD-RECHERCHE-SPEICHER-UND-RING.md](OSD-RECHERCHE-SPEICHER-UND-RING.md). Empfehlung dort: selbst schreiben mit `rustix`/`nix` (+ optional `io-uring`), Entwurf nach BlueStore (Label mit Kopien, crc32c je Block) und Blobstore (A/B-Verzeichnis, Wiederaufbau vorwärts); SPDK und BlueStore selbst nicht übernehmen. Als **eigene Phase nach P1**; Entscheidung D-14 (siehe Tabelle) steht aus.

## Gerätearten

| Art | Bareos-Gegenstück | osd-Umfang |
| --- | --- | --- |
| Datei/Verzeichnis | `DeviceType = File` | wie bareos-sd, Volume = Datei |
| Wechselplatte (USB) | `File` + `RemovableMedia = yes`, Mount-Befehle | dazu ein **Device-Watcher** (D-6): erkennt das Einstecken (udev oder Dateisystem-Label), hängt ein, prüft und liest das Volume-Label, meldet „bereit“ und löst wartende Mount-Anforderungen aus. Beim Abziehen gibt er das Device sauber frei |
| Objektspeicher (S3) | `DeviceType = droplet`/`dplcompat` (Chunks) | Volume in Stücken als Objekte, Chunk-Größe einstellbar, Zugangsdaten aus Datei |
| Band | `DeviceType = Tape`, Autochanger mit `mtx` | Bareos-Bandformat (Blockgröße, EOF-Marken), Autochanger-Befehle. Größter Aufwand, zuletzt |

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
| P1 (Stufe 1 ✅, Stufe 2 ✅ 20.11.2026) | osd als Ersatz für bareos-sd mit **einem File-Device**: Label, Mount, Append, Read, Status, CatReq, Bareos-Volume-Format. Steuer- und Datenweg über **gRPC** (odir, ofd) und BNET (bareos-dir, bareos-fd). Stufe 1: Backup über BNET, aktiver und passiver Client, Datenweg über den Ring; Stufe 2: Lesen/Restore; Stufe 3: gRPC | Testgerüst Director+FD+SD (`onesimus-rig`): Backup und Restore mit odir+ofd über gRPC und mit bareos-fd über BNET. `bls` liest das Volume. Stufe 1 erledigt, siehe [P1 Stufe 1 umgesetzt](#p1-stufe-1-umgesetzt-20112026); Stufe 2 erledigt, siehe [P1 Stufe 2 umgesetzt](#p1-stufe-2-umgesetzt-20112026) |
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

Offen für **Stufe 3** (gRPC):

- Zweite Umsetzung von `transport::Channel` über `onesimus-proto-stored` zu odir und ofd, für Schreiben und Lesen; Onesimus-Erweiterungen nur dort.
- Der Lesepfad schickt heute je Datensatz `rechdr` und Daten als zwei BNET-Nachrichten; über gRPC sollte ein Datensatz eine Nachricht sein (Kopf als Felder).
- Medien-Ereignisse (D-7) und Stripe-Angaben brauchen Proto-Felder.

Weiter offen aus Stufe 1 und 2 (siehe Abweichungen in der Wiki): Warten auf ein Volume beim Schreiben (`JS_WaitMedia`, `mount`) und beim Lesen (Mount-Anforderung statt Abbruch), ein anderes Device beim Lesen, wenn der MediaType des Bootstraps nicht passt, Recycling und `relabel`, Spooling, Bandbreite, mehrere Jobs je Device (getrennte Lese-/Schreibzählung), Hintergrundbetrieb/PID-Datei, die übrigen bareos-sd-Befehle (`autochanger`, `readlabel`, `resolve` …), Copy/Migration (SD-zu-SD, `replicate`), die macOS-/Windows-Teile des Ring, Striping im Ring (Verbraucher k liest Fach ≡ k mod N; beim Lesen setzt die Netzsitzung nach Folgenummer zusammen).

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
| D-14 | Eigenes Backup-Dateisystem auf Rohgeräten nach dem Entwurf der Recherche (A, Punkte 1–6), als eigene Phase nach P1 | **offen** (Empfehlung: ja) |
| D-15 | Ring: `memmap2` + eigene Plattformschicht, Logik nach Disruptor/Aeron, Warten immer mit Zeitlimit (Recherche B) | **offen** (Empfehlung: ja) |
