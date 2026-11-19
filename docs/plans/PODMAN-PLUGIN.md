# Planung: Podman-Plugin für ofd

Stand: 19.11.2026. Status: **Grundplan freigegeben (18.10.2026), am 19.11.2026
nach drei Teamberichten und vier Entscheidungen des Nutzers neu gefasst und am
selben Tag nach zwei Verifikationsberichten überarbeitet.** P0 ist umgesetzt;
offen sind der Bareos-FD-Lauf von (1) und der Host-root-Lauf von (3)/(4) auf
einer Test-VM (Abschnitt „Umsetzungsstand“, „Stand P0“).

Kennzeichnung in diesem Plan:

- **(Nutzer, TT.MM.JJJJ)**: verbindliche Entscheidung des Nutzers.
- **Vorschlag, offen**: Ergebnis der Planung, noch nicht freigegeben.
- Belege verweisen auf Arbeitspapiere der Planungssitzung vom 19.11.2026
  (nicht im Repository):
  - Teamberichte **[S]** Storage, Images, Volumes; **[A]** API, Erkennung,
    Abhängigkeiten; **[K]** Konsistenz, Restore, Broker-Vertrag; jeweils mit
    Abschnitt, z. B. [S §1].
  - Verifikation der Architektur, Befunde **(V-1)** bis **(V-21)**; V-1
    blockierend, V-2 bis V-8 und V-13 hoch.
  - Praxisprüfung mit sieben Versuchen, zitiert als **(Praxis 1)** bis
    **(Praxis 7)**.

  Alle Versuche liefen mit Podman 5.4.2 (netavark, sqlite), systemd 257 und
  Kernel 6.12 rootless in Wegwerf-Speicher; forgeiec.io wurde nur lesend
  untersucht.

## Ziel und Rahmen

Wiederherstellbare Sicherung ausgewählter Podman-Workloads: Inspect-Daten von
Pods und Containern, deklarative Quellen (Quadlet), Images, Netze und die
Daten in Named Volumes. Bind Mounts sichert der FD wie gewohnt als Dateien.
Rootful und jede rootless Host-UID sind getrennte Sicherungsdomänen; ein
rootful Inventar ersetzt nicht die Inventare anderer Konten.

Es gelten die [Plugin-Architektur](../wiki/plugin-architektur.md), der
[Protobuf-Vertrag](../../proto/filed_plugin.proto) und die Sicherheitsregeln
aus [STAND-2026-11-19](STAND-2026-11-19.md): nur signierte Plugins, der Gast
hat keine Datei-, Netz- oder Prozess-Importe, nur einen typisierten
Broker-Import, wenn Manifest und Policy die Capability gewähren; der Host
prüft jede Broker-Anfrage; **Broker laufen nie als root**; Restore in Dateien
führt keinen Plugin-Code aus; **S-13**: Plugin-Einträge tragen im Strom nur
die Rechte-Bits `0o777` (`plugin_ff`, `e1ecd00`). Grants stehen im
Signierzertifikat, die lokale Policy schränkt nur ein
([Signier-Infrastruktur](SIGNIER-INFRASTRUKTUR.md)). Das Broker-Muster folgt
[PostgreSQL Phase 2](POSTGRESQL-PHASE2.md): nativer Broker aus geprüftem
Deskriptor, `fexecve`, leere Umgebung, `PR_SET_NO_NEW_PRIVS`,
`PR_SET_PDEATHSIG`, Protobuf über `socketpair`. Gemeinsame Bausteine (Journal,
Writergruppe, Stagingvolume, Restore-Verfahren) teilt das Plugin mit dem
[Docker-Plugin](DOCKER-PLUGIN.md); UID-Domänen, Pods, Quadlet und Namespaces
bleiben Podman-spezifisch.

Paket: `ofd-plugin-podman` (`Architecture: any`, `Depends: ofd (= <Version>)`)
mit `/usr/lib/<triplet>/onesimus/plugins/podman.pb` und dem Broker
`/usr/lib/<triplet>/onesimus/plugins/podman/onesimus-podman-broker`.
Das [Container-Dump-Skript](../wiki/container-dump.md) bleibt Übergangshilfe.

## Entscheidungen des Nutzers

| Thema | Entscheidung |
| --- | --- |
| Grundplan | freigegeben (Nutzer, 18.10.2026) |
| conmon-Erkennung | hostweite read-only Erkennung von Konten mit Podman über conmon; ergänzt registrierte Konten, gewährt keine Rechte (Nutzer, 18.10.2026) |
| Parameter | Auswahl und Umfang stehen in der FileSet-Plugin-Zeile; eine Zeile gilt genau einem Konto (Nutzer, 19.11.2026) |
| Arbeitsweise | wie ein Dateisystem: virtueller Dateibaum im SD-Strom, jede Einheit eine eigene Datei, **kein zusätzliches tar** (Nutzer, 19.11.2026) |
| Container | nur Inspect-Daten und Quellen, kein `podman export` des Container-Dateisystems; Writable Layer bleibt ausgeschlossen (Nutzer, 19.11.2026) |
| Images | wählbar (`images=`), nicht Pflicht (Nutzer, 19.11.2026) |
| Netze | die verwendeten Netze gehören zur Sicherung (Nutzer, 19.11.2026) |
| Zusammenhänge | erkennt das Plugin selbst; der Administrator wählt nur aus, was er sichern will (Nutzer, 19.11.2026) |
| **Rootful** | Variante B: die rootful Logik liegt in `ofd` selbst. `ofd` holt als root Inspect-Daten, Quadlet-Dateien, Netze, Image-Blobs und Volume-Daten mit fester Logik ohne Plugin-Code. Kein Broker und kein Plugin-Code läuft als root. Rootless bleibt beim Broker als das jeweilige Konto (Nutzer, 19.11.2026) |
| **Umgebungsvariablen** | `env=omit` ist Vorgabe: gesichert werden nur die Namen der Variablen und der Verweis auf EnvironmentFiles, keine Werte; die EnvironmentFiles sichert der FD normal. `env=keep` bleibt ausdrückliche Option: Werte nur auf das Volume, nie in Joblog oder Katalog (Nutzer, 19.11.2026) |
| **Pods** | werden immer ganz gesichert (Nutzer, 19.11.2026) |
| **Konsistenz-Umfang** | der Stop umfasst auch die Dateisicherung des Jobs, `consistency_scope=job` ist Vorgabe, weil Anwendungen wie tapir nur Bind Mounts nutzen (Nutzer, 19.11.2026) |

Ältere Planungsentscheidungen, die weiter gelten: Broker je freigegebener
Ziel-UID statt eines zentralen Rootbrokers; kein `podman volume import` in
bestehende Volumes, Restore nur in neue leere Stagingvolumes; Stop bei Quadlet
über den zuständigen systemd-Manager; Wiederanlauf nur eigener, unveränderter
Stops; eigener Adapter statt Docker-Kompatibilitätsschicht.

## Zusammenspiel von Gast, Host und Broker

Grundsatz nach der Verifikation (V-2, V-3, V-9, V-17): **Alle Entscheidungen,
die Sicherheit betreffen, trifft der Host.** Der Host berechnet die erlaubte
Objektmenge, steuert den Lifecycle, liest die Bytes und Attribute von
Volume-Dateien und Image-Blobs selbst und erzeugt die bereinigten Dokumente.
Der Gast (WASM, signiert, in beiden Profilen derselbe) bestimmt Format,
Dateibaum, Manifest und Reihenfolge. Er sieht nur, was der Host für diesen Job
freigibt.

**Verhältnis zu „Broker als Konto“ (V-2, V-3):** Der rootless Broker läuft
weiter als Konto-UID, aber in einem User-Namespace, **den root besitzt**.
Beim Sichern liefert er nur noch **Metadaten**: Inventar, Inspect,
`generate spec`, `quadlet -dryrun -user`. Er liefert **keine Bytes und keine
Attribute**. Alles, was der Broker meldet, gilt als Angabe des Kontos und
damit als nicht vertrauenswürdig. Ein Konto kann seinen Broker nicht so
steuern, dass er Eigentümer, Sonderbits oder Gerätedateien erfindet, weil der
Host diese Werte selbst per `openat2`/`fstatat` vom Datenträger liest. Beim
Online-Restore schreibt der Broker im Ziel-Namespace; welche Attribute er
setzt, bestimmt der Host (V-9).

```mermaid
flowchart LR
    G[WASM-Gast podman<br/>signiert, ohne WASI<br/>Baum, Manifest, Reihenfolge] -- typisierter Import<br/>onesimus:filed-plugin/podman --> P[ofd: Podman-Port<br/>Grants, Plugin-ID, Objektmenge,<br/>Lifecycle, Journal, Leases]
    P -- Scan-Handle: openat2 als root<br/>RESOLVE_BENEATH, NO_SYMLINKS, NO_XDEV --> D[(Storage der Domäne:<br/>Volumes, Layer, tar-split)]
    P -- user=konto: Protobuf über socketpair --> B[onesimus-podman-broker<br/>Konto-UID im root-eigenen<br/>User- und Mount-Namespace<br/>nur Metadaten]
    B -- feste Argumente, --format json --> C1[podman / quadlet -dryrun -user]
    P -- user=konto: feste Argumente,<br/>als Konto-UID ohne Namespace --> S1[systemctl --user /<br/>systemd-run --user]
    P -- user=root: feste Logik in ofd --> R[Rootful-Backend in ofd<br/>kein Plugin-Code]
    R -- feste Argumente, leere Umgebung --> C2[podman / quadlet -dryrun /<br/>systemctl]
    P --> SD[(SD-Strom, Bareos-Format)]
```

| Aufgabe | Gast (WASM) | Host `ofd` (root) | Broker (rootless, Konto-UID) |
| --- | --- | --- | --- |
| Plugin-Zeile auswerten | Baum, Manifest | Zeile prüfen, Konto gegen Kontopolicy, Grants und Plugin-ID prüfen | – |
| Inventar, Inspect, Zusammenhänge | ordnet die gelieferte Menge in den Baum | berechnet die maßgebliche Objektmenge (V-17), erzeugt bereinigte Dokumente über eine Positivliste (V-6) | ruft `podman … --format json` mit fester Argumentliste |
| Quadlet, systemd | ordnet zu | prüft Pfade gegen Quadlet-Suchpfade; ruft `systemctl --user show` mit fester Property-Liste als Konto-UID außerhalb des Namespace (V-8) | `quadlet -dryrun -user` |
| Volume-Daten, Layer | nimmt Einträge per `source_ref` auf, bestimmt die Reihenfolge | liest Bytes **und** Attribute selbst über den Scan-Handle (V-2, V-3), erzeugt `ownership.tsv` | – |
| Stop/Wiederanlauf | höchstens `member-state` lesend (V-9) | allein: Journal, Lifecycle-Lease, Totmannschalter, Recovery | `podman stop` einfacher Container |
| Restore online | Plan, Reihenfolge | prüft jede Anlage (Suffix, keine neuen Rechte), Attribute aus dem Strom und `ownership.tsv` (V-9), Restore-Journal | legt Netze, Volumes, Container an, schreibt Volume-Dateien im Ziel-Namespace |

**Rootless** (`user=<konto>`): `ofd` prüft Konto, `/etc/subuid` und
`/etc/subgid` und legt den User-Namespace als root an, bevor der Broker als
Konto-UID darin startet (Abschnitt „Broker und Rootful-Backend“). Der Broker
ist im Namespace UID 0 und hat Rechte nur über die IDs des Kontos, genau wie
`podman unshare` [S §2, K §3.1]. Lifecycle-Aufrufe gehen über den
Benutzermanager des Kontos, außerhalb des Namespace (V-8).

**Rootful** (`user=root`, Nutzer, 19.11.2026): kein Brokerprozess. Das
Rootful-Backend ist Teil des signierten `ofd`-Releases (nicht des
Plugin-Pakets) und implementiert dieselben Operationen mit fester Logik:
`podman` und `/usr/libexec/podman/quadlet -dryrun` als root mit fester
Argumentliste und leerer Umgebung, `systemctl` gegen den Systemmanager.
Begründung: rootful Storage (0700 root) und `/run/podman/podman.sock` sind
root-äquivalent; ein Nicht-root-Broker mit Capabilities wäre nur formal nicht
root [S §2, A §1, K §3.4]. Kosten: Podman-Client-Code im Kern von `ofd`.
Begrenzung: keine Shell, kein generisches Exec, nur die Operationen der
Tabelle im Abschnitt „Broker und Rootful-Backend“, nie `--showsecret`, nie
`system reset|migrate|renumber`, Programmpfade root-eigen und nicht
beschreibbar geprüft, JSON-Größen begrenzt. Kindprozesse mit `close_range`,
eigener Prozessgruppe und RLIMITs wie in `broker.rs` (V-21).

**Bindung an das Plugin** (V-11): Der Podman-Import und das Rootful-Backend
werden nur für die Plugin-ID `org.onesimus.podman` verknüpft; die Prüfung
steht fest im Code von `ofd`, zusätzlich zu den Grants. Vorschlag, offen.

Vorschlag, offen: Gemeinsamer Code von Broker und Host (Inventar-Parser,
Zusammenhangs-Algorithmus, Positivlisten, tar-split-Leser) in einem Crate
`onesimus-podman-core`, das `ofd` und `onesimus-podman-broker` einbinden.

### Grants

Grants im Signierzertifikat des Plugins (Vorschlag, offen, Namen nach dem
Muster `postgresql.read`):

| Grant | erlaubt | rootless | rootful |
| --- | --- | --- | --- |
| `podman.inventory` | Engine-Identität, Inventar, Inspect, Quadlet-Quellen, Unit-Eigenschaften | Pflicht | Pflicht |
| `podman.image.read` | Image-Metadaten und Blobs (`images=blobs`), vom Host gelesen | bei `images=blobs` | bei `images=blobs` |
| `podman.volume.read` | Volume-Daten mit Attributen, vom Host gelesen | bei `volumes=yes` | bei `volumes=yes` |
| `podman.lifecycle` | Host stoppt und startet die Writergruppe dieser Zeile | bei `consistency=stop` | bei `consistency=stop` |
| `podman.restore` | Online-Restore: Netze, Volumes, Images, Pods, Container anlegen, Quadlet stagen | Restore | Restore |
| `podman.rootful` | Zugriff auf das Rootful-Backend in `ofd` | – | **Pflicht, zusätzlich** |

Grants sind **keine Auswahl je Job** (V-11): Das Manifest des Plugins fordert
alle Grants an, die es überhaupt nutzen kann; die lokale Policy schränkt je
Host ein, z. B. `podman.rootful` auf Hosts ohne rootful Podman. Wer
`podman.restore` entzieht, kann auf diesem Host nur `restore_mode=files`
nutzen. Der Host verknüpft den Import nur, wenn mindestens `podman.inventory`
gewährt ist, und prüft jede Funktion gegen ihren Grant. Fehlt ein Grant, den
die Zeile braucht (etwa `podman.lifecycle` bei `consistency=stop`), ist das
ein Planfehler, kein stiller Wechsel zu `crash`; bei `consistency_scope=job`
fällt diese Prüfung in den Vorlauf vor dem ersten Stop (V-5).

**Offen (V-11):** Das Signierzertifikat gewährt seine Capabilities heute jeder
seiner Plugin-IDs (`verify`: Capability ∈ `chain.grants.capabilities`). Ein
Suite-Zertifikat macht `podman.rootful` damit für jedes Plugin anforderbar,
das danach fragt. Die feste Bindung an `org.onesimus.podman` (oben) schließt
das für die Podman-Grants. Die allgemeine Lösung, Grants je Plugin-ID im
Zertifikat, gehört in die [Signier-Infrastruktur](SIGNIER-INFRASTRUKTUR.md)
und ist dort offen.

### Vertragsversion

`FileEntry` v2 (Abschnitt „Attribute“) braucht `api_version` 2. Heute prüft
`verify` Gleichheit mit `API_VERSION = 1`; ein Plugin mit 2 scheitert daher,
ebenso jedes Plugin mit 1 nach einem Wechsel (V-16). Vorschlag, offen: `ofd`
akzeptiert eine Menge unterstützter Versionen (`{1, 2}`); das
PostgreSQL-Plugin bleibt bei 1, Podman verlangt 2. Der Import heißt nach dem
vorhandenen Muster `onesimus:filed-plugin/podman`, die Welt `podman-plugin`;
`link_brokers` (heute eine Capability je Import, fest `postgresql.read`) wird
auf Grants je Funktionsgruppe umgebaut (V-16).

