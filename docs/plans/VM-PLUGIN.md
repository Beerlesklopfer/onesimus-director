# Planung: Sicherung virtueller Maschinen durch ofd (libvirt/KVM, VirtualBox, VMware)

Stand: 22.11.2026. Status: **Entwurf, nichts freigegeben, nichts umgesetzt.**
Anlass ist die Frage des Maintainers vom 22.11.2026 („Wie können wir virsh,
vmware und virtualbox sichern?“). Dieser Plan übernimmt die Antwort und
arbeitet sie aus, soweit sie belegbar ist. Er folgt im Aufbau dem
[Podman-Plugin](PODMAN-PLUGIN.md) und baut auf dessen Bausteinen auf.

Kennzeichnung in diesem Plan:

- **(Nutzer, TT.MM.JJJJ)**: verbindliche Entscheidung des Nutzers.
- **Vorschlag, offen**: Ergebnis der Planung, nicht freigegeben. Dazu zählt
  alles, was aus dem Podman-Plan übernommen wird (z. B. das Dateibaum-Format).
- **[unbelegt]**: Aussage, die ich nicht aus der offiziellen Dokumentation
  belegen konnte; vor der Umsetzung messen oder nachlesen.
- Belege (Abruf 22.11.2026): libvirt.org (virsh, formatbackup, formatcheckpoint,
  kbase/live_full_disk_backup), virtualbox.org (Handbuch Kap. 8), Broadcom
  (VDDK-Handbuch, Wissensdatenbank). Nichts wurde ausprobiert; alle Befehle
  stammen aus der Dokumentation.

## Ziel und Rahmen

Wiederherstellbare Sicherung ausgewählter virtueller Maschinen: Konfiguration,
Platten (auch laufender Maschinen) und der zugehörige Zusatzzustand. Je
Hypervisor ein eigenes Plugin nach dem Podman-Muster: Metadaten und
Steuerung über einen Broker, **die Bytes der Platten liest der Host selbst**,
soweit sie als Datei vorliegen. Reihenfolge (Vorschlag, offen): libvirt/KVM
zuerst, VirtualBox danach, VMware zuletzt und nur auf ausdrücklichen Wunsch
(Abschnitt „VMware“).

Es gelten die [Plugin-Architektur](../wiki/plugin-architektur.md), der
[Protobuf-Vertrag](../../proto/filed_plugin.proto) und die Regeln aus dem
Podman-Plan: nur signierte Plugins; der WASM-Gast hat keine Datei-, Netz- oder
Prozess-Importe, nur einen typisierten Broker-Import, wenn Manifest und Policy
die Capability gewähren; der Host prüft jede Anfrage; **Broker laufen nie als
root**; Restore in Dateien führt keinen Plugin-Code aus; Plugin-Einträge tragen
im Strom nur die Rechte-Bits `0o777` (S-13). Grants stehen im Signierzertifikat,
die lokale Policy schränkt nur ein ([Signier-Infrastruktur](SIGNIER-INFRASTRUKTUR.md)).
Der Broker folgt [PostgreSQL Phase 2](POSTGRESQL-PHASE2.md): nativer Broker aus
geprüftem Deskriptor, `fexecve`, leere Umgebung, `PR_SET_NO_NEW_PRIVS`,
`PR_SET_PDEATHSIG`, Protobuf über `socketpair`.

Pakete (Vorschlag, offen): `ofd-plugin-libvirt`, `-virtualbox`, `-vmware`
(Schema `ofd-plugin-<name>`). Bis dahin gilt die „Übergangshilfe“ unten.

Nicht Ziel: Live-Migration, RAM-Abbild, Container (siehe Podman/Docker).

## Entscheidungen

