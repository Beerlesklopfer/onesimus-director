# Planung: Docker-Plugin für ofd

Stand: 18.10.2026. Status: **Plan freigegeben; Umsetzung begonnen.**
Freigabe durch den Nutzer am 18.10.2026. Die produktiven Container-Plugins sind
noch nicht implementiert; die Infrastrukturphase D0/P0 läuft.
Die Planung entstand aus drei kontroversen Agentenperspektiven: Funktion,
Sicherheit und Betrieb/Recovery. Die wesentlichen Entscheidungen stehen unten;
die abschließenden Docker-Berichte wurden durch ein Agenten-Nutzungslimit
abgebrochen. Die bereits ausgetauschten Beiträge und Quellen sind berücksichtigt.

## Ziel und bestehende Grenzen

Ein wiederherstellbares Backup einer ausdrücklich ausgewählten Docker-Workload:
Inventar und Konfigurationsquellen, benötigte Images und persistente Daten.
Ein Inventar- oder Imagearchiv allein darf nicht als vollständiges Backup gelten.
Das [vorhandene Container-Dump-Skript](../wiki/container-dump.md) bleibt eine
Übergangshilfe. Es wird weder eingebettet noch zum Plugin-Ausführungspfad.

Verbindlich gelten die [signierbare Plugin-Architektur](../wiki/plugin-architektur.md)
und der [Protobuf-Vertrag](../../proto/filed_plugin.proto): Rust/WASM, kein
Python, getrennte Plugin-/Device-Worker, begrenzte Queues und außerbandiger
Abbruch. DIR↔FD bevorzugt gRPC mit **dauerhaftem BNET-Fallback**; FD↔SD bleibt BNET.
Transportfallback darf keine Pluginrechte erweitern oder einen Job erneut starten.

Aktuell existieren Signaturprüfung, Protobuf-Vertrag und der getestete generische
Worker-Kern (`onesimus-plugins::executor`). WASM-Host, Broker und Engine-Anbindung
fehlen noch; der Worker-Kern allein führt keine WASM-Komponenten aus. Der aktuelle `FileEntry`-
Vertrag enthält keine UID/GID, ACLs, xattrs, Hardlinkidentität oder Sparsebereiche.
Ein vollständiger Metadatenrestore benötigt zuerst einen versionierten erweiterten
Vertrag oder einen ausdrücklich beschriebenen und getesteten FD-Metadatenpfad.

## Kontroversen und Entscheidungen

| Streitpunkt | Funktion | Gegenposition | Planungsentscheidung |
| --- | --- | --- | --- |
| Inventar zuerst oder echter Restore? | Images und Volumes müssen im MVP enthalten sein. | Zu großer Umfang vor fertiggestellter Runtime. | Infrastrukturphase vor Produkt-MVP; Inventarisierung allein ist kein fertiges Backup-Plugin. |
| Volumezugriff über Helper-Container? | Portabel über Engine möglich. | Security lehnt frei konfigurierbare Create-/Mount-/Exec-Rechte ab; Betrieb bevorzugte zuerst Helper. | MVP: administrierter lokaler Dateisystembroker mit begrenzten Roots, ohne Helper oder CLI. |
| Images aus Registry nachziehen? | Kleineres Backup durch Digests. | Registry kann fehlen; Digest allein garantiert keinen späteren Zugriff. | Offline-Profil archiviert alle erforderlichen Images; externes Wiederbeschaffungsprofil nur ausdrücklich auswählbar. |
| Alle Writer stoppen? | Quieszierende Sicherung der gemeinsamen Volumegruppe. | Stop-Erfolg beweist keinen sauberen DB-Shutdown und erfasst keine beliebigen Hostwriter. | Deklarierte Schreiberdomäne und getestete Konsistenzklasse zwingend; keine pauschale Anwendungskonsistenz. |
| Writable Layer per commit? | Nützlich für bestehende handgepflegte Container. | Keine Volumes, kein universeller Anwendungssnapshot. | Spätere eigene Sicherungsklasse; im MVP explizite Ausschluss-/Ablehnungsregel. |
| Wiederanlauf im finally-Block? | Verfügbarkeit wiederherstellen. | Operatoränderungen und FD-Absturz verhindern blindes Replay. | Dauerhaftes Journal und bedingte Wiederaufnahme nur selbst gestoppter unveränderter Objekte. |