### Kontopolicy

Welche Konten gesichert werden dürfen, legt der Administrator in einer
root-eigenen Kontopolicy fest (Vorschlag, offen: eigene Datei neben der
`Plugin Policy`, weil deren Format unbekannte Felder ablehnt). Inhalt je
Konto: Name und UID, erlaubte Storage-Domänen mit `graphroot` (realer Pfad),
`rootful: true` nur für `root`, `volume_bind_roots` (erlaubte
`device`-Präfixe für `o=bind`-Volumes beim Sichern), erlaubte Registries für
den Restore per Pull. Ein Konto, das nur die Discovery kennt, ist nicht
freigegeben.

## Erkennung der Konten

conmon allein findet nur Konten mit **laufenden** Containern; gestoppte
Container, Konten mit nur Volumes oder Images und gestoppte Quadlet-Units
(deren Container per `--rm` gelöscht sind) bleiben unsichtbar [A §6]. Die
Discovery (root, nur lesend, erweitert `discovery.rs`) sammelt deshalb
Indizien ohne Inhalte zu lesen und ohne Kontowechsel:

| Indiz | Quelle | findet gestoppte |
| --- | --- | --- |
| conmon-Prozess | `/proc`, Host-UID, Executable über Device/Inode | nein |
| Speicherort aus conmon | `--exit-command-arg --root/--runroot/--tmpdir/--volumepath/--db-backend` in `/proc/<pid>/cmdline`; vom Benutzer bestimmt, nur Hinweis, Pfade müssen der UID gehören, sonst `suspicious` | – |
| Storage | `db.sql` bzw. `bolt_state.db` unter `<home>/.local/share/containers/storage`, rootful `/var/lib/containers/storage` | ja |
| Quadlet-Orte | `/etc/containers/systemd/users/<UID>/`, `<home>/.config/containers/systemd/` (`readdir`) | ja (Units) |
| Linger | `/var/lib/systemd/linger/<name>` | Hinweis |
| Sub-IDs | `/etc/subuid`, `/etc/subgid` | Hinweis |
| Laufzeitreste | `/run/user/<uid>/libpod`, `/tmp/storage-run-<uid>` | seit dem Boot benutzt |

Zugriff als root in Verzeichnissen der Konten (V-15): nur `lstat` bzw.
`openat2(RESOLVE_NO_SYMLINKS)`, nie einem Symlink folgen, Zeitlimit je
Zugriff (Automount- und NFS-Homes), Konfigurationsdateien der Konten
(`storage.conf`, `containers.conf`) werden **nie** als root ausgewertet. Für
abweichende Speicherorte gelten nur conmon-Hinweis und der Eintrag des
Administrators in der Kontopolicy.

Zustände: `discovered_unconfirmed` (Discovery) → `confirmed_podman` erst durch
den Broker des freigegebenen Kontos bzw. das Rootful-Backend
(`engine-identify`: `podman info` mit graphroot, runroot, tmpdir, Treiber,
DB- und Netz-Backend, ID-Mappings, Version). Die Registry hält
`(Host, UID, graphroot, graphroot-Inode, runroot, tmpdir)`; das `tmpdir` gehört
zur Identität, weil sich mehrere Speicher eines Kontos `tmpdir` und
Lock-Tabelle teilen [K §0]. Eine UID mit mehreren Speichern ergibt mehrere
Domänen. Bestätigte Konten werden auch ohne aktuellen conmon-Fund weiter
inventarisiert. Der graphroot, den der Broker meldet, muss mit Pfad und Inode
dem Eintrag der Kontopolicy entsprechen; sonst Planfehler.

Nicht freigegebene Konten werden angezeigt, nie heimlich gesichert. Beispiel
forgeiec.io: `www-data` (Matomo, vermutlich Compose) mit Linger, Sub-IDs,
eigenem Storage und runroot `/tmp/storage-run-33` steht in keiner
Plugin-Zeile und erscheint nur als Kandidat [A §6]. Eine Plugin-Zeile mit
einem nicht freigegebenen Konto ist ein Planfehler.

Grenzen: `hidepid`, PID-Namespaces und ein FD im Container schränken die Sicht
ein; fehlende Sicht ist `discovery_incomplete`, kein Beleg für „keine
Container“. Rootful-Funde verraten nicht, welcher Mensch `sudo` benutzt hat.
CRI-O-conmon wird nicht als Podman klassifiziert.

## Parameter der Plugin-Zeile

Schreibweise wie beim PostgreSQL-Plugin (`name:schlüssel=wert:…`); eine
Zeile gilt genau einem Konto (Nutzer, 19.11.2026). Die Schlüssel `env`,
`consistency_scope` und das Verhalten bei Pods sind entschieden; die übrigen
Schlüssel und Vorgaben sind Vorschlag, offen.

```text
Plugin = "podman:user=forgejo:pods=*:exclude=test-*"
Plugin = "podman:user=1001:containers=keycloak,postgres:images=blobs"
Plugin = "podman:user=root:quadlet=tapir-*:images=blobs"
```

| Schlüssel | Werte | Vorgabe | Bedeutung |
| --- | --- | --- | --- |
| `user` | Kontoname, UID, `root` | – (Pflicht) | Konto der Sicherung; muss in der Kontopolicy freigegeben sein. `root` ist das rootful Profil. |
| `pods`, `containers`, `quadlet` | Namen oder Globs | – | **mindestens einer** der drei Schlüssel ist Pflicht, mehrere werden vereinigt (V-18); `*` wählt alles dieses Kontos. Ohne Auswahl Planfehler, damit eine leere Zeile nicht ungewollt alles sichert. |
| `exclude` | Namen oder Globs | – | von der Auswahl abgezogen; schließt keine automatisch erkannten Abhängigkeiten aus und **keine Mitglieder eines Pods**, der wegen „Pods immer ganz“ dazugehört (Manifest: `exclude_overridden_by_pod`, V-18). |
| `volumes` | `yes`, `no` | `yes` | Named Volumes der Auswahl. Bind Mounts nie, die sichert der FD (eigene `File =`-Zeile). |
| `images` | `reference`, `blobs` | `reference` | `reference`: Image-Inspect im Baum, Restore per Pull. `blobs`: zusätzlich Config, Original-Manifest, Index und Layer als einzelne Dateien, je DiffID einmal. |
| `networks` | `yes`, `no` | `yes` | die verwendeten Netze als Inspect-Daten plus Zuordnung je Container. |
| `quadlet_sources` | `yes`, `no` | `yes` | Quadlet-Quellen, Quadlet-Drop-ins, systemd-Drop-ins und verwandte lokale Units (z. B. `*.target`); bei `env=omit` mit geschwärzten Werten. |
| `env` | `omit`, `keep` | `omit` (Nutzer, 19.11.2026) | `omit`: nur Namen und Verweise auf EnvironmentFiles. `keep`: Werte nur auf dem Volume. |
| `consistency` | `stop`, `crash` | `stop` | `stop`: autorisierter Stop der Writergruppe mit Journal. `crash`: Kopie im laufenden Betrieb, im Manifest als abgeschwächt vermerkt. |
| `consistency_scope` | `job`, `plugin` | `job` (Nutzer, 19.11.2026) | `job`: Stop vor der ersten Datei des Jobs, Wiederanlauf nach der letzten; umfasst die FD-Dateisicherung der Bind Mounts. `plugin`: nur für die Dateien dieser Zeile. |
| `stop_timeout` | Sekunden | `StopTimeout` bzw. `TimeoutStopSec` | Obergrenze je Unit oder Container. |
| `max_downtime` | Sekunden | **7200** (Vorschlag, offen, V-4) | Obergrenze der Ausfallzeit ab dem ersten Stop; bei `consistency_scope=job` Pflicht, `0` (unbegrenzt) unzulässig. Überschritten: Cancel und Wiederanlauf durch `ofd`, fällt `ofd` aus, durch den Totmannschalter. |
| `clean_exit_codes` | Liste | `0,143` | Exit-Codes, die als sauberer Stop gelten. |
| `on_unclean` | `fail`, `warn` | `fail` | `fail`: Job-Fehler, Daten trotzdem gesichert und im Manifest als `crash` markiert. |
| `external` | `warn`, `error` | `warn` | Bind-Quellen, EnvironmentFiles und geschwärzte Quadlet-Quellen, die keine `File =`-Zeile des FileSets abdeckt. |
| `secrets` | `reference` | `reference` | Secrets nur als Verweis, nie ihr Inhalt; kein anderer Wert zulässig. |
| `endpoint` | – | – | erst Remote (P4); im MVP unzulässig. |

Regeln: Unbekannte Schlüssel sind ein Fehler. Eine Auswahl, die nicht
unterstützte Quellen trifft (fremder Volumetreiber, unbekanntes Storage- oder
Mappingprofil, `ns:`-Netzmodus), ist ein Planfehler mit Nennung der Quelle,
kein stilles Auslassen. Gleiche Namen verschiedener Konten bleiben getrennt,
weil die Domäne Teil jeder Identität ist. **Pods immer ganz** (Nutzer,
19.11.2026): Trifft die Auswahl einen Container in einem Pod, gehören der Pod,
sein Infra-Container und alle Mitglieder dazu; das Manifest nennt den Grund.
Mehrere Zeilen desselben Kontos entdoppeln untereinander nicht (getrennte
Instanzen); überschneiden sich ihre Writergruppen, vereinigt der Host sie zu
einer Lifecycle-Gruppe (Vorschlag, offen).

Restore-Parameter (`pluginoptions=` des Restore-Jobs), Vorschlag, offen:

| Schlüssel | Werte | Vorgabe | Bedeutung |
| --- | --- | --- | --- |
| `restore_mode` | `files`, `online` | `files` | `files`: Baum unter `Where`, ohne Plugin-Code. `online`: gestagter Restore in neue, gestoppte Ressourcen. Der Name `online` gilt in `ofd` bereits (`restore.rs`); kein Synonym `stage`. |
| `target_user` | Kontoname, UID, `root` | Original | Zielkonto; muss freigegeben sein; `root` braucht `podman.rootful`. |
| `suffix` | Text | `-restored` | Namenszusatz aller neuen Objekte. |
| `networks` | `create`, `existing` | `create` | fehlende Netze anlegen bzw. nur vorhandene gleichen Namens mit passendem Subnetz nutzen. |
| `network_subnet` | `keep`, `auto` | `keep` | `keep` mit Überlappung: Planfehler. `auto`: Podman wählt, statische IPs entfallen (Warnung). |
| `restore_ip` | `auto`, `keep`, `drop` | `auto` | `auto`: statische IP/MAC nur, wenn als statisch belegt. |
| `ports` | `keep`, `drop` | `keep` | Portfreigaben übernehmen; Konflikte stehen im Plan. |
| `bind_map` | `<alt>=<neu>`, mehrfach | – | Bind Mounts von Containern **und** Hostpfade in Quadlet-`Volume=`/`Mount=` nur über diese Zuordnung, sonst Planfehler; die Identität `<pfad>=<pfad>` ist zulässig, muss aber ausdrücklich angegeben sein (V-13). |
| `image_pin` | `yes`, `no` | `yes` bei veränderlichem Tag | Quadlet-Staging bekommt ein Drop-in `Image=<name>@<digest>` mit dem Digest, der im Ziel tatsächlich vorhanden ist (Abschnitt Images). |
| `image_digest` | `new`, `original` | `new` | `original` nur mit zertifizierter Version (Abschnitt Images). |
| `selinux` | `relabel`, `keep`, `drop` | `relabel` | `relabel`: Label nicht schreiben, Podman labelt über `:z`/`:Z`. |

Die früheren Schalter `files_owner` und `files_special` entfallen: Der Strom
enthält Host-IDs und nie Sonderbits oder Capabilities (V-1, V-12).

## Dateibaum und Manifest

Das Plugin legt im vorhandenen SD-Strom (`NextFile`/`ReadFile`, Bareos-Format)
einen virtuellen Dateibaum an (Nutzer, 19.11.2026). Jede Einheit ist eine
eigene Datei mit eigenen Attributen; so bleiben Restore einzelner Teile,
Prüfsummen je Datei und später Incrementals möglich. Eine Image-Layer ist nach
OCI-Definition selbst ein tar; sie ist die Original-Einheit des Images, kein
zusätzlicher Container um andere Dateien [S §1, A §4].

```text
/@podman/<domäne>/engine/info.json                     gefiltertes podman info/version
/@podman/<domäne>/containers/<name>/inspect.json       Inspect nach Positivliste, Env nach env=
/@podman/<domäne>/containers/<name>/create-command.json CreateCommand geparst (Positivliste), Env nach env=
/@podman/<domäne>/containers/<name>/spec.json          podman generate spec nach Positivliste, Env nach env=
/@podman/<domäne>/containers/<name>/env.json           Namen, Herkunft, EnvironmentFiles; Werte nur bei env=keep
/@podman/<domäne>/containers/<name>/systemd.json       Unit-Eigenschaften (feste Property-Liste) bei PODMAN_SYSTEMD_UNIT
/@podman/<domäne>/pods/<name>/inspect.json, spec.json
/@podman/<domäne>/networks/<netz>/inspect.json         network inspect ohne "containers"
/@podman/<domäne>/networks/<netz>/used-by.json         Zuordnung je Container (abgeleitet)
/@podman/<domäne>/volumes/<vol>/inspect.json
/@podman/<domäne>/volumes/<vol>/data/<pfad>            Dateien des Volumes, Host-Sicht, ohne Sonderbits/Caps; data/ trägt den Stat der Volume-Wurzel
/@podman/<domäne>/volumes/<vol>/ownership.tsv          vom Host erzeugt, nach data/: Maps, ns-Sicht, Sonderbits, Caps, ACL
/@podman/<domäne>/images/<image-id>/inspect.json       Namen stehen im Manifest
/@podman/<domäne>/images/layers.json                   je DiffID: Layer-ID, compressed-diff-digest, Größen, uid/gidset
/@podman/<domäne>/images/oci/oci-layout                nur images=blobs
/@podman/<domäne>/images/oci/index.json                erzeugtes Manifest je Image, ref.name = name:tag
/@podman/<domäne>/images/oci/blobs/sha256/<digest>     Config (= Image-ID), Original-Manifest, Original-Index, erzeugtes Manifest, Layer (= DiffID)
/@podman/<domäne>/secrets/<name>/inspect.json          nur Metadaten
/@podman/<domäne>/quadlet/<suchpfad-kürzel>/<rel.pfad> z. B. etc/web.container; bei env=omit Vorlage mit Platzhaltern
/@podman/<domäne>/systemd/<unit>                       verwandte Nicht-Quadlet-Units, z. B. *.target; Drop-ins bei env=omit geschwärzt
/@podman/<domäne>/manifest.json                        zuletzt
```

- `<domäne>`: `root` bzw. `uid-<n>`, mit Kürzel, wenn eine UID mehrere Speicher
  hat; der Kontoname steht im Manifest, weil Namen sich ändern [A §4].
- Images nach **ID** benannt: Namen enthalten `/` und `:`, mehrere Tags
  zeigen auf dieselbe ID [A §4].
- `images/oci/` ist ein gültiges OCI-Layout; nach `restore_mode=files` genügt
  `podman pull oci:<Where>/@podman/<domäne>/images/oci:<name>` ohne
  Plugin-Code; Pfade mit `@` und Ref-Namen mit `:` funktionieren
  [S §1, K §2.3, Praxis 1].
- Alle Dateien außer den Volume-Daten haben synthetische Attribute nach S-13;
  Blobs: Modus 0444, `mtime = ctime =` `created` der Layer, vorbereitet für
  Accurate [S §4]. `ownership.tsv`: synthetisch, Modus 0600.

### manifest.json

Das Manifest (Format `onesimus-podman/1`) beschreibt Domäne, Auswahl,
Objekte, Kanten, externe Abhängigkeiten, Writergruppen und die
Restore-Reihenfolge [A §4]. Ausschnitt für forgeiec.io:

