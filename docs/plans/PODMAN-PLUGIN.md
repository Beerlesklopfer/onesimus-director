# Planung: Podman-Plugin für ofd

Stand: 19.11.2026. Status: **Grundplan freigegeben (18.10.2026), am 19.11.2026
nach drei Teamberichten und vier neuen Entscheidungen des Nutzers neu gefasst.**
Produktiver Code existiert noch nicht; vorhanden sind Worker-Kern und
read-only conmon-Discovery (Abschnitt „Umsetzungsstand“).

Kennzeichnung in diesem Plan:

- **(Nutzer, TT.MM.JJJJ)**: verbindliche Entscheidung des Nutzers.
- **Vorschlag, offen**: Ergebnis der Planung, noch nicht freigegeben.
- Belege verweisen auf die Teamberichte vom 19.11.2026 (Arbeitspapiere der
  Planungssitzung, nicht im Repository): **[S]** Storage, Images, Volumes;
  **[A]** API, Erkennung, Abhängigkeiten; **[K]** Konsistenz, Restore,
  Broker-Vertrag; jeweils mit Abschnitt, z. B. [S §1]. Alle Versuche liefen
  mit Podman 5.4.2 (netavark, sqlite) rootless in Wegwerf-Speicher;
  forgeiec.io wurde nur lesend untersucht.

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
führt keinen Plugin-Code aus. Grants stehen im Signierzertifikat, die lokale
Policy schränkt nur ein ([Signier-Infrastruktur](SIGNIER-INFRASTRUKTUR.md)).
Das Broker-Muster folgt [PostgreSQL Phase 2](POSTGRESQL-PHASE2.md): nativer
Broker aus geprüftem Deskriptor, `fexecve`, leere Umgebung,
`PR_SET_NO_NEW_PRIVS`, `PR_SET_PDEATHSIG`, Protobuf über `socketpair`.
Gemeinsame Bausteine (Journal, Writergruppe, Stagingvolume, Restore-Verfahren)
teilt das Plugin mit dem [Docker-Plugin](DOCKER-PLUGIN.md); UID-Domänen,
Pods, Quadlet und Namespaces bleiben Podman-spezifisch.

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
| Container | nur Inspect-Daten, kein `podman export` des Container-Dateisystems; Writable Layer bleibt ausgeschlossen (Nutzer, 19.11.2026) |
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

Der Gast (WASM, signiert) ist in beiden Profilen derselbe: Er baut aus den
Antworten des typisierten Imports `onesimus:broker/podman` den Dateibaum und
das Manifest. Er sieht nur, was der Host für diesen Job freigibt. Hinter dem
Import steht je nach Profil ein anderes Backend; der Gast kann das nicht
unterscheiden und braucht es nicht.

```mermaid
flowchart LR
    G[WASM-Gast podman<br/>signiert, ohne WASI] -- typisierter Import<br/>onesimus:broker/podman --> P[ofd: Podman-Port<br/>Grants, erlaubte Objektmenge,<br/>Journal, Leases, Attributkontrolle]
    P -- user=konto: Protobuf über socketpair --> B[onesimus-podman-broker<br/>als Konto-UID, im User-Namespace<br/>den ofd angelegt hat]
    B -- feste Argumente, --format json --> C1[podman / quadlet -dryrun -user /<br/>systemctl --user]
    B -- openat2 im Namespace --> D1[(Storage des Kontos:<br/>Volumes, Layer, tar-split)]
    P -- user=root: feste Logik in ofd --> R[Rootful-Backend in ofd<br/>kein Plugin-Code]
    R -- feste Argumente, leere Umgebung --> C2[podman / quadlet -dryrun /<br/>systemctl]
    R -- openat2 als root --> D2[(/var/lib/containers/storage,<br/>/etc/containers)]
    P --> SD[(SD-Strom, Bareos-Format)]
```

| Aufgabe | Gast (WASM) | Host `ofd` (root) | Broker (rootless, Konto-UID) |
| --- | --- | --- | --- |
| Plugin-Zeile auswerten | Auswahl, Baum, Manifest | Zeile prüfen, Konto gegen Kontopolicy, Grants gegen Zeile | – |
| Inventar, Inspect | fordert an | prüft jede Anfrage; berechnet die erlaubte Objektmenge selbst (Auswahl plus Zusammenhänge, derselbe Algorithmus wie im Gast, im Host maßgeblich); filtert Env-Werte bei `env=omit` | ruft `podman … --format json` mit fester Argumentliste |
| Quadlet, systemd | ordnet zu | prüft Pfade gegen Quadlet-Suchpfade | `quadlet -dryrun -user`, `systemctl --user show` |
| Volume-Daten, Layer | liefert Einträge mit Verweis | übernimmt Eigentümer, Modus, xattrs **aus eigener Tabelle**, nicht vom Gast | liest im User-Namespace (`openat2`), rekonstruiert Layer aus tar-split |
| Stop/Wiederanlauf | – | Journal, Lifecycle-Lease, Recovery | `systemctl --user mask --runtime`/`stop`/`start`, `podman stop/start` |
| Restore online | Plan, Reihenfolge | prüft jede Anlage (Suffix, keine neuen Rechte), Restore-Journal | legt Netze, Volumes, Container an, schreibt Volume-Dateien im Ziel-Namespace |

**Rootless** (`user=<konto>`): `ofd` prüft Konto, `/etc/subuid` und
`/etc/subgid`, legt den User-Namespace an und schreibt `uid_map`/`gid_map`
selbst als root, bevor der Broker als Konto-UID startet (Abschnitt „Broker“).
Der Broker ist im Namespace UID 0 und hat Rechte nur über die IDs des Kontos,
genau wie `podman unshare` [S §2, K §3.1]. Lifecycle-Aufrufe gehen über den
Benutzermanager des Kontos.

**Rootful** (`user=root`, Nutzer, 19.11.2026): kein Brokerprozess. Das
Rootful-Backend ist Teil des signierten `ofd`-Releases (nicht des
Plugin-Pakets) und implementiert dieselben Operationen mit fester Logik:
`podman` und `/usr/libexec/podman/quadlet -dryrun` als root mit fester
Argumentliste und leerer Umgebung, `systemctl` gegen den Systemmanager,
Layer-Rekonstruktion aus `/var/lib/containers/storage` und Volume-Daten per
`openat2` mit echten Host-IDs. Begründung: rootful Storage (0700 root) und
`/run/podman/podman.sock` sind root-äquivalent; ein Nicht-root-Broker mit
Capabilities wäre nur formal nicht root [S §2, A §1, K §3.4]. Kosten:
Podman-Client-Code im Kern von `ofd`. Begrenzung: keine Shell, kein
generisches Exec, nur die Operationen der Tabelle im Abschnitt „Broker“,
nie `--showsecret`, nie `system reset|migrate|renumber`, Programmpfade
root-eigen und nicht beschreibbar geprüft, JSON-Größen begrenzt.

Vorschlag, offen: Gemeinsamer Code von Broker und Rootful-Backend
(tar-split-Leser, Inventar-Parser, Zusammenhangs-Algorithmus, Positivliste
für `CreateCommand`) in einem Crate `onesimus-podman-core`, das `ofd` und
`onesimus-podman-broker` gleichermaßen einbinden.

### Grants

Grants im Signierzertifikat des Plugins (Vorschlag, offen, Namen nach dem
Muster `postgresql.read`):

| Grant | erlaubt | rootless | rootful |
| --- | --- | --- | --- |
| `podman.inventory` | Engine-Identität, Inventar, Inspect, Quadlet-Quellen, Unit-Eigenschaften | Pflicht | Pflicht |
| `podman.image.read` | Image-Metadaten und Blobs (`images=blobs`) | bei `images=blobs` | bei `images=blobs` |
| `podman.volume.read` | Volume-Daten mit erweiterten Attributen | bei `volumes=yes` | bei `volumes=yes` |
| `podman.lifecycle` | Stop und Wiederanlauf der Writergruppe | bei `consistency=stop` | bei `consistency=stop` |
| `podman.restore` | Online-Restore: Netze, Volumes, Images, Pods, Container anlegen, Quadlet stagen | Restore | Restore |
| `podman.rootful` | Zugriff auf das Rootful-Backend in `ofd` | – | **Pflicht, zusätzlich** |

Das rootful Profil braucht also im Zertifikat `podman.rootful` plus die
funktionalen Grants des gewünschten Umfangs; für die forgeiec-Sicherung
(Inspect, Quadlet, Image-Blobs, Stop) sind das `podman.rootful`,
`podman.inventory`, `podman.image.read` und `podman.lifecycle`, für den
Restore zusätzlich `podman.restore`. Die lokale Policy kann jeden Grant
entziehen, z. B. `podman.rootful` auf Hosts ohne rootful Podman. Der Host
verknüpft den Import nur, wenn mindestens `podman.inventory` gewährt ist, und
prüft jede Funktion gegen ihren Grant. Fehlt ein Grant, den die Zeile
braucht (etwa `podman.lifecycle` bei `consistency=stop`), ist das ein
Planfehler, kein stiller Wechsel zu `crash`.

### Kontopolicy