Noch offen ist die konkrete WASM-Engine und die Broker-Implementierung, nicht
das Sicherheits- oder Thread-Modell. Architekturentscheidungen werden vor den
jeweiligen Implementierungsphasen in Tests überführt.

## MVP und Sicherungsobjekte

Linux, lokaler ausdrücklich registrierter Docker-Endpoint, zertifizierter
Engine-/API-Bereich und lokale Named Volumes. Rootless Docker benötigt ein
eigenes UID-/Mappingprofil; Freigabe erst nach denselben Restore-Nachweisen.
Remote-Engines, Swarm, externe Volumetreiber, Live-Migration/CRIU und unbekannte
Storageprofile sind außerhalb der ersten Abnahme. Ausgewählte nicht unterstützte
Objekte verursachen einen Planfehler, statt still ausgelassen zu werden.

| Objekt | Sicherung | Restore / Grenze |
| --- | --- | --- |
| Container | Vollständiges gesichertes Inventar plus normalisierte freigegebene Create-Felder; Engine-/Objekt-ID, Mounts und Imagebezug. | Neues gestopptes Objekt, keine ungeprüfte Wiedergabe von Inspect-JSON. |
| Images | Save-Archive, Image-ID, Tags, Digests und Plattform; Abhängigkeiten im Manifest. | Load in freigegebenen Zielendpoint, Identitäts-/Plattformprüfung. |
| Lokale Named Volumes | Vollständiger Inhalt und zertifiziertes Metadatenprofil über Hostbroker. | Neues leeres Stagingvolume; keine Vermischung mit Produktionsdaten. |
| Bind Mounts | Separate explizit autorisierte FD-Dateisicherung. | Eigene Zielpfadzuordnung; kein Zugriff allein aufgrund eines Inspect-Pfads. |
| Compose/sonstige Quellen | Explizit konfigurierte Quelldateien und Abhängigkeiten. | Separat bereitstellen; Deploy/Up nicht automatisch ausführen. |
| Netze / sonstige Ressourcen | Deklarative Metadaten und referenzierte Abhängigkeiten. | Nur erlaubte Felder; Namenkonflikte führen zum Restoreplan, nicht zum Überschreiben. |
| Secrets / Environment | Geheimhaltungsprofil; sensitive Nutzdaten nur im geschützten Backupstrom. | Standardmäßig Referenz und Wiederbereitstellungsanforderung; keine Secrets im Klartextlog/Katalog. |
| Writable Layer | Im MVP ausgeschlossen und im Manifest kenntlich. | Restoreprofil mit relevanten ungesicherten Änderungen ablehnen. |