```json
{
  "format": "onesimus-podman/1",
  "domain": {"host": "forgeiec.io", "host_uid": 0, "user": "root", "domain_id": "sha256(host|uid|graphroot|inode|runroot|tmpdir)",
             "engine": {"version": "5.4.2", "network_backend": "netavark", "db_backend": "sqlite", "graph_driver": "overlay",
                        "cgroup_manager": "systemd", "graphroot": "/var/lib/containers/storage", "rootless": false,
                        "uid_map": null, "gid_map": null}},
  "selection": {"quadlet": ["tapir-*"], "env": "omit", "images": "blobs", "consistency": "stop", "consistency_scope": "job",
                "max_downtime": 7200},
  "objects": [
    {"kind": "quadlet", "id": "/etc/containers/systemd/tapir-ackervielfalt-web.container", "generated_unit": "tapir-ackervielfalt-web.service",
     "fingerprint": "sha256:…", "redacted": false, "reason": "selected"},
    {"kind": "container", "id": "<64hex>", "name": "tapir-ackervielfalt-web", "state": "running",
     "restore_strategy": "quadlet", "lossy_fields": ["RestartPolicy"], "env_names": ["SECRET_KEY", "…"],
     "reason": "created_by:quadlet:tapir-ackervielfalt-web.service"},
    {"kind": "image", "id": "8efa6985d967…", "names": ["ghcr.io/…/wirgarten-tapir:latest"], "image_digest": "sha256:b8bb8759…",
     "restore_digest": "sha256:<sha256 des erzeugten Manifests>",
     "diff_ids": ["sha256:…"], "platform": "linux/amd64", "mode": "blobs", "mutable_tag": true, "used_by": ["<web>", "<worker>", "<beat>"]},
    {"kind": "systemd_unit", "id": "tapir-ackervielfalt.target", "source": "/etc/systemd/system/tapir-ackervielfalt.target", "reason": "PartOf"}
  ],
  "edges": [
    {"from": "container:<web>", "to": "image:8efa…", "type": "image"},
    {"from": "quadlet:tapir-ackervielfalt-web.container", "to": "quadlet:tapir-redis.container", "type": "systemd:Wants"}
  ],
  "external": {
    "paths": [{"path": "/etc/tapir/instances/ackervielfalt.env", "via": "EnvironmentFile", "sensitive": true, "covered_by_fd": true},
              {"path": "/var/lib/tapir/ackervielfalt", "via": "bind:/data", "rw": true, "covered_by_fd": true}],
    "units": [{"unit": "postgresql.service", "via": "After", "note": "Host-Dienst, eigenes Plugin, nicht gestoppt"}],
    "secrets": []
  },
  "dependents_not_selected": [],
  "writer_groups": [{"members": ["tapir-ackervielfalt-web.service", "tapir-ackervielfalt-worker.service", "tapir-ackervielfalt-beat.service"],
                     "shared": ["/var/lib/tapir/ackervielfalt"], "stop_order": ["web", "worker", "beat", "tapir-redis"]}],
  "restore_order": ["network", "volume", "image", "secret_ref", "pod", "container", "quadlet", "systemd_unit"],
  "exclusions": ["writable_layer"],
  "rejected": [],
  "consistency_result": "clean",
  "downtime_seconds": 0,
  "warnings": ["mutable_tag:ghcr.io/…:latest"],
  "discovery": {"source": "registry", "conmon_seen": true, "incomplete": []}
}
```

## Was gesichert wird

### Container: Inspect und Quellen-Rangfolge

Je Container nur Inspect-Daten und Quellen, kein Container-Dateisystem
(Nutzer, 19.11.2026). Für den Neuaufbau gilt eine feste Rangfolge nach der
Verlusttreue, gemessen in Praxis 3; das Manifest nennt je Container die
gewählte `restore_strategy`:

1. **`quadlet`**: Quadlet-Datei samt Drop-ins, wenn der Container das Label
   `PODMAN_SYSTEMD_UNIT` trägt und die Quelle gefunden wurde. Sie ist die
   Quelle der Wahrheit, weil Quadlet den Container bei jedem Start neu
   erzeugt (`--rm --replace`). Auch Quadlet-Container haben ein
   `CreateCommand` (die `podman run`-Zeile der Unit, Praxis 3); es wird mit
   gesichert und dient beim Restore als Gegenprobe.
2. **`create-command`**: `Config.CreateCommand` bei CLI-erzeugten
   Containern, fast verlustfrei: Neu angelegt aus dem `CreateCommand`
   unterschied sich im Versuch nur der automatische Alias (Praxis 3). Geparst
   gegen eine **Positivliste** der `podman create`-Optionen; Podman-Optionen
   (`--root`, `--runroot`, `--tmpdir`, `--replace`, `--rm`, `--cidfile`)
   werden verworfen, Namen, Netze, Volumes und Pod beim Restore umgeschrieben.
   Eine unbekannte Option ist ein Planfehler, nie ein rohes Replay. API- oder
   Compose-erzeugte Container haben kein `CreateCommand` (bestätigt, Praxis 3).
3. **`spec`**: `spec.json` aus `podman generate spec`, **ergänzt** um die
   Felder, die Spec verliert, aus Inspect (Positivliste); fehlt eine nötige
   Angabe in beiden, ist das ein Planfehler.

Was die Quellen verlieren (Praxis 3, ersetzt die frühere Liste):

| Quelle | verliert | behält |
| --- | --- | --- |
| Inspect | statische IP/MAC (`NetworkSettings` leer), Secret-Ziele (`Config.Secrets` nur Name/ID), pasta-Optionen (`NetworkMode: "pasta"`); Quadlet `Restart=always` erscheint als `RestartPolicy: no` | `keep-id` als Annotation `io.podman.annotations.userns`, Sicherheitsfelder, Healthcheck, Hostname, DNS |
| `generate spec` | Annotations, Hostname, WorkingDir, StopSignal, StopTimeout, Healthcheck, CapAdd, CapDrop, Dns, ExtraHosts, ReadonlyRootfs, `no-new-privileges`, Ulimits, LogConfig, Init, Secret-Name (`Source: ""`) | statische IP/MAC, pasta-Optionen, Secret-Ziel, `userns: keep-id`, Ports, Mounts mit `Z`, Restart-Policy, Ressourcenlimits |

Spec schleppt außerdem die konkreten `idmappings` des Quellhosts und den
automatischen Alias des Originals mit; beides verwirft der Host. Bekannter
Fehler (Podman 5.4.2): `generate spec` scheitert bei
`--secret …,type=env` („cannot unmarshal object … secret_env“). Solche
Container brauchen `quadlet` oder `create-command`, sonst Planfehler
(Praxis 3).

**Sicherheitsfelder als Pflichtvergleich** (Praxis 3): CapAdd, CapDrop,
`no-new-privileges`, ReadonlyRootfs, Ulimits, SecurityOpt (seccomp, AppArmor,
Label), Devices, Privileged sowie die Namespace-Modi (pid, ipc, net, uts,
userns, cgroupns). Der Host vergleicht sie zweimal mit dem Original-Inspect:
im Restore-Plan für die berechnete Anlage und nach dem Anlegen (vor jedem
Start) am Inspect des neuen Containers. **Hat der Restore mehr Rechte als das
Original, ist das ein Planfehler**; nach dem Anlegen bedeutet es Rückbau
dieses Containers. Schon eine verlorene Einschränkung (fehlendes CapDrop,
fehlendes `no-new-privileges`, `ReadonlyRootfs` false statt true, höhere
Ulimits) zählt als „mehr Rechte“.

Laufzeitfelder werden beim Neuaufbau nie übernommen: `Id`, `Created`,
`State`, `Pid`, Pfade unter `StaticDir`, `GraphDriver`, `NetworkSettings`
(Istwerte), `EffectiveCaps`, `Mounts[].Source` bei Volumes, automatisch
vergebene Aliase, Quell-`idmappings` [A §2, Praxis 3]. Die libpod-Datenbank
(`db.sql`) ist intern und unversioniert; das Plugin baut nicht darauf [A §2].
`podman container clone` und `kube generate` verlieren mehr und scheiden aus
[A §2].

**Vor dem Stop** (Praxis 4, V-5): Quadlet-Container sind nach `systemctl
stop` gelöscht [K §1.1, A §5]. Inventar (`ps -a`, `pod ps`, …), Inspect,
`CreateCommand`, `generate spec` und Unit-Eigenschaften werden deshalb im
Zustand `PREPARED` erfasst, vor jeder Mutation. Bei
`consistency_scope=job` hält der Host diesen Schnappschuss bis zur
Plugin-Sitzung in einem begrenzten Puffer; der Gast arbeitet auf dem
Schnappschuss, nicht auf einem neuen Inventar.

### Umgebungsvariablen

Inspect-Daten enthalten Klartext-Kennwörter: Werte aus `--env-file` bzw.
`EnvironmentFile=` stehen in `Config.Env` (forgeiec: `KC_DB_PASSWORD`,
`SECRET_KEY` u. a.; Werte nicht ausgegeben) [A §2, K §3.1]. Werte stehen an
mehr Stellen als zuerst geplant (V-6, Praxis 5).

**`env=omit`** (Vorgabe, Nutzer, 19.11.2026). Der Host übernimmt aus den
Antworten des Brokers bzw. des Rootful-Backends **nur Felder einer
Positivliste** (V-6), statt Werte aus der vollen Antwort herauszufiltern.
Unbekannte Felder entfallen; die Liste gehört zum zertifizierten
Versionsprofil. Erst das Ergebnis sieht der Gast.

| Ort | Behandlung bei `env=omit` |
| --- | --- |
| `Config.Env` (Inspect), `env` (Spec) | nur Namen |
| `CreateCommand` (Inspect) und `containerCreateCommand` (Spec), auch von Quadlet-Containern | nur geparst gespeichert; jede Env-Form (`-e K=V`, `-eK=V`, `--env K=V`, `--env=K=V`) wird zum Namen in `env.json`, `--env-file <pfad>` bleibt Verweis |
| Quadlet-Quelle und Quadlet-Drop-ins `Environment=` | Werte geschwärzt, Datei wird **Vorlage mit Platzhaltern** (`Environment=K=@@ONESIMUS_ENV:K@@`), Manifest `redacted: true` |
| systemd-Drop-ins `[Service] Environment=` (unter `systemd/`) | ebenso geschwärzt |
| `quadlet -dryrun` (`ExecStart … --env K=V`) | nur zur Zuordnung im Host verwendet, nie gespeichert |
| `systemctl show` (`unit-info`) | feste Property-Liste **ohne** `Environment`, `ExecStart*`, `ExecStop*`, `SetCredential*`, `LoadCredential*`; `EnvironmentFiles` nur als Pfade |
| Healthcheck (`Config.Healthcheck.Test`), `Cmd`, `Entrypoint`, `Args`, Quadlet `Exec=`, Labels, Annotations | **Wertabgleich**: Kommt ein Env-Wert des Containers darin wörtlich vor (ab 4 Zeichen), ersetzt der Host ihn durch `${K}`; der Wert verlässt den Host nie. Andere Literale bleiben, weil der Restore sie braucht (Vorschlag, offen; siehe Nutzerentscheidungen) |
| stderr von `podman`/`systemctl` | nie roh in Joblog oder Manifest, nur feste Meldungstexte mit Exit-Code (V-6) |
| erzeugte Unit unter `/run/…/systemd/generator/` | wird nicht gelesen |

`env.json` enthält je Variable Namen und Herkunft: `image` (der Host
vergleicht mit dem Image-Env, ohne den Wert weiterzugeben), `envfile` (rootful
liest der Host nur die **Namen** aus root-eigenen, nicht fremd beschreibbaren
EnvironmentFiles; rootless nie), sonst `unknown`, denn Werte aus
`--env-file` stehen in `Config.Env` ununterscheidbar (Praxis 5). Dazu die
Verweise auf EnvironmentFiles (Pfad, Eigentümer, Modus, vorhanden,
`covered_by_fd`). Den Inhalt der EnvironmentFiles liest der Broker nie; der FD
sichert sie über eine `File =`-Zeile. Den Originaltext geschwärzter
Quadlet-Quellen deckt der FD ab, wenn der Administrator
`/etc/containers/systemd` als `File =` aufnimmt; fehlt das, meldet
`external=warn|error` die Quelle.

**Restore ohne Werte** (Praxis 5): `-e K` ohne Wert übernimmt den Wert aus
der Umgebung des aufrufenden Prozesses oder lässt die Variable weg; die frühere
Regel „`-e K=V` wird `-e K`“ ist deshalb gestrichen. Variablen ohne Wert
werden beim Restore **weggelassen** und im Bericht mit Namen genannt;
`--env-file <pfad>` bzw. `EnvironmentFile=` bleibt Verweis und wirkt im Ziel.
Eine gestagte Quadlet-Vorlage mit Platzhaltern wird nicht promoviert, solange
ein Platzhalter offen ist.

**`env=keep`** (Option): Werte stehen in `env.json`, Inspect, Spec und
Quadlet-Quellen, nur auf dem Volume; nie in Joblog, Katalog oder
`FileEntry.attributes`. Meldungen nennen nur Namen. Das Manifest vermerkt
`env_values: keep`.

### Pods

Pods immer ganz (Nutzer, 19.11.2026): Inspect und Spec des Pods
(`InfraConfig` mit Netzen, Ports, statischer IP/MAC, DNS; `SharedNamespaces`),
Infra-Container und alle Mitglieder. Das Infra-Image
(`localhost/podman-pause:<version>`) baut Podman lokal; es wird nur
referenziert [A §2].

### Images

containers-storage hält Original-Manifest, Original-Index und Config
byte-genau, die Layer aber **nur unkomprimiert** als Verzeichnisbaum plus
`tar-split`-Metadaten; die komprimierten Registry-Blobs gibt es nicht mehr
[S §1, A §4].

- **Immer** (`images=reference`): `images/<id>/inspect.json`; im Manifest
  ID, Namen, RepoDigests, `image_digest`, DiffIDs, Plattform, `mutable_tag`.
- **`images=blobs`**: Config (= Image-ID), Original-Manifest und -Index
  byte-genau; jede Layer **bit-genau als Stream** aus `tar-split.gz` und dem
  Layer-Verzeichnis rekonstruiert, ohne Zwischenspeicher, Größe vorab bekannt
  (`diff-size`), SHA-256 beim Lesen mitgerechnet. Bestätigt für vfs, overlay
  (rootless nativ) und btrfs mit Whiteouts, Hardlink, Symlink, FIFO, setuid,
  Sparse-Datei, Name mit 0xff und xattr (Praxis 1). Benennung nach
  **DiffID**, je Digest einmal pro Job; die Zuordnung zur Layer-ID steht in
  `layers.json` [S §1]. Kein `podman save`: `oci-archive` komprimiert neu,
  `oci-dir --uncompressed` braucht Spool in Imagegröße [S §1, K §2.1].
- **Wer liest** (V-2, V-10): Die Blobs liest der Host selbst, rootful und
  rootless, über einen Scan-Handle ab dem Deskriptor des graphroot. Die
  tar-split-Einträge sind Daten des Kontos: Nutzlast nur über
  `openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_XDEV)` ab dem
  Deskriptor des Layer-Verzeichnisses, nur reguläre Dateien, Typ und Größe
  werden **vor** dem ersten Byte gegen tar-split geprüft. Eine Abweichung
  meldet die Layer als beschädigt, bevor Daten fließen; die DiffID-Prüfung am
  Ende bleibt zusätzlich. Der tar-split-Leser ist begrenzt (Eintragsgröße,
  Anzahl, Namenslänge).
- Storage-Treiber im MVP: overlay, btrfs, vfs. zfs, `additionalimagestores`
  und Layer ohne `tar-split.gz` sind Planfehler mit Nennung der Layer [S §1].
- Sperren: Lese-Sperren (`fcntl`) auf `storage.lock`, `layers.lock`,
  `images.lock` wie Podman, während des Metadatenlesens und je Layer-Stream
  [S §1]. Ob ein langer Stream `rmi`/`pull` zu lange blockiert, ist offen.