Welche Konten gesichert werden dürfen, legt der Administrator in einer
root-eigenen Kontopolicy fest (Vorschlag, offen: eigene Datei neben der
`Plugin Policy`, weil deren Format unbekannte Felder ablehnt). Inhalt je
Konto: Name und UID, erlaubte Storage-Domänen, `rootful: true` nur für
`root`, `volume_bind_roots` (erlaubte `device`-Präfixe für `o=bind`-Volumes),
erlaubte Registries für den Restore per Pull. Ein Konto, das nur die
Discovery kennt, ist nicht freigegeben.

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
| Storage | `stat` auf `<home>/.local/share/containers/storage/db.sql` bzw. `bolt_state.db`, rootful `/var/lib/containers/storage` | ja |
| Quadlet-Orte | `/etc/containers/systemd/users/<UID>/`, `<home>/.config/containers/systemd/` | ja (Units) |
| Linger | `/var/lib/systemd/linger/<name>` | Hinweis |
| Sub-IDs | `/etc/subuid`, `/etc/subgid` | Hinweis |
| Laufzeitreste | `/run/user/<uid>/libpod`, `/tmp/storage-run-<uid>` | seit dem Boot benutzt |

Zustände: `discovered_unconfirmed` (Discovery) → `confirmed_podman` erst durch
den Broker des freigegebenen Kontos bzw. das Rootful-Backend
(`engine-identify`: `podman info` mit graphroot, runroot, tmpdir, Treiber,
DB- und Netz-Backend, ID-Mappings, Version). Die Registry hält
`(Host, UID, graphroot, graphroot-Inode, runroot, tmpdir)`; das `tmpdir` gehört
zur Identität, weil sich mehrere Speicher eines Kontos `tmpdir` und
Lock-Tabelle teilen [K §0]. Eine UID mit mehreren Speichern ergibt mehrere
Domänen. Bestätigte Konten werden auch ohne aktuellen conmon-Fund weiter
inventarisiert.

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
| `pods`, `containers`, `quadlet` | Namen oder Globs | – | genau eine Auswahl ist Pflicht; `*` wählt alles dieses Kontos. Ohne Auswahl Planfehler, damit eine leere Zeile nicht ungewollt alles sichert. |
| `exclude` | Namen oder Globs | – | von der Auswahl abgezogen; schließt keine automatisch erkannten Abhängigkeiten aus. |
| `volumes` | `yes`, `no` | `yes` | Named Volumes der Auswahl. Bind Mounts nie, die sichert der FD (eigene `File =`-Zeile). |
| `images` | `reference`, `blobs` | `reference` | `reference`: Image-Inspect im Baum, Restore per Pull. `blobs`: zusätzlich Config, Original-Manifest, Index und Layer als einzelne Dateien, je DiffID einmal. |
| `networks` | `yes`, `no` | `yes` | die verwendeten Netze als Inspect-Daten plus Zuordnung je Container. |
| `quadlet_sources` | `yes`, `no` | `yes` | Quadlet-Quellen, Quadlet-Drop-ins, systemd-Drop-ins und verwandte lokale Units (z. B. `*.target`). |
| `env` | `omit`, `keep` | `omit` (Nutzer, 19.11.2026) | `omit`: nur Namen und Verweise auf EnvironmentFiles. `keep`: Werte nur auf dem Volume. |
| `consistency` | `stop`, `crash` | `stop` | `stop`: autorisierter Stop der Writergruppe mit Journal. `crash`: Kopie im laufenden Betrieb, im Manifest als abgeschwächt vermerkt. |
| `consistency_scope` | `job`, `plugin` | `job` (Nutzer, 19.11.2026) | `job`: Stop vor der ersten Datei des Jobs, Wiederanlauf nach der letzten; umfasst die FD-Dateisicherung der Bind Mounts. `plugin`: nur für die Dateien dieser Zeile. |
| `stop_timeout` | Sekunden | `StopTimeout` bzw. `TimeoutStopSec` | Obergrenze je Unit oder Container. |
| `max_downtime` | Sekunden | keine | überschritten: Abbruch und Wiederanlauf (eigenes Zeitbudget). |
| `clean_exit_codes` | Liste | `0,143` | Exit-Codes, die als sauberer Stop gelten. |
| `on_unclean` | `fail`, `warn` | `fail` | `fail`: Job-Fehler, Daten trotzdem gesichert und im Manifest als `crash` markiert. |
| `external` | `warn`, `error` | `warn` | Bind-Quellen und EnvironmentFiles, die keine `File =`-Zeile des FileSets abdeckt. |
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
| `restore_mode` | `files`, `online` | `files` | `files`: Baum unter `Where`, ohne Plugin-Code. `online`: gestagter Restore in neue, gestoppte Ressourcen (im Plan „stage“). In `ofd` heißt der Online-Restore bereits `restore_mode=online` (`restore.rs`); `stage` als Synonym ist offen. |
| `target_user` | Kontoname, UID, `root` | Original | Zielkonto; muss freigegeben sein; `root` braucht `podman.rootful`. |
| `suffix` | Text | `-restored` | Namenszusatz aller neuen Objekte. |
| `networks` | `create`, `existing` | `create` | fehlende Netze anlegen bzw. nur vorhandene gleichen Namens mit passendem Subnetz nutzen. |
| `network_subnet` | `keep`, `auto` | `keep` | `keep` mit Überlappung: Planfehler. `auto`: Podman wählt, statische IPs entfallen (Warnung). |
| `restore_ip` | `auto`, `keep`, `drop` | `auto` | `auto`: statische IP/MAC nur, wenn als statisch belegt. |
| `ports` | `keep`, `drop` | `keep` | Portfreigaben übernehmen; Konflikte stehen im Plan. |
| `bind_map` | `<alt>=<neu>`, mehrfach | – | Bind Mounts nur über diese Zuordnung, sonst Planfehler. |
| `image_pin` | `yes`, `no` | `yes` bei veränderlichem Tag | Quadlet-Staging bekommt ein Drop-in `Image=<name>@<digest>` mit dem im Ziel tatsächlich vorhandenen Digest. |
| `image_digest` | `new`, `original` | `new` | `original` nur mit zertifizierter Version (Abschnitt Images). |
| `selinux` | `relabel`, `keep`, `drop` | `relabel` | `relabel`: Label nicht schreiben, Podman labelt über `:z`/`:Z`. |
| `files_owner` | `ns`, `host:<konto>` | `ns` | nur `restore_mode=files`, siehe Abschnitt Attribute. |
| `files_special` | `drop`, `keep` | `drop` | nur `restore_mode=files`: setuid/setgid und `security.capability` aus Volume-Daten. |

## Dateibaum und Manifest

Das Plugin legt im vorhandenen SD-Strom (`NextFile`/`ReadFile`, Bareos-Format)
einen virtuellen Dateibaum an (Nutzer, 19.11.2026). Jede Einheit ist eine
eigene Datei mit eigenen Attributen; so bleiben Restore einzelner Teile,
Prüfsummen je Datei und später Incrementals möglich. Eine Image-Layer ist nach
OCI-Definition selbst ein tar; sie ist die Original-Einheit des Images, kein
zusätzlicher Container um andere Dateien [S §1, A §4].

```text
/@podman/<domäne>/engine/info.json                     gefiltertes podman info/version
/@podman/<domäne>/containers/<name>/inspect.json       Inspect, Env-Werte nach env=
/@podman/<domäne>/containers/<name>/create-command.json CreateCommand, falls vorhanden; Podman-Optionen getrennt, -e-Werte nach env=
/@podman/<domäne>/containers/<name>/spec.json          podman generate spec, Env-Werte nach env=
/@podman/<domäne>/containers/<name>/env.json           Namen, Herkunft, EnvironmentFiles; Werte nur bei env=keep
/@podman/<domäne>/containers/<name>/systemd.json       Unit-Eigenschaften bei PODMAN_SYSTEMD_UNIT
/@podman/<domäne>/pods/<name>/inspect.json, spec.json
/@podman/<domäne>/networks/<netz>/inspect.json         network inspect ohne "containers"
/@podman/<domäne>/networks/<netz>/used-by.json         Zuordnung je Container (abgeleitet)
/@podman/<domäne>/volumes/<vol>/inspect.json
/@podman/<domäne>/volumes/<vol>/data/<pfad>            Dateien des Volumes; data/ trägt den Stat der Volume-Wurzel
/@podman/<domäne>/volumes/<vol>/ownership.tsv          ns-UID/GID, Sonderbits und Caps je Pfad (für restore_mode=files)
/@podman/<domäne>/images/<image-id>/inspect.json       Namen stehen im Manifest
/@podman/<domäne>/images/layers.json                   je DiffID: Layer-ID, compressed-diff-digest, Größen, uid/gidset
/@podman/<domäne>/images/oci/oci-layout                nur images=blobs
/@podman/<domäne>/images/oci/index.json                erzeugtes Manifest je Image, ref.name = name:tag
/@podman/<domäne>/images/oci/blobs/sha256/<digest>     Config (= Image-ID), Original-Manifest, Original-Index, erzeugtes Manifest, Layer (= DiffID)
/@podman/<domäne>/secrets/<name>/inspect.json          nur Metadaten
/@podman/<domäne>/quadlet/<suchpfad-kürzel>/<rel.pfad> z. B. etc/web.container, etc/web.container.d/10.conf
/@podman/<domäne>/systemd/<unit>                       verwandte Nicht-Quadlet-Units, z. B. *.target
/@podman/<domäne>/manifest.json                        zuletzt
```

- `<domäne>`: `root` bzw. `uid-<n>`, mit Kürzel, wenn eine UID mehrere Speicher
  hat; der Kontoname steht im Manifest, weil Namen sich ändern [A §4].
- Images nach **ID** benannt: Namen enthalten `/` und `:`, mehrere Tags
  zeigen auf dieselbe ID [A §4].