| Thema | Stand |
| --- | --- |
| Ein Plugin je Hypervisor, Podman-Muster (Broker liefert Metadaten, Host liest Bytes) | Antwort des Maintainers vom 22.11.2026; Aufnahme als Plan: Vorschlag, offen |
| Reihenfolge libvirt, VirtualBox, VMware (zuletzt, nur auf Wunsch) | Vorschlag, offen |
| Format: virtueller Dateibaum im SD-Strom, jede Einheit eine eigene Datei, **kein zusätzliches tar** | für Podman (Nutzer, 19.11.2026); hier Vorschlag, offen |
| rootless (`qemu:///session`) und rootful (`qemu:///system`) als getrennte Sicherungsdomänen | Vorschlag, offen, wie bei Podman |
| Rootful: feste Logik in `ofd`, kein Broker und kein Plugin-Code als root | für Podman (Nutzer, 19.11.2026); hier Vorschlag, offen; siehe Frage 6 |
| Restore nie in eine laufende Maschine, nie über vorhandene Platten | Vorschlag, offen |
| Änderungsverfolgung (inkrementell) | libvirt: ja, über Dirty-Bitmaps; VirtualBox: nein; VMware: nur VADP. Details und Speicherort der Bitmaps offen (Frage 3) |
| VMware | nur über VADP/VDDK; Lizenz- und Abhängigkeitsfrage ausdrücklich beim Nutzer (Frage 1) |

## Zusammenspiel von Gast, Host und Broker

Grundsatz wie bei Podman: **Alle Entscheidungen, die Sicherheit betreffen,
trifft der Host.** Der Gast (WASM, signiert) bestimmt Format, Dateibaum,
Manifest und Reihenfolge und sieht nur, was der Host für diesen Job freigibt.

| Aufgabe | Gast (WASM) | Host `ofd` | Broker (Konto-UID, nie root) |
| --- | --- | --- | --- |
| Plugin-Zeile auswerten | Baum, Manifest | Zeile prüfen, Konto gegen Kontopolicy, Grants und Plugin-ID prüfen | – |
| Inventar, Konfiguration | ordnet die gelieferte Menge in den Baum | berechnet die erlaubte Objektmenge, bereinigt Dokumente per Positivliste | `virsh` bzw. `VBoxManage` mit fester Argumentliste, nur lesend |
| Platten bestimmen | ordnet zu | prüft, dass jede Platte unter einem in der Kontopolicy freigegebenen Wurzelpfad liegt | `virsh domblklist` (Syntax: `domblklist domain [--inactive] [--details]`) |
| Plattenbytes | nimmt Einträge per `source_ref` auf | liest Bytes **und** Attribute selbst per `openat2` (`RESOLVE_BENEATH`, `NO_SYMLINKS`, `NO_XDEV`) über einen Scan-Handle | – (bei libvirt-Pull-Modus: siehe unten, der Host ist der NBD-Client) |
| Snapshot/Backup-Steuerung | höchstens lesend | allein: Journal, Lease, Totmannschalter, Recovery, Aufräumen | führt die freigegebenen Befehle aus (`backup-begin`, `snapshot-create-as`, `blockcommit`, …) |
| Restore | Plan, Reihenfolge | prüft jede Anlage (Suffix, neue Pfade, keine vorhandenen Platten) | `define`, Platten schreiben im Zielkonto |

Gast und Host reden über einen typisierten Import nach dem vorhandenen Muster
(`onesimus:filed-plugin/libvirt`, entsprechend `/virtualbox`, `/vmware`); die
Funktionsnamen sind **geplant**, keine vorhandenen RPCs. Der Broker bekommt nie
ein freies Kommando, keine Shell, kein generisches Exec; Programmpfade
root-eigen geprüft, Ausgaben begrenzt, stderr nie roh in Joblog oder Katalog.

**Wer liest die Plattenbytes.** Liegt eine Platte als Datei vor (qcow2, raw,
vdi, vmdk), liest sie der Host wie bei Podman-Volumes selbst; Eigentümer,
Modus und Größe stammen vom Datenträger, nicht vom Broker. Laufende Maschinen
liefern über den Snapshot- bzw. Backup-Mechanismus eine stabile Quelle (siehe
„Konsistenz“). Der Pull-Modus von libvirt liefert die Bytes nicht als Datei,
sondern über NBD; wie der Host sie annimmt (eigener NBD-Client in `ofd`, Socket
des Kontos, Rechte des Sockets) ist offen (Frage 5). Der NBD-Server wird laut
Dokumentation über ein `server`-Element des Backup-XML mit den Attributen eines
`protocol`-Elements (Transport, Socket, Name, Port, TLS) angelegt.

