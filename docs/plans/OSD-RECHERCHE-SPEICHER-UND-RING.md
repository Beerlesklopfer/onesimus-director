# osd: Recherche Speicherschicht und Ringpuffer

Stand: 20.11.2026. Alle Versions- und Datumsangaben stammen von crates.io bzw. der GitHub-API vom selben Tag. **[unbelegt]** heißt: aus zweiter Hand oder aus dem Gedächtnis, nicht an einer Primärquelle geprüft. Ergänzt den Plan [OSD-GERAETE-UND-VERTEILUNG.md](OSD-GERAETE-UND-VERTEILUNG.md), Abschnitte „Datenpfad: Ringpuffer“ und „Eigenes Backup-Dateisystem“.

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