- `images/oci/` ist ein gültiges OCI-Layout; nach `restore_mode=files` genügt
  `podman pull oci:<Where>/@podman/<domäne>/images/oci:<name>` ohne
  Plugin-Code [S §1, K §2.3].
- Alle Dateien außer den Volume-Daten haben synthetische Attribute (Regel
  S-13, Abschnitt Attribute); Blobs: Modus 0444, `mtime = ctime =` `created`
  der Layer, vorbereitet für Accurate [S §4].

### manifest.json

Das Manifest (Format `onesimus-podman/1`) beschreibt Domäne, Auswahl,
Objekte, Kanten, externe Abhängigkeiten, Writergruppen und die
Restore-Reihenfolge [A §4]. Ausschnitt für forgeiec.io:

```json
{
  "format": "onesimus-podman/1",
  "domain": {"host": "forgeiec.io", "host_uid": 0, "user": "root", "domain_id": "sha256(host|uid|graphroot|inode|runroot|tmpdir)",
             "engine": {"version": "5.4.2", "network_backend": "netavark", "db_backend": "sqlite", "graph_driver": "overlay",
                        "graphroot": "/var/lib/containers/storage", "rootless": false, "uid_map": null, "gid_map": null}},
  "selection": {"quadlet": ["tapir-*"], "env": "omit", "images": "blobs", "consistency": "stop", "consistency_scope": "job"},
  "objects": [
    {"kind": "quadlet", "id": "/etc/containers/systemd/tapir-ackervielfalt-web.container", "generated_unit": "tapir-ackervielfalt-web.service",
     "fingerprint": "sha256:…", "reason": "selected"},
    {"kind": "container", "id": "<64hex>", "name": "tapir-ackervielfalt-web", "state": "running",
     "restore_strategy": "quadlet", "lossy_fields": ["RestartPolicy"], "env_names": ["SECRET_KEY", "…"],
     "reason": "created_by:quadlet:tapir-ackervielfalt-web.service"},
    {"kind": "image", "id": "8efa6985d967…", "names": ["ghcr.io/…/wirgarten-tapir:latest"], "image_digest": "sha256:b8bb8759…",
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
  "restore_order": ["network", "volume", "image", "secret_ref", "pod", "container", "quadlet"],
  "exclusions": ["writable_layer"],
  "consistency_result": "clean",
  "warnings": ["mutable_tag:ghcr.io/…:latest"],
  "discovery": {"source": "registry", "conmon_seen": true, "incomplete": []}
}
```

## Was gesichert wird

### Container: Inspect und Quellen-Rangfolge

Je Container nur Inspect-Daten, kein Container-Dateisystem (Nutzer,
19.11.2026). Inspect zeigt aber den *resultierenden* Zustand: Vorgaben aus
Image und Laufzeit sind eingemischt, manches fehlt ganz (statisch oder
dynamisch vergebene IP/MAC, Ziel von Env- und Mount-Secrets, pasta-Optionen,
`--userns keep-id` aufgelöst, Image- und Container-Labels vermischt,
Quadlet `Restart=always` erscheint als `RestartPolicy: no`) [A §2].
`podman container clone` und `kube generate` verlieren noch mehr und
scheiden aus [A §2]. Deshalb gilt für den Restore eine feste
Quellen-Rangfolge; das Manifest nennt je Container die gewählte
`restore_strategy`:

1. **`quadlet`**: Quadlet-Datei samt Drop-ins, wenn der Container das Label
   `PODMAN_SYSTEMD_UNIT` trägt und die Quelle gefunden wurde. Sie ist die
   Quelle der Wahrheit, weil Quadlet den Container bei jedem Start neu
   erzeugt (`--rm --replace`).
2. **`create-command`**: `Config.CreateCommand` bei CLI-erzeugten
   Containern, geparst gegen eine **Positivliste** der `podman create`-Optionen.
   Podman-Optionen (`--root`, `--runroot`, `--tmpdir`, `--replace`, `--rm`,
   `--cidfile`) werden verworfen, Namen, Netze, Volumes und Pod beim Restore
   umgeschrieben. Eine unbekannte Option ist ein Planfehler, nie ein rohes
   Replay. API- oder Compose-erzeugte Container haben kein `CreateCommand`
   [A §1].
3. **`inspect`**: `spec.json` aus `podman generate spec` mit der Liste der
   verlorenen Felder (`lossy_fields`) im Manifest und im Restore-Plan.

Laufzeitfelder werden beim Neuaufbau nie übernommen: `Id`, `Created`,
`State`, `Pid`, Pfade unter `StaticDir`, `GraphDriver`, `NetworkSettings`
(Istwerte), `EffectiveCaps`, `Mounts[].Source` bei Volumes, automatisch
vergebene Aliase [A §2]. Die libpod-Datenbank (`db.sql`) wäre die einzige
verlustfreie Quelle, ist aber intern und unversioniert; das Plugin baut nicht
darauf [A §2].

**Inspect vor dem Stop**: Quadlet-Container sind nach `systemctl stop`
gelöscht [K §1.1, A §5]. Inspect, `generate spec` und Unit-Eigenschaften
werden deshalb im Zustand `PREPARED` erfasst, vor jeder Mutation. Bei
`consistency_scope=job` hält der Host sie bis zur Plugin-Sitzung in einem
begrenzten Puffer.

### Umgebungsvariablen

Inspect-Daten enthalten Klartext-Kennwörter: Werte aus `--env-file` bzw.
`EnvironmentFile=` stehen in `Config.Env` (forgeiec: `KC_DB_PASSWORD`,
`SECRET_KEY` u. a.; Werte nicht ausgegeben) [A §2, K §3.1].

- **`env=omit`** (Vorgabe, Nutzer, 19.11.2026): Der **Host** entfernt die
  Werte, bevor der Gast die Daten sieht: in `Config.Env` von Inspect und
  Spec, in `-e K=V` des `CreateCommand` (wird `-e K`) und in `ExecStart` des
  Quadlet-Dryruns. `env.json` enthält je Variable Name und Herkunft (`image`,
  wenn der Wert dem Image-Env gleicht, was der Host vergleicht, ohne ihn
  weiterzugeben; `podman`; `user`) sowie die Verweise auf EnvironmentFiles
  (Pfad, Eigentümer, Modus, vorhanden, `covered_by_fd`). Der Inhalt der
  EnvironmentFiles wird nie vom Broker gelesen; der FD sichert die Dateien
  normal über eine `File =`-Zeile. Beim Restore listet der Plan Variablen
  ohne Wert, die weder aus dem Image noch aus einer EnvironmentFile kommen.
- **`env=keep`** (Option): Werte stehen in `env.json`, Inspect und Spec, nur
  auf dem Volume; nie in Joblog, Katalog oder `FileEntry.attributes`.
  Meldungen nennen nur Namen. Das Manifest vermerkt `env_values: keep`.

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
  (`diff-size`), SHA-256 beim Lesen mitgerechnet; Abweichung von der DiffID ist
  ein Fehler und meldet die Layer als beschädigt [S §1]. Benennung nach
  **DiffID**, je Digest einmal pro Job über alle Objekte der Zeile; die
  Layer-ID gleicht der DiffID nur bei Basis-Layern, die Zuordnung steht in
  `layers.json` [S §1]. Kein `podman save`: `oci-archive` komprimiert neu,
  `oci-dir --uncompressed` braucht Spool in Imagegröße [S §1, K §2.1].
- Storage-Treiber im MVP: overlay, btrfs, vfs. zfs, `additionalimagestores`
  und Layer ohne `tar-split.gz` (etwa nach partieller zstd:chunked-Übernahme,
  Vermutung) sind Planfehler mit Nennung der Layer [S §1].
- Sperren: Lese-Sperren (`fcntl`) auf `storage.lock`, `layers.lock`,
  `images.lock` wie Podman, während des Metadatenlesens und je Layer-Stream
  [S §1]. Ob ein langer Stream `rmi`/`pull` zu lange blockiert, ist offen;
  Alternative: nach dem Stream prüfen, ob die Layer noch existiert.

**Restore** [S §1, A §4, K §2.1]: gleiche Image-ID und gleiche DiffIDs, aber
ein **neuer Manifest-Digest**, weil die gzip-Bytes nicht reproduzierbar sind
und Podman beim Import ein neues Manifest schreibt. Geprüft wird deshalb über
die Kette Original-Manifest-Digest → Config-Digest (= Image-ID) → DiffIDs →
Layer-Dateien; nur die gzip-Digests im Manifest lassen sich offline nicht
nachrechnen, das steht so im Manifest.

- `image_digest=new` (Vorgabe): Der Restore-Plan meldet jeden Container und
  jede Quadlet-Unit, die das Image per `@sha256:` des Originals referenziert.
- `image_digest=original` (Vorschlag, offen, frühestens P4): Original-Index und
  -Manifest zusätzlich unter ihren Digests einspielen; geht nur über den
  internen Blob-Info-Cache von c/image (eine `DigestUncompressedPairs`-Zeile je
  Layer in einem eigenen Cache des Pull-Prozesses, nie im Cache des Kontos),
  daher nur mit zertifizierter Podman-Version [S §1].
- `image_pin`: Bei veränderlichen Tags (forgeiec: `:latest`) zieht eine
  unveränderte Quadlet-Datei ein **anderes** Image. Das Staging legt ein
  Drop-in `Image=<name>@<digest>` an, mit dem Digest, der im Ziel nach dem
  Restore tatsächlich vorhanden ist [A §4]. Für veränderliche Tags wird
  `images=blobs` empfohlen, weil die Registry den alten Digest löschen kann.