### Grants (Vorschlag, offen)

Nach dem Muster `podman.*`, Namen vorläufig: `libvirt.inventory` (Pflicht:
Inventar, Konfiguration, Plattenliste), `libvirt.disk.read`, `libvirt.snapshot`
(Eingriff in die Maschine), `libvirt.restore`, `libvirt.rootful` (zusätzlich);
entsprechend `virtualbox.*`, `vmware.*`. Grants sind keine Auswahl je Job; die
lokale Policy schränkt je Host ein. Die allgemeine Lösung „Grants je Plugin-ID
im Zertifikat“ bleibt in der Signier-Infrastruktur offen; bis dahin bindet
`ofd` den Import fest an die Plugin-ID.

## Hypervisor 1: libvirt/KVM

Verbindung: `virsh connect [URI] [--readonly]`; Sicherungsdomänen sind
`qemu:///system` (rootful, Maschinen des Systems) und `qemu:///session`
(rootless, je Konto). Dass beide URIs so heißen und getrennt verwaltet werden,
ist hier aus dem Maintainer-Entwurf übernommen **[unbelegt]**: die URI-Seite
der Dokumentation wurde nicht abgerufen. Domänen verschiedener Konten bleiben
getrennte Objekte (Domäne ist Teil jeder Identität).

**Was gesichert wird**

| Objekt | Quelle | Anmerkung |
| --- | --- | --- |
| Konfiguration | `virsh dumpxml [--inactive] [--security-info] [--update-cpu] [--migratable] [--xpath EXPRESSION] [--wrap] domain` | `--inactive` liefert die Konfiguration des nächsten Starts; `--security-info` enthält Sicherheitsdaten (Kennwörter) und ist **nicht** Vorgabe |
| Plattenliste | `virsh domblklist domain [--inactive] [--details]` | Quelle für Zuordnung Ziel-Name zu Datei |
| Platten | Dateien, Zugriff durch den Host | qcow2, raw u. a.; Netzspeicher (RBD, iSCSI) im MVP Planfehler |
| NVRAM (UEFI-Variablen) | Datei, im XML referenziert | Ort und Besitz **[unbelegt]**; Host liest sie wie eine Platte |
| TPM-Zustand | Zustandsverzeichnis des emulierten TPM | Ort und Format **[unbelegt]**; ohne ihn lässt sich eine Maschine mit BitLocker/TPM-gebundenen Schlüsseln nicht sinnvoll wiederherstellen |
| Netze, Speicherpools | Definitionen als Metadaten | nur Definition, nie Poolinhalt; Befehle dafür **[unbelegt]** (nicht abgerufen) |

Alle Einheiten sind einzelne Dateien im virtuellen Baum (Vorschlag, offen):

```text
/@libvirt/<domäne>/vms/<name>/domain.xml, disks.json   bereinigt (Positivliste)
/@libvirt/<domäne>/vms/<name>/disk/<ziel>/full.<fmt>   volle Platte zum Zeitpunkt T
/@libvirt/<domäne>/vms/<name>/disk/<ziel>/inc-<n>.<fmt> Änderungen seit Prüfpunkt
/@libvirt/<domäne>/vms/<name>/nvram, tpm/<datei>       Zusatzzustand
/@libvirt/<domäne>/networks/…, pools/…                 Definitionen
/@libvirt/<domäne>/manifest.json                       zuletzt
```

**Format der Plattenabbilder im Strom** ist offen (Frage 4): die Platte als
Abbild im Quellformat (qcow2 bleibt qcow2) oder als rohe Blöcke mit Sparse-
Information. Der FD-Strom kennt heute keine Sparsebereiche im `FileEntry`
(siehe [Docker-Plan](DOCKER-PLUGIN.md)); ein Erweiterungsbedarf analog zu
`FileEntry` v2 ist zu prüfen.

## Konsistenz (libvirt)

Je Maschine eine Konsistenzklasse, im Manifest vermerkt (Vorschlag, offen):