Image `save/load`, Container `export/import` und `commit` sind unterschiedliche
Datenklassen. `export` lässt Volumedaten aus; `commit` übernimmt ebenfalls keine
Volumeinhalte. Keines ersetzt die separate Sicherung persistenter Daten.
[Docker export](https://docs.docker.com/reference/cli/docker/container/export/),
[Docker commit](https://docs.docker.com/reference/cli/docker/container/commit/),
[Docker save](https://docs.docker.com/reference/cli/docker/image/save/).

## Hostbroker und Berechtigungen

Das Plugin erhält keinen Docker-Socket, keine freie HTTP-Schnittstelle und keine
Shell-/Exec-Funktion. Der Hostbroker stellt typisierte Operationen bereit:
`Inventory`, `ReadImage`, `ReadVolume`, `CreateStagingVolume`,
`RestoreVolume`, `CreateStoppedContainer` und separat gewährte Lifecycleaktionen.
Diese Namen sind **geplante** Hostoperationen, noch keine vorhandenen RPCs.

Jede Operation ist an Host, Endpoint, UID, Job, Instanz und unveränderliche
Objektidentität gebunden. Backup-, Restore- und Lifecycle-Capabilities sind
getrennt. Der administrierte Broker besitzt die nötige Engineautorität; er muss
selbst jede Operation prüfen. Eine WASM-Sandbox allein begrenzt die Autorität
des dahinterliegenden Brokers nicht.

Volume-Pfade werden über autorisierte Verzeichnis-Handles geöffnet. Der Broker
prüft Rootbindung, Symlink-/Hardlink-Ausbruch, Mountgrenzen, Eigentümer und
Namespaceabbildung; er traversiert nicht pauschal `/var/lib/docker`.
Freie Helper-Container, privilegierte Restorecontainer, Host-Namespace-/Device-
Freigaben und ungeprüfte Hostmounts gehören nicht zum MVP.
[Docker Daemon Angriffsfläche](https://docs.docker.com/engine/security/).

## Konsistenz und Recovery

Vor Mutation: Engineidentität/API-Fähigkeiten prüfen, ausgewählte Volumes und
alle bekannten Writer auflösen, fremde Writer/Controller erkennen und das
Konsistenzprofil festlegen. Eine unbekannte Schreiberdomäne blockiert das
quieszierende Profil. Ein ausdrücklich gewähltes Live-/Crash-Profil muss seine
Grenzen im Manifest festhalten; eine Live-Kopie mehrerer Dateien ist nicht
bereits ein atomarer Crash-Snapshot.

Stop/Lifecycle benötigt eine eigene Betriebsfreigabe. Für Datenbanken gilt
Anwendungskonsistenz erst nach einem getesteten sauberen Shutdown oder einem
geprüften Anwendungsadapter. Pause oder ein erfolgreicher Stop-Aufruf allein
beweisen das nicht. Generisches Container-Exec ist kein Ersatzadapter.

Journalzustände:
`PREPARED → STOP_REQUESTED → QUIESCED → CAPTURING → RESUME_PENDING → RESUMED`.
Vor jeder Mutation werden Absicht und bekannte Identität dauerhaft gesichert,
danach das beobachtete Ergebnis. Ursprungszustand, Konfigurationsfingerprint,
Writergruppe und Jobbesitz werden festgehalten. Wiederaufnahme nur für zuvor
aktive, nachweislich selbst gestoppte und unverändert gebliebene Ressourcen;
Konflikte werden sichtbar zur manuellen Auflösung zurückgelassen. Dies betrifft
die Backupquelle; restaurierte Workloads werden nicht automatisch gestartet.

## Thread-Modell und Datenpfad

Eine Plugininstanz pro Job-/Endpoint-/Quellbindung mit eigenem WASM-Store und
geordnetem Worker. Verschiedene Quellen und Device-Worker laufen parallel.
Gemeinsame Volumes erhalten einen quellbezogenen Lease/Lock für die gesamte
Stop-/Capture-/Resume-Transaktion, keinen globalen Daemonlock. Plugin- und
Broker-I/O halten keine Registrylocks.

Bytes- und Auftragsqueues sowie Spoolspace sind begrenzt. Image-/Volumedaten
laufen blockweise über den FD zum Storage. Ein nicht seekbarer Engine-Stream
braucht vor Rückgabe an offsetbasierte `ReadFile`-Aufrufe einen begrenzten Spool
oder einen ausdrücklich definierten Streamvertrag; kein unbegrenztes RAM-Puffern.
Cancel und Shutdown erreichen die Instanz unabhängig von einer vollen Datenqueue.
Abbruch beendet Produzent, Brokerstream und eigene temporäre Artefakte, gefolgt
von journalgestützter Quellen-Recovery.

## Restoreverfahren

1. Backupvollständigkeit, Signaturpolicy, Zielendpoint, Plattform und Metadatenprofil prüfen.
2. Nur einen prüfbaren Restoreplan erzeugen; Konflikte und ausgeschlossene Daten anzeigen.
3. Nach Freigabe neue eindeutig benannte leere Stagingressourcen anlegen.
4. Images, Daten und Metadaten wiederherstellen; keine bestehenden Volumes überschreiben.
5. Inhalte, Objektbezüge und Konfiguration validieren; Fehler verbleiben isoliert.
6. Neue Container gestoppt und mit deaktiviertem automatischem Wiederanlauf bereitstellen.
7. Promotion/Start als separaten expliziten Vorgang mit Bereitschaftskriterien anbieten.

MVP: Full-only. Image-Deduplizierung darf keine unprotokollierten externen
Restoreabhängigkeiten schaffen. Inkrementelle Volumes/Änderungserkennung erst
nach nachgewiesener Wiederherstellbarkeit einschließlich Löschungen.

## Umsetzung und Abnahme

| Phase | Liefergegenstand | Exitkriterium |
| --- | --- | --- |
| D0 | WASM-Host, signierter Ladepfad, Broker-Capabilities, Metadaten-/Streamvertrag. | Keine generische Socket-/Exec-Fähigkeit; Metadaten- und Cancel-Vertrag getestet. |
| D1 | Lokaler Inventory-/Plan-Modus, API-Probing und ausgeschlossene Datenklassen. | Engine-/Objektidentitäten stabil; unvollständiges Profil wird nicht als vollständig angeboten. |
| D2 | Full-Backup von Images und freigegebenen Volumes mit Journal. | Crash-/Abbruchtests an jeder Lifecyclegrenze; Inhalte und Metadaten reproduzierbar. |
| D3 | Gestageter Restore und optional getrennte Promotion. | Neuinstallation ohne Registryzugriff erfolgreich; vorhandene Ressourcen unverändert. |
| D4 | Weitere Treiber, Rootlessprofile, Remote, App-Adapter, Incrementals. | Je Profil eigene Zertifizierungs- und Roundtripmatrix. |

Pflichttests: manipuliertes signiertes Plugin gegen Brokerpolicy; abgelaufene/
widerrufene Signatur; Archiv-Pfadtraversal und Bomben; Geheimnisse in Fehlerlogs;
Shared Volume und unbekannte Writer; Operatoränderung während Stop; FD-/Broker-
Crash vor/nach jeder Journalgrenze; Engineverlust; volle Queue; Cancel bei
blockiertem Lesen; Plugin A blockiert Device B nicht; ACL/xattr/SELinux,
UID/GID-Mapping, Hardlinks, Sparsefiles, nicht-UTF-8-Namen; Restore auf sauberem
Host mit Offline-Images; gRPC-Ausfall mit BNET-Verbindungsfallback ohne Jobduplikat.

## Referenzen und Nachverfolgung

- [Docker Engine API](https://docs.docker.com/reference/api/engine/): Versionsschnittmenge und Featureprobe statt fest verdrahtetem `latest`.
- [Volumes](https://docs.docker.com/engine/storage/volumes/): persistente Daten separat behandeln.
- [Rootless Docker](https://docs.docker.com/engine/security/rootless/): eigene Identitäts-/Mappingprofile.
- [Podman-Planung](PODMAN-PLUGIN.md): getrennte UID-/Manager- und Discoverysemantik.

Konkrete Engine-/API-/Storage-Versionen werden beim Implementierungsstart
festgeschrieben. `latest`-Dokumentation ist ein Planungsbeleg, keine Abnahme einer
noch nicht getesteten Version. Alle Roadmappunkte sind offen, bis die jeweiligen
Artefakte und Testnachweise vorliegen.

## Umsetzungsstand der freigegebenen Grundlage

Der Worker-Kern besitzt pro Instanz einen Thread, begrenzte Auftrags- und
Nachrichtengrößen, globale Instanzlimits, eindeutige Instanz-/Job-/Device-Bindung
und einen separaten Deadline-Watchdog. Cancel unterbricht den Hostadapter über
einen direkten Interrupt-Handle, auch bei voller Queue. Tests prüfen Parallelität,
Abbruch, Timeout und Fehlerisolation. Eine echte WASM-Engine muss diesen Handle
mit WASM-Unterbrechung und Broker-I/O-Abbruch verbinden; D0 bleibt bis dahin offen.

## Freigegebener Nachtrag: X.509 als Zertifizierungsbasis

Privater Signierschlüssel ausschließlich im Compiler-/Build-System; öffentliche
X.509-Ketten- und Artefaktsignaturprüfung beim Laden ist vom Nutzer bestätigt.
Der Verifier verwendet Manifest mit SPKI-Fingerprint als Schlüssel-ID,
Code-Signing-EKU/digitalSignature, administrierten CA-Vertrauensankern und
Plugin-/Capability-Freigaben. Lokale Schlüssel-/Zertifikatswiderrufe und die
Gültigkeit des Offline-Policystands werden geprüft. Erneuerung mit gleichem
Key und Rotation eines kompromittierten Keys sind getrennt getestet.

[Prüfvertrag und Rotation](../wiki/plugin-architektur.md#zertifikatserneuerung-und-schlüsselwechsel).
Automatische CRL-/OCSP-Verteilung und produktive WASM-Loader-Einbindung bleiben
Teil der offenen Infrastrukturphase. Die private Signieridentität wird nicht
an Plugins oder den produktiven Daemon verteilt.