### Netze

- Orte: rootless `<graphroot>/networks/<netz>.json`, rootful
  `/etc/containers/networks/<netz>.json`. Das Standardnetz `podman` ist
  **keine Datei**, sondern wird aus `containers.conf` erzeugt (`created` =
  Abfragezeit); es wird nur referenziert, mit Subnetz [A §3].
- Gesichert: `network inspect` ohne den Block `containers` (gleich dem
  Dateiinhalt: Name, ID, Treiber, Interface, Subnetze mit Gateway und
  Lease-Bereich, IPv6, internal, DNS, Labels, Optionen, IPAM-Optionen).
- Zuordnung **vom Container her**: `network inspect .containers` listet nur
  laufende, `ps --filter network=` verfehlt `--network container:` [A §3].
  Die gewünschte Zuordnung (Netz, statische IP/MAC, eigene Aliase,
  Interface-Name, Ports) kommt aus Quadlet `Network=` bzw. `CreateCommand`;
  ohne beides aus den Istwerten mit `static: unknown`.
- Nicht gesichert: aardvark-dns, `ipam.db`, `rootless-netns` (Laufzeit).
- `host`, `none`, `pasta`, `slirp4netns`, `container:<id>` sind Netzmodi,
  keine Netzobjekte; `ns:<pfad>` ist ein Planfehler.
- Falle `--ignore`: Quadlet legt Netze mit `podman network create --ignore`
  an, das ein vorhandenes Netz gleichen Namens auch bei anderen Parametern
  stillschweigend behält. Der Restore-Plan prüft das ausdrücklich [A §3, A §5].

### Named Volumes

- Daten in `volume_path/<name>/_data` (rootless `<graphroot>/volumes`,
  rootful `/var/lib/containers/storage/volumes`, änderbar in
  `containers.conf`) [S §2].
- **Rootless im User-Namespace lesen**: Die Dateien gehören Sub-UIDs; als
  Konto-UID scheitert das Lesen, in einem Namespace mit demselben Mapping wie
  Podman gelingt es [S §2, K §2.1]. Die **IDs werden im Storage-Namespace**
  gesichert (Containersicht, z. B. 0, 1000); das Manifest hält Quell-`uid_map`
  und `gid_map` sowie je Volume die vorkommenden IDs (`uidset`, `gidset`),
  damit der Restore vorab prüfen kann, ob das Ziel sie abbilden kann. ACL und
  `security.capability` stehen auf der Platte in Host-IDs (Caps v3 mit
  rootid) und werden deshalb ebenfalls im Namespace gelesen [S §3].
- Rootful: echte Host-IDs, `ofd` liest direkt; Container mit `--userns=auto`
  oder `--uidmap` haben ihre Dateien trotzdem im Init-Namespace beschrieben.
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

Lesen: Tiefensuche über `openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_XDEV)`
ab dem `_data`-Deskriptor, dessen Gerät/Inode der Host gegen das Inventar
prüft; Hardlinks je Volume über `(st_dev, st_ino)`, Löcher über
`SEEK_DATA`/`SEEK_HOLE`; Stat beim Schließen erneut vergleichen [S §2, S §3].

### Quadlet und systemd

- Suchpfade rootful: `/run/containers/systemd/` > `/etc/containers/systemd/` >
  `/usr/share/containers/systemd/`; rootless: `$XDG_RUNTIME_DIR/containers/systemd/`,
  `~/.config/containers/systemd/`, `/etc/containers/systemd/users/<UID>/`,
  `/etc/containers/systemd/users/`; rekursiv, Symlinks erlaubt. Arten
  `.container`, `.pod`, `.network`, `.volume`, `.kube`, `.image`, `.build`
  [A §5].
- **Zuordnung** Container → Unit über das Label `PODMAN_SYSTEMD_UNIT`, dann
  `systemctl [--user] show -p SourcePath,FragmentPath,DropInPaths`: `SourcePath`
  ist die Quadlet-Datei; `DropInPaths` nennt nur systemd-Drop-ins, keine
  Quadlet-Drop-ins [A §5, K §1.3].
- **Vollständige Quellen** über `/usr/libexec/podman/quadlet -dryrun [-user]`
  mit leerer Umgebung (ohne `QUADLET_UNIT_DIRS`): schreibt nichts, gibt die
  erzeugten Units aus und nennt jede geladene Quelle und jedes Drop-in. So
  bleibt die Drop-in-Logik bei Podman [A §5].
- Gesichert unter `quadlet/` mit erhaltenem relativem Pfad je Suchpfad, dazu
  verwandte lokale Units (z. B. `tapir-ackervielfalt.target`) unter
  `systemd/`. Fingerprint je Unit: SHA-256 über Quelle, Drop-ins und
  erzeugte Unit (für das Journal).
- Quadlet-Quellen können selbst Werte enthalten (`Environment=PASSWORD=…`)
  [A §5]; Behandlung bei `env=omit` siehe „Offene Punkte“.
- Ohne laufenden Benutzermanager genügt der Dryrun; der Unit-Zustand ist dann
  `unknown`.

### Secrets, Bind Mounts, externe Abhängigkeiten

- Secrets nur als Metadaten und Verweis (Name, ID, Treiber, Ziele aus
  `CreateCommand`/Quadlet `Secret=`); nie `--showsecret` [A §1].
- Bind Mounts und EnvironmentFiles sind `external` im Manifest. Der Host kennt
  das FileSet und setzt `covered_by_fd`; nicht abgedeckte Pfade nach
  `external=warn|error`. Auf Inspect-Pfade greift das Plugin nie zu.
- Host-Dienste (`After=postgresql.service`) stehen als `external.units` im
  Manifest, ohne Stop.

### Automatische Zusammenhänge

Vom Ausgewählten aus **vorwärts** als Hülle (was es braucht); rückwärts nur
zur Information und für die Writergruppe. Jedes Objekt wird über
`(Domäne, Art, ID)` erkannt und steht einmal im Baum, mit `used_by` [A §4].

```text
Eingabe: Domäne D, Auswahl S, exclude E
1. Inventar: ps -a, pod ps, network ls, volume ls, images, secret ls, quadlet -dryrun
2. W := Treffer(S) − E                   (leer → Planfehler)
3. solange W nicht leer: x := pop(W), schon besucht → weiter
   Container: Pod (ganz) → W; Dependencies → W (requires | volumes-from |
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
   secret_ref, pod, container, quadlet), Warnungen, Planfehler
```

Der Host rechnet denselben Algorithmus auf den Inventardaten nach; seine
Objektmenge ist maßgeblich, jede spätere Anfrage wird gegen sie geprüft
(ID **und** Name; geänderte ID bedeutet Abweisung) [K §3.2].

## Attribute der Volume-Dateien

Das Bareos-Format trägt alles (voller Stat, `FT_LNKSAVED`, FIFO/Geräte,
Sparse mit Offset, ACL-Text, xattrs), der Plugin-Weg heute nicht: `plugin_ff`
setzt UID/GID auf euid/egid von `ofd`, löscht `FO_SPARSE`, `FO_ACL`,
`FO_XATTR`, `FO_CHKCHANGES`, kennt nur REG, DIR, LNK; seit dem Fix zu S-13
(`e1ecd00`) übernimmt es aus Plugin-Einträgen nur die Rechte-Bits `0o777`,
damit ein Restore in Dateien keine root-eigenen setuid-Dateien aus
Plugin-Daten erzeugt [S §3]. Für Podman-Volumes braucht es echte Eigentümer,
Sonderbits, Caps und Dateitypen, **unter Kontrolle des Hosts**.

### Erweiterung von `FileEntry` (versioniert, `api_version` 2)

```protobuf
message FileEntry {
  // Felder 1–7 unverändert (path, kind, size, modified_unix_nanos, mode, symlink_target, attributes)
  uint32 uid = 8;  uint32 gid = 9;           // im Storage-Namespace
  IdView id_view = 10;                       // SYNTHETIC (heute) | STORAGE_NS | HOST
  int64 atime_unix_nanos = 11; int64 ctime_unix_nanos = 12;
  uint64 rdev = 13;                          // FIFO/CHAR/BLOCK
  bytes hardlink_target = 14;                // Kind HARDLINK: früherer Pfad desselben Laufs
  repeated Xattr xattrs = 15;                // ohne system.posix_acl_*, ohne security.selinux
  bytes acl_access_text = 16;                // numerisch (kein NSS)
  bytes acl_default_text = 17;               // nur Verzeichnisse
  bytes selinux_label = 18;                  // nur informativ
  bool sparse = 19;
  bytes source_ref = 20;                     // Verweis auf den Scan-Eintrag des Brokers/Backends
}
// FileKind: FIFO = 5, CHAR = 6, BLOCK = 7, HARDLINK = 8; Sockets werden nicht gesichert.
message ReadFileResponse { … uint64 data_offset = 4; }   // Lücke vor data_offset = Loch
```

Vorschlag, offen: Feld `source_ref` statt Vertrauen in die vom Gast
gelieferten Werte (siehe unten). `JobContext` bleibt in P2 unverändert (ohne
Level/Since).

### Wie der Host begrenzt

- **Vorgabe bleibt S-13**: Für jeden Eintrag gilt `id_view = SYNTHETIC`,
  UID/GID = euid/egid, Modus `& 0o777`, keine xattrs, nur REG/DIR/LNK.
