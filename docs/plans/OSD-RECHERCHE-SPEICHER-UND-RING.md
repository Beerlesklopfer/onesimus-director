# osd: Recherche Speicherschicht und Ringpuffer

Stand: 20.11.2026. Alle Versions- und Datumsangaben stammen von crates.io bzw. der GitHub-API vom selben Tag. **[unbelegt]** heißt: aus zweiter Hand oder aus dem Gedächtnis, nicht an einer Primärquelle geprüft. Ergänzt den Plan [OSD-GERAETE-UND-VERTEILUNG.md](OSD-GERAETE-UND-VERTEILUNG.md), Abschnitte „Datenpfad: Ringpuffer“ und „Platte als emuliertes Band (D-14)“. Nachtrag B vom 21.11.2026.

## Ergebnis in Kürze

| Teil | Empfehlung | Begründung |
| --- | --- | --- |
| **A** Speicherschicht (Striping, Rohgeräte, eigenes Backup-Dateisystem) | **Selbst schreiben** mit `rustix`/`nix` (O_DIRECT, ioctls, statx), optional `io-uring`, `memmap2`. **Weder SPDK noch BlueStore** | SPDK braucht root, Hugepages und vfio, nimmt dem Kernel das Gerät weg und gilt nur für NVMe unter Linux; für Datei, USB, S3 und Band hilft es nicht. BlueStore ist C++, LGPL und für Objekte mit wahlfreiem Zugriff gebaut; osd-Volumes werden fast nur angehängt |
| **B** Portabler Ring (Linux, macOS, Windows) | **`memmap2` + eigene dünne Plattformschicht**, Ringlogik nach Disruptor/Aeron. iceoryx2 nur als Vergleich und Rückfall | iceoryx2 deckt als einziges Crate alle drei Plattformen ab, wartet aber durch Abfragen statt futex, bringt ein eigenes Dateilayout mit, kennt kein Striping, und seine 0.x-API bricht |

## A. Speicherschicht