| Klasse | Verfahren | Gast-Mitwirkung |
| --- | --- | --- |
| `offline` | Maschine ausgeschaltet, Platten werden gelesen | keine |
| `backup-begin` | laufende Maschine, Platten zum Zeitpunkt T aus QEMU | optional qemu-guest-agent (Frage 2) |
| `snapshot` | Rückfall: externer Disk-Snapshot, Platte lesen, Blockcommit | qemu-guest-agent für `--quiesce` |
| `crash` | direkte Kopie der laufenden Platte; abgeschwächt vermerkt, nie Vorgabe | keine |

**`backup-begin` (bevorzugt).** Syntax laut virsh-Handbuch:
`backup-begin domain [backupxml] [checkpointxml] [--reuse-external] [--preserve-domain-on-shutdown]`.
Die libvirt-Dokumentation nennt für die „moderne“ Backup-API libvirt 7.2.0 und
QEMU 4.2 als Voraussetzung; `--preserve-domain-on-shutdown` gibt es erst ab
libvirt 11.10 (Fortschritt: `virsh domjobinfo domain [--completed]`; Abbruch:
`domjobabort domain [--postcopy]`). Das Backup-XML kennt `mode` `push` (Vorgabe)
und `pull`, je Platte `backupmode` (`full` oder `incremental`), `incremental`
(Name eines vorhandenen Prüfpunkts), beim Pull `exportname`, `exportbitmap` und
`scratch`, beim Push `target`. Ein Vollbackup kann über das zweite Dokument
zugleich einen Prüfpunkt anlegen; ein inkrementelles Backup braucht einen
vorhandenen Prüfpunkt.

- **Push**: libvirt/QEMU schreibt die Zielplatten selbst in ein Verzeichnis;
  der Host liest danach die fertigen Dateien. Einfach, braucht aber Platz auf
  dem Hypervisor-Host in Plattengröße (Staging) **[unbelegt]**, ob das Ziel
  streamend sein kann.
- **Pull**: QEMU startet einen NBD-Server, der den Plattenzustand zu T anbietet;
  Änderungen währenddessen landen im `scratch`. Ein Client liest daraus; bei
  inkrementell meldet der Server über `NBD_OPT_SET_META_CONTEXT` die geänderten
  Bereiche. Der Host wäre der NBD-Client (kein Staging in Plattengröße).
  Aufwendiger; Entscheidung in Frage 5.
- **Prüfpunkte**: `checkpoint-create-as domain [--print-xml] [name] [description] [--quiesce] [--diskspec] diskspec]...`,
  `checkpoint-list`, `checkpoint-delete domain checkpoint [--metadata] [{--children | --children-only}]`.
  Laut Dokumentation sind inkrementelle Sicherungen nur mit QEMU und
  **qcow2-Platten auf der aktiven Ebene** möglich, die Verfolgung liegt als
  persistente Bitmap im qcow2-Image; raw-Platten bekommen keinen Prüfpunkt
  und werden immer voll gesichert. Prüfpunkte sind bei vorhandenen externen
  Snapshots verboten.
- Was mit den Bitmaps bei Absturz der Maschine, Livemigration oder Kopie der
  Platte geschieht, steht auf den abgerufenen Seiten nicht: **[unbelegt]**.
  Fehlt ein Prüfpunkt oder ist die Bitmap ungültig, fällt der Lauf auf Voll
  zurück (Vorschlag, offen) und meldet das im Manifest.

**Rückfall: externer Snapshot.** Syntax laut Handbuch:
`snapshot-create-as domain {[--print-xml] [--no-metadata] [--halt] [--reuse-external]} [name] [description] [--disk-only [--quiesce]] [--atomic] [--validate] [[--live] [--memspec memspec]] [--diskspec] diskspec]...`.
`--quiesce` bedeutet laut Handbuch: libvirt versucht, über den Gast-Agenten die
eingehängten Dateisysteme einzufrieren und wieder aufzutauen. Danach liest der
Host die jetzt eingefrorene Basisplatte; zum Abschluss wird die Überlagerung
mit `blockcommit domain path [bandwidth] [--bytes] [base] [--shallow] [top] [--delete] [--keep-relative] [--wait [--async] [--verbose]] [--timeout seconds] [--active] [{--pivot | --keep-overlay}]`
zurückgeführt. Die libvirt-Seite „live full disk backup“ zeigt den Snapshot
mit `--disk-only --no-metadata` und `--diskspec`. Das ist die empfindliche
Stelle: bleibt der Job hier hängen, läuft die Maschine weiter auf einer
Überlagerung (Journal und Recovery, unten). Den Ablauf mit `--active --pivot`
auf laufender Maschine habe ich nur aus der Syntax abgeleitet **[unbelegt]**.