- Erweiterte Attribute übernimmt der Host **nur**, wenn alles zutrifft:
  das Plugin hat `podman.volume.read`; der Pfad liegt unter
  `/@podman/<domäne>/volumes/<vol>/data/` eines Volumes der erlaubten
  Objektmenge; der Eintrag verweist per `source_ref` auf einen Scan-Eintrag,
  den Broker bzw. Rootful-Backend in diesem Job geliefert haben.
- Eigentümer, Modus (einschließlich setuid/setgid/sticky), Zeiten, `rdev`,
  xattrs, ACL und Sparse-Angabe nimmt der Host **aus seiner eigenen Tabelle**
  (Antwort des Brokers bzw. eigener `fstatat`), nicht vom Gast; weichen die
  Gastwerte ab, ist das ein Fehler. Der Gast kann so keine root-eigene
  setuid-Datei erfinden.
- Grenzen: rootless UID/GID kleiner als die Größe des Mappings; xattr-Namen aus
  Erlaubnisliste (`user.*`, `security.capability`, ACL nur als ACL-Stream,
  `security.selinux` nur informativ); xattr höchstens 64 KiB; Pfad höchstens
  4096 Byte, ohne `..`.
- `plugin_ff` setzt `FO_ACL`, `FO_XATTR`, `FO_SPARSE` aus diesen Feldern und
  schreibt die Streams selbst (ACL-Text, xattr im Bareos-Format, Sparse mit
  `data_offset`); `HARDLINK` wird `FT_LNKSAVED`. Das Volume-Format bleibt
  Bareos-kompatibel.
- **Restore in Dateien** (ohne Plugin-Code, `ofd` schreibt als root): Ein
  rootless Container-root (ns-UID 0) würde sonst echtes root. Vorschlag,
  offen: Bei `id_view = STORAGE_NS` setzt `ofd` setuid/setgid und
  `security.capability` nicht (`files_special=drop`, Vorgabe) und vermerkt sie
  im Restore-Bericht; die vollständigen Angaben stehen in `ownership.tsv`.
  IDs werden zahlengleich geschrieben (`files_owner=ns`) oder über das Mapping
  eines Kontos verschoben (`files_owner=host:<konto>`), dann genügt
  `podman unshare cp -a --sparse=always` ohne Nacharbeit [K §2.3].
- **Restore online**: rootless schreibt der Broker im Ziel-Namespace, der
  Kernel bildet die IDs auf den Zielbereich ab; rootful schreibt `ofd` echte
  IDs, aber nur in ein in diesem Job neu angelegtes Volume.

### Restore-Reihenfolge je Datei

Im Ziel-Namespace bzw. als root im neuen Volume, relativ zum `_data`-Deskriptor
mit `openat2(RESOLVE_BENEATH|RESOLVE_NO_MAGICLINKS)` und `O_EXCL` [S §3, K §2.2]:

1. `mknod`/`mkfifo`/`open(O_EXCL)`; Symlink-Ziel unverändert, nie verfolgt
2. Daten, Löcher per `ftruncate` und `pwrite`
3. `fchown`
4. `fchmod` (nach `chown`, sonst fehlen setuid/setgid)
5. xattrs
6. `security.capability` (nach `chown`, das sie löscht)
7. ACL
8. `utimensat`; Verzeichniszeiten ganz zum Schluss, wie `restore.rs`

Gerätedateien gehen rootless nicht (`mknod` im User-Namespace verboten):
Planfehler, oder mit Option übersprungen und vermerkt. Hardlinks über
Volume-Grenzen werden nicht erhalten und vermerkt. SELinux nach
`selinux=relabel|keep|drop`; MCS-Kategorien von `:z`/`:Z` gelten auf einem
anderen Host nicht [S §3].

## Broker und Rootful-Backend

**libpod über die CLI** (Vorschlag, offen; ersetzt die REST-Variante aus
[K §3.2]): `podman` mit `--format json`, fester Argumentliste und leerer
Umgebung, statt eines Sockets. Begründung: REST und CLI liefern byte-gleiche Inspect-Daten;
`podman.socket` ist auf forgeiec.io deaktiviert und darf nicht aktiviert
werden; `podman info` meldet `RemoteSocket.Exists: true` auch ohne Socket
[A §1]. Umgebung: nur `HOME` (aus passwd), `XDG_RUNTIME_DIR=/run/user/<uid>`
falls vorhanden, festes `PATH`, `LANG=C`; nie `CONTAINERS_CONF`,
`CONTAINERS_STORAGE_CONF`, `QUADLET_UNIT_DIRS`, `DOCKER_HOST`. Die libpod-DB
erzwingt die passende Speicherkonfiguration („database run root … does not
match“), stilles Arbeiten auf falschem Speicher ist damit ausgeschlossen
[A §1, A §6].

**NNP und `newuidmap`** [K §3.1]: `PR_SET_NO_NEW_PRIVS` blockiert die
setuid-Helfer `newuidmap`/`newgidmap`; rootless Podman scheitert dann, wenn
noch kein Pause-Prozess existiert. Lösung: **`ofd` legt den User-Namespace vor
dem Benutzerwechsel an** und schreibt `uid_map`/`gid_map` als root nach
`/etc/subuid`/`/etc/subgid`; läuft ein Pause-Prozess, vergleicht `ofd` die
Maps mit dessen `/proc/<pid>/uid_map` und bricht bei Abweichung ab. Der Broker
startet danach wie in `broker.rs` (Deskriptor, Hash, `fexecve`, leere
Umgebung, NNP, PDEATHSIG) als Konto-UID im Namespace und braucht weder
`newuidmap` noch den Pause-Prozess. Offen und in P0 nachzuweisen: Eigentümer
des Namespace (Konto-UID), ob die Podman-CLI den vorbereiteten Namespace
annimmt (interne Variable `_CONTAINERS_USERNS_CONFIGURED`, Vermutung) und wie
sie mit einem vorhandenen Pause-Prozess und dessen Mount-Namespace
zusammenspielt.

Ein Broker je (Job, Host-UID, Domäne). Nachrichten in
`proto/podman_broker.proto` (Paket `onesimus.filed.broker.podman.v1`), Rahmen
wie PostgreSQL: 4 Byte Länge + Nachricht, `MAX_FRAME` 2 MiB, Daten höchstens
ein Chunk (`buf_size`, höchstens 256 KiB) [K §3.3]. Das Rootful-Backend
implementiert dieselben Operationen in `ofd`.

| Operation | Ausführung (fest) | Grant | Host-Prüfung |
| --- | --- | --- | --- |
| `engine-identify` | `podman info/version --format json` | `podman.inventory` | UID, Maps gegen `/etc/subuid`, Version/Treiber gegen zertifiziertes Profil, Domänen-ID |
| `inventory` | `ps -a`, `pod ps`, `network ls`, `volume ls`, `images`, `secret ls` (`--format json`) | `podman.inventory` | erlaubte Objektmenge berechnen, Antwort an den Gast filtern |
| `inspect(kind, ids)` | `container/pod/network/volume/image/secret inspect`, `generate spec`; nie `--showsecret` | `podman.inventory` | ID in der Menge, Größe ≤ 1 MiB, Env nach `env=` |
| `quadlet-resolve` | `quadlet -dryrun [-user]` | `podman.inventory` | – |
| `unit-info(units)` | `systemctl [--user] show -p <feste Liste>` | `podman.inventory` | Unit gehört zur Menge |
| `quadlet-read(unit, path)` | Datei lesen | `podman.inventory` | Pfad aus Dryrun bzw. `SourcePath` **und** unter einem Suchpfad; EnvironmentFiles nie |
| `image-list`, `image-bigdata-open` | `images.json`/`layers.json` unter Lese-Sperre, gegen `image inspect` geprüft | `podman.image.read` | Digest vor Auslieferung geprüft |
| `layer-open/read/close(diff_id)` | tar-split-Rekonstruktion, nur sequentiell | `podman.image.read` | DiffID in der Menge; SHA-256 am Ende |
| `volume-open/scan/read/xattr/close` | `openat2` ab `_data` | `podman.volume.read` | (dev, ino) gegen Inventar; Einträge in Host-Tabelle |
| `member-state`, `stop-unit`, `stop-container`, `resume-*`, `unmask` | `systemctl [--user] mask --runtime/stop/unmask/start`, `podman stop/start` | `podman.lifecycle` | nur nach Journal-Eintrag; erwartete InvocationID/StartedAt |
| `preflight`, `create-network/volume/pod/container`, `volume-write-*`, `image-import-*`, `pull`, `stage-quadlet`, `rollback` | siehe Restore | `podman.restore` | Suffix, keine neuen Rechte, Restore-Label |

Der Broker ruft nie `system reset`, `system migrate` oder `system renumber`
auf [K §0]. Ein rootless CLI-Aufruf kann den Pause-Prozess (catatonit)
starten; das ist eine sichtbare Nebenwirkung, keine Rechteausweitung [A §1].

## Konsistenz

**Writergruppe automatisch** [K §1.3]: Zusammenhangskomponente der Auswahl
über gleiche Named-Volume-ID, gleiche Bind-Quelle (realpath, Gerät/Inode),
Pod, `--volumes-from`, systemd `Requires/Wants/BindsTo/PartOf` zwischen Units
mit `PODMAN_SYSTEMD_UNIT` und Quadlet-`.volume`/`.network`-Verweise. Units
außerhalb von Podman (z. B. `postgresql.service`) nur als Information.

**Stop-Verfahren** [K §1.1]:

- Container mit `PODMAN_SYSTEMD_UNIT`: **`systemctl [--user] mask --runtime
  <unit>`, dann `stop`**. Ein `podman stop` auf einen Quadlet-Container ist
  wirkungslos (systemd startet mit neuer Container-ID neu). Die Laufzeit-Maske
  liegt vor `/run/systemd/generator`, wirkt gegen `Restart=`, gegen `Wants=`
  anderer Units, Timer, manuelles `start` und `podman auto-update`, und
  verschwindet beim Neustart von selbst.
- Container ohne Unit: `podman stop -t <stop_timeout>`; `StoppedByUser`
  verhindert die Restart-Policy. `podman-restart.service` startet beim Boot
  auch vom Benutzer gestoppte Container (Recovery berücksichtigt das).
- Reihenfolge: abhängige Units zuerst (ackervielfalt-web/worker/beat vor
  redis), Wiederanlauf umgekehrt, jeweils auf `ActiveState=active` warten.
- **Sauberkeitsnachweis**: Exit-Code in `clean_exit_codes`, kein SIGKILL nach
  Frist (137), systemd `Result=success`. Sonst `on_unclean`.
- Pause/Freeze ergibt nur einen Absturzzeitpunkt bei voller Ausfallzeit, Podman
  und systemd sehen dann verschiedene Zustände; App-Hooks brauchen Exec. Beides
  P4 [K §1.2].

**`consistency_scope=job`** (Vorgabe, Nutzer, 19.11.2026): tapir hat keine
Named Volumes; seine Daten liegen in Bind Mounts, die der FD als Dateien
sichert. Ein Stop nur für die Dateien der Plugin-Zeile schützte sie nicht
[K §1.2]. Heute öffnet `ofd` eine Plugin-Sitzung erst, wenn die Traversierung
die Plugin-Zeile erreicht, und sendet erst dann `JobStarted`
(`backup.rs` `save_plugin`/`run_plugin`). Nötige Host-Änderung: Vor der ersten
Datei des Jobs wertet `ofd` alle Podman-Zeilen mit `scope=job` aus
(Inventar, Inspect, Journal `PREPARED`, Stop), hält die erfassten Daten
begrenzt bis zur Plugin-Sitzung und fährt die Gruppen nach der letzten Datei
des Jobs wieder an, ebenso bei Fehler und Cancel. Mehrere Zeilen mit
überschneidenden Gruppen bilden eine Lifecycle-Gruppe.

`consistency=crash`: keine Mutation; je Datei `size`/`mtime`/`ctime` vor und
nach dem Lesen vergleichen, Änderungen als `changed_during_backup` im
Manifest (Warnung).

### Journal mit Recovery

Beim Host, nicht im Broker oder Gast: root-eigene Datei
`/var/lib/onesimus/ofd/podman-journal/<domäne>/<jobid>.json`, `fsync` und
`rename`, geschrieben **vor** jeder Mutation, für rootless und rootful
gleich [K §1.3]. Inhalt: Boot-ID, Job und Instanz, Host-UID und Domäne
(einschließlich `tmpdir`), je Mitglied Art, Name, Unit-`InvocationID` bzw.
Container-ID und `StartedAt`, Definitions-Fingerprint (Quadlet-Quelle,
Drop-ins, erzeugte Unit bzw. Spec), Ursprungszustand, eigene Maske (ja/nein),
Stop-Ergebnis (Exit-Code, sauber, Zeit).

| Zustand | wird geschrieben | Recovery, wenn dies der letzte Zustand ist |
| --- | --- | --- |
| `PREPARED` | nach Inventar und Inspect aller Mitglieder (Pflicht vor dem Stop) | nichts mutiert → `ABORTED` |
| `STOP_REQUESTED` | vor Maske und Stop, je Mitglied | aktiv mit gleicher InvocationID und ohne eigene Maske: nichts passiert. Inaktiv mit eigener Maske: wie `RESUME_PENDING`. Sonst `CONFLICT` |
| `QUIESCED` | alle Mitglieder gestoppt, Ergebnisse festgehalten | wie `RESUME_PENDING` |
| `CAPTURING` | vor der ersten Datei | wie `RESUME_PENDING`; Job fehlgeschlagen |
| `RESUME_PENDING` | nach der letzten Datei, bei Cancel oder Fehler | Wiederanlauf **nur**, wenn gleiche Boot-ID, Mitglied inaktiv, Maske die eigene, Fingerprint unverändert, bei Containern `StoppedByUser` und `FinishedAt` = eigener Stop. Sonst `CONFLICT` |
| `RESUMED` | alle Mitglieder im Ursprungszustand | fertig, archivieren |
| `CONFLICT` | – | manuelle Klärung; `ofd --podman-journal` zeigt, `--resolve <job>` entfernt nur die eigene Maske |

Sonderfälle: **Neue Boot-ID**: die Maske ist weg, systemd hat nach `WantedBy`
gestartet; Units nur prüfen, einfache Container ohne Restart-Policy nur bei
unverändertem Fingerprint starten. **Bediener startet während der
Sicherung** (InvocationID bzw. Container-ID geändert): Job fehlerhaft,
`consistency_violated`, kein zweiter Stop, kein Wiederanlauf durch uns.
**Cancel**: sofort `RESUME_PENDING`, Wiederanlauf mit eigenem Zeitbudget und
frischer Broker-Instanz; ein begonnener `systemctl stop` wird nicht
wiederholt, sondern sein Ergebnis gelesen. **`ofd` stirbt**: Maske bleibt bis
zur Recovery beim Start von `ofd` und vor jedem neuen Podman-Job der Domäne.
Ein Totmannschalter (`systemd-run --on-active=<max_downtime>`) prüft keinen
Fingerprint und ist nicht im MVP [K §1.3].

## Threading, Grenzen, Abbruch

Eine Plugin-Instanz und ein Broker je (Job, Host-UID, Domäne); verschiedene
Konten laufen voll parallel, innerhalb einer Instanz ein geordneter Worker
(`NextFile`/`ReadFile` sind seriell). Leases beim Host, Schlüssel
(Host, Domäne, Objekt-ID): **Lifecycle-Lease** über die ganze Writergruppe von
`STOP_REQUESTED` bis `RESUMED` (ein zweiter Job wartet bis zur Frist oder
bekommt einen Planfehler und nimmt nie eine fremd gestoppte Gruppe wieder
auf); teilbare Lese-Leases; Restore-Leases auf Zielnamen. Kein globaler Lock
über I/O [K §4].

Kein Spool beim Sichern: Layer und Volume-Dateien werden offsetbasiert
gelesen. Spool nur für den Image-Import beim Restore, auf der Platte
(`/var/tmp/onesimus-podman/<domäne>/<job>/`, Eigentümer das Konto, 0700),
Quote = Summe der `diff-size` der benötigten Layer, `statvfs` vorab, Löschen
bei Close, Cancel und Recovery. Grenzen je Instanz: offene Handles (64),
Einträge je Scan-Antwort (1024), Pfadlänge (4096), xattr (64 KiB),
Inspect/Spec (1 MiB), Objekte im Inventar (konfigurierbar). Abbruch: Epoch-
Unterbrechung des WASM-Stores, SIGTERM/SIGKILL an die Broker-Prozessgruppe,
danach Recovery mit eigenem Budget (`cancel_timeout`, `recovery_timeout`).

## Restore

### `restore_mode=files`

Der Baum landet unter `Where` wie im SD-Strom, ohne Plugin-Code. Von Hand
nutzbar: Images per `podman pull oci:<Where>/@podman/<domäne>/images/oci:<name>`;
Volume-Daten mit `files_owner`/`files_special` wie im Abschnitt Attribute,
danach `podman volume create` und `podman unshare cp -a --sparse=always`
[K §2.3]. Quadlet-Quellen und Inspect-Daten dienen als Vorlage.

### `restore_mode=online` (gestagt)

Ablauf mit Restore-Journal `PLANNED → CREATING(n) → FILLED → VERIFIED →
STAGED`; jede angelegte Ressource trägt das Label
`org.onesimus.restore-job=<jobid>` und steht in der Liste `created` [K §2.2].

0. **Vorprüfung ohne Mutation**: Zielkonto freigegeben; Mapping groß genug
   (größte gesicherte ns-UID/GID kleiner als das Ziel-Mapping); Plattform;
   alle Zielnamen mit Suffix frei; Subnetze frei; Secrets vorhanden; Images
   verfügbar; Ports und statische IPs gegen Bestand. Ergebnis: Plan mit
   Konfliktliste; Planfehler brechen vor jeder Mutation ab.
1. **Netze** aus Inspect mit Erlaubnisliste (bridge/macvlan/ipvlan, Subnetz,
   Gateway, IP-Bereich, IPv6, internal, DNS, Optionen, Labels), Name mit
   Suffix.
2. **Volumes** neu anlegen (Treiber `local`, Optionen aus Erlaubnisliste);
   prüfen: leer, `CreatedAt` ≥ Jobstart, Restore-Label. Befüllen im
   Ziel-Namespace mit der Reihenfolge aus dem Abschnitt Attribute; Ergebnis wie
   `cp -a --sparse=always` im Namespace (Eigentümer, Modus, Hardlinks, Symlinks,
   Löcher) [K §2.1]. Danach Vergleich mit dem Manifest (Anzahl, Größen,
   Prüfsummen). Nie `podman volume import`: es führt Inhalte zusammen [K §2.1].
3. **Images**: `reference` per Pull nur als `name@sha256:…` aus erlaubten
   Registries, dann Image-ID prüfen; `blobs` als OCI-Layout im Spool, feste
   Befehlszeile `podman pull oci:<spool>:<ref>`, Image-ID gleich Config-Digest.
   Ein vorhandenes gleiches Image wird wiederverwendet.