| Kandidat | Lizenz | Reife | Striping | Ausrichtung, Metadaten, Wiederherstellung | Zugriff | Nutzen |
| --- | --- | --- | --- | --- | --- | --- |
| Ceph BlueStore | LGPL-2.1/3 ([COPYING](https://github.com/ceph/ceph/blob/main/COPYING)) | sehr reif | keins, RADOS verteilt | roh aufs Gerät, Metadaten in RocksDB, crc32c je 4 KiB, min_alloc 4 KiB ([config](https://docs.ceph.com/en/latest/rados/configuration/bluestore-config-ref/)); Label am Anfang mit Kopien bei 1/10/100/1000 GiB ([tool](https://docs.ceph.com/en/squid/man/8/ceph-bluestore-tool/)) | ganzes Gerät | **Entwurf übernehmen:** Label mit redundanten Kopien, crc32c je Block |
| SPDK (raid0/1/5f, Blobstore) | BSD-3-Clause | v26.09 (25.09.2026) | raid0 mit Strip-Größe; RAID-Superblock optional ([bdev](https://spdk.io/doc/bdev.html)) | Blobstore: Superblock in Cluster 0, Wiederaufbau nach unsauberem Ende ([blob](https://spdk.io/doc/blob.html)) | root, Hugepages, vfio-pci; nur Linux/NVMe | nicht nutzen; Idee „Superblock + Wiederaufbau“ übernehmen |
| MinIO-Erasure-Layout | AGPL-3.0, **Repo archiviert** | letztes Release 16.10.2025 | Erasure-Sets, Quorum K/K+1 ([EC](https://docs.min.io/community/minio-object-store/operations/concepts/erasure-coding.html)) | HighwayHash, xl.meta je Objekt | Userspace auf Dateien | nur Entwurf, falls später Parität |
| SeaweedFS | Apache-2.0 | 4.54 (16.11.2026) | große Anhänge-Dateien mit „Needles“ **[unbelegt]** | | Userspace | Entwurf: Volume als Anhänge-Container |
| Lustre / BeeGFS / DAOS | GPL-2.0 **[unbelegt]** / proprietär / BSD-2-Clause-Patent | | stripe_count × stripe_size, beim Anlegen fest ([wiki](https://wiki.lustre.org/Configuring_Lustre_File_Striping)) | | Kernel-FS | nur Begriff: Layout beim Anlegen fest und beim Volume gespeichert |
| LTFS | BSD-3-Clause | v2.4.9.1 (11.11.2026) | – | eigenes FS-Format | | nicht relevant (kein Bareos-Format) |
| NetBackup bptm | proprietär | | Shared-Memory-Puffer zwischen Eltern- und Kindprozessen, Puffergröße = Bandblockgröße ([Tuning](https://docs.cohesity.com/docs/netbackup/10.1.1/21414900-146141073-0/v19527810-146141073)) | | | **bestätigt unseren Entwurf** (Ring, Fach = Geräteblock). In Bareos/Bacula kein solches Verteilen gefunden **[unbelegt, Verneinung]** |
| `io-uring` | MIT/Apache-2.0 | 0.7.16 (09.11.2026) | | ohne Async-Laufzeit | Linux | **nutzen** (optionaler schneller Weg) |
| `rustix` / `nix` | Apache/MIT, MIT | 1.1.6 (27.10.2026) / 0.32.0 (17.11.2026) | | O_DIRECT, ioctls, statx | | **nutzen** |
| glommio, tokio-uring, monoio | | 2024, veraltet | | eigene Laufzeit | nur Linux | meiden |

**Rohzugriff je Plattform**
- **Linux:** O_DIRECT, Ausrichtung über `statx` STATX_DIOALIGN (ab 6.1); `O_EXCL` auf einem Blockgerät scheitert mit EBUSY, wenn es eingehängt ist, und sichert so den alleinigen Zugriff ([open(2)](https://man7.org/linux/man-pages/man2/open.2.html)).
- **macOS:** kein O_DIRECT; `F_NOCACHE` auf `/dev/rdiskN`, für Dauerhaftigkeit `F_FULLFSYNC` (fsync erreicht nur den Laufwerkscache). Sektor-Ausrichtung auf rdisk **[unbelegt]**.
- **Windows:** `FILE_FLAG_NO_BUFFERING` verlangt Größen, Offsets und Puffer in Vielfachen der Sektorgröße ([Doku](https://learn.microsoft.com/en-us/windows/win32/fileio/file-buffering)); Schreiben auf `\\.\PhysicalDriveN` in einem eingehängten Volume ist gesperrt, bis das Volume per `FSCTL_LOCK_VOLUME`/`FSCTL_DISMOUNT_VOLUME` gesperrt bzw. ausgehängt ist ([KB942448](https://mskb.pkisolutions.com/kb/942448)).
- Ein seitenweise ausgerichteter 64-KiB-Block erfüllt die Regeln aller drei Plattformen. Rohgeräte brauchen überall root, die Gruppe `disk` oder Admin.

**Zu übernehmender Entwurf (Grundlage für D-14)**
1. **Stripe-Einheit = Bareos-Block (64 KiB).** Block *n* geht an Mitglied *n mod N*. Jeder Block behält seinen Bareos-Kopf (Blocknummer, VolSessionId/-Time, Prüfsumme), jedes Mitglied ist also ein Bareos-artiger Blockstrom. Ein Volume ohne Striping bleibt reines Bareos-Format (D-4/D-5). Restore liest alle Mitglieder parallel und sortiert nach Blocknummer, das ist das Umsortieren im Ring.
2. **Mitglieds-Label wie bei BlueStore:** an LBA 0 mit Kopien an festen Stellen: Stripe-Satz-UUID, Mitgliedsnummer, N, Stripe-Einheit, MediaType, Generationszähler, crc32c. Danach das Bareos-Volume-Label dieses Mitglieds.
3. **Volume-Verzeichnis auf dem Rohgerät:** kleiner Bereich in zwei Kopien (A/B) mit Generationszähler und CRC; geschrieben wird immer die andere Kopie. Inhalt: Extents der Volumes und der bestätigte Schreibzeiger.
4. **Wiederherstellung nach Absturz:** vom letzten bestätigten Zeiger vorwärts lesen, bis CRC oder Folge bricht, logstrukturiert wie der Blobstore-Wiederaufbau. Maßgeblich bleibt der Katalog (JobMedia je Mitglied).
5. **Rohzugriff:** Linux `O_DIRECT|O_EXCL`, Ausrichtung aus STATX_DIOALIGN/BLKSSZGET; macOS `/dev/rdiskN` mit F_NOCACHE und F_FULLFSYNC; Windows Volume sperren und aushängen, dann FILE_FLAG_NO_BUFFERING|WRITE_THROUGH auf Sektorgröße ausgerichtet. Ringfächer seitenweise ausrichten, dann ist O_DIRECT direkt aus dem mmap denkbar (ohne Kopie) **[unbelegt, muss getestet werden]**.
6. **Mitglieder je Geräteart:** Band = je Laufwerk ein eigenes Band-Volume (kein LTFS); S3 = je Mitglied ein eigener Objektstrom; Parität (Erasure-Sets mit Quorum) höchstens später.

## B. Portabler Ring

| Kandidat | Lizenz | Version | Plattformen | Eigene Datei, Daemon? | Modi, Freigabe | Absturz | Warten | Urteil |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| iceoryx2 | MIT/Apache-2.0 | 0.10.0 (18.09.2026) | Linux/macOS/Windows (Tier 2), FreeBSD ([README](https://github.com/eclipse-iceoryx/iceoryx2)) | kein Daemon (anders als RouDi in v1), eigene Dateien unter einstellbarem Pfad ([config](https://github.com/eclipse-iceoryx/iceoryx2/blob/main/config/README.md)) | Pub/Sub mit Warteschlange je Abonnent; Route und Zweige passen, Striping bräuchte N Dienste; ohne Kopie über `loan_uninit` | Suche nach toten Knoten, `try_remove_stale_resources` | Gegendruck durch Abfragen (yield, dann sleep), kein futex ([sender.rs](https://github.com/eclipse-iceoryx/iceoryx2/blob/main/iceoryx2/src/port/details/sender.rs)) | beste fertige portable Wahl, passt aber nicht zu „ein Ring je Job, unser Layout“; Vergleich und Rückfall |
| Aeron (rusteron, aeron-rs) | Apache-2.0 | Aeron 1.54.2 (11.11.2026) | alle drei | braucht einen Media-Driver (einbettbar), aeron.dir = /dev/shm ([config](https://github.com/aeron-io/aeron/wiki/Configuration-Options)) | IPC-MTU 1408, keine zusammenhängenden 64-KiB-Fächer | Zeitlimits für Lebendigkeit und Entsperren | Abfragen/Idle-Strategien | **Entwurf übernehmen**, nicht nutzen |
| disruptor-rs | MIT | 4.4.0 (08.08.2026) | nur innerhalb eines Prozesses | – | Gating-Modell genau passend | – | spin/yield | nur Entwurf |
| bus, rtrb, ringbuf | MIT/Apache | | nur im Prozess | – | | | | nicht verwendbar |
| shared_memory + raw_sync | MIT/Apache | 0.12.4 (2022) / 0.1.5 (2020) | alle drei | eigene Namen über shm_open/CreateFileMapping | | | pthread_cond bzw. benannte Events | **nicht gepflegt**; Ansatz brauchbar, Crates nicht |
| `memmap2` | MIT/Apache | 0.9.12 (21.10.2026) | alle drei | bildet eine Datei ab, die wir kontrollieren | | | | **nutzen** |
| linux-futex / atomic-wait | BSD-2-Clause | 1.0.0 (2024) / 1.1.0 (2023) | atomic-wait: private libc++-Symbole auf macOS, WaitOnAddress auf Windows | | | | futex | linux-futex unter Linux gut; atomic-wait unter Windows **nicht prozessübergreifend** |
| ipc-channel, interprocess, shmem-ipc, ipmpsc | | | teilweise | | kein Broadcast mit Gating | | | nicht geeignet (höchstens Steuerkanal) |

**Warten und Wecken über Prozessgrenzen**
- **Linux:** futex auf dem gemeinsamen Wort.
- **macOS:** `os_sync_wait_on_address` mit `OS_SYNC_WAIT_ON_ADDRESS_SHARED` ab macOS 14.4 ([Apple](https://developer.apple.com/documentation/os/os_sync_wait_on_address.md)); darunter privates ulock oder Abfragen. Gemeinsame pthread-Bedingungsvariablen haben auf macOS gemeldete Probleme.
- **Windows:** `WaitOnAddress` wirkt nur innerhalb eines Prozesses (bestätigt, [Microsoft](https://learn.microsoft.com/windows/win32/api/synchapi/nf-synchapi-wakebyaddresssingle), [Raymond Chen](https://devblogs.microsoft.com/oldnewthing/20170601-00/?p=96265)); stattdessen benannte Auto-Reset-Events oder Semaphoren.

**Zu übernehmender Entwurf**
- **Abbildung:** auf allen Plattformen eine gewöhnliche Datei über `memmap2`: `/dev/shm` unter Linux, ein privates Temp-Verzeichnis unter macOS, eine Datei mit FILE_ATTRIBUTE_TEMPORARY unter Windows (Leistung dort **[unbelegt]**). Ein Codepfad, und die Suche nach verwaisten Ringen funktioniert überall gleich.
- **Warten:** futex (Linux), `os_sync_wait_on_address` SHARED (macOS), benannte Events je Wartendem, Namen im Kopf (Windows). **Immer mit Zeitlimit (etwa 100 ms)** warten, damit Lebendigkeitsprüfungen auch bei einem verlorenen Wecken laufen.
- **Disruptor:** ein Zeiger des Erzeugers und eine Folgenummer je Verbraucher. Zweige: freigeben bis zur Gating-Folge = min(Folge der lebenden Verbraucher); ein als gescheitert markierter Verbraucher fällt aus dem Gating. Striping: Verbraucher *k* wartet, bis der Zeiger sein nächstes Fach (≡ *k* mod N) erreicht; Freigabe weiterhin min(Folge je Verbraucher). Route: N = 1. Der Erzeuger wartet, solange Zeiger − Kapazität ≥ kleinstes Gate ([LMAX](https://lmax-exchange.github.io/disruptor/disruptor.html), in dieser Recherche nicht abgerufen **[unbelegt]**).
- **Aeron:** Positionszähler auf eigenen Cache-Zeilen; Zeitlimits nach dem Vorbild `client.liveness` (10 s) und `publication.unblock` (15 s): ein langsamer oder toter Verbraucher wird nach Ablauf entfernt, statt den Job anzuhalten.
- **Lebendigkeit:** PID, Prozess-Startzeit und Herzschlagzähler je Teilnehmer im Kopf. Der osd-Elternprozess startet die Schreiber und überwacht sie (SIGCHLD/pidfd unter Linux, Prozess-Handles unter Windows, kqueue EVFILT_PROC unter macOS **[unbelegt]**). Beim Start verwaiste Ringe löschen.

## Risiken

- O_DIRECT aus Shared-Memory-Seiten ist ungetestet.
- Sperren eines Windows-Volumes mit aktivem Dateisystem ist ungetestet.
- Unter macOS vor 14.4 gibt es kein öffentliches prozessübergreifendes futex-Gegenstück.
- Rohgeräte brauchen auf jeder Plattform Rechte.
- Fällt die Wahl doch auf iceoryx2, ist dessen API noch nicht stabil.

## Nachtrag A: eigenes Backup-Dateisystem („osdfs“)

> **Ersetzt durch Nachtrag B** (D-14, 21.11.2026): Die Platte wird als Band emuliert, nicht als Dateisystem organisiert. Übernommen bleiben Label mit Kopien, crc32c, Vorwärtslesen nach Absturz, Schutz fremder Platten und der Bau zuerst gegen eine Datei bzw. ein Loop-Device.

**Ergebnis:** machbar, ohne die Bareos-Kompatibilität aufzugeben, **wenn jedes osd-Volume ein bytegleiches Bareos-Datei-Volume bleibt** (Volume-Label, dann Bareos-Blöcke). Die Dateisystem-Schicht verwaltet dann nur Extents und Volume-Namen. Das ist die eingesparte doppelte Organisation.

### Vorbilder

| Entwurf | Auf der Platte | Übernehmen | Lizenz |
| --- | --- | --- | --- |
| NILFS2 | logstrukturiert, gleich große Segmente mit Zusammenfassung, Checkpoints, Wiederherstellung über Segment-Grenzmarken ([Doku](https://docs.kernel.org/filesystems/nilfs2.html)) | feste Segmente, Zusammenfassung je Segment, Vorwärtslesen; **kein Cleaner nötig**, weil Volumes als Ganzes recycelt werden | GPL-2.0, nur Entwurf |
| F2FS | getrennte Logs für Knoten und Daten ([Doku](https://www.kernel.org/doc/html/latest/filesystems/f2fs.html)) | Metadaten-Log von den Daten trennen | nur Entwurf |
| Zoned Storage (ZNS/SMR) | Zonen nur sequenziell beschreibbar, mit Schreibzeiger, Reset löscht ([zonedstorage.io](https://zonedstorage.io/docs/introduction/zoned-storage)) | Segment = Zonengröße, Recyceln = Reset, Metadaten in kleinem Bereich mit wahlfreiem Schreiben; später SMR/ZNS-fähig | Begriff |
| Ceph BlueStore | Label an Offset 0 mit Kopien, crc32c; Belegungsstand nur im Speicher, nach Absturz durch Durchsuchen neu aufgebaut ([fast-onode-scan](https://docs.ceph.com/en/latest/rados/bluestore/fast-onode-scan/)) | Label mit Kopien, CRC je Block; Belegung **nicht** bei jedem Schreiben sichern, sondern aus der Volume-Tabelle ableiten | LGPL, nur Entwurf |
| SPDK Blobstore | Superblock in Cluster 0, Blob = Liste von Clustern (~1 MiB), Wiederaufbau aus den Metadaten je Blob ([Doku](https://spdk.io/doc/blob.html)) | **nächstes Vorbild:** Volume = Blob = Liste von Segmenten | BSD-3, nur Entwurf |
| LTFS | Index-Partition mit XML-Index, Generationsnummer, Zeiger auf den vorigen Index ([SNIA 2.4](https://www.snia.org/sites/default/files/technical-work/ltfs/release/SNIA-LTFS-Format-2.4.0-TechPosition.pdf)) | Index-Generationen mit Rückzeigern | BSD-3; für Band bleibt das Bareos-Format |
| Proxmox Backup Server | auf normalem FS, Chunks nach SHA-256 mit CRC; auf Band ein Katalog auf dem Medium ([Überblick](https://pbs.proxmox.com/docs/technical-overview.html), [Band](https://pbs.proxmox.com/docs/tape-backup.html)) | Medien, die sich selbst beschreiben | AGPL-3.0 **[unbelegt]** |
| IBM Spectrum Protect | DISK-Pools; auf AIX rohe LVs erlaubt, **unter Linux verboten**; Warnung, dass der Server nicht immer erkennt, ob ein rohes LV schon Daten enthält ([DEFINE VOLUME](https://www.ibm.com/docs/SSGSG7_7.1.5/srv.reference/r_cmd_volume_define.html)) | **Magisches Label und Ablehnung fremder Platten sind Pflicht** | proprietär |
| Veeam | kein eigenes FS, nutzt XFS-Reflink bzw. ReFS-Block-Cloning ([Fast Clone](https://helpcenter.veeam.com/docs/vbr/userguide/backup_repository_block_cloning.html)) | Gegenbeispiel: der Marktführer bleibt auf einem FS | proprietär |

### Skizze (eine osdfs je Platte oder Partition)

```
LBA 0      Platten-Label (4 KiB): Magic "OSDFS001", fs-UUID, Stripe-Satz-UUID,
           Mitglied i/N, Stripe-Einheit (64 KiB), Segmentgröße, Sektorgröße,
           MediaType, Geräteart (fest/USB), Generation, crc32c
           (Kopien an festen Stellen; letztes Segment spiegelt es)
Metadaten  2 × Volume-Tabelle (A/B, Copy-on-Write, Generation + crc32c):
           je Volume: Name, Bareos-MediaType, Zustand (Append/Full/Used/Purged),
           Segmentliste, bestätigte Länge, erste/letzte JobId, Label-Zeit
Daten      Segmente, z. B. 256 MiB (Zonengröße), nur Anhängen;
           je Segment: 4 KiB Zusammenfassung (Volume-Id, Folge, CRC) + Bareos-Blöcke
```

- **Volume = benannte Extent-Menge** mit dem Inhalt eines bytegleichen Bareos-Datei-Volumes. Export = Segmente hintereinander in eine Datei; `bls`, `bextract` und bareos-sd lesen sie. Ein Export-Rundlauftest genügt als Nachweis.
- **Belegung:** nur Anhängen aus einer Liste freier Segmente. Frei wird Platz erst, wenn Bareos ein ganzes Volume purged oder recycelt; dann gehen seine Segmente zurück (bzw. Zonen-Reset bei ZNS). Eine Bitmap über die Segmente reicht und wird aus der Volume-Tabelle neu aufgebaut.
- **Absturzsicherheit:** Volume-Tabelle A/B mit Generation. Reihenfolge beim Bestätigen: Blöcke fsync, dann die andere Tabellenkopie schreiben, dann fsync. Wiederherstellung: neueste gültige Tabelle laden, dann im letzten Segment vorwärts lesen, solange Zusammenfassung, Block-CRC und Blocknummer stimmen (wie NILFS). Maßgeblich bleibt der Katalog (JobMedia). Kein Journal nötig, solange die Metadaten nur die Volume-Tabelle sind.
- **Striping (D-4/D-5):** jede Mitgliedsplatte hat ihre eigene osdfs; ein Stripe-Volume ist ein gleichnamiges Volume auf jedem Mitglied des Satzes, nur innerhalb einer Geräteart. Block *n* geht an Mitglied *n mod N*.
- **Wechselplatten (D-6):** das Platten-Label beschreibt sich selbst; osd erkennt eine eingesteckte Platte an der fs-UUID, nicht am Gerätepfad.
- **Ausrichtung:** alles auf 64 KiB, Metadaten mindestens auf 4 KiB bzw. die physische Sektorgröße.
- **Prüfsummen:** crc32c in Label, Tabelle, Segment-Zusammenfassung und Bareos-Block; optional xxh3 oder BLAKE3 je Segment zum Scrubben.

### Rust-Bausteine (crates.io, 20.11.2026)

`crc32c` 0.6.9 oder `crc-fast` 1.10.0; `twox-hash` 2.1.5, `xxhash-rust` 0.8.21 (BSL-1.0, verträglich **[unbelegt]**), `blake3` 1.8.7; Bitmap mit `roaring` 0.12.0 oder `bitvec` 1.1.1. `redb` 5.0.0 hat einen `StorageBackend`-Trait und könnte auf einem reservierten Rohbereich liegen (Blockausrichtung **[unbelegt]**), ist für eine Tabelle mit einigen tausend Einträgen aber überdimensioniert. `fjall` zu schwer, `sled` veraltet, `persy` MPL-2.0. **Kein Allokator- oder Rohgeräte-FS-Crate lohnt die Übernahme**; die Belegung ist eine Bitmap von einigen hundert Zeilen.

### Folgen für die Bareos-Kompatibilität

- Volumes ohne Striping bleiben bytegleiche Bareos-Volumes, portabel über den Export.
- Stripe-Volumes sind nur für osd (D-4/D-5); jedes Mitglied ist trotzdem ein gültiger Bareos-Blockstrom.
- Ein bareos-sd kann osdfs-Platten nicht direkt lesen. Abhilfe: `osd export <volume>`, später eine schreibgeschützte FUSE-Sicht mit Volumes als Dateien **[Vorschlag]**.
- Band und S3 sind nicht betroffen: osdfs gilt nur für Platten und Wechselplatten.

### Risiken

1. Eine fremde Platte überschreiben (die IBM-Warnung): Magic-Label verlangen, alles mit Partitionstabelle oder FS-Signatur ohne `--force` ablehnen, `O_EXCL`.
2. Fehler in der Wiederherstellung bedeuten Datenverlust: Tests mit eingeschleusten Abstürzen (kill während des Tabellen-Commits) und ein Scrubber.
3. Rechte: root/Gruppe `disk` (Linux), Admin plus Volume-Sperre (Windows), rdisk (macOS).
4. Admins verlieren gewohnte Werkzeuge (ls, df, rsync); Export und FUSE mildern das.
5. Rohzugriff unter Windows und macOS ungetestet.
6. Wenig Marktbeleg: Veeam und PBS bleiben auf Dateisystemen, IBM verbietet rohe LVs unter Linux.

### Empfohlener erster Schritt

1. **osdfs v1** festlegen: Platten-Label mit Kopien, A/B-Volume-Tabelle, feste Segmente, Volumes als Segmentlisten mit unveränderten Bareos-Volume-Bytes.
2. Zuerst gegen eine **vorbelegte gewöhnliche Datei oder ein Loop-Device** als „Platte“ bauen: derselbe Codepfad, ohne root testbar. Danach `/dev/sdX` mit `O_DIRECT|O_EXCL`, zunächst nur unter Linux.
3. Umfang: eine Platte, kein Striping; Label, Volume anlegen/anhängen/schließen, Recyceln (Segmente frei), `osd export` in eine einfache Bareos-Datei, Wiederherstellung nach kill -9.
4. Abnahme: ein exportiertes Volume besteht `bls`; Tests mit eingeschleusten Abstürzen stellen ohne Verlust bestätigter Blöcke wieder her.
5. Danach in dieser Reihenfolge: Striping über N Mitglieder, Wechselplatten mit Selbstbeschreibung, dann Rohzugriff unter macOS und Windows.

## Nachtrag B: Platte als emuliertes Band (D-14, 21.11.2026)

**Beschluss (Maintainer):** „Für uns wäre das die Emulation eines Tape-Devices, also ein Treiber, der eine HDD wie ein Tape anspricht. Linear Tape File System sollte ebenfalls funktionieren.“

**Ergebnis:** Band-Semantik passt besser als ein Dateisystem, weil Bareos ohnehin nur anhängt und volumeweise recycelt. Volume-Tabelle, Segmentlisten und Belegungs-Bitmap aus Nachtrag A entfallen. LTFS lässt sich ohne Kernelmodul anbinden, über die Treiber-Schnittstelle der Referenzimplementierung.

### LTFS-Referenzimplementierung (GitHub-API, 21.11.2026)

| Punkt | Befund |
| --- | --- |
| Repository | `LinearTapeFileSystem/ltfs`, BSD-3-Clause, nicht archiviert, letzter Push 12.11.2026, Release v2.4.9.1-10524 (11.11.2026) |
| Plattformen (README) | Linux, macOS, FreeBSD, NetBSD; Windows nicht aufgeführt (der Treiber `file` enthält `#ifdef _WIN32`-Zweige) |
| Treiber-Schnittstelle | `struct tape_ops` in `src/libltfs/tape_ops.h`, geladen über `src/libltfs/plugin.c` (Plugins per `ltfs.conf`) |
| Mitgelieferte Treiber ohne SCSI | `src/tape_drivers/generic/file` (`filedebug_tc.c`: Band in Dateien emuliert, `MAX_PARTITIONS 2`, Marke `MISSING_EOD`) und `generic/itdtimg` (Band-Abbilder) |
| Standard-Blockgröße | `LTFS_DEFAULT_BLOCKSIZE` = 512 KiB (`src/libltfs/ltfs.h`, außer wo `MAXPHYS` gilt) |

**Funktionen von `tape_ops`**, die der Bandkern tragen muss: `open`/`close`, `inquiry`, `test_unit_ready`, `read` (variable Satzgröße, `unusual_size`), `write`, `writefm`, `rewind`, `locate` (Partition + Block), `space` (Sätze/Dateimarken/EOD), `erase`, `load`/`unload`, `readpos`, `setcap`, `format` (Partitionierung, Volume-Name, Barcode, MAM-UUID), `remaining_capacity`, `read_attribute`/`write_attribute` (MAM je Partition), `allow_overwrite`, `get_eod_status`, `get_device_list`, `is_readonly`, `get_worm_status`, `get_serialnumber`. Mit Festwerten bzw. „nicht unterstützt“: `logsense`, `modesense`/`modeselect`, `reserve_unit`/`release_unit`, `set_compression`, `get_cartridge_health`, `get_tape_alert`, `set_key`/`get_keyalias`, `takedump_drive`, `grao`/`rrao`, `set_profiler`.

### Bandkern (Skizze)

```
Platte/Partition
LBA 0      Wechsler-Label (4 KiB, Kopien an festen Stellen): Magic "OSDTAPE1",
           Magazin-Barcode, Platten-UUID, Seriennummer bzw. LUN-Kennung (VPD 0x83),
           Eigentümer + Lease (SAN), Kassetten-Präfix,
           Sektorgröße, Kassettenzahl, Kassettengröße,
           Geräteart (fest/USB), Generation, crc32c
Fachtabelle  2 × (A/B, Generation + crc32c): Fach → Barcode, Zustand
           (leer/geladen/geschützt), Laufwerk
Kassette k fester Bereich, 64-KiB-ausgerichtet:
           Kopf: MAM-Bereich (Attribute je Partition), Partitionsgrenzen
           Partition 0..1: Sätze variabler Länge, je Satz Kopf
             (Typ Satz/Dateimarke, Länge, Satznummer, crc32c) + Daten;
           Satzverzeichnis (Checkpoints alle n Sätze) für locate/readpos;
           bestätigter EOD-Zeiger je Partition (A/B im Kassettenkopf)
```

- **Bareos-Frontend:** eine Kassette mit einer Partition; Satz = Bareos-Block, Dateimarken wie bareos-sd auf Band; JobMedia mit Datei/Block. Export einer Kassette als Bandabbild bzw. Satzstrom, 1:1 auf LTO kopierbar.
- **LTFS-Frontend:** `libtape-osd.so` implementiert `tape_ops` in Rust mit C-ABI (`#[no_mangle] extern "C"`) und spricht den Bandkern direkt (Platte im selben Prozess, `O_EXCL`) oder über osd (gRPC/Unix-Socket). Zwei Partitionen (Index, Daten), MAM für Barcode und Volume-Kohärenz.
- **SCSI-Frontend (optional):** LIO/TCMU oder mhVTL; ob TCMU Geräte vom Typ „sequential access“ trägt, ist **[unbelegt]**.
- **Labels (D-16):** Kassetten-Barcode `<Präfix><Nr>D1` (Präfix frei wählbar) in Fachtabelle und MAM; der Wechsler meldet ihn an Bareos (`update slots`/`label barcodes`) und an LTFS (`format` mit `barcode_name`, MAM). Etikett als PDF beim `osd disk init`.
- **Schadstellen (D-17):** bei einem Lesefehler ab der nächsten Sektorgrenze nach einem gültigen Satzkopf suchen (Magic, Satznummer > letzte, crc32c) und dort fortsetzen; das Satzverzeichnis begrenzt die Suche.
- **SAN/iSCSI (D-20):** Persistent Reservations über SG_IO (`PERSISTENT RESERVE OUT`, Typ „Write Exclusive – Registrants Only“, Schlüssel je osd-Instanz); Lease im Magazin-Label als zweite Sicherung für Speichersysteme ohne PR-Unterstützung. Zu prüfen: PR über dm-multipath (`reservation_key` in multipath.conf) **[unbelegt]**.
- **Absturz:** je Partition ab dem bestätigten EOD vorwärts lesen, solange Satzkopf, Folge und crc32c stimmen; dann EOD neu setzen. Bestätigen: Sätze fsync, dann EOD-Kopie A/B schreiben, fsync.
- **Recyceln:** `rewind` + Überschreiben ab Satz 0 (bzw. `format`); bei ZNS/SMR später Zonen-Reset.

### Risiken

1. Die LTFS-Plugin-Schnittstelle ist ein C-Header ohne ABI-Zusage; Versionen der Referenzimplementierung müssen getestet werden.
2. LTFS setzt Semantik voraus, die nur `tape_ops` dokumentiert (z. B. Verhalten bei EOD, Early Warning, Fehlercodes); die Treiber `file` und `itdtimg` sind die Vorlage.
3. Windows: von der LTFS-Referenz nicht unterstützt; dort nur das Bareos-Frontend.
4. Weiter gelten die Risiken aus Nachtrag A: fremde Platte, Wiederherstellungsfehler, Rechte, fehlende gewohnte Werkzeuge.

### Erster Schritt (Phase PT)

1. Bandkern gegen eine vorbelegte Datei bzw. ein Loop-Device: Wechsler-Label, Fächer, Kassetten mit zwei Partitionen, Sätze, Dateimarken, `locate`/`space`/`readpos`, MAM, Wiederherstellung nach kill -9.
2. Bareos-Frontend in osd: Backup und Restore im Rig auf eine emulierte Kassette; Export besteht `bls`.
3. LTFS-Plugin: `mkltfs` formatiert eine emulierte Kassette, `ltfs` hängt sie ein, Dateien schreiben, aushängen, wieder einhängen, vergleichen.
4. Danach `/dev/sdX` mit `O_DIRECT|O_EXCL` (Linux), dann Striping über mehrere emulierte Laufwerke, Wechselplatten, macOS.

## Nachtrag C: Ring auf Windows und macOS – Alternativen (21.11.2026)

Anlass: Der Ring (`crates/osd/ring`) wartet und weckt über Prozessgrenzen nur unter Linux (futex); `sys/wait_other.rs` ist auf macOS und Windows `unimplemented!()`, `pid_alive` fehlt unter Windows. Maintainer-Vorgabe: Aeron und iceoryx2 sind zu groß. Drei Agenten haben gesucht (Rust-Crates, Fremdbibliotheken und Betriebssystem-Bausteine, Architektur); Stand der Angaben: READMEs und Herstellerdoku, Quelltext der Crates nicht gelesen.

**Ergebnis:** Kein Crate und keine Bibliothek liefert Gating, Absturzerkennung und prozessübergreifendes Wecken auf allen drei Systemen belegt und zugleich ausgereift. Es bleibt beim eigenen Ring (`memmap2`, Disruptor-Gating, Zeitlimit-Warten).

| Kandidat | Befund | Urteil |
| --- | --- | --- |
| fipc 1.0.0 (28.10.2026, 20 Downloads, MIT) | Broadcast mit Gating, futex / `os_sync` / Windows-Event, Peer-Ende sofort erkannt; Zig-Kern als Binärbibliothek, 2 Commits, x86-64-v3, kein Striping | Messlatte und Prüfling, kein Fundament |
| SubEtha 0.6.0 (MIT) | Ring, Heartbeat-Tabelle, Eigentümer-Lease; Gating und Wartemechanismus offen | Quelltext prüfen, wahrscheinlichster Rust-Kandidat |
| shm-rings 1.0.0 | Gating mit Cursor je Leser, aber feste Slots und Wecken nur Linux | Muster |
| crossbar, ringfire, bcast, outcry, xchannel, orbitive | Linux-only, überschreibend oder ohne Gating | ausgeschieden |
| shmem_providers (LibAFL) | nur Speicherabbildung, kein Ring | nicht nötig (`memmap2`) |
| cpp-ipc/libipc (MIT) | C++-Ring mit Broadcast, höchstens 32 Empfänger | nur Entwurf |
| Boost.Interprocess, nng, ZeroMQ, Cap'n Proto, io_uring | lösen Warten und Gating nicht oder zu groß | nein (nng/ZeroMQ höchstens Steuerkanal) |

**Betriebssystem-Bausteine (Herstellerdoku):** Linux futex ohne `_PRIVATE` wirkt über Prozesse, `pidfd` (ab 5.3) meldet das Prozessende. macOS: `os_sync_wait_on_address` mit `OS_SYNC_WAIT_ON_ADDRESS_SHARED` ab 14.4, `kqueue` `EVFILT_PROC`/`NOTE_EXIT` für das Prozessende. Windows: `WaitOnAddress` nur im selben Prozess; benannte Events und Semaphoren sowie `DuplicateHandle` prozessübergreifend, Prozess-Handles sind selbst wartbar.

**Vergleichssysteme:** NetBackup schreibt mehrere Kopien aus einem Puffersatz (Leistung des langsamsten Geräts); Bareos kopiert nur nachträglich; SPDK RAID1 markiert ein ausgefallenes Mitglied als gescheitert und schreibt weiter.

**Architektur:** `Launcher::Thread` besteht schon (`crates/osd/daemon/src/daemon.rs:30`), nutzt aber denselben mmap-Ring. Die einzige belegte Begründung für eigene Prozesse (D-13) ist: Fällt ein Geräteprozess aus, bleiben die anderen Geräte stehen. Handle-Weitergabe und ein Weck-Byte über eine Pipe lösen das Weckproblem nicht besser.

**Empfehlung (für D-15):** Auf Windows und macOS zuerst Abfrage mit Zeitlimit (yield/sleep bis 1 ms, der Ring wartet ohnehin höchstens 100 ms) plus `pid_alive`; benannte Events (Windows) und `os_sync_wait_on_address` SHARED (macOS ab 14.4) nur als spätere Latenzoptimierung, falls Messungen sie rechtfertigen. Offen: Polling-Latenz und Durchsatz sind nicht gemessen (Windows-VM `joerg.bernau@vm`, kein wine); Prozessstart und -überwachung auf Windows/macOS sind eine zweite, ungetestete Lücke.

**Umgesetzt (22.11.2026):** Abfragen mit Zeitgrenze in `sys/wait_other.rs` (macOS und Windows), `pid_alive` unter Windows mit `OpenProcess`/`GetExitCodeProcess`, die Tests des Rings laufen auf allen Systemen. Gemessen auf `joerg.bernau@vm` (Windows 11, MSVC): alle 11 Tests bestehen, darunter „ein zweiter Prozess liest über den gemeinsamen Speicher“ und „ein verschwundener Verbraucher wird als gescheitert markiert“ (1,0 s). macOS ist nicht getestet. Durchsatz und Latenz sind noch nicht gemessen. SubEtha wurde im Quelltext geprüft und verworfen (52-Byte-Slots, 16 Verbraucher, kein blockierendes Gating, tote Verbraucher bleiben im Gating); lesenswert nur `park_event.rs` (benanntes Event, Nachprüfung alle 20 ms).