**Gast-Agent** (`qemu-guest-agent`): ohne ihn gibt es kein `--quiesce`, also
Absturzkonsistenz der Gastdateisysteme (wie ein Stromausfall). Ob er Pflicht
oder Option ist, ist Frage 2. Pro Maschine im Manifest: `quiesce: yes|no|failed`.
`domfsfreeze`/`domfsthaw` (Syntax laut Handbuch: `domfsfreeze domain [[--mountpoint] mountpoint...]`)
sind die manuelle Entsprechung; ihr Einsatz neben `backup-begin` ist offen.

### Journal, Abbruch, Totmannschalter

Eingriffe in eine laufende Maschine sind wie die Stops im Podman-Plan
Mutationen mit Wiederherstellungspflicht (Vorschlag, offen):

- Dauerhaftes Journal des Hosts vor jeder Mutation (Zustand vorher, angelegter
  Snapshot/Backup, Name der Überlagerung); Wiederaufnahme nur eigener,
  unveränderter Objekte.
- Cancel/SIGTERM/Absturz von `ofd`: Recovery beim Start beendet ein offenes
  Backup (`domjobabort`), führt Überlagerungen zurück (Blockcommit mit Pivot)
  oder meldet sie, löscht angelegte Prüfpunkte nur, wenn der Lauf sie
  erzeugt hat.
- Totmannschalter: ein Backup-Job oder eine Überlagerung über `max_overlay`
  (Frage 7) wird durch `ofd` beendet; wie das ohne laufendes `ofd` geschieht,
  ist **[unbelegt]** (beim Podman-Plan ein systemd-Mechanismus, für libvirt
  nicht geprüft). Ein verlorener Prüfpunkt erzwingt Voll.

## Hypervisor 2: VirtualBox

Befehle laut Handbuch (Kapitel 8):

- Snapshot: `VBoxManage snapshot <uuid|vmname> take <snapshot-name> [--description=description] [--live]`,
  `… delete <snapshot-name>`, `… list [[--details] | [--machinereadable]]`.
  Für den Zeitraum der Sicherung liest der Host die **Basisplatte**; neue
  Schreibvorgänge gehen in die Differenzplatte. Welche Datei dabei welche
  Platte ist, bestimmt `showvminfo <uuid | vmname> [--details] [--machinereadable]`;
  die genaue Zuordnung Snapshot zu Platten-Dateien **[unbelegt]**.
- Portable Kopie: `VBoxManage export <machines> <--output=name> [--legacy09 | --ovf09 | --ovf10 | --ovf20] [--manifest]`
  (OVF/OVA); das ist ein **Archiv**, das nicht einzeln lesbar im Baum liegt
  (siehe Frage 4: ob es trotzdem Teil des Plans bleibt).
- Konfiguration: Datei `.vbox` der Maschine, Inventar `VBoxManage list vms`.
  Ob `export` oder die `.vbox` plus Platten die bessere Sicherungsquelle ist,
  hängt von der Restore-Anforderung ab (Frage 4).
- **Kein Block-Change-Tracking**: das Handbuch erwähnt keines, die Aussage
  „gibt es nicht“ ist daher nur **[unbelegt]**. Wir planen deshalb immer die
  ganze Platte; helfen kann nur Dateideduplizierung beim Speicher.
- Konsistenz: `snapshot take --live` liefert Absturzkonsistenz; Quiesce über
  Guest Additions **[unbelegt]**.
- Rootless-Sicht: VirtualBox-Maschinen gehören einem Konto; Domäne = Konto, kein
  Rootful-Pendant (VirtualBox läuft je Benutzer) **[unbelegt]**. Broker wie bei
  libvirt als Konto-UID, nie root.