**Restore** [S §1, A §4, K §2.1, Praxis 1]: gleiche Image-ID und gleiche
DiffIDs, aber ein **neuer Manifest-Digest**. Podman übernimmt beim Import aus
`oci:` das vom Plugin erzeugte Manifest byte-gleich; der Digest im Ziel ist
`sha256(erzeugtes Manifest)`. Er ist damit **deterministisch und schon beim
Backup bekannt** und steht als `restore_digest` im Manifest und in
`images/oci/index.json` (Praxis 1). Geprüft wird über die Kette
Original-Manifest-Digest → Config-Digest (= Image-ID) → DiffIDs →
Layer-Dateien; nur die gzip-Digests im Original-Manifest lassen sich offline
nicht nachrechnen, das steht so im Manifest.

- **Vorhandene Image-ID im Ziel: nicht pullen** (Praxis 1). Ein `pull oci:`
  in einen Speicher, der das Image schon hat, ändert Digest und RepoDigests
  des bestehenden Images. Der Restore referenziert dann nur das vorhandene
  Image; der Original-Digest bleibt auflösbar.
- `image_digest=new` (Vorgabe): Der Restore-Plan meldet jeden Container und
  jede Quadlet-Unit, die das Image per `@sha256:` des Originals referenziert,
  mit dem `restore_digest` als Ersatz.
- `image_digest=original` (Vorschlag, offen, frühestens P4): Original-Index und
  -Manifest zusätzlich unter ihren Digests einspielen; geht nur über den
  internen Blob-Info-Cache von c/image in einem eigenen Cache des
  Pull-Prozesses, daher nur mit zertifizierter Podman-Version [S §1].
- `image_pin`: Bei veränderlichen Tags (forgeiec: `:latest`) zieht eine
  unveränderte Quadlet-Datei ein **anderes** Image [A §4]. Das Staging legt ein
  Drop-in `Image=<name>@<digest>` an: `restore_digest`, wenn importiert wurde,
  sonst der Digest des vorhandenen Images. Nachweis in P3, dass Quadlet
  (`Pull=missing`) diesen lokal vorhandenen Digest ohne Registry auflöst;
  sonst Pin über die Image-ID (V-13). Für veränderliche Tags wird
  `images=blobs` empfohlen, weil die Registry den alten Digest löschen kann.

### Netze

- Orte: rootless `<graphroot>/networks/<netz>.json`, rootful
  `/etc/containers/networks/<netz>.json`. Das Standardnetz `podman` ist
  **keine Datei**, sondern wird aus `containers.conf` erzeugt (`created` =
  Abfragezeit); es wird nur referenziert, mit Subnetz [A §3].
- Gesichert: `network inspect` ohne den Block `containers` (Name, ID, Treiber,
  Interface, Subnetze mit Gateway und Lease-Bereich, IPv6, internal, DNS,
  Labels, Optionen, IPAM-Optionen).
- Zuordnung **vom Container her**: `network inspect .containers` listet nur
  laufende, `ps --filter network=` verfehlt `--network container:` [A §3].
  Die gewünschte Zuordnung (Netz, statische IP/MAC, eigene Aliase,
  Interface-Name, Ports) kommt aus Quadlet `Network=`, `CreateCommand` oder
  Spec; Inspect allein kennt keine statischen Angaben (`static: unknown`).
- Nicht gesichert: aardvark-dns, `ipam.db`, `rootless-netns` (Laufzeit; liegt
  je runroot, Praxis 7).
- `host`, `none`, `pasta`, `slirp4netns`, `container:<id>` sind Netzmodi,
  keine Netzobjekte; `ns:<pfad>` ist ein Planfehler.
- Falle `--ignore` (bestätigt, Praxis 7): Quadlet legt Netze mit `podman
  network create --ignore` an, das ein vorhandenes Netz gleichen Namens auch
  bei anderem Subnetz stillschweigend behält. Der Restore-Plan prüft das
  ausdrücklich.

### Named Volumes

- Daten in `volume_path/<name>/_data` (rootless `<graphroot>/volumes`,
  rootful `/var/lib/containers/storage/volumes`) [S §2]. Im MVP nur der
  Standardort unter dem bestätigten graphroot; ein abweichender `volume_path`
  aus `containers.conf` ist ein Planfehler, außer die Kontopolicy nennt ihn.
- **Der Host liest Daten und Attribute selbst** (V-2, V-3), rootful wie
  rootless: `ofd` öffnet als root den graphroot aus der Kontopolicy (Pfad und
  Inode geprüft) und von dort `volumes/<name>/_data` mit
  `openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_XDEV)`; Gerät und
  Inode des `_data`-Deskriptors prüft er gegen das Inventar. Root kann die
  Dateien der Sub-UIDs lesen; ein User-Namespace ist zum Lesen nicht nötig.
  Der Scan liefert den echten Stat vom Datenträger, also **Host-IDs**; ACL und
  `security.capability` stehen auf der Platte ebenfalls in Host-IDs (Caps v3
  mit rootid = Konto-UID, bestätigt in Praxis 2).
- Rootless prüft der Host jede ID gegen die Quell-Map (Konto-UID und
  Sub-ID-Bereich). Eine Datei mit einer ID außerhalb, auch in einem
  ACL-Eintrag, wird nicht gesichert und unter `rejected` vermerkt; ebenso
  **Gerätedateien (CHAR/BLOCK) aus rootless Domänen**, die grundsätzlich
  abgelehnt werden (V-1, V-3). Job-Status dann „mit Warnungen“.
- Das Manifest hält Quell-`uid_map` und `gid_map` sowie je Volume die
  vorkommenden IDs in Namespace-Sicht (`uidset`, `gidset`), **einschließlich
  der IDs aus ACL-Einträgen** (Access und Default), damit der Restore vorab
  prüfen kann, ob das Ziel sie abbilden kann (Praxis 2).
- Rootful: echte Host-IDs; Container mit `--userns=auto` oder `--uidmap`
  haben ihre Dateien trotzdem im Init-Namespace beschrieben.
- Treiber und Optionen [S §2]:

| Fall | Behandlung |
| --- | --- |
| `local` ohne Optionen | Daten aus `_data` |
| `local`, `type=none,o=bind,device=<pfad>` | nur wenn `device` unter `volume_bind_roots` der Kontopolicy liegt, sonst Planfehler (`_data` ist außerhalb laufender Container leer) |
| `local`, `type=tmpfs` | nur die Definition |
| `local`, `type=ext4/nfs/…` mit `device` | Planfehler im MVP |
| Treiber `image` | Referenz auf das Image |
| fremde Treiber (Volume-Plugins) | Planfehler |
| anonymes Volume (64-Hex-Name) | wie ein Named Volume, Warnung |

Lesen: Tiefensuche ab dem `_data`-Deskriptor; Hardlinks je Volume über
`(st_dev, st_ino)`, Löcher über `SEEK_DATA`/`SEEK_HOLE`; Stat beim Schließen
erneut vergleichen [S §2, S §3]. Jeder Scan-Eintrag bekommt eine
`source_ref`, die nur einmal verwendbar ist (Abschnitt „Attribute“).

### Quadlet und systemd

- Suchpfade rootful: `/run/containers/systemd/` > `/etc/containers/systemd/` >
  `/usr/share/containers/systemd/`; rootless: `$XDG_RUNTIME_DIR/containers/systemd/`,
  `~/.config/containers/systemd/`, `/etc/containers/systemd/users/<UID>/`,
  `/etc/containers/systemd/users/`; rekursiv, Symlinks erlaubt. Arten
  `.container`, `.pod`, `.network`, `.volume`, `.kube`, `.image`, `.build`
  [A §5].
- **Zuordnung** Container → Unit über das Label `PODMAN_SYSTEMD_UNIT`, dann
  `systemctl [--user] show` mit fester Property-Liste: `SourcePath` ist die
  Quadlet-Datei; `DropInPaths` nennt nur systemd-Drop-ins, keine
  Quadlet-Drop-ins [A §5, K §1.3].
- **Feste Property-Liste** für `unit-info` (V-6): `Id`, `Names`,
  `SourcePath`, `FragmentPath`, `DropInPaths`, `LoadState`, `ActiveState`,
  `SubState`, `UnitFileState`, `InvocationID`, `ActiveEnterTimestampMonotonic`,
  `Result`, `ExecMainCode`, `ExecMainStatus`, `NRestarts`, `Restart`,
  `TimeoutStopUSec`, `KillMode`, `Requires`, `Wants`, `BindsTo`, `PartOf`,
  `After`, `Before`, `WantedBy`, `EnvironmentFiles` (nur Pfade).
- **Vollständige Quellen** über `/usr/libexec/podman/quadlet -dryrun [-user]`
  mit leerer Umgebung (ohne `QUADLET_UNIT_DIRS`): schreibt nichts, gibt die
  erzeugten Units aus und nennt jede geladene Quelle und jedes Drop-in. So
  bleibt die Drop-in-Logik bei Podman [A §5].
- Gesichert unter `quadlet/` mit erhaltenem relativem Pfad je Suchpfad, dazu
  verwandte lokale Units (z. B. `tapir-ackervielfalt.target`) und ihre
  Drop-ins unter `systemd/`; bei `env=omit` mit geschwärzten
  `Environment=`-Werten (Abschnitt Umgebungsvariablen). Fingerprint je Unit:
  SHA-256 über die **Originale** von Quelle, Drop-ins und erzeugter Unit,
  vom Host berechnet und nur im Journal und Manifest geführt.
- Ohne laufenden Benutzermanager genügt der Dryrun; der Unit-Zustand ist dann
  `unknown`.

### Secrets, Bind Mounts, externe Abhängigkeiten

- Secrets nur als Metadaten und Verweis (Name, ID, Treiber, Ziele aus
  `CreateCommand`, Spec oder Quadlet `Secret=`); nie `--showsecret` [A §1].
- Bind Mounts und EnvironmentFiles sind `external` im Manifest. Der Host kennt
  das FileSet und setzt `covered_by_fd`; nicht abgedeckte Pfade nach
  `external=warn|error`. Auf Inspect-Pfade greift das Plugin nie zu.
- Host-Dienste (`After=postgresql.service`) stehen als `external.units` im
  Manifest, ohne Stop.

### Automatische Zusammenhänge

Der **Host** rechnet den Algorithmus auf den Inventardaten (in
`onesimus-podman-core`) und übergibt dem Gast die maßgebliche Objektmenge
samt Kanten; der Gast baut daraus nur Baum und Manifest (V-17). Jede spätere
Anfrage prüft der Host gegen diese Menge (ID **und** Name; geänderte ID
bedeutet Abweisung) [K §3.2].

Vom Ausgewählten aus **vorwärts** als Hülle (was es braucht); rückwärts nur
zur Information und für die Writergruppe. Jedes Objekt wird über
`(Domäne, Art, ID)` erkannt und steht einmal im Baum, mit `used_by` [A §4].

```text
Eingabe: Domäne D, Auswahl S, exclude E
1. Inventar: ps -a, pod ps, network ls, volume ls, images, secret ls, quadlet -dryrun
2. W := Treffer(S) − E                   (leer → Planfehler)
3. solange W nicht leer: x := pop(W), schon besucht → weiter
   Container: Pod (ganz, auch gegen E) → W; Dependencies → W (requires | volumes-from |
     netns/ipc/pid/uts container:<id> | pod-infra, unterschieden über HostConfig);
     Image; Mounts volume → volume, bind → external, image → image;
     Netze (außer Modi); Secrets → secret_ref; PODMAN_SYSTEMD_UNIT → Quadlet über SourcePath
   Pod: Infra und alle Mitglieder → W; InfraConfig.Networks → network
   Quadlet-Quelle: Drop-ins; Pod=/Network=/Volume=/Mount=/Image=-Verweise → W;
     [Unit] Requires/Wants/BindsTo/PartOf auf Quadlet-Units → W, auf lokale Units
     → systemd_unit, auf Host-Dienste → external.units;
     EnvironmentFile=, Hostpfade in Volume=/Mount= → external.paths
   Volume mit fremdem Treiber oder nicht erlaubten Optionen → Planfehler
4. Rückwärts: weitere Benutzer jedes Volumes/Netzes → dependents_not_selected;
   Mitbenutzer → Writergruppe
5. Ausgabe: Objekte, Kanten, Restore-Reihenfolge (network, volume, image,
   secret_ref, pod, container, quadlet, systemd_unit), Warnungen, Planfehler
```

## Attribute der Volume-Dateien

Das Bareos-Format trägt alles (voller Stat, `FT_LNKSAVED`, FIFO/Geräte,
Sparse mit Offset, ACL-Text, xattrs), der Plugin-Weg heute nicht: `plugin_ff`
setzt UID/GID auf euid/egid von `ofd`, löscht `FO_SPARSE`, `FO_ACL`,
`FO_XATTR`, `FO_CHKCHANGES`, kennt nur REG, DIR, LNK und übernimmt seit S-13
(`e1ecd00`) nur die Rechte-Bits `0o777` [S §3].

### Grundsatz: beim Sichern bereinigen (V-1)

Ein Restore in Dateien läuft ohne Plugin-Code, oft mit einem anderen FD
(Bareos, älteres `ofd`), und kennt nur den Attributsatz des Stroms. Ob eine
Datei aus einem rootless Konto stammt, ist dort nicht mehr erkennbar. Ein
Restore-Schalter schützt deshalb nicht; der Schutz muss beim Sichern
greifen. Sonst legt ein Konto, das im Namespace root ist, eine Datei mit
ns-UID 0 und Modus 4755 oder `cap_setuid+ep` an, und ein späterer Restore in
Dateien erzeugt daraus eine root-eigene setuid- oder Cap-Datei auf dem Host.

Regeln für Volume-Dateien im **Bareos-Strom** (rootless und rootful gleich,
damit auch V-19 erledigt ist):

- **IDs in Host-Sicht**: rootless die echten Sub-UIDs/Sub-GIDs vom
  Datenträger (ns 0 → Konto-UID, ns n → Sub-ID), rootful die Host-IDs.
  `podman unshare cp -a --sparse=always` kann eine solche Kopie auf demselben
  Host direkt übernehmen [K §2.3].
- **Modus nach S-13**: nur `0o777`; setuid, setgid und sticky stehen nie im
  Strom.
- **Nie `security.capability`**, nie `trusted.*`; xattrs nur `user.*`;
  `security.selinux` nur informativ in `ownership.tsv`.
- **ACL** als ACL-Stream mit numerischen Host-IDs (so wie auf der Platte).
- Dateitypen: REG, DIR, LNK, FIFO, Hardlink (`FT_LNKSAVED`). CHAR/BLOCK nie
  im Strom: rootless abgelehnt (oben), rootful im MVP ausgelassen und unter
  `rejected` vermerkt. Sockets werden nicht gesichert.
- Sparse mit Offsets (`FO_SPARSE`), Zeiten aus dem Scan.

**`ownership.tsv`** je Volume enthält die Namespace-Sicht. Der **Host**
erzeugt sie, nicht der Gast, und sie folgt im Strom auf `data/`. Kopf:
Domäne, Quell-`uid_map` und `gid_map`; damit ist die ns-Sicht jeder ID im
Strom eindeutig bestimmt. Je Pfad eine Zeile für jeden Eintrag mit
Sonderbits, Caps oder ACL: ns-UID, ns-GID, voller Modus, Caps (Text und
Rohwert in ns-Sicht, v2), ACL in ns-IDs, SELinux-Label. Grenze der
Zeilenzahl konfigurierbar, darüber Fehler. Angewandt wird `ownership.tsv`
**nur beim Online-Restore unter Kontrolle des Hosts**. Ein Restore in Dateien
legt sie als gewöhnliche Datei (0600, euid) ab; der Restore-Bericht nennt
die Zahl der nicht angewandten Sonderbits und Caps.

### Erweiterung von `FileEntry` (`api_version` 2)

Der Gast liefert für Volume-Dateien und Blobs **weder Bytes noch Attribute**
(V-2), sondern nur den Verweis auf einen Scan-Eintrag des Hosts:

