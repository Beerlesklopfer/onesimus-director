# Planung: Podman-Plugin für ofd

Stand: 18.10.2026. Status: **Plan freigegeben; Umsetzung begonnen.**
Freigabe durch den Nutzer am 18.10.2026. Die produktiven Container-Plugins sind
noch nicht implementiert; die Infrastrukturphase D0/P0 läuft.
Drei Agenten haben Funktion, Sicherheit und Betrieb/Recovery getrennt vertreten
und ihre Vorschläge gegenseitig kritisiert. Die Nutzerergänzung zur
kontenübergreifenden `conmon`-Erkennung ist ausdrücklich aufgenommen.

## Ziel und verbindlicher Rahmen

Wiederherstellbare Sicherung ausgewählter Podman-Workloads einschließlich
Images, persistenten Daten, Pods/Containern und deklarativen Quellen wie Quadlet.
Rootful und jede Rootless-Host-UID bilden getrennte Sicherungsdomänen.
Ein rootful Inventar ersetzt nicht die Inventare anderer Benutzer.
[Podman Rootless](https://docs.podman.io/en/latest/markdown/podman.1.html).

Es gelten [Plugin-Architektur](../wiki/plugin-architektur.md) und
[Protobuf-Vertrag](../../proto/filed_plugin.proto): signiertes Rust/WASM, kein
Python-/Shell-Plugin, eigene Instanzen/Worker parallel zu Device-Workern,
begrenzte Queues und außerbandiges Cancel. DIR↔FD über gRPC mit **immer
verfügbarem BNET-Fallback**; FD↔SD bleibt BNET. Kein Transportfallback umgeht
Signaturen oder startet eine zweite Sicherungsinstanz.

Vorhanden sind Verifier, Schnittstellenvertrag, generischer Worker-Kern und
read-only conmon-Discovery. Ausführender WASM-Host, per-UID-Broker und deren
konkrete Engine-Anbindung sind noch umzusetzen.
Das [Container-Dump-Skript](../wiki/container-dump.md) bleibt eine Übergangshilfe,
kein Ausführungspfad des neuen Plugins.

## Automatische Benutzererkennung über conmon

**Nutzervorgabe:** Hostweit laufende `conmon`-Prozesse nach ihrer Host-UID
zuordnen, um Benutzer mit aktiven Podman-Containern kontenübergreifend zu finden.
Diese read-only Discovery ergänzt ausdrücklich registrierte Konten.

`conmon` überwacht einen Container zwischen Containerverwaltung und OCI-Runtime;
es wird unter anderem von Podman und CRI-O eingesetzt. Ein Fund ist deshalb ein
Kandidat mit Herkunftsnachweis, noch kein bestätigter Podman-Endpoint.
[Offizielle conmon-Beschreibung](https://github.com/containers/conmon).

Geplanter Ablauf im administrierten Host-Discovery-Dienst:

1. Den sichtbaren Host-Prozessraum lesen. Boot-ID, PID, Startzeit und Host-UID erfassen;
   diese UID ist nicht die im Container sichtbare Benutzerkennung.
2. `comm`/`cmdline` nur vorfiltern. Soweit Berechtigungen es erlauben, Executable-
   Identität und Prozesskontext prüfen; Prozessname oder Kommandozeile allein
   gelten nicht als Beweis. Prozessinformationen vor Verwendung erneut auf
   PID/Startzeit/UID prüfen; verschwundene Prozesse sind ein normaler Racefall.
3. Kandidaten nach `(Hostidentität, Host-UID)` zusammenfassen. Benutzername ist
   eine zusätzliche Anzeige über NSS, nicht der persistente Identitätsschlüssel.
   UID-Wiederverwendung muss gegen die registrierte Kontoidentität geprüft werden.
4. Kandidaten als `discovered_unconfirmed` ins Inventar aufnehmen; Quellen-PID,
   Beobachtungszeit und Sichtbarkeitsgrenzen festhalten. CRI-O/andere Verbraucher
   dürfen nicht automatisch als Podman klassifiziert werden.
5. Für administrativ freigegebene Konten den registrierten per-UID-Broker nutzen
   und Podman-Engineidentität, tatsächliche Container und Endpoint bestätigen.
   Erst dies erzeugt `confirmed_podman`. Kein willkürliches Socketprobing und
   kein automatischer Wechsel in beliebige Benutzerkonten.
6. Neue unfreigegebene Konten sichtbar als Kandidaten melden. Eine explizite
   Discovery-/Kontopolicy kann festlegen, welche UIDs bestätigt werden dürfen;
   Discovery selbst gewährt keine Backup-, Restore- oder Lifecycle-Rechte.
7. Bestätigte Konten in einer Registry behalten und auch ohne aktuellen
   `conmon`-Fund weiter inventarisieren, soweit freigegeben. Dadurch bleiben
   gestoppte Container und unbenutzte Volumes im Sicherungsumfang.

Grenzen: `/proc`-Berechtigungen, `hidepid`, PID-Namespaces und die Ausführung des
FD in einem Container können die Sicht einschränken. Fehlende Sichtbarkeit ist
`discovery_incomplete`, kein Beleg für „keine Container“. Ein laufender Monitor
sagt nichts über vollständige Inventarisierung, verwendeten Storage oder einen
verfügbaren API-Service. Rootful-Funde laufen unter der rootful Domäne; sie
verraten nicht zuverlässig, welcher menschliche Benutzer ursprünglich `sudo`
benutzt hat. Alternative Monitor-/Runtimeprofile brauchen eigene Erkennung.

Eine Host-UID kann mehrere konfigurierte Storage-/Runtime-Kontexte besitzen.
Die Bestätigung muss deshalb zusätzlich die tatsächliche Endpointidentität
festhalten, statt alle Objekte einer UID als eine Engine zu behandeln.
Die Sichtbarkeitsgrenzen richten sich nach der
[Kernel-Dokumentation zu `/proc`](https://docs.kernel.org/filesystems/proc.html).

Der Discovery-Dienst benötigt nur freigegebenen Lesezugriff. Er liefert weder
Socket-Handles noch allgemeine Benutzerwechselrechte an WASM-Plugins. Sein
Inventar und die für einen Job freigegebenen Quellen sind getrennte Zustände.

## Kontroversen und Entscheidungen

| Streitpunkt | Position / Einwand | Planungsentscheidung |
| --- | --- | --- |
| Ausschließlich registrierte Benutzer? | Sicherheit/Betrieb wollten zunächst keine automatische Kontensuche; Nutzer fordert conmon-Erkennung. | Automatische read-only Host-Discovery plus dauerhafte Kontoregistry; Zugriffsfreigabe bleibt separat. |
| Ein zentraler Rootbroker für alle UIDs? | Komfortabel; ein Quell-Lock trennt aber keine Identitäten. | Eigener Brokerprozess je freigegebener Ziel-UID; kein frei parametrisierbarer Benutzerwechsel aus dem Plugin. |
| API-Volumeexport als einfaches MVP? | Funktion schlug es vor, Sicherheit bevorzugt geringe Schnittstellenfläche. Betrieb bestreitet unbelegte Metadatentreue. | Vollständiger lokaler Metadatenpfad ist Voraussetzung für das vollständige Restoreprofil. Export nur als separat zertifizierte eingeschränkte Datenklasse. |
| Bestehendes Volume importieren? | Einfacher Wiederanlauf. | Verworfen: Import führt Inhalte zusammen; ausschließlich neues leeres Stagingvolume. |
| Container direkt stoppen? | Einfacher Engine-Aufruf. | Bei Quadlet über den zuständigen systemd-Manager und kontrollierte Aktivierungsquellen; Writergruppe prüfen. |
| Nach Fehler automatisch starten? | Betrieb möchte Verfügbarkeit. | Nur eigene dokumentierte Stops identischer unveränderter Objekte bedingt aufheben; sonst sichtbarer Konflikt. |
| Podman als Docker-kompatibles Plugin? | Teilweise API-Kompatibilität vorhanden. | Eigener Adapter mit Libpod-, UID-, Pod- und Quadletsemantik; gemeinsame Verifikations-/Streambausteine dürfen wiederverwendet werden. |

## MVP und ausdrücklich ausgeschlossene Klassen

Linux lokal, freigegebene per-UID-Endpoints, zertifizierte Usernamespace-/Storage-
Profile und normale lokale Named Volumes. Erste Abnahme mit einer Rootless-UID;
danach zwei gleichzeitig arbeitende Rootless-UIDs und ein getrenntes rootful
Profil. Nicht registrierte Konten werden gefunden und angezeigt, nicht heimlich
mitgesichert. Der API-Service und Usermanager werden nicht automatisch aktiviert;
kein implizites `linger`, `runuser` oder Hochstufen auf root.

Remote-Libpod, unbekannte Storage-/Mappingprofile, externe Volumetreiber,
CRIU/Checkpoint, Live-Migration, beliebige Hooks und generisches Container-Exec
sind außerhalb der ersten Abnahme. Ausgewählte nicht unterstützte Quellen führen
zu einem Planfehler; kein stilles Auslassen oder Wechsel des Sicherungsverfahrens.

| Objekt | Sicherung und Identität | Restore / Grenze |
| --- | --- | --- |
| UID-/Endpoint-Domäne | Host, UID, Engine-/Endpoint-ID, Version und Mappingprofil. | Zielkonto und Berechtigungen explizit zuordnen. |
| Pods / Container | Konfiguration, unveränderliche IDs, Abhängigkeiten und Mounts. | Neue getrennte Ressourcen, gestoppt; kein ungeprüftes Inspect-Replay. |
| Images | Save-Archive, IDs, Tags, Digests und Plattform. | Load plus Identitätsprüfung. Offlineprofil enthält alle benötigten Images. |
| Quadlet | Quelldateien, Includes, Drop-ins und freigegebene Abhängigkeiten. | Zuerst gestaged; generierte Services ersetzen die Quellen nicht. Keine automatische Aktivierung. |
| Named Volumes | Daten und zertifiziertes Metadatenprofil unter passendem Usernamespace. | Neu angelegtes leeres Stagingvolume; keine Inhaltsvermischung. |
| Bind Mounts | Eigene autorisierte FD-Dateisicherung. | Freigegebene Zielpfadzuordnung, kein automatischer Zugriff auf Inspect-Pfade. |
| Netzwerke / Secrets | Deklarative Abhängigkeiten; Secrets standardmäßig als Wiederbereitstellungsreferenz. | Erlaubte Optionen, separate Bereitstellung; keine Secrets in Logs/Katalog. |
| Writable Layer | MVP-Ausschluss ausdrücklich manifestieren. | Vollständiges Restoreprofil bei relevanten ungesicherten Änderungen ablehnen. |

Image-save und Container-export sind keine austauschbaren Verfahren.
[Podman save](https://docs.podman.io/en/latest/markdown/podman-save.1.html),
[Podman export](https://docs.podman.io/en/latest/markdown/podman-export.1.html).
Registry-Wiederbeschaffung allein ist nur in einem ausdrücklich gewählten Profil
zulässig; fehlender Registryzugriff darf das Offline-Restoreprofil nicht brechen.

## Broker, Namespaces und Metadaten

Die Podman-API erlaubt weitreichende Aktionen mit den Rechten des Servicebenutzers.
WASM erhält daher weder Raw-Socket noch generische HTTP-/Exec-Aufrufe.
Ein per-UID-Broker bietet nur typisierte jobgebundene Operationen mit separaten
Inventory-, Image-, Volume-, Restore- und Lifecycle-Capabilities. Jede Bindung
enthält Host, Endpoint, UID, Job, Instanz und Objektidentität. Gleiche Namen unter
zwei UIDs müssen vollständig getrennte Objekte bleiben.
[Podman Service und Sicherheitsmodell](https://docs.podman.io/en/latest/markdown/podman-system-service.1.html).

Der Host legt den passenden Usernamespace und autorisierte Quellhandles fest.
Source- und Target-UID/GID-Mappings einschließlich relevanter Sub-ID-Zuordnungen
werden dokumentiert und geprüft. Keine blinde Übernahme von Host-UIDs, kein
pauschales rekursives `chown`. SELinux erhält eine zum Ziel passende explizite
Strategie; historische Labels sind nicht automatisch auf jedem Ziel gültig.
[Podman unshare](https://docs.podman.io/en/latest/markdown/podman-unshare.1.html).

**Implementierungsblocker:** Dem aktuellen Protobuf-`FileEntry` fehlen UID/GID,
ACL/xattr-/SELinux-Daten, Hardlinkidentität und Sparse-Extents. Vor dem vollständigen
Volumerestore muss entweder der versionierte Vertrag erweitert oder ein
vollständig spezifizierter Host-Metadatenpfad/Archivvertrag nachgewiesen werden.
Tar-Export allein ist kein Nachweis dieser Eigenschaften.
[Volume export](https://docs.podman.io/en/latest/markdown/podman-volume-export.1.html).

## Konsistenz, Quadlet und Journal

Pro Quelle wird eine deklarierte Writergruppe bestimmt, einschließlich geteilter
Volumes und beteiligter Container/Pods. Bei systemd-/Quadlet-Verwaltung gehören
Usermanager, Units und Aktivierungsquellen in die autorisierte Lifecyclegruppe.
Ein `podman stop` allein verhindert weder fremde Writer noch alle erneuten Starts.
[Quadlet](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html).

Anwendungskonsistenz verlangt einen nachgewiesenen sauberen Shutdown oder einen
späteren geprüften App-Adapter. Stop oder Pause allein beweisen sie nicht. Ohne
belegte Schreibruhe wird ein entsprechendes Konsistenzprofil abgelehnt. Eine
laufende Mehrdateienkopie ist ebenfalls kein atomarer Crash-Snapshot; abgeschwächte
Profile müssen ausdrücklich gewählt und im Manifest beschrieben werden.

Journal vor Mutation dauerhaft schreiben:
`PREPARED → STOP_REQUESTED → QUIESCED → CAPTURING → RESUME_PENDING → RESUMED`.
Gespeichert werden UID/Endpoint, konkrete Ressourcen- und Unitidentität,
Definitionsfingerprint, Ursprungszustand, eigene Stop-Absicht und bestätigte
Resultate. Nach Crash/Cancel nur selbst gestoppte, zuvor aktive und unveränderte
Quellen bedingt wieder aufnehmen; fremde Änderungen und unklare Ergebnisse
verlangen manuelle Klärung. Mutationen werden nicht blind wiederholt.

## Threaded Environment

Je Job-/UID-/Endpoint-/Quellbindung eine isolierte Plugininstanz mit eigenem
WASM-Store und geordnetem Worker. Unterschiedliche Benutzer, Quellen und Device-
Worker laufen parallel. Ein Quellenlease schützt gemeinsame Volumes über die
gesamte Stop-/Capture-/Resume-Transaktion. Kein globaler Engine-/Pluginlock wird
über I/O gehalten. Benutzergrenzen werden durch Broker und Capability-Prüfung
durchgesetzt, nicht durch diesen Lease.

Auftragsqueues, Datenpuffer und Spoolquota sind begrenzt. Nicht seekbare API-Streams
benötigen einen begrenzten Spool oder einen definierten Streamvertrag für das
bisher offsetbasierte `ReadFile`. Cancel/Shutdown verwenden den separaten
Steuerpfad und können eine volle Queue umgehen. Abbruch und Quellen-Recovery
haben getrennte Zeitbudgets. Abgelehnte Plugins erhalten keinen BNET-/Exec-Ersatz.

## Restore

1. Zielkonto/Endpoint, Mapping, Plattform, Backupvollständigkeit und Capability-Freigaben prüfen.
2. Plan mit Konflikten, Secret-Referenzen und nicht unterstützten Datenklassen anzeigen.
3. Nach Freigabe frische leere Stagingvolumes und eigene Ressourcenidentitäten anlegen.
4. Images, Volumeinhalte und freigegebene Metadaten wiederherstellen.
5. Inhalte, Mapping und Metadaten gegen das deklarierte Profil prüfen.
6. Pods/Container gestoppt und Quadlet-Quellen ohne Aktivierung bereitstellen.
7. Promotion/Start separat freigeben; Bereitschaft der Anwendungen prüfen, bevor Abhängigkeiten starten.

`podman volume import` benötigt ein vorhandenes Volume und führt Inhalte mit
dessen bestehendem Inhalt zusammen. Deshalb niemals in ein Produktionsvolume
importieren. [Volume import](https://docs.podman.io/en/latest/markdown/podman-volume-import.1.html).

## Phasen und Abnahme

| Phase | Liefergegenstand | Exitkriterium |
| --- | --- | --- |
| P0 | Runtime-/Brokervertrag, Metadatenpfad, UID-/Endpointpolicy und Discovery. | Keine automatische Rechteausweitung; conmon-Fund und bestätigte Podman-Quelle klar getrennt. |
| P1 | Inventory-/Plan-Modus mit Quadlet und dauerhaft registrierten Konten. | Gestoppte Container fehlen nicht; gleiche Namen verschiedener UIDs bleiben getrennt. |
| P2 | Lokales Full-only-Backup, Journal, Imagearchive und zertifizierte Volumes. | Crash-/Cancel-/Konsistenztests und Metadaten-Roundtrip bestanden. |
| P3 | Restore in neue Ressourcen und separate Promotion. | Sauberer Zielhost und Offline-Images; bestehende Ressourcen unverändert. |
| P4 | Remote, weitere Storage-/Mappingprofile, App-Adapter und Incrementals. | Je Profil eigene Zertifizierung; Remote-Pfade niemals als lokale FD-Pfade verwenden. |

Pflichttests:

- conmon von zwei Rootless-UIDs, rootful conmon, CRI-O-Fehlkandidat,
  manipulierter Prozessname, PID-Reuse und Exit während Discovery.
- Eingeschränkte `/proc`-Sicht, fehlendes NSS-Konto, UID-Wiederverwendung,
  Benutzer ohne laufenden Container, fehlender API-Socket: korrekte Kandidaten-
  und Unvollständigkeitszustände, keine unautorisierten Kontoaktionen.
- Zwei UIDs mit identischen Namen; manipulierte UID/Endpoint/Handles und
  Socket-Symlinks; kein Zugriff über fremde Benutzer- oder Jobgrenzen.
- Andere subuid/subgid-Bereiche, SELinux enforcing, ACL/xattr, Hardlinks,
  Sparsefiles, Symlinks, nicht-UTF-8-Namen und nicht unterstützte Dateitypen.
- Import in nichtleeres Ziel wird vor Änderung abgewiesen; Archive mit Traversal,
  Linkausbruch und übergroßen Daten bleiben außerhalb produktiver Pfade.
- Quadlet-/Timer-Neustartversuch, unbekannte Writer, Operatoränderungen und
  FD-/Broker-Crash vor/nach jedem Journalübergang.
- Cancel bei vollem Puffer/blockiertem Broker; Plugin A hält Device B nicht an.
- Offline-Restore auf neuem Host; tatsächliche Anwendungsprüfung nach explizitem
  Start, nicht nur Prüfsumme des Archivs. Kein automatischer Start durch Restore.
- gRPC-Ausfall mit BNET-Verbindungsfallback ohne doppelte Jobausführung.

## Offene Entscheidungen

Konkrete Podman-/API-/Storage-Versionen und Mappingprofile werden vor der
Implementierung gepinnt und getestet. Die verwendeten offiziellen `latest`-
Dokumente sind Planungsquellen, keine pauschale Versionsfreigabe. Für vollständige
Metadaten wird der lokale Hostpfad bevorzugt; portable Exportprofile bleiben
separate eingeschränkte Klassen. Remote erfordert später eine explizite Trennung
von FD-Host, Podman-Servicehost und Storage.

Gemeinsame Basiskomponenten dürfen mit dem [Docker-Plugin](DOCKER-PLUGIN.md)
geteilt werden; UID-Discovery, Pod-/Quadlet-Lifecycle und Mapping bleiben
Podman-spezifisch. Alle Implementierungsphasen sind bis zum jeweiligen
Testnachweis offen.

## Umsetzungsstand der freigegebenen Grundlage

`onesimus-plugins::executor` implementiert isolierte Worker, begrenzte Queues,
Instanz-/Job-/Device-Prüfung sowie direkte Interrupts und einen Deadline-Watchdog.
Die Tests nutzen einen vertrauenswürdigen Testadapter; eine WASM-Engine ist noch
nicht integriert.

`onesimus-plugins::discovery` und der folgende FD-Modus erfassen conmon-Kandidaten:

```bash
ofd --discover-podman
# Eigene administrierte Installation angeben; mehrfach verwendbar:
ofd --discover-podman --conmon-executable /usr/bin/conmon
```

Der Modus benötigt keine FD-Konfiguration, startet keinen Listener und gibt JSON
auf stdout aus. Er prüft root-eigene, nicht gruppen-/weltbeschreibbare Executables
über Device/Inode und vergleicht PID/Startzeit/UID/Executable doppelt. Kandidaten
bleiben `discovered_unconfirmed`; CRI-O wird damit nicht zu Podman erklärt.
Die Host-PID-Namespace-Provenienz kann der Scanner selbst nicht beweisen;
`host_scope_verified` bleibt false und Sichtbarkeitsprobleme werden ausgewiesen.
Persistente Kontoregistry, NSS-Anzeige, per-UID-Bestätigung und Aufnahme in Jobs
sind noch offen. P0 ist damit begonnen, nicht abgeschlossen.

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