- Wiederherstellung: neue Maschine aus `.vbox` und Platten per Registrierung
  oder über Import des OVF; die genauen Befehle (`registervm`, `import`)
  stehen **[unbelegt]** nicht in dieser Planungsrunde und werden vor Umsetzung
  nachgelesen. Nie Überschreiben einer vorhandenen Maschine.

## Hypervisor 3: VMware (vSphere/ESXi) – zuletzt, nur auf Wunsch

Es gibt keinen Dateisicherungsweg über den Host: ESXi-Maschinen werden über die
VMware-Schnittstellen VADP (vSphere Storage APIs – Data Protection) gesichert,
mit Snapshot, Changed Block Tracking (CBT) und der Bibliothek VDDK (Virtual
Disk Development Kit). Belegt:

- CBT ist standardmäßig aus und muss pro Maschine aktiviert sein;
  `QueryChangedDiskAreas` liefert geänderte Bereiche seit einer `changeId`; ändert
  sich der UUID-Teil der `changeId`, ist die Verfolgung ungültig und es ist ein
  Vollbackup nötig (VDDK-Handbuch 8.0, Broadcom-Wissensdatenbank).
- VDDK fällt unter eine **eigene Entwicklerlizenz** und war nie Teil eines
  Broadcom-Softwarekaufs (TechTarget-Bericht über eine Bestätigung von
  Broadcom; die Bezugsmöglichkeit hat sich geändert). Die Weitergabebedingungen
  habe ich nicht gelesen: **[unbelegt]**, ob und wie VDDK mit einem Paket
  ausgeliefert werden darf.
- Daraus für den Plan (Vorschlag, offen): **VDDK wird nicht ausgeliefert und
  nicht in ein Paket des Projekts aufgenommen.** Der Broker kapselt es, der
  Betreiber installiert es selbst; der Broker lädt die Bibliothek erst nach
  einer Pfadprüfung (root-eigen, nicht beschreibbar) und erst, wenn der Nutzer
  das ausdrücklich konfiguriert hat. Auch Hashbindung des VDDK-Teils im Manifest
  ist offen (er stammt nicht vom Signierer).
- Zugangsdaten zu vCenter/ESXi: nur über eine root-eigene Datei, nie über die
  Plugin-Zeile, nie im Joblog.
- Vergleich: Bareos liefert ein eigenes VMware-Plugin (in der Bareos-Dokumentation
  als „VMware Python Plugin“ genannt). Es wird hier nur als Vergleich genannt,
  nicht übernommen; seine Anforderungen und sein Umfang wurden nicht gelesen
  (**[unbelegt]**).
- Konfigurationen (`.vmx`), Datastores und CBT-Zustand sind Metadaten wie bei
  libvirt. Konkrete Aufrufe der VDDK-/vSphere-API werden hier nicht festgelegt
  **[unbelegt]**; erst nach der Entscheidung zu Frage 1 geplant.

## Restore

Grundsatz (Vorschlag, offen): **neue Maschine, neue leere Platten, nie
Überschreiben.** Eine laufende Maschine wird nie angefasst; eine vorhandene
Maschine gleichen Namens ist Planfehler oder wird mit Suffix angelegt.

- `restore_mode=files` (Vorgabe): Baum unter `Where`, ohne Plugin-Code. Der
  Administrator definiert die Maschine mit dem gesicherten XML selbst; für
  libvirt gibt es `define FILE [--validate]`.
- `restore_mode=online` (später): der Broker legt als Zielkonto eine neue Maschine
  an, schreibt Platten in neue Dateien, der Host prüft jede Anlage (Pfade unter
  freigegebenen Wurzeln, keine Namens- und UUID-Konflikte, keine zusätzlichen
  Geräte oder Rechte gegenüber dem Original). Netze: vorhandene gleichen Namens
  nutzen oder fehlende anlegen, wie `networks=create|existing` bei Podman.
  MAC-Adressen und UUID werden bei Restore neu vergeben, damit keine
  Doppelung mit einer noch laufenden Maschine entsteht (Frage 8).
- Inkrementelle Kette: Restore setzt `full` plus alle `inc-<n>` bis zum
  gewünschten Stand zusammen; wie, ist Teil der Formatfrage (Frage 4). Eine
  unvollständige Kette ist Planfehler.