```protobuf
message FileEntry {
  // Felder 1–7 unverändert (path, kind, size, modified_unix_nanos, mode, symlink_target, attributes)
  bytes source_ref = 8;   // Scan-Eintrag des Hosts (Volume-Datei, Blob, ownership.tsv); leer = synthetischer Eintrag
}
// FileKind: FIFO = 5, HARDLINK = 6 (nur mit source_ref)
```

- Synthetische Einträge (Inspect, Manifest usw.) bleiben wie heute: Bytes vom
  Gast über `ReadFile`, Attribute nach S-13.
- Einträge mit `source_ref`: Pfad = Präfix des Objekts
  (`/@podman/<domäne>/volumes/<vol>/data/` bzw. `images/oci/blobs/sha256/`)
  plus `relpath` bzw. Digest des Scan-Eintrags; eine Abweichung ist ein
  Fehler. Jede `source_ref` gilt einmal und nur in diesem Job und dieser
  Instanz. Der Host liest die Bytes selbst über den Scan-Handle, `ReadFile`
  wird für solche Einträge nicht aufgerufen, und `plugin_ff` baut Stat,
  ACL-, xattr- und Sparse-Angaben aus der eigenen Tabelle nach den Regeln
  oben. Der Gast bestimmt nur, **ob** und **in welcher Reihenfolge** ein
  Eintrag aufgenommen wird.
- Voraussetzungen: das Plugin hat `podman.volume.read` bzw.
  `podman.image.read`, das Objekt gehört zur erlaubten Objektmenge.
- `JobContext` bleibt in P2 unverändert (ohne Level/Since).

### Restore online: Attribute und Reihenfolge je Datei

Die Attribute bestimmt der Host aus dem Attributsatz des Stroms und aus
`ownership.tsv`, nicht aus der Anfrage des Gastes (V-9). Die Bytes fließen
aus dem Strom über den Host zum Schreiber; der Gast plant nur. IDs rechnet
der Host über die Quell-Map in die ns-Sicht um. Rootless schreibt der
Broker im Ziel-Namespace, der Kernel bildet die IDs auf den Zielbereich ab
und schreibt Caps als v3 mit der rootid des Ziels um (bestätigt, Praxis 2).
Rootful schreibt `ofd` echte IDs, nur in ein in diesem Job neu angelegtes
Volume.

Relativ zum `_data`-Deskriptor mit
`openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS)` und `O_EXCL` [S §3, K §2.2]:

1. `mkfifo`/`open(O_EXCL)`; Symlink-Ziel unverändert, nie verfolgt
2. Daten, Löcher per `ftruncate` und `pwrite`
3. `fchown`
4. `fchmod` mit den Rechte-Bits
5. `user.*`-xattrs
6. ACL
7. `utimensat`; Verzeichniszeiten ganz zum Schluss, wie `restore.rs`
8. Zweiter Durchgang nach `ownership.tsv` (folgt im Strom auf `data/`):
   setuid/setgid/sticky per `fchmod` und `security.capability`, beides nach
   dem `chown` des ersten Durchgangs, über erneut per `openat2` geöffnete
   Deskriptoren; danach `utimensat` erneut für diese Einträge.

Hardlinks über Volume-Grenzen werden nicht erhalten und vermerkt. SELinux
nach `selinux=relabel|keep|drop`; MCS-Kategorien von `:z`/`:Z` gelten auf
einem anderen Host nicht [S §3].

## Broker und Rootful-Backend

**libpod über die CLI** (Vorschlag, offen; ersetzt die REST-Variante aus
[K §3.2]): `podman` mit `--format json`, fester Argumentliste und fester
Umgebung, statt eines Sockets. Begründung: REST und CLI liefern byte-gleiche
Inspect-Daten; `podman.socket` ist auf forgeiec.io deaktiviert und darf nicht
aktiviert werden; `podman info` meldet `RemoteSocket.Exists: true` auch ohne
Socket [A §1]. Die libpod-DB erzwingt die passende Speicherkonfiguration
(„database run root … does not match“) [A §1, A §6].

**Umgebung des rootless Brokers** (V-8, Praxis 6): `HOME` (aus passwd),
`XDG_RUNTIME_DIR=/run/user/<uid>` falls vorhanden, festes `PATH`, `LANG=C`
und **alle drei** internen Variablen
`_CONTAINERS_USERNS_CONFIGURED=done`, `_CONTAINERS_ROOTLESS_UID=<uid>`,
`_CONTAINERS_ROOTLESS_GID=<gid>`. Mit nur einer davon hält sich Podman für
rootful und benutzt `/run/libpod`, `/var/cache/containers` usw. (Praxis 6).
Die Variablen sind intern und gehören zum zertifizierten Versionsprofil. Nie
`CONTAINERS_CONF`, `CONTAINERS_STORAGE_CONF`, `QUADLET_UNIT_DIRS`,
`DOCKER_HOST`.

**Namespace und Start** (V-3, V-8, Praxis 6): `PR_SET_NO_NEW_PRIVS`
blockiert `newuidmap`/`newgidmap`; ohne Pause-Prozess scheitert rootless
Podman dann (bestätigt, Praxis 6). Deshalb legt `ofd` den Namespace selbst an:

1. Ein Kind von `ofd` ruft **als root** `unshare(CLONE_NEWUSER|CLONE_NEWNS)`.
   Damit **gehört der Namespace root**, und das Konto hat darin kein
   `CAP_SYS_PTRACE`. Der eigene Mount-Namespace ist nötig, sonst scheitert
   z. B. das shm-Mount von `podman run` (Praxis 6).
2. `ofd` schreibt `uid_map`/`gid_map` aus `/etc/subuid`/`/etc/subgid`
   (`0 <uid> 1`, `1 <subuid> <anzahl>`).
3. Das Kind wechselt im Namespace auf ns-UID/GID 0 (= Konto-UID/GID),
   `setgroups([])`, setzt NNP, `PR_SET_PDEATHSIG`, RLIMIT_CORE 0 und
   startet den Broker per `fexecve` aus dem geprüften Deskriptor.
4. Der Broker setzt als **erste Handlung `PR_SET_DUMPABLE(0)`**, weil
   `execve` dumpable wieder auf 1 setzt (V-3).

Einem vorhandenen Pause-Prozess tritt `ofd` **nicht** bei: dessen Namespace
gehört dem Konto. Der Broker braucht weder `newuidmap` noch den
Pause-Prozess; im Versuch blieben nach `podman stop` aus einem fremden
Namespace keine verwaisten Mounts (Praxis 6). Die Praxisprüfung lief mit dem
Konto als Eigentümer des Namespace; P0 muss die Reihenfolge mit root als
Eigentümer nachweisen (Exitkriterium P0).

**systemd und Start von Containern außerhalb des Namespace** (V-8): Aus dem
Namespace heraus kann `systemctl --user` an der D-Bus-Authentisierung
scheitern, und ein `podman start` legte die Mounts im Namespace des Brokers
an, der mit ihm verschwindet. Deshalb ruft `ofd` als Konto-UID **ohne**
User-Namespace, mit NNP, leerer Umgebung plus `XDG_RUNTIME_DIR` und fester
Argumentliste:

- `systemctl --user show|stop|mask --runtime|unmask --runtime|start`;
- für einfache Container ohne Unit `systemd-run --user --wait --collect
  podman start <id>`, damit der Container unter dem Benutzermanager im
  Namespace des Pause-Prozesses startet.

Ohne laufenden Benutzermanager ist `consistency=stop` für diese Domäne ein
Planfehler (Vorschlag, offen). Rootful prüft das Profil
`cgroup_manager=systemd`; sonst ist `podman start` aus `ofd` ein Planfehler,
weil conmon und Container in der cgroup von `ofd.service` blieben und mit
`KillMode=control-group` beim nächsten Neustart von `ofd` endeten (V-21).

Ein Broker je (Job, Host-UID, Domäne). Nachrichten in
`proto/podman_broker.proto` (Paket `onesimus.filed.broker.podman.v1`), Rahmen
wie PostgreSQL: 4 Byte Länge + Nachricht, `MAX_FRAME` 2 MiB [K §3.3]. Das
Rootful-Backend implementiert dieselben Operationen in `ofd`.

| Operation | Ausführung (fest) | Grant | Host-Prüfung |
| --- | --- | --- | --- |
| `engine-identify` | Broker/Backend: `podman info/version --format json` | `podman.inventory` | UID, Maps gegen `/etc/subuid`, graphroot gegen Kontopolicy, Version/Treiber/`cgroup_manager` gegen zertifiziertes Profil, Domänen-ID |
| `inventory` | Broker/Backend: `ps -a`, `pod ps`, `network ls`, `volume ls`, `images`, `secret ls` | `podman.inventory` | Objektmenge berechnen, Schnappschuss für `PREPARED` |
| `inspect(kind, ids)` | Broker/Backend: `… inspect`, `generate spec`; nie `--showsecret` | `podman.inventory` | ID in der Menge, Größe ≤ 1 MiB, Positivliste, Env nach `env=` |
| `quadlet-resolve` | Broker/Backend: `quadlet -dryrun [-user]` | `podman.inventory` | nur Zuordnung, nie gespeichert |
| `unit-info(units)` | Host: `systemctl [--user] show -p <feste Liste>` | `podman.inventory` | Unit gehört zur Menge |
| `quadlet-read(unit, path)` | Host: `openat2` ab dem Suchpfad | `podman.inventory` | Pfad aus Dryrun bzw. `SourcePath` **und** unter einem Suchpfad; EnvironmentFiles nie; Schwärzung bei `env=omit` |
| `blob-scan`, `layer-stream` | Host: `images.json`/`layers.json` unter Lese-Sperre, tar-split, `openat2` ab Layer-Verzeichnis | `podman.image.read` | DiffID in der Menge; Typ und Größe vorab, SHA-256 am Ende |
| `volume-scan` | Host: `openat2` ab `_data` | `podman.volume.read` | (dev, ino) gegen Inventar; IDs gegen Quell-Map; CHAR/BLOCK rootless abgelehnt |
| `member-state` | Host: `systemctl show`, `podman inspect` | `podman.inventory` | lesend, auch für den Gast |
| Stop, Maske, Wiederanlauf | **nur Host**, nie Gastfunktion (V-9): `systemctl [--user] stop`, `mask --runtime`, `unmask --runtime`, `start`; `podman stop/start` | `podman.lifecycle` | nur nach Journal-Eintrag; erwartete InvocationID/StartedAt |
| `preflight`, `create-network/volume/pod/container`, `volume-write-*`, `image-import`, `pull`, `stage-quadlet`, `stage-unit`, `rollback` | siehe Restore | `podman.restore` | Suffix, Pflichtvergleich der Sicherheitsfelder, Attribute vom Host, Restore-Label |

Der Broker ruft nie `system reset`, `system migrate` oder `system renumber`
auf [K §0]. Für das Rootful-Backend protokolliert `ofd` jede ausgeführte
Argumentliste (Exec-Protokoll); der Test vergleicht es mit dieser Tabelle
(V-18).

## Konsistenz

**Writergruppe automatisch** [K §1.3]: Zusammenhangskomponente der Auswahl
über gleiche Named-Volume-ID, gleiche Bind-Quelle (realpath, Gerät/Inode),
Pod, `--volumes-from`, systemd `Requires/Wants/BindsTo/PartOf` zwischen Units
mit `PODMAN_SYSTEMD_UNIT` und Quadlet-`.volume`/`.network`-Verweise. Units
außerhalb von Podman (z. B. `postgresql.service`) nur als Information.

### Stop-Verfahren

Gemessen (Praxis 4): `mask --runtime` löst ein `daemon-reload` aus. Danach
sind bei der maskierten, noch laufenden Unit `ExecStop=podman rm …`,
`KillMode=mixed`, `TimeoutStopSec` und `ExecStopPost` weg. systemd schickt
SIGTERM an die ganze cgroup, wartet die Vorgabe von 90 s und schickt
SIGKILL; die libpod-DB behält „Up“, `--rm` läuft nicht. Die frühere
Reihenfolge „erst maskieren, dann stoppen“ ist damit **widerlegt** und
gestrichen. forgeiec.io hat dieselbe Lage (systemd 257, alle tapir-Units mit
`ExecStop=/usr/bin/podman rm -v -f -i --cidfile=…`, `KillMode=mixed`,
`TimeoutStopUSec=1min 30s`).

- **Container mit `PODMAN_SYSTEMD_UNIT`** (Vorschlag, offen; siehe
  Nutzerentscheidungen):
  1. Zustand der Maske **vorher** ins Journal (`mask_before`: keine, runtime,
     persistent; V-5). Eine vorher vorhandene Maske ist nie die eigene; ist
     eine Unit der Gruppe vorab maskiert, ist das ein Planfehler.
  2. `systemctl [--user] stop <unit>` mit intakter Unit-Definition, also über
     ExecStop, KillMode und TimeoutStopSec (sauber in 2,2 s im Versuch).
  3. `systemctl [--user] mask --runtime <unit>`.
  4. Prüfen: `LoadState=masked`, `ActiveState=inactive`, keine neue
     InvocationID. Hat ein `Wants=`, ein Timer oder ein Bediener die Unit im
     kurzen Fenster zwischen 2 und 3 gestartet, gilt das als
     `consistency_violated`: Maske entfernen, kein zweiter Stop, Job fehlerhaft.

  Die Alternative `mask --runtime --no-reload` → `stop` → `daemon-reload`
  hält ebenfalls sauber an; die Maske wirkt aber erst nach dem Reload, das
  ungeschützte Fenster umfasst also die ganze Stop-Dauer (bis 90 s) statt
  Millisekunden (Praxis 4). Ein `podman stop` auf einen Quadlet-Container ist
  wirkungslos: systemd startet mit neuer Container-ID neu (bestätigt,
  Praxis 4). Die Laufzeit-Maske liegt vor `/run/systemd/generator`, wirkt
  gegen `Restart=`, `Wants=` anderer Units, Timer, manuelles `start` und
  `podman auto-update`, verschwindet beim Neustart von selbst und **bleibt
  nach einem Absturz des steuernden Prozesses bestehen** (bestätigt,
  Praxis 4).
- **Wiederanlauf**: `systemctl [--user] unmask --runtime <unit>`, dann
  `start`. Ausdrücklich `--runtime`, weil `unmask` ohne diese Option auch
  eine dauerhafte Maske des Administrators entfernt (V-5).
- **Container ohne Unit**: `podman stop -t <stop_timeout>`; `StoppedByUser`
  verhindert die Restart-Policy; Wiederanlauf über den Benutzermanager bzw.
  rootful mit `cgroup_manager=systemd` (Abschnitt Broker).
  `podman-restart.service` startet beim Boot auch vom Benutzer gestoppte
  Container (Recovery berücksichtigt das).
- Reihenfolge: abhängige Units zuerst (ackervielfalt-web/worker/beat vor
  redis), Wiederanlauf umgekehrt, jeweils auf `ActiveState=active` warten.
- **Sauberkeitsnachweis**: Exit-Code in `clean_exit_codes`, kein SIGKILL nach
  Frist (137), systemd `Result=success`. Sonst `on_unclean`.
- Pause/Freeze ergibt nur einen Absturzzeitpunkt bei voller Ausfallzeit; P4
  [K §1.2].

### `consistency_scope=job`

Vorgabe (Nutzer, 19.11.2026): tapir hat keine Named Volumes; seine Daten
liegen in Bind Mounts, die der FD als Dateien sichert. Heute öffnet `ofd`
eine Plugin-Sitzung erst, wenn die Traversierung die Plugin-Zeile erreicht
(`backup.rs` `save_plugin`/`run_plugin`). Nötige Host-Änderungen (V-5):

1. **Einhängepunkt** in `job/backup_cmds.rs` `blast_data()` **vor**
   `Finder::find_files`.
2. **Vorlauf für alle Zeilen vor dem ersten Stop**: Bundle und Signatur,
   Plugin-ID, Grants (`podman.lifecycle`, `podman.rootful`), Kontopolicy,
   Broker starten, Inventar-Schnappschuss, Inspect, Plan rechnen, Journal
   `PREPARED` für jede Gruppe. Ein Planfehler in irgendeiner Zeile bricht ab,
   bevor eine Gruppe gestoppt ist.