4. **Pods, dann Container** nach `restore_strategy`: Basis ist `spec.json`
   aus `podman generate spec`; bei `create-command` überschreiben die
   geprüften Optionen der Positivliste die verlorenen Felder (statische IP,
   Secret-Ziele, pasta-Optionen, `keep-id`). Name mit Suffix, Volumes nur aus
   diesem Job, Netze aus diesem Job oder freigegeben, Bind Mounts nur über
   `bind_map`, `--requires` auf Suffix-Namen, Label `restored-from`, **nicht
   starten**. Anlage über libpod create; der Host prüft die Spec: keine neuen
   Rechte gegenüber dem Original (privileged, cap_add, devices,
   pid/ipc/net=host nur, wenn im Original **und** von der Policy erlaubt)
   [K §2.1, K §3.3].
5. **Quadlet**: Quellen umschreiben (Suffix für `ContainerName`, `.volume`- und
   `.network`-Verweise, `[Install]` entfernen, `image_pin`-Drop-in) und
   **außerhalb aller Quadlet-Suchpfade** ablegen: rootless
   `~/.local/share/onesimus/staged/<jobid>/`, rootful
   `/var/lib/onesimus/podman-staged/<jobid>/`. Unterverzeichnisse der
   Suchpfade sind verboten, weil Quadlet sie rekursiv liest [K §2.1].
   Quadlet-Container werden nicht vorab angelegt; Quadlet erzeugt sie beim
   Start. `EnvironmentFile=` bleibt ein Verweis.
6. **Bericht**: angelegte Objekte, offene Punkte (Secrets, Env-Werte, Ports,
   EnvironmentFiles, `lossy_fields`). Promotion (Quadlet in den Suchpfad,
   `daemon-reload`) und Start gibt der Administrator getrennt frei.

Fehler nach Schritt 0: Rückbau **nur** der Objekte in `created` mit eigenem
Label; teilweise befüllte Volumes werden gelöscht, nicht repariert; nach
Absturz Recovery aus dem Restore-Journal nach derselben Regel.

libpod create ohne Socket: Vorschlag, offen: Broker bzw. Rootful-Backend
starten für die Dauer des Restores einen privaten
`podman system service --time=0 unix://<privates-verzeichnis>/s.sock` als
eigenes Kind und beenden ihn danach; kein fremder Socket, keine
Unit-Aktivierung [A §1]. Alternative: der Host erzeugt aus der Spec eine
geprüfte `podman create`-Argumentliste.

### Konflikte

| Konflikt | Verhalten von Podman | Behandlung |
| --- | --- | --- |
| gleichnamiges Netz | „network name … already used“ | Suffix; bei `existing` muss das Subnetz passen, sonst Planfehler |
| Subnetz überlappt (auch `10.88.0.0/16`) oder Host-Route | „subnet … is already used“ [K §2.1] | `network_subnet=keep`: Planfehler; `auto`: neues Subnetz, Warnung |
| Quadlet-`.network` existiert mit anderem Subnetz | `--ignore` verschweigt es [A §3] | Planfehler |
| `network_interface` (Bridge-Name) belegt | – | Feld weglassen, netavark vergibt; vermerken |
| macvlan/ipvlan-Parent fehlt | – | Planfehler (später `netmap=`) |
| statische IP belegt | `create` gelingt, `start` scheitert („already allocated“) [K §2.1] | Planfehler bei `restore_ip=keep`, sonst Warnung |
| Host-Port belegt, rootless < 1024 | `create` gelingt, `start` scheitert; unter `Restart=always` Neustartschleife [K §2.1] | Warnung im Plan; Container bleiben gestoppt |
| fehlendes Secret | `create` scheitert sofort [K §2.1] | Planfehler mit Name |
| gleichnamige Quadlet-Quelle im Ziel | – | Planfehler bei Promotion |
| Standardnetz `podman` abweichend | – | Referenz plus Subnetzangabe, Warnung |
| rootful → rootless | macvlan/ipvlan, feste Bridge-Namen eingeschränkt | Planfehler bei nicht unterstützten Treibern |
| ns-IDs außerhalb des Ziel-Mappings | – | Planfehler „subuid-Bereich zu klein“ |
| `NeedsCopyUp`/`NeedsChown` am neuen Volume | Podman kopiert nur in leere Volumes, `chown` nur der Wurzel (Vermutung) [S §2] | Testfall; Original-Flags im Inspect |

## Später: Accurate und Incrementals

MVP ist Full-only. `JobContext` hat weder Level noch Since; `save_plugin_file`
ruft `accurate_check_file` nicht auf und `plugin_ff` löscht `FO_CHKCHANGES`
[S §4]. Für P4: `JobContext` um `level` und `since_unix` erweitern,
`accurate_check_file` mit dem Stat aus `FileEntry` aufrufen, Löschungen über
`accurate_finish` (`FT_DELETED`). Image-Blobs fallen dank Digest-Pfad und fester
`mtime`/`ctime` automatisch aus Incrementals heraus; Volumes liefern alle
Einträge, Daten nur für geänderte Dateien (ctime erfasst auch `chown` und
xattr). Ebenfalls P4: `consistency=freeze` (Pause plus Snapshot, sobald
Volumes als Subvolumes zertifiziert sind; btrfs-Volumes sind heute keine
Subvolumes [K §1.1]), signierte App-Adapter, Remote, weitere Storage-Treiber.

## Abnahmefall forgeiec.io

Rootful Debian 13, Podman 5.4.2, overlay auf ext4, SELinux aus, netavark
[S Einleitung, A §4, K §1.1]: fünf Quadlet-Units `tapir-*` in
`/etc/containers/systemd/` (alle `Restart=always`, `Network=host`, Label
`PODMAN_SYSTEMD_UNIT`), `tapir-ackervielfalt.target` in `/etc/systemd/system`
(`PartOf`), `Wants=tapir-redis.service`, `After=postgresql.service` (Host),
`EnvironmentFile=/etc/tapir/keycloak.env` und
`/etc/tapir/instances/ackervielfalt.env`, Bind Mounts
`/var/lib/tapir/{ackervielfalt,redis,keycloak/themes/wirgarten}`, keine
Named Volumes, keine eigenen Netze, keine Pods, keine Podman-Secrets; drei
Images (tapir `:latest` 1,11 GB, keycloak 26.8.0, redis 7.4-alpine), keine
gemeinsamen Layer. Daneben `www-data` mit Matomo, nicht freigegeben.

```text
FileSet {
  Name = "forgeiec-tapir"
  Include {
    Options { signature = XXH128; compression = LZ4 }
    File = /etc/tapir
    File = /var/lib/tapir
    Plugin = "podman:user=root:quadlet=tapir-*:images=blobs"
    Plugin = "postgresql:instance=main"
  }
}
```

Erwartet:

- Plan: fünf Quadlet-Quellen, das Target, drei Images, keine Volumes und
  Netze, `external.paths` für beide EnvironmentFiles und die Bind-Quellen mit
  `covered_by_fd: true`, `external.units` `postgresql.service`, `www-data` als
  `discovered_unconfirmed` angezeigt und nicht gesichert.
- Sicherung: Inspect und Spec vor dem Stop; Writergruppe aus allen tapir-Units
  über `Wants`/`PartOf` und gemeinsame Bind-Quelle; Stop über Laufzeit-Maske für
  den ganzen Job (`consistency_scope=job`), also auch während der FD die
  Dateien unter `/var/lib/tapir` und `/etc/tapir` sichert; PostgreSQL läuft
  weiter und wird über sein Plugin gesichert. `env.json` nur mit Namen.
  Image-Blobs je DiffID. Journal endet in `RESUMED`, alle Units wieder aktiv.
- Grants: `podman.rootful`, `podman.inventory`, `podman.image.read`,
  `podman.lifecycle`.
- Restore auf einem frischen Testhost: `files` für `/etc/tapir` und
  `/var/lib/tapir`, `online` für Podman: gleiche Image-IDs, Quadlet-Quellen
  gestagt mit `image_pin`-Drop-in, keine Aktivierung; Anwendungsprüfung erst
  nach ausdrücklicher Promotion und Start.

## Testaufbau in Containern

Alle Versuche ohne root und ohne Produktivsystem, auf dem Entwicklungsrechner
und in CI [S Testaufbau, K §0]:

```sh
T=$(mktemp -d /tmp/podtest.XXXX)
export XDG_RUNTIME_DIR=$T/xdg-run XDG_DATA_HOME=$T/xdg-data   # eigener Blob-Info-Cache und Laufzeitordner
P="podman --root $T/root --runroot $T/run --tmpdir $T/tmp --storage-driver vfs"     # Quelle
Q="podman --root $T/root2 --runroot $T/run2 --tmpdir $T/tmp2 --storage-driver vfs"  # Ziel
```

**Isolationsregeln** (Lehre aus einem Zwischenfall [K §0]): `--root` und
`--runroot` allein trennen weder das libpod-`tmpdir` (`alive`, `pause.pid`,
`exits/`) noch die Lock-Tabelle in `/dev/shm`; ein `system reset` auf einem
Wegwerf-Speicher ließ Podman einen Neustart annehmen und den echten Speicher
des Kontos „auffrischen“. Deshalb: immer eigenes `--root`, `--runroot`,
`--tmpdir` und `XDG_RUNTIME_DIR`, besser ein eigenes Testkonto; **nie
`podman system reset`**; Aufräumen mit `$P unshare rm -rf $T`.