- **Einzeldateirestore aus Gast-Platten** (Option, zu prüfen): Das Plattenabbild
  über `qemu-nbd` oder libguestfs lesend einhängen und einzelne Dateien des
  Gasts auslesen. Beides **[unbelegt]**; es sind Werkzeuge außerhalb des
  Projekts, die selbst Gastdaten parsen (Angriffsfläche, siehe Sicherheit).
  Eine Alternative ist die normale FD-Dateisicherung **im** Gast mit eigenem
  Client; das bleibt unberührt.

## Sicherheit

- Der Host liest Plattenbytes nur unter freigegebenen Wurzeln; Pfade, die der
  Broker meldet, gelten als Angabe des Kontos und werden gegen die Policy und
  per `openat2` geprüft (Symlink und Mountwechsel verboten).
- Konfigurations-XML kann Kennwörter enthalten: `--security-info` ist nicht
  Vorgabe, nur Felder einer Positivliste; Hostdev-Passthrough (USB, PCI) steht
  im Manifest als „nicht wiederherstellbar“.
- Jeder Befehl des Brokers hat eine feste Argumentliste; Namen der Maschinen
  werden gegen eine Positivliste an Zeichen geprüft, nie in eine Shell gegeben.
- Rootful libvirt: wie bei Podman soll ein Backend in `ofd` mit fester Logik
  gelten, nicht ein Broker mit Capabilities; für libvirt ungeprüft (Frage 6).
- VDDK und Gastwerkzeuge (`qemu-nbd`, libguestfs) verarbeiten nicht
  vertrauenswürdige Daten: Aufruf nur im Broker bzw. in einer eigenen
  Sandbox, nie im Host.

## Übergangshilfe (heute möglich, ohne Plugin)

Bis ein Plugin besteht, genügt ein **RunScript** (`RunsWhen = Before` und
`After`, `RunsOnClient = Yes`) wie beim
[Container-Dump-Skript](../wiki/container-dump.md):

1. Vor dem Job: Snapshot anlegen, z. B. bei libvirt
   `virsh snapshot-create-as --disk-only --quiesce --atomic --no-metadata`
   (Optionen nach dem virsh-Handbuch; für `--no-metadata` bleibt zu prüfen,
   ob das spätere Zurückführen mit `blockcommit --pivot` noch möglich ist,
   **[unbelegt]**), bei VirtualBox `VBoxManage snapshot <vm> take …`.
2. Das Image (die Basisplatte) liegt als **Datei im FileSet** und wird vom FD
   wie jede Datei gesichert.
3. Nach dem Job: Überlagerung zurückführen (libvirt `blockcommit … --pivot`)
   bzw. `VBoxManage snapshot <vm> delete <name>`.

Grenzen: immer ganze Platte, ohne Gast-Agent nur absturzkonsistent, bei
fehlgeschlagenem Nachskript bleibt die Maschine auf einer Überlagerung, kein
Totmannschalter, Konfiguration (`virsh dumpxml`, `.vbox`) separat sichern. Das
RunScript braucht `Allowed Script Dir` auf dem FD.

## Paketierung

- `ofd-plugin-libvirt`: `Architecture: any`, `Depends: ofd (= <Version>)`,
  `/usr/lib/<triplet>/onesimus/plugins/libvirt.pb` und
  `…/plugins/libvirt/onesimus-libvirt-broker`. Keine Abhängigkeit von
  libvirt-Bibliotheken: der Broker ruft `virsh` mit festem Pfad auf (Vorschlag,
  offen; Alternative: libvirt-API per Bibliothek, Frage 9). `Recommends:
  libvirt-clients`.
- `ofd-plugin-virtualbox` entsprechend (`VBoxManage` kommt vom Betreiber);
  `ofd-plugin-vmware` **ohne** VDDK.
- Gemeinsamer Code in einem Crate `onesimus-vm-core` (Vorschlag, offen).
  Installieren bleibt beim Nutzer; `postinst` startet `odir` nicht neu, die
  Installationsanweisung ist nur `sudo dpkg -i`.