3. **SD bereit**: Der Stop beginnt erst, wenn das SD die Schreibsitzung
   angenommen hat (Gerät reserviert, Volume eingelegt), damit ein Warten auf
   ein Medium nicht in die Ausfallzeit fällt (V-4). Welche SD-Antwort das
   garantiert, wird in P2 nachgewiesen.
4. **Wiederanlauf als Guard**: ein Guard-Objekt fährt die Gruppen nach der
   letzten Datei des Jobs wieder an, ebenso bei frühem `return`, Cancel,
   SD-Fehler und Panik (`ofd` baut mit `panic=unwind`, heute Vorgabe; mit
   `abort` bliebe nur der Totmannschalter).
5. **SIGTERM** (`systemctl stop ofd`, auch beim Paket-Upgrade wegen
   `Depends: ofd (= Version)`): SIGTERM setzt das Cancel-Flag, der Guard fährt
   innerhalb von `cancel_timeout` wieder an, erst dann endet `ofd`.
   `TimeoutStopSec` von `ofd.service` muss größer sein als `cancel_timeout`
   plus Wiederanlauf aller Gruppen (Paketierung anpassen).
6. **Estimate und Verify stoppen nie** (`EstimateHandler::plugin` meldet
   heute nur „not included“; bleibt Regel).
7. Mehrere Zeilen mit überschneidenden Gruppen bilden eine Lifecycle-Gruppe.

**Obergrenze und Totmannschalter** (V-4): `max_downtime` (Vorgabe 7200 s,
Vorschlag, offen) zählt ab dem ersten Stop. Wird sie erreicht, bricht `ofd`
den Job ab und fährt wieder an. Damit das auch gilt, wenn `ofd` hängt (etwa
blockiert im Senden an das SD) oder stirbt, legt `ofd` **vor** dem ersten Stop
einen Totmannschalter an:

```sh
systemd-run --unit=onesimus-podman-recover-<jobid> --on-active=<max_downtime> \
  --timer-property=AccuracySec=1s /usr/sbin/ofd --podman-recover <domäne>/<jobid>
```

`ofd --podman-recover` führt **dieselbe Recovery** aus wie der Start von
`ofd`, mit derselben Fingerprint-Prüfung; das frühere Gegenargument („prüft
keinen Fingerprint“ [K §1.3]) entfällt damit. Eine Sperre (`flock`) auf der
Journaldatei verhindert doppelte Ausführung. Der Timer steht im Journal und
wird bei `RESUMED` entfernt. Hat der Timer wieder angefahren, während `ofd`
noch sichert, sieht `ofd` den Journalzustand und beendet den Job als
`consistency_violated`. Bei `kill -9` von `ofd` greift zusätzlich die
Recovery beim Neustart (`Restart=on-failure`); nach `StartLimitBurst` bleibt
der Timer der einzige Weg, deshalb ist er Pflicht.

`consistency=crash`: keine Mutation; je Datei `size`/`mtime`/`ctime` vor und
nach dem Lesen vergleichen, Änderungen als `changed_during_backup` im
Manifest (Warnung).

### Journal mit Recovery

Beim Host, nicht im Broker oder Gast: root-eigene Datei
`/var/lib/onesimus/ofd/podman-journal/<domäne>/<jobid>.json`, `fsync` und
`rename`, geschrieben **vor** jeder Mutation, für rootless und rootful
gleich [K §1.3]. Inhalt: Boot-ID, Job und Instanz, Host-UID und Domäne
(einschließlich `tmpdir`), Name des Totmannschalters, Zeit des ersten Stops,
je Mitglied Art, Name, Unit-`InvocationID` und `ActiveEnterTimestamp` bzw.
Container-ID und `StartedAt`, Definitions-Fingerprint (Quadlet-Quelle,
Drop-ins, erzeugte Unit bzw. Spec), Ursprungszustand, **`mask_before`**,
eigene Maske (ja/nein), Stop-Ergebnis (Exit-Code, sauber, Zeit).

| Zustand | wird geschrieben | Recovery, wenn dies der letzte Zustand ist |
| --- | --- | --- |
| `PREPARED` | nach Inventar und Inspect aller Mitglieder, Totmannschalter angelegt (Pflicht vor dem Stop) | nichts mutiert → Timer entfernen, `ABORTED` |
| `STOP_REQUESTED` | je Mitglied vor `stop`, mit `mask_before` | aktiv mit gleicher InvocationID: nichts passiert. Inaktiv, keine eigene Maske, Fingerprint unverändert, kein späterer Start: `start`. Sonst `CONFLICT` |
| `MASK_REQUESTED` | je Mitglied nach dem Stop, vor `mask --runtime` | Laufzeit-Maske vorhanden und `mask_before` = keine: eigene Maske → wie `RESUME_PENDING`. Keine Maske: wie `STOP_REQUESTED` |
| `QUIESCED` | alle Mitglieder gestoppt und maskiert, geprüft | wie `RESUME_PENDING` |
| `CAPTURING` | vor der ersten Datei | wie `RESUME_PENDING`; Job fehlgeschlagen |
| `RESUME_PENDING` | nach der letzten Datei, bei Cancel, Fehler, SIGTERM oder `max_downtime` | `unmask --runtime` und `start` **nur**, wenn gleiche Boot-ID, Mitglied inaktiv, Maske die eigene, Fingerprint unverändert, bei Containern `StoppedByUser` und `FinishedAt` = eigener Stop. Sonst `CONFLICT` |
| `RESUMED` | alle Mitglieder im Ursprungszustand | Timer entfernen, archivieren |
| `CONFLICT` | – | manuelle Klärung; `ofd --podman-journal` zeigt, `--resolve <job>` entfernt nur die eigene Maske |

Sonderfälle: **Neue Boot-ID**: die Maske ist weg, systemd hat nach `WantedBy`
gestartet; Units nur prüfen, einfache Container ohne Restart-Policy nur bei
unverändertem Fingerprint starten. **Bediener startet während der
Sicherung** (InvocationID bzw. Container-ID geändert): Job fehlerhaft,
`consistency_violated`, kein zweiter Stop, kein Wiederanlauf durch uns.
**Cancel**: sofort `RESUME_PENDING`, Wiederanlauf mit eigenem Zeitbudget und
frischer Broker-Instanz; ein begonnener `systemctl stop` wird nicht
wiederholt, sondern sein Ergebnis gelesen. **`ofd` stirbt oder hängt**:
Recovery beim Start von `ofd`, vor jedem neuen Podman-Job der Domäne und
spätestens durch den Totmannschalter nach `max_downtime`.

## Threading, Grenzen, Abbruch

Eine Plugin-Instanz und ein Broker je (Job, Host-UID, Domäne); verschiedene
Konten laufen voll parallel, innerhalb einer Instanz ein geordneter Worker
(`NextFile`/`ReadFile` sind seriell). Leases beim Host, Schlüssel
(Host, Domäne, Objekt-ID): **Lifecycle-Lease** über die ganze Writergruppe von
`STOP_REQUESTED` bis `RESUMED` (ein zweiter Job wartet bis zur Frist oder
bekommt einen Planfehler und nimmt nie eine fremd gestoppte Gruppe wieder
auf); teilbare Lese-Leases; Restore-Leases auf Zielnamen. Kein globaler Lock
über I/O [K §4].

Kein Spool beim Sichern: Layer und Volume-Dateien liest der Host
offsetbasiert. **Spool nur für den Image-Import beim Restore** (V-7, [K §4]):

- Basis `/var/lib/onesimus/podman-spool/`, Eigentümer root, Modus **0700**,
  nie unter `/var/tmp` oder `/tmp`.
- Angelegt mit einer **`O_NOFOLLOW`-Kette**: jedes Element per `mkdirat` und
  `openat(O_DIRECTORY|O_NOFOLLOW)` ab dem Deskriptor des Elternverzeichnisses,
  Eigentümer und Modus nach dem Öffnen geprüft; ein vorgefundener Symlink oder
  fremder Eigentümer bricht ab.
- Je Job ein Unterverzeichnis `<domäne>/<jobid>`. Rootless wird es per
  `fchown` dem Konto übergeben; der Broker erreicht es nur über einen
  geerbten Verzeichnis-Deskriptor (`/proc/self/fd/<n>`), weil er die
  0700-Basis nicht durchlaufen kann. Nachweis in P3, dass
  `podman pull oci:/proc/self/fd/<n>/…` funktioniert; Ausweichweg ist
  `podman load` aus stdin ohne Spool (Vorschlag, offen).
- Löschen nur per `unlinkat` ab dem geprüften Deskriptor mit
  `openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS)`, bei Close, Cancel und
  Recovery; root folgt dabei nie einem Pfad, den ein Konto gewählt hat.
- Quote = Summe der `diff-size` der benötigten Layer, `statvfs` vorab.

Grenzen je Instanz: offene Handles (64), Einträge je Scan-Antwort (1024),
Pfadlänge (4096), xattr (64 KiB), Inspect/Spec (1 MiB), Zeilen in
`ownership.tsv`, Objekte im Inventar (konfigurierbar). Abbruch:
Epoch-Unterbrechung des WASM-Stores, SIGTERM/SIGKILL an die
Broker-Prozessgruppe, danach Recovery mit eigenem Budget (`cancel_timeout`,
`recovery_timeout`).

## Restore

### `restore_mode=files`

Der Baum landet unter `Where` wie im SD-Strom, ohne Plugin-Code. Weil der
Strom schon bereinigt ist (V-1), entstehen dabei keine setuid-, setgid- oder
Cap-Dateien, unabhängig davon, welcher FD restauriert; Volume-Dateien tragen
Host-IDs. Von Hand nutzbar: Images per
`podman pull oci:<Where>/@podman/<domäne>/images/oci:<name>`; Volume-Daten
auf demselben Host per `podman volume create` und
`podman unshare cp -a --sparse=always` [K §2.3]; Sonderbits und Caps nennt
`ownership.tsv`. Quadlet-Quellen (bei `env=omit` als Vorlage) und
Inspect-Daten dienen als Vorlage.

### `restore_mode=online` (gestagt)

Ablauf mit Restore-Journal `PLANNED → CREATING(n) → FILLED → VERIFIED →
STAGED`; jede angelegte Ressource trägt das Label
`org.onesimus.restore-job=<jobid>` und steht in der Liste `created` [K §2.2].

0. **Vorprüfung ohne Mutation**: Zielkonto freigegeben; Mapping groß genug
   für `uidset`/`gidset` **einschließlich der IDs aus ACL-Einträgen** (sonst
   scheitert `setxattr(system.posix_acl_*)` mit EINVAL, Praxis 2);
   Plattform; alle Zielnamen mit Suffix frei; Subnetze und Bridge-Namen frei;
   Secrets vorhanden; Images verfügbar oder schon vorhanden; Ports und
   statische IPs gegen Bestand; Pflichtvergleich der Sicherheitsfelder für
   jede geplante Anlage. Ergebnis: Plan mit Konfliktliste; Planfehler brechen
   vor jeder Mutation ab.
1. **Netze** aus Inspect mit Erlaubnisliste (bridge/macvlan/ipvlan, Subnetz,
   Gateway, IP-Bereich, IPv6, internal, DNS, Optionen, Labels), Name mit
   Suffix.
2. **Volumes** neu anlegen: Treiber `local` **ohne** `o=bind`/`device`
   (V-14); prüfen: leer, `CreatedAt` ≥ Jobstart, Restore-Label. Befüllen nach
   dem Abschnitt „Restore online: Attribute“; Ergebnis wie
   `cp -a --sparse=always` im Namespace [K §2.1, Praxis 2]. Danach Vergleich
   mit dem Manifest (Anzahl, Größen, Prüfsummen). Nie `podman volume
   import`: es führt Inhalte zusammen [K §2.1].
3. **Images**: **vorhandene Image-ID nur referenzieren, nicht pullen**
   (Praxis 1). Sonst `reference` per Pull nur als `name@sha256:…` aus
   erlaubten Registries, dann Image-ID prüfen; `blobs` als OCI-Layout im
   Spool, feste Befehlszeile `podman pull oci:<spool>:<ref>`, danach Image-ID
   gleich Config-Digest und Manifest-Digest gleich `restore_digest`.
4. **Pods, dann Container** nach `restore_strategy`: `create-command` aus der
   geprüften Positivliste; `spec` aus Spec plus Inspect-Ergänzung, ohne
   Quell-`idmappings` und automatische Aliase. Name mit Suffix, Volumes nur
   aus diesem Job, Netze aus diesem Job oder freigegeben, Bind Mounts nur über
   `bind_map`, `--requires` auf Suffix-Namen, Variablen ohne Wert
   weggelassen, Label `restored-from`, **nicht starten**. Anlage über libpod
   create; danach Pflichtvergleich der Sicherheitsfelder am neuen Inspect
   (Abschnitt Container); mehr Rechte als das Original → Rückbau. pid/ipc/
   net=host nur, wenn im Original **und** von der Policy erlaubt
   [K §2.1, K §3.3].
5. **Quadlet und lokale Units**: Quellen umschreiben (Suffix für
   `ContainerName` und Unit-Namen, `.volume`- und `.network`-Verweise,
   Hostpfade in `Volume=`/`Mount=` nur über `bind_map`, sonst Planfehler,
   `[Install]` entfernen, `image_pin`-Drop-in). Verwandte lokale Units unter
   `systemd/` (z. B. `tapir-ackervielfalt.target`) werden **mitgestagt**, mit
   Suffix und umgeschriebenen `PartOf=`/`Wants=` (V-13). Alles **außerhalb
   aller Quadlet- und Unit-Suchpfade** ablegen: rootless
   `~/.local/share/onesimus/staged/<jobid>/`, rootful
   `/var/lib/onesimus/podman-staged/<jobid>/`. Unterverzeichnisse der
   Suchpfade sind verboten, weil Quadlet sie rekursiv liest [K §2.1].
   Quadlet-Container werden nicht vorab angelegt; Quadlet erzeugt sie beim
   Start. `EnvironmentFile=` bleibt ein Verweis; offene Platzhalter
   (`env=omit`) verhindern die Promotion.
6. **Bericht**: angelegte Objekte, offene Punkte (Secrets, weggelassene
   Variablen, offene Platzhalter, Ports, EnvironmentFiles, `lossy_fields`,
   nicht angewandte Einträge). Promotion (Quellen in den Suchpfad,
   `daemon-reload`) und Start gibt der Administrator getrennt frei.

Fehler nach Schritt 0: Rückbau **nur** der Objekte in `created` mit eigenem
Label; teilweise befüllte Volumes werden gelöscht, nicht repariert; nach
Absturz Recovery aus dem Restore-Journal nach derselben Regel.

libpod create ohne Socket (Vorschlag, offen): Broker bzw. Rootful-Backend
starten für die Dauer des Restores einen privaten
`podman system service --time=0 unix://<privates-verzeichnis>/s.sock` als
eigenes Kind, beenden ihn danach und prüfen das Ende per PID (ein solcher
Dienst überlebt sonst die Shell, Praxis 3); kein fremder Socket, keine
Unit-Aktivierung [A §1]. Technisch bestätigt (Praxis 3). Alternative: der
Host erzeugt aus der Spec eine geprüfte `podman create`-Argumentliste.

### Konflikte