- Image-Fixtures: busybox und alpine (Multi-Arch-Index, gzip-Layer), ein
  abgeleitetes Image mit gemeinsamer Basis und abweichender Layer-ID/DiffID,
  ein lokal gebautes Image ohne Registry-Manifest; später zstd und
  zstd:chunked.
- Volume-Fixture (per `$P run --network none` und `$P unshare`): Verzeichnis
  0700 von UID 1000, Hardlink, Symlink, FIFO, Sparse-Datei mit 100 MiB Loch,
  `user.*`-xattr, ACL und Default-ACL, `security.capability`, setuid-Datei,
  Name mit Byte 0xff, leeres Verzeichnis, UID außerhalb des Mappings
  (Negativfall); Varianten `o=bind`, `tmpfs`, Treiber `image`.
- Rundreise: Sicherung über `ofd` in den SD-Strom (Ende-zu-Ende-Aufbau aus
  Phase 1), Restore `online` nach `$Q`; Vergleich je Datei im Namespace:
  SHA-256, `stat -c '%u %g %a %h %s %b'`, `getfacl -n`, xattrs, Caps,
  Inode-Gleichheit der Hardlinks, `SEEK_DATA`; Images: `Id`, `RootFS.Layers`,
  `RepoTags`, dann `$Q run --rm --network none <id> true`.
- Anderer subuid-Bereich: verschachtelter Namespace
  `unshare --user --map-users=0:5000:2000`; echte zweite Konten mit anderem
  `/etc/subuid` und das rootful Profil in einem Container
  (`quay.io/podman/stable`, privilegiert im rootless Podman) oder einer
  Test-VM.
- Quadlet-Tests nur in einem eigenen Benutzermanager des Testkontos bzw. im
  Container, mit Quellen unter dessen `$XDG_RUNTIME_DIR/containers/systemd`.

## Phasen und Exitkriterien

| Phase | Liefergegenstand | Exitkriterium |
| --- | --- | --- |
| P0 | Grundlagen: `FileEntry` v2 mit Host-Kontrolle, WIT-Schnittstelle `podman`, `podman_broker.proto`, Broker-Start mit von `ofd` angelegtem User-Namespace, Gerüst des Rootful-Backends, Kontopolicy, Grants, erweiterte Discovery, Journaldatei | Gast kann keine Eigentümer, Sonderbits oder Caps erfinden; Broker ohne Pause-Prozess führt unter NNP `podman info` aus; Rootful-Backend ohne generisches Exec; conmon-Fund und bestätigte Domäne getrennt |
| P1 | Inventar und Plan: Zusammenhangs-Algorithmus in Gast und Host, Quellen-Rangfolge, Env-Filter, Manifest ohne Daten | gestoppte Container und Quadlet-Units ohne Container sichtbar; gleiche Namen verschiedener UIDs getrennt; Host- und Gast-Objektmenge stimmen überein; Plan für forgeiec.io wie erwartet |
| P2 | Full-Backup: Dateibaum, Image-Blobs per tar-split, Volumes mit Attributen, Stop mit `consistency_scope=job`, Journal und Recovery | Crash-/Cancel-Tests an jedem Journalübergang; Metadaten-Rundreise im Testaufbau; Abnahme-Sicherung forgeiec.io |
| P3 | Restore `files` und `online` mit Konflikttabelle, Rückbau und separater Promotion | sauberer Zielhost offline, gleiche Image-IDs, bestehende Ressourcen unverändert, Rückbau nur eigener Objekte; forgeiec-Restore auf Testhost mit Anwendungsprüfung |
| P4 | Accurate/Incrementals, `freeze`, App-Adapter, `image_digest=original`, Remote, weitere Treiber | je Profil eigene Zertifizierung und Rundreise |

## Pflichttests

- Discovery: conmon zweier rootless UIDs, rootful conmon, CRI-O-Fehlkandidat,
  manipulierter Prozessname, PID-Reuse, Exit während der Erkennung; Konto mit
  runroot `/tmp/storage-run-<uid>`, abweichender graphroot, Konto nur mit
  gestoppter Quadlet-Unit; eingeschränkte `/proc`-Sicht, fehlendes NSS-Konto,
  UID-Wiederverwendung; Plugin-Zeile mit nicht freigegebenem Konto.
- Grants: fehlendes `podman.rootful` bei `user=root`, fehlendes
  `podman.lifecycle` bei `consistency=stop`, lokale Policy entzieht Grants.
- Host-Kontrolle: Gast liefert abweichende UID/Modus/xattrs, setuid ohne
  `source_ref`, Pfad außerhalb eines Volumes; Restore in Dateien mit
  ns-UID 0 und setuid (`files_special=drop`).
- Zwei UIDs mit identischen Namen; manipulierte IDs, Handles und Symlinks;
  kein Zugriff über Konto- oder Jobgrenzen.
- Env: `env=omit` liefert keinen Wert an Gast, Volume, Joblog oder Katalog
  (Inspect, Spec, `CreateCommand`, Dryrun); `env=keep` nur auf dem Volume.
- Pods: Auswahl eines Mitglieds sichert den ganzen Pod.
- Images: Layer-Rekonstruktion bit-genau (SHA-256 = DiffID), Layer verschwindet
  während des Streams, Layer ohne tar-split, Restore mit gleicher Image-ID und
  neuem Manifest-Digest, veränderlicher Tag mit `image_pin`.
- Volumes: andere subuid-Bereiche, ACL/xattr/Caps, Hardlinks, Sparse,
  Symlinks, nicht-UTF-8-Namen, Gerätedateien rootless, `o=bind` außerhalb der
  Allowlist, fremder Treiber; Restore in nichtleeres Ziel abgewiesen.
- Konsistenz: Quadlet `Restart=always` unter Maske, `Wants=` einer anderen
  Unit, Timer, Bediener startet während der Sicherung, SIGKILL nach Frist,
  `scope=job` deckt die Include-Dateien ab; FD-/Broker-Crash vor und nach
  jedem Journalübergang; neue Boot-ID.
- Restore: Port-, IP-, Subnetz- und Secret-Konflikte wie in der Tabelle;
  `--ignore`-Falle; Staging nie unter einem Quadlet-Suchpfad; Rückbau nur mit
  eigenem Label; kein automatischer Start.
- Cancel bei vollem Puffer, während `StopUnit` und während eines Layer-Streams;
  Plugin A hält Device B nicht an.
- gRPC-Ausfall mit BNET-Fallback ohne doppelte Jobausführung.

## Offene Punkte

1. **Quadlet-Quellen mit Werten bei `env=omit`**: Quellen können
   `Environment=PASSWORD=…` enthalten. Vorschlag, offen: byte-genau sichern,
   weil sie Konfigurationsdateien wie EnvironmentFiles sind, mit Warnung
   `quadlet_env_values` (nur Namen); Alternative: Werte entfernen und die
   Quelle als verändert kennzeichnen.
2. **Podman-CLI im von `ofd` angelegten Namespace** (Eigentümer,
   `_CONTAINERS_USERNS_CONFIGURED`, Pause-Prozess, Mount-Namespace): P0-Nachweis.
3. **libpod create ohne Socket**: privater kurzlebiger Dienst oder geprüfte
   `podman create`-Argumentliste.
4. **Grant-Namen** und **Ort/Format der Kontopolicy**.
5. **`restore_mode`**: `online` (wie in `ofd`) mit `stage` als Synonym oder nur
   `online`.
6. **Restore in Dateien**: `files_special`, `files_owner`, `ownership.tsv`.
7. **Lese-Sperren** über lange Layer-Streams oder Prüfung danach.
8. **Discovery**: ob `storage.conf` der Benutzer (nur Schlüssel `graphroot`)
   gelesen werden darf oder nur Standardpfad, conmon-Hinweis und Admin-Eintrag.
9. **Healthcheck** `--health-on-failure=restart` während Stop/Maske (nicht
   geprüft); `NeedsCopyUp`/`NeedsChown` an neuen Volumes.
10. **SELinux** `relabel` ungeprüft (beide untersuchten Rechner ohne SELinux).
11. **Versionen**: erstes zertifiziertes Profil Podman 5.4.2 (netavark, sqlite,
    overlay/btrfs/vfs); andere Versionen erst nach eigenem Test. Die
    `latest`-Dokumentation ist Planungsquelle, keine Freigabe.
12. **Totmannschalter** für Masken nach `ofd`-Absturz: nicht im MVP.

## Umsetzungsstand

`onesimus-plugins::executor` implementiert isolierte Worker, begrenzte Queues,
Instanz-/Job-/Device-Prüfung, direkte Interrupts und einen Deadline-Watchdog.
Der WASM-Host lädt signierte Gäste (Phase 1); der Broker-Start aus geprüftem
Deskriptor existiert für PostgreSQL (`broker.rs`) und ist die Vorlage für
`onesimus-podman-broker`.

`onesimus-plugins::discovery` erfasst conmon-Kandidaten:

```bash
ofd --discover-podman
ofd --discover-podman --conmon-executable /usr/bin/conmon   # mehrfach verwendbar
```

Der Modus braucht keine FD-Konfiguration, startet keinen Listener und gibt
JSON aus. Er prüft root-eigene, nicht gruppen-/weltbeschreibbare Executables
über Device/Inode und vergleicht PID/Startzeit/UID/Executable doppelt.
Kandidaten bleiben `discovered_unconfirmed`; `host_scope_verified` bleibt
false. Kontoregistry, Storage-Indizien, NSS-Anzeige und Bestätigung durch den
Broker fehlen noch. P0 ist begonnen, nicht abgeschlossen.