## Phasen und Exitkriterien

Jedes Kriterium ist ein benannter, automatisierter Test oder eine Messung
(Vorschlag, offen). Tests nur mit Wegwerf-Maschinen.

| Phase | Liefergegenstand | Exitkriterium |
| --- | --- | --- |
| V1 | libvirt-Plugin: Inventar, bereinigtes XML, Plattenliste, Plan; `offline`-Sicherung als Dateien | Gast bootet nach `files`-Restore und `define`; Test: Broker meldet fremden Pfad, wird abgewiesen |
| V2 | `backup-begin` Push, volle Sicherung laufender Maschine; Journal, Cancel, Recovery | `kill -9 ofd` an jedem Journalübergang: kein offener Backup-Job, keine Überlagerung übrig; Gast konsistent geprüft (Dateisystemprüfung) |
| V3 | Prüfpunkte, inkrementell; Pull-Modus (nach Frage 5) | Kette full+inc rekonstruiert die Platte bitgleich zu einer Vergleichskopie; fehlender Prüfpunkt führt zu Voll |
| V4 | Restore `online` (neue Maschine), Rootful-Backend | Restore neben laufender Originalmaschine ohne Konflikt (MAC, UUID, Pfade) |
| V5 | VirtualBox-Plugin | Rundreise auf Wegwerf-Maschine; Domäne je Konto |
| V6 | VMware (nur nach Entscheidung zu Frage 1) | eigene Planung, eigener Testaufbau mit Lizenz |

## Offene Fragen an den Nutzer

1. **VMware ja oder nein?** Braucht VDDK von Broadcom (eigene Entwicklerlizenz,
   Zugang geändert); das Projekt kann es nicht ausliefern. Wer stellt Lizenz
   und Testumgebung?
2. **qemu-guest-agent Pflicht oder Option?** Pflicht heißt: Maschinen ohne Agent
   werden nicht gesichert (oder nur `offline`/`crash` mit ausdrücklichem
   Schalter). Option heißt: Absturzkonsistenz wird im Manifest vermerkt.
3. **Inkrementell über Dirty-Bitmaps**: Die Bitmaps liegen laut Dokumentation im
   qcow2-Image der Maschine. Wo steht der Stand der Prüfpunktkette
   (im Image, in libvirt, zusätzlich im Katalog)? Wer löscht Prüfpunkte, und wie
   verhalten wir uns bei Kopieren, Migration, Absturz der Maschine?
4. **Format der Platten im Strom**: qcow2 als Datei, rohe Blöcke mit Sparse-
   Information oder nur Änderungsextents? Brauchen wir für Sparse
   eine `FileEntry`-Erweiterung? Bleibt VirtualBox-`export` (OVA) im Plan,
   obwohl es ein Archiv ist?
5. **Push oder Pull** bei `backup-begin`: Push braucht Stagingplatz auf dem
   Hypervisor-Host; Pull braucht einen NBD-Client in `ofd` (Socket-Rechte,
   Abhängigkeit).
6. **Rootful**: Variante wie bei Podman (feste Logik in `ofd`, kein Broker als
   root) auch für `qemu:///system`? Das ist ein größerer Eingriff in den Kern.
7. **Abbruch, Journal, Totmannschalter**: Obergrenze für eine Überlagerung
   (`max_overlay`), und wie wird sie ohne laufendes `ofd` durchgesetzt?
8. **Eigentum der Disks**: Plattendateien gehören oft `libvirt-qemu` oder einem
   Konto; der Host liest als root per `openat2`. Beim Restore: welcher Eigentümer,
   welche SELinux-/AppArmor-Labels, und darf der Broker sie setzen?
9. **Einzeldateirestore** aus Gast-Platten (qemu-nbd/libguestfs) gewünscht, trotz Parser-Angriffsfläche?
10. **`virsh` per Prozess oder Bibliothek** im Broker?
11. **TPM und NVRAM**: Pflicht im Umfang? Ohne TPM-Zustand sind Maschinen mit
    TPM-gebundenen Schlüsseln nicht wiederherstellbar.
12. **Reihenfolge** libvirt, dann VirtualBox: passt das zum Bestand?