| Konflikt | Verhalten von Podman | Behandlung |
| --- | --- | --- |
| gleichnamiges Netz | „network name … already used“ (Praxis 7) | Suffix; bei `existing` muss das Subnetz passen, sonst Planfehler |
| Subnetz überlappt (auch `10.88.0.0/16`) oder Host-Route | „subnet … is already used on the host or by another config“ (Praxis 7) | `network_subnet=keep`: Planfehler; `auto`: neues Subnetz, Warnung |
| Quadlet-`.network` existiert mit anderem Subnetz | `--ignore` verschweigt es (Praxis 7) | Planfehler |
| `network_interface` (Bridge-Name) belegt | „bridge name … already in use“, `create` scheitert (Praxis 7) | Vorprüfung; Feld weglassen, netavark vergibt; vermerken |
| macvlan/ipvlan-Parent fehlt | – | Planfehler (später `netmap=`) |
| statische IP belegt | `create` gelingt, `start` scheitert („already allocated“, Praxis 7) | Planfehler bei `restore_ip=keep`, sonst Warnung |
| Host-Port belegt, rootless < 1024 | `create` gelingt, `start` scheitert; unter `Restart=always` Neustartschleife [K §2.1] | Warnung im Plan; Container bleiben gestoppt |
| fehlendes Secret | `create` scheitert sofort [K §2.1] | Planfehler mit Name |
| gleichnamige Quadlet-Quelle oder Unit im Ziel | – | Planfehler bei Promotion |
| Standardnetz `podman` abweichend | – | Referenz plus Subnetzangabe, Warnung |
| rootful → rootless | macvlan/ipvlan, feste Bridge-Namen eingeschränkt | Planfehler bei nicht unterstützten Treibern |
| ns-IDs außerhalb des Ziel-Mappings, auch in ACLs | `setxattr` EINVAL bzw. `chown` EINVAL (Praxis 2) | Planfehler „subuid-Bereich zu klein“ |
| Image-ID im Ziel schon vorhanden | `pull oci:` ändert Digest und RepoDigests (Praxis 1) | nicht pullen, nur referenzieren |
| `NeedsCopyUp`/`NeedsChown` am neuen Volume | Podman kopiert nur in leere Volumes, `chown` nur der Wurzel (Vermutung) [S §2] | Testfall; Original-Flags im Inspect |

Zwei Speicher desselben Kontos dürfen gleiches Subnetz und gleichen
Bridge-Namen haben, weil `rootless-netns` je runroot liegt; die Vorprüfung je
Domäne genügt (Praxis 7).

## Später: Accurate und Incrementals

MVP ist Full-only. `JobContext` hat weder Level noch Since; `save_plugin_file`
ruft `accurate_check_file` nicht auf und `plugin_ff` löscht `FO_CHKCHANGES`
[S §4]. Für P4: `JobContext` um `level` und `since_unix` erweitern,
`accurate_check_file` mit dem Stat des Scan-Eintrags aufrufen, Löschungen über
`accurate_finish` (`FT_DELETED`). Image-Blobs fallen dank Digest-Pfad und
fester `mtime`/`ctime` automatisch aus Incrementals heraus; Volumes liefern
alle Einträge, Daten nur für geänderte Dateien (ctime erfasst auch `chown` und
xattr). Ebenfalls P4: `consistency=freeze` (Pause plus Snapshot, sobald
Volumes als Subvolumes zertifiziert sind [K §1.1]), signierte App-Adapter,
Remote, weitere Storage-Treiber, rootful CHAR/BLOCK.

## Abnahmefall forgeiec.io

Rootful Debian 13, Podman 5.4.2, systemd 257, overlay auf ext4, SELinux aus,
netavark [S Einleitung, A §4, K §1.1]: fünf Quadlet-Units `tapir-*` in
`/etc/containers/systemd/` (alle `Restart=always`, `Network=host`, Label
`PODMAN_SYSTEMD_UNIT`, `ExecStop=podman rm -v -f -i --cidfile=…`,
`KillMode=mixed`), **`tapir-ackervielfalt.target`** in `/etc/systemd/system`
(`PartOf`), `Wants=tapir-redis.service`, `After=postgresql.service` (Host),
`EnvironmentFile=/etc/tapir/keycloak.env` und
`/etc/tapir/instances/ackervielfalt.env`, Bind Mounts
`/var/lib/tapir/{ackervielfalt,redis,keycloak/themes/wirgarten}`, keine
Named Volumes, keine eigenen Netze, keine Pods, keine Podman-Secrets; drei
Images (tapir `:latest` 1,11 GB, keycloak 26.8.0, redis 7.4-alpine), keine
gemeinsamen Layer. Datenbanken von tapir und Keycloak im Host-PostgreSQL.
Daneben `www-data` mit Matomo, nicht freigegeben.

```text
FileSet {
  Name = "forgeiec-tapir"
  Include {
    Options { signature = XXH128; compression = LZ4 }
    File = /etc/tapir
    File = /var/lib/tapir
    File = /etc/containers                                  # Quadlet-Originale, containers/registries/storage.conf
    File = /etc/systemd/system/tapir-ackervielfalt.target
    Plugin = "podman:user=root:quadlet=tapir-*:images=blobs:max_downtime=3600"
    Plugin = "postgresql:instance=main"                     # Datenbanken von tapir und Keycloak, siehe POSTGRESQL-PHASE2
  }
}
```

`File = /etc/containers` deckt die Quadlet-Originale (einschließlich
etwaiger `Environment=`-Werte, bewusst über den FD) und die
Podman-Konfiguration ab (V-13 e). Registry-Anmeldungen
(`/run/containers/0/auth.json`, `/root/.config/containers/auth.json`) werden
nicht gesichert; mit `images=blobs` braucht der Restore keinen Pull.
`max_downtime=3600` ist ein Beispielwert; er wird nach dem Probelauf
festgelegt.

**Vorbedingungen** (V-13 d): Die Abnahme-Sicherung **stoppt die produktiven
tapir-Units** für die Dauer des ganzen Jobs (Dateien, PostgreSQL-Dump und
1,1 GB Image-Blobs). Sie läuft deshalb nur in einem **Wartungsfenster mit
ausdrücklicher Freigabe des Nutzers**. Vorher ein Probelauf derselben Zeile
mit `consistency=crash` (kein Stop) zur Messung der Jobdauer; daraus folgen
das Fenster und `max_downtime` (Messdauer plus Reserve). Bis zur Freigabe
bleibt forgeiec.io nur lesend.

Erwartet:

- Plan: fünf Quadlet-Quellen, das Target, drei Images, keine Volumes und
  Netze, `external.paths` für beide EnvironmentFiles und die Bind-Quellen mit
  `covered_by_fd: true`, `external.units` `postgresql.service`, `www-data` als
  `discovered_unconfirmed` angezeigt und nicht gesichert.
- Sicherung: Inventar, Inspect, `CreateCommand` und Unit-Eigenschaften vor dem
  Stop; Writergruppe aus allen tapir-Units über `Wants`/`PartOf` und
  gemeinsame Bind-Quelle; Totmannschalter angelegt; Stop je Unit mit `stop`,
  dann `mask --runtime`, für den ganzen Job (`consistency_scope=job`), also
  auch während der FD `/var/lib/tapir`, `/etc/tapir` und `/etc/containers`
  sichert; PostgreSQL läuft weiter und wird über sein Plugin gesichert.
  `env.json` nur mit Namen. Image-Blobs je DiffID, `restore_digest` im
  Manifest. Journal endet in `RESUMED`, alle Units wieder aktiv, gemessene
  Ausfallzeit im Manifest (`downtime_seconds`).
- Grants im Zertifikat: alle Podman-Grants; die Policy auf forgeiec.io lässt
  `podman.rootful`, `podman.inventory`, `podman.image.read`,
  `podman.lifecycle` und, falls dort auch restauriert werden soll,
  `podman.restore` zu (V-11).
- **Restore auf einem frischen Testhost**:
  1. `files` für `/etc/tapir`, `/var/lib/tapir` und `/etc/containers`
     (Konfiguration; Quadlet-Originale nur als Vorlage, nicht in den
     Suchpfad).
  2. **Datenbank als Handschritt**: Der Online-Restore des PostgreSQL-Plugins
     ist offen und „in neue Datenbank“ geplant ([STAND](STAND-2026-11-19.md),
     [PostgreSQL Phase 2](POSTGRESQL-PHASE2.md)); tapir und Keycloak erwarten
     die Datenbanknamen aus `/etc/tapir/*.env`. Für die Abnahme daher
     PostgreSQL-Restore als Datei und `pg_restore` in die **Originalnamen**
     auf dem Testhost, dokumentiert als Handschritt.
  3. `online` für Podman mit
     `bind_map=/var/lib/tapir=/var/lib/tapir` (Identität ausdrücklich, V-13 c):
     gleiche Image-IDs, Quadlet-Quellen und `tapir-ackervielfalt.target`
     gestagt mit Suffix und `image_pin`-Drop-in (`restore_digest`), keine
     Aktivierung.
  4. Promotion und Start nach ausdrücklicher Freigabe, dann
     Anwendungsprüfung.

## Testaufbau

Versuche ohne root und ohne Produktivsystem, auf dem Entwicklungsrechner und
in CI [S Testaufbau, K §0]. **Lifecycle-, Quadlet-, systemd- und
rootful-Tests brauchen eine Test-VM** mit eigenem systemd; ein Container
(`quay.io/podman/stable`) bildet Quadlet, Systemmanager und
`mask --runtime` nicht ab (V-18).

```sh
T=$(mktemp -d /tmp/podtest.XXXX)       # 0700
export HOME=$T/home XDG_CONFIG_HOME=$T/config XDG_DATA_HOME=$T/xdg-data XDG_RUNTIME_DIR=$T/xdg-run
export CONTAINERS_CONF=$T/containers.conf CONTAINERS_STORAGE_CONF=$T/storage.conf   # lock_type = "file"
P="podman --root $T/root --runroot $T/run --tmpdir $T/tmp --storage-driver vfs"     # Quelle
Q="podman --root $T/root2 --runroot $T/run2 --tmpdir $T/tmp2 --storage-driver vfs"  # Ziel
```

**Isolationsregeln** (Lehre aus einem Zwischenfall [K §0] und Praxis):

- `--root` und `--runroot` allein trennen weder das libpod-`tmpdir`
  (`alive`, `pause.pid`, `exits/`) noch die Lock-Tabelle in `/dev/shm`; immer
  eigenes `--root`, `--runroot`, `--tmpdir` und `XDG_RUNTIME_DIR`, besser ein
  eigenes Testkonto; **nie `podman system reset`**.
- Die shm-Lock-Tabelle ist je UID gemeinsam: eigene `containers.conf` mit
  `lock_type = "file"`. Ohne eigenes `HOME`/`XDG_CONFIG_HOME` liest Podman
  `storage.conf` und `containers.conf` des Nutzers.
- Aufräumen in dieser Reihenfolge: Unterverzeichnisse per `$P unshare rm -rf`
  löschen (ein `unshare rm -rf $T` legt den Speicher sofort wieder an), den
  eigenen Pause-Prozess beenden (PID aus
  `$XDG_RUNTIME_DIR/libpod/tmp/pause.pid`, per `environ` geprüft), dann
  `rm -rf $T` als Konto. Private `podman system service` per PID beenden.
- Netze für Laufzeittests mit `--disable-dns`; aardvark-dns startet ohne
  Benutzerbus nicht.
- Quadlet-Tests nur im Benutzermanager eines Testkontos bzw. der Test-VM.

Fixtures:

- Images: busybox und alpine (Multi-Arch-Index, gzip-Layer), ein abgeleitetes
  Image mit gemeinsamer Basis, Whiteouts, Hardlink, Symlink, FIFO, setuid,
  Sparse-Datei, Name mit 0xff, xattr; ein lokal gebautes Image ohne
  Registry-Manifest; später zstd und zstd:chunked.
- Volume (per `$P run --network none` und `$P unshare`): Verzeichnis 0700
  von UID 1000, Hardlink, Symlink, FIFO, Sparse-Datei mit 100 MiB Loch,
  `user.*`-xattr, ACL und Default-ACL (auch mit einer ID, die kein Eigentümer
  ist), `security.capability`, setuid-Datei, Name mit Byte 0xff, leeres
  Verzeichnis, UID außerhalb des Mappings (Negativfall); Varianten `o=bind`,
  `tmpfs`, Treiber `image`.
- Container: CLI-Container mit allen Optionen aus Praxis 3 (Env, env-file,
  statische IP/MAC, Alias, Secret mit Ziel und `type=env`, `keep-id`,
  Healthcheck, CapAdd/CapDrop, `--read-only`, `no-new-privileges`, Ulimits,
  Stop-Signal/-Timeout, `--init`); API-Container ohne `CreateCommand`;
  Quadlet-Unit, deren PID 1 SIGTERM ignoriert.
- Sentinels: eindeutige Testwerte in jeder Env-Quelle der Tabelle
  „Umgebungsvariablen“, in Kommandoargumenten, Healthcheck, Labels und
  Drop-ins.
- Rundreise: Sicherung über `ofd` in den SD-Strom, Restore `online` nach `$Q`;
  Vergleich je Datei im Namespace: SHA-256, `stat -c '%u %g %a %h %s %b'`,
  `getfacl -n`, xattrs, Caps, Inode-Gleichheit der Hardlinks, `SEEK_DATA`;
  Images: `Id`, `RootFS.Layers`, `RepoTags`, Manifest-Digest gleich
  `restore_digest`, dann `$Q run --rm --network none <id> true`.
- Anderer subuid-Bereich: verschachtelter Namespace (Maps vom Elternprozess,
  z. B. `0:5000:2000`) und echte zweite Konten mit anderem `/etc/subuid` in
  der Test-VM.

## Phasen und Exitkriterien

Jedes Kriterium ist ein benannter, automatisierter Test oder eine Messung mit
Schwelle (V-18).

| Phase | Liefergegenstand | Exitkriterium |
| --- | --- | --- |
| P0 | Grundlagen: `FileEntry` v2 mit `source_ref`, Scan-Handle im Host, bereinigter Strom, `api_version {1,2}`, WIT `onesimus:filed-plugin/podman`, `podman_broker.proto`, Broker-Start im root-eigenen Namespace, Gerüst des Rootful-Backends mit Exec-Protokoll, Bindung an `org.onesimus.podman`, Kontopolicy, Grants, erweiterte Discovery, Journaldatei | (1) Restore in Dateien eines Fixture-Volumes mit ns-UID-0-setuid-Datei und Cap-Datei, je mit `ofd` und mit einem Bareos-FD: `find <Where> -perm /7000` leer, `getcap -r <Where>` leer, Eigentümer = Host-IDs. (2) Tests „Gast liefert fremde `source_ref`, falschen Pfad, doppelte `source_ref`, Bytes für `source_ref`-Eintrag“: alle abgewiesen. (3) Broker im root-eigenen Namespace unter NNP mit den drei Variablen, jeweils **mit und ohne** Pause-Prozess: `podman info` meldet `Rootless=true` und den graphroot des Kontos; `ps -a`, `inspect`, `volume ls`, `create` und `run --rm` eines busybox-Containers gelingen; `systemctl --user show` als Konto-UID gelingt. (4) Test „Konto versucht `ptrace` auf den Broker“: EPERM. (5) Test „Konto liefert Scan-Antwort mit CHAR-Gerät/ID außerhalb der Map“: nicht im Strom, unter `rejected`. (6) Exec-Protokoll der ganzen P0-Testsuite enthält nur Argumentlisten der Operationstabelle. (7) PostgreSQL-Plugin (`api_version` 1) und Podman-Testplugin (2) laden nebeneinander. (8) Discovery folgt im Test keinem Symlink im Kontoverzeichnis; conmon-Fund und bestätigte Domäne sind getrennte JSON-Felder |
| P1 | Inventar und Plan im Host, Quellen-Rangfolge, Positivlisten, Env-Behandlung, Manifest ohne Daten | (1) Fixture mit gestopptem Container und Quadlet-Unit ohne Container: beide im Plan. (2) Gleiche Namen unter zwei UIDs: zwei getrennte Objekte. (3) Sentinel-Test `env=omit`: `grep -r <sentinel>` über alle Gastantworten, den Strom, Joblog und Katalog ergibt 0 Treffer für jeden Env-Wert; bei `env=keep` Treffer nur im Strom. (4) Container-Fixture: für jede Strategie enthält der Plan alle Sicherheitsfelder des Originals, oder ein Planfehler nennt das fehlende Feld; `--secret type=env` ohne Quadlet/CreateCommand ergibt Planfehler. (5) Plan für forgeiec.io (lesend) gleicht der erwarteten Objektliste (Diff leer) |
| P2 | Full-Backup: Dateibaum, Image-Blobs per tar-split, Volumes mit Attributen, Stop mit `consistency_scope=job`, Totmannschalter, Journal und Recovery | (1) An jedem Journalübergang je `kill -9 ofd`, SIGTERM und Cancel: danach alle Units aktiv, Fingerprint unverändert, keine eigene Maske übrig (`/run/systemd/…` ohne unsere `/dev/null`-Links). (2) SD ohne Medium bzw. `ofd` per SIGSTOP angehalten: Units spätestens `max_downtime + 60 s` nach dem ersten Stop wieder aktiv. (3) Quadlet-Unit, deren PID 1 SIGTERM ignoriert: Stop über ExecStop in höchstens `StopTimeout + 5 s`, kein 90-s-Timeout, Container danach gelöscht. (4) Layer-Fixtures auf vfs, overlay, btrfs: SHA-256 = DiffID. (5) Volume-Rundreise ohne Abweichung außer dokumentierten (Verzeichnis-nlink/-größe). (6) Abnahme-Sicherung forgeiec.io im freigegebenen Wartungsfenster: Journal `RESUMED`, `downtime_seconds` ≤ `max_downtime`, alle fünf Units aktiv, Anwendung antwortet (HTTP 200) |
| P3 | Restore `files` und `online` mit Konflikttabelle, Rückbau und separater Promotion | (1) Zielhost ohne Route zu Registries: gleiche Image-IDs, Manifest-Digest = `restore_digest`, `run --network none <id> true` gelingt. (2) JSON-Schnappschuss von `ps -a`, `images`, `volume ls`, `network ls` der Zieldomäne vorher/nachher unterscheidet sich nur um Objekte mit Restore-Label; vorhandene Images unverändert (Digest, RepoDigests). (3) Fehler eingespielt in jedem Restore-Journalzustand: Rückbau entfernt genau die Objekte in `created`. (4) Pflichtvergleich: kein restaurierter Fixture-Container hat mehr Rechte als das Original. (5) `image_pin` mit `restore_digest` startet unter Quadlet ohne Pull. (6) forgeiec-Restore auf dem Testhost mit den dokumentierten Handschritten: Anmeldeseite HTTP 200, Stichprobe der Daten gleich |
| P4 | Accurate/Incrementals, `freeze`, App-Adapter, `image_digest=original`, Remote, weitere Treiber | je Profil eigene Zertifizierung und Rundreise |

## Pflichttests

- Discovery: conmon zweier rootless UIDs, rootful conmon, CRI-O-Fehlkandidat,
  manipulierter Prozessname, PID-Reuse, Exit während der Erkennung; Konto mit
  runroot `/tmp/storage-run-<uid>`, abweichender graphroot, Konto nur mit
  gestoppter Quadlet-Unit; Symlink statt `db.sql`; eingeschränkte
  `/proc`-Sicht, fehlendes NSS-Konto, UID-Wiederverwendung; Plugin-Zeile mit
  nicht freigegebenem Konto.
- Grants und Bindung: fehlendes `podman.rootful` bei `user=root`, fehlendes
  `podman.lifecycle` bei `consistency=stop` (bei `scope=job` vor jedem Stop
  erkannt), lokale Policy entzieht Grants, anderes Plugin mit Podman-Grants
  im Zertifikat wird nicht verknüpft.
- Host-Kontrolle: fremde, doppelte oder umgehängte `source_ref`; Gast
  liefert Bytes oder Attribute zu einem `source_ref`-Eintrag; Pfad außerhalb
  eines Volumes; Restore in Dateien mit ns-UID 0, setuid und Caps (Ergebnis
  ohne Sonderbits und Caps); Konto manipuliert Broker-Antwort; `ptrace` auf
  den Broker.
- Zwei UIDs mit identischen Namen; manipulierte IDs, Handles und Symlinks;
  kein Zugriff über Konto- oder Jobgrenzen.
- Env: Sentinel-Test über alle Orte der Tabelle, Kommandoargumente,
  Healthcheck, Labels, Drop-ins und stderr; Restore lässt Variablen ohne Wert
  weg; `env=keep` nur auf dem Volume.
- Pods: Auswahl eines Mitglieds sichert den ganzen Pod, auch gegen `exclude`.
- Images: Layer-Rekonstruktion bit-genau (SHA-256 = DiffID), Layer
  verschwindet während des Streams, Layer ohne tar-split, tar-split mit
  `../` oder Symlink im Layer-Verzeichnis, Restore mit gleicher Image-ID und
  `restore_digest`, vorhandenes Image wird nicht gepullt, veränderlicher Tag
  mit `image_pin`.
- Volumes: andere subuid-Bereiche, ACL-IDs außerhalb des Ziel-Mappings,
  xattr/Caps, Hardlinks, Sparse, Symlinks, nicht-UTF-8-Namen, Gerätedateien
  rootless, `o=bind` außerhalb der Allowlist, fremder Treiber; Restore in
  nichtleeres Ziel abgewiesen; Restore legt nie `o=bind` an.
- Container: Quellen-Rangfolge je Strategie, `generate spec` mit
  `--secret type=env`, Pflichtvergleich der Sicherheitsfelder.
- Konsistenz: Quadlet `Restart=always` unter Maske, `Wants=` einer anderen
  Unit, Timer, Start im Fenster zwischen `stop` und `mask`, vorab maskierte
  Unit, PID 1 ignoriert SIGTERM, Bediener startet während der Sicherung,
  SIGKILL nach Frist, `scope=job` deckt die Include-Dateien ab, Planfehler in
  der zweiten Zeile stoppt nichts, Estimate stoppt nie; FD-/Broker-Crash vor
  und nach jedem Journalübergang; Totmannschalter bei hängendem `ofd`;
  SIGTERM an `ofd`; neue Boot-ID.
- Restore: Port-, IP-, Subnetz-, Bridge-Namen- und Secret-Konflikte wie in
  der Tabelle; `--ignore`-Falle; Staging nie unter einem Suchpfad; lokale
  Units mitgestagt; Rückbau nur mit eigenem Label; kein automatischer Start;
  Spool mit vorab angelegtem Symlink abgewiesen.
- Cancel bei vollem Puffer, während `systemctl stop` und während eines
  Layer-Streams.

## Offene Punkte

1. **Lese-Sperren** über lange Layer-Streams oder Prüfung danach (Images).
2. **libpod create ohne Socket**: privater kurzlebiger Dienst (bestätigt
   machbar, Praxis 3) oder geprüfte `podman create`-Argumentliste.
3. **Grant-Namen**, **Ort/Format der Kontopolicy** und Grants je Plugin-ID im
   Zertifikat (V-11, Signier-Infrastruktur).
4. **Image-Import rootless** über `oci:/proc/self/fd/<n>` oder `podman load`
   aus stdin (Spool, P3-Nachweis).
5. **`systemctl --user` als Konto-UID mit NNP** oder `--machine=<konto>@`;
   Verhalten ohne Benutzermanager (P0-Nachweis).
6. **SD-Antwort**, die Gerät und Volume garantiert, als Startsignal für den
   Stop (P2).
7. **Healthcheck** `--health-on-failure=restart` während Stop und Maske (nicht
   geprüft); `NeedsCopyUp`/`NeedsChown` an neuen Volumes.
8. **SELinux** `relabel` ungeprüft (beide untersuchten Rechner ohne SELinux).
9. **Versionen**: erstes zertifiziertes Profil Podman 5.4.2 (netavark, sqlite,
   overlay/btrfs/vfs, `cgroup_manager=systemd`); Positivlisten und interne
   Variablen gehören zum Profil; andere Versionen erst nach eigenem Test.
10. **`image_pin`**: Auflösung des lokalen `restore_digest` durch Quadlet ohne
    Pull (P3), sonst Pin über die Image-ID.
11. **rootful CHAR/BLOCK** in Volumes: im MVP ausgelassen; ob sie beim
    Online-Restore aus `ownership.tsv` neu angelegt werden dürfen.

Erledigt durch diese Überarbeitung: Quadlet-Quellen mit Werten bei
`env=omit` (geschwärzte Vorlage), Podman-CLI im vorbereiteten Namespace
(Variablen, Mount-Namespace; Eigentümer root bleibt P0-Nachweis),
`restore_mode` (nur `online`), Restore in Dateien (bereinigter Strom,
`ownership.tsv`), Discovery ohne Auswertung von Kontodateien als root,
Totmannschalter (im MVP).

## Entscheidungen des Nutzers vom 19.11.2026 (zuvor offen)

Der Nutzer hat alle sechs Empfehlungen übernommen („podman go“, 19.11.2026).
Die folgenden Punkte sind damit entschieden, wie jeweils empfohlen:


1. **Vorgabe für `max_downtime`** (V-4). Empfehlung: 7200 s als Vorgabe,
   Pflicht bei `consistency_scope=job`; für forgeiec.io nach dem Probelauf mit
   `consistency=crash` die Messdauer plus Reserve.
2. **Stop-Reihenfolge bei Quadlet** (Praxis 4). Empfehlung: `stop`, dann
   `mask --runtime` mit Nachprüfung, weil das ungeschützte Fenster nur
   Millisekunden statt der ganzen Stop-Dauer umfasst.
3. **Literale Kennwörter in Kommandoargumenten, Healthcheck und Labels** bei
   `env=omit` (V-6). Empfehlung: Wertabgleich mit den Env-Werten, andere
   Literale behalten, weil der Restore sie braucht; eine Option
   `args=redact` erst bei Bedarf.
4. **Bindung der Podman-Grants** (V-11). Empfehlung: jetzt feste Bindung an
   `org.onesimus.podman` im Code von `ofd`, Grants je Plugin-ID später in der
   Signier-Infrastruktur.
5. **Vertragsversion** (V-16). Empfehlung: `ofd` akzeptiert `{1, 2}`; kein
   harter Suite-Schnitt, das PostgreSQL-Plugin bleibt bei 1.
6. **Abnahme forgeiec.io** (V-13). Empfehlung: Freigabe eines Wartungsfensters
   erst nach dem Probelauf; Datenbank auf dem Testhost per Datei-Restore und
   `pg_restore` in die Originalnamen als dokumentierter Handschritt, bis der
   Online-Restore des PostgreSQL-Plugins steht.

## Umsetzungsstand

`onesimus-plugins::executor` implementiert isolierte Worker, begrenzte Queues,
Instanz-/Job-/Device-Prüfung, direkte Interrupts und einen Deadline-Watchdog.
Der WASM-Host lädt signierte Gäste (Phase 1); der Broker-Start aus geprüftem
Deskriptor existiert für PostgreSQL (`broker.rs`, ohne `PR_SET_DUMPABLE`) und
ist die Vorlage für `onesimus-podman-broker`.

`onesimus-plugins::discovery` erfasst conmon-Kandidaten:

```bash
ofd --discover-podman
ofd --discover-podman --conmon-executable /usr/bin/conmon   # mehrfach verwendbar
```

Der Modus braucht keine FD-Konfiguration, startet keinen Listener und gibt
JSON aus. Er prüft root-eigene, nicht gruppen-/weltbeschreibbare Executables
über Device/Inode und vergleicht PID/Startzeit/UID/Executable doppelt.
Kandidaten bleiben `discovered_unconfirmed`; `host_scope_verified` bleibt
false.

### Stand P0 (19.11.2026)

Umgesetzt (Proto-Zweig `podman`, Crates `onesimus-podman-core` und
`onesimus-podman-broker`, `ofd`-Module `podman_port`, `podman_scan`,
`podman_rootful`, `podman_e2e`, Erweiterungen in `broker.rs`, `backup.rs`,
`plugin.rs`, `onesimus-plugins`):

| Exitkriterium | Nachweis (Test) | Stand |
| --- | --- | --- |
| (1) Restore in Dateien sauber | `podman_e2e::volume_restore_to_files_is_clean` (echtes Podman, isoliert): `find -perm /7000` und `getcap -r` leer, Strom mit Host-IDs, `security.capability` nie im Strom; dazu `sanitize::tests::*`, `podman_scan::tests::*` | mit `ofd` nachgewiesen; **mit Bareos-FD offen** (kein bareos-fd auf dem Entwicklungsrechner) |
| (2) fremde/falsche/doppelte `source_ref`, Bytes des Gastes | `podman_port::tests::foreign_wrong_and_duplicate_references_are_refused`, `…::guest_listed_references_carry_host_bytes_and_attributes`, `podman_scan::tests::references_are_checked_and_used_once`, `wasm::tests::scan_references_need_contract_two` | nachgewiesen |
| (3) Broker im root-eigenen Namespace | `podman_e2e::broker_in_root_owned_namespace` (verschachtelter Namespace: Wurzel des äußeren Namespace besitzt den Broker-Namespace; NNP, drei Variablen, ohne Pause-Prozess: info rootless + graphroot, load, ps, volume ls, create, inspect, run --rm; echter Broker: engine-identify, inventory); `podman_e2e::systemctl_user_show_as_account` | im verschachtelten Namespace nachgewiesen; **mit echtem Host-root und mit vorhandenem Pause-Prozess offen** (Test-VM: `podman_e2e::broker_with_host_root_on_test_vm`, `#[ignore]` mit Anleitung) |
| (4) `ptrace` des Kontos auf den Broker | Teil von `broker_in_root_owned_namespace`: EPERM | im verschachtelten Namespace nachgewiesen; Host-root wie (3) |
| (5) CHAR-Gerät/ID außerhalb der Map | `sanitize::tests::devices_sockets_and_foreign_ids_are_rejected`, `podman_scan::tests::foreign_ids_are_rejected_not_streamed` | nachgewiesen (Gerätedatei nur als Unit-Test, ohne root nicht anlegbar) |
| (6) Exec-Protokoll nur aus der Tabelle | `podman_rootful::tests::requests_map_to_table_rows_only`, `exec::tests::*` | nachgewiesen für das Rootful-Backend mit Test-Runner; der echte Lauf als root braucht die Test-VM |
| (7) Vertrag 1 und 2 nebeneinander | `podman_port::tests::contract_one_and_two_plugins_load_side_by_side`, `tests::contract_versions_one_and_two_are_accepted` | nachgewiesen mit skriptierten Gästen (nicht mit dem gebauten PostgreSQL-Gast) |
| (8) Discovery folgt keinem Symlink, getrennte Felder | `discovery::tests::indicators_never_follow_links_and_stay_separate` | nachgewiesen |

Bindung und Grants: `tests::podman_grants_are_bound_to_the_podman_plugin_id`,
`podman_port::tests::the_line_is_bound_to_the_podman_plugin_and_its_grants`,
`wasm::tests::podman_imports_follow_the_grants`. Journal mit
Recovery-Entscheidung und Datei (`fsync`, `rename`, `O_NOFOLLOW`, `flock`):
`journal::tests::*`; in einen Lifecycle eingebaut wird es erst in P2.

Entscheidungen der Umsetzung, jeweils die sicherere Variante
(**Vorschlag, offen**):

- Kontopolicy: eigene Datei `/etc/onesimus/podman-accounts.json`, Format
  `onesimus-podman-accounts/1`, `deny_unknown_fields`, Prüfung wie die
  Plugin-Policy (root-eigen, nicht beschreibbar bis `/`).
- `consistency=stop` ist bis P2 ein Planfehler (kein stiller Wechsel zu
  `crash`); P0 nimmt nur `consistency=crash` an.
- Volumes mit Optionen (`type=`, `o=bind`, `device=`) sind in P0 ein
  Planfehler; `volume_bind_roots` wird erst mit P1/P2 ausgewertet. Der
  `Mountpoint` aus `volume inspect` muss `<graphroot>/volumes/<name>/_data`
  sein.
- Podman-Grants verlangen `api_version` 2 auch in `verify`; `sign-component
  prepare` setzt 2, sobald ein Podman-Grant angefordert wird.
- Der Broker bekommt `--root/--runroot/--tmpdir` nur vom Host aus der
  Kontopolicy (nie vom Gast); der graphroot, den Podman meldet, muss Pfad
  und Inode der Kontopolicy haben.
- ACL im Strom als ACL-Stream mit numerischen Host-IDs (nie Namen); xattrs
  nur `user.*` als `STREAM_XATTR_LINUX`; FIFO als `FT_SPEC`; `ownership.tsv`
  gehört der euid/egid von `ofd`.

Offen nach P0: Sparse (`SEEK_DATA`), Zeitlimit je Discovery-Zugriff,
`inspect`/`member-state` und Positivlisten für Inspect/Spec (P1),
Image-Blobs (P2), Lifecycle mit Journal, Totmannschalter und
`--podman-recover` (P2), Gast `plugins/podman` und Paket
`ofd-plugin-podman`.
