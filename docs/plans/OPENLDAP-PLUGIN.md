# Planung: OpenLDAP-Plugin für ofd

Stand: 19.11.2026. Status: **neu geschrieben nach zwei Prüfberichten
(Sicherheit S-1 bis S-16 / E-1 bis E-12, LDAP-Mechanik L-1 bis L-20) und den
verbindlichen Entscheidungen des Nutzers vom 19.11.2026; umgesetzt und in
Podman für beide Konfigurationsarten getestet (Abschnitt „Umsetzung“). Ende zu Ende gegen `odir` + `bareos-sd` mit Katalog
am 19.11.2026 gemessen (O-7, Wegwerf-CA). Offen: Bau mit echtem
Signierschlüssel, Audit-Spur beim Laden (B-1).** Ersetzt den Entwurf vom 27.10.2026 (LDAP-Protokoll-
Broker über ldapi als root, Eintragsbaum), der nicht mehr zur Architektur passt.

Das Plugin sichert einen lokalen OpenLDAP-Server (`slapd` 2.5/2.6, Backend
`mdb`) vollständig und konsistent – Daten, Konfiguration, Overlays, ACLs,
Passwort-Hashes – und ersetzt das Python-Plugin `bareos-fd-ldap` (Job
`BackupBgLage`). Es gelten [Plugin-Architektur](../wiki/plugin-architektur.md),
[Signier-Infrastruktur](SIGNIER-INFRASTRUKTUR.md) und das Vorbild
[PostgreSQL](POSTGRESQL-PLUGIN.md) / [Phase 2](POSTGRESQL-PHASE2.md):
signiertes Rust/WASM ohne Importe außer einem typisierten Broker-Import,
nativer Broker als Instanzbenutzer, nie als root, der Host prüft jede Anfrage.

## Entscheidungen (19.11.2026, verbindlich)

| # | Thema | Entscheidung |
| --- | --- | --- |
| E-1 | Zugriff | `slapcat` über den eigenen signierten Broker `onesimus-ldap-broker`, der als Instanzbenutzer (`openldap`) läuft, nie als root. **Kein Online-Lesen über ldapi**, kein LDAP-Protokoll im Broker. |
| E-2 | Entfernte Server | nicht unterstützt; `ofd` läuft auf dem LDAP-Host (BgLage: dort installieren). |
| E-3 | Passwörter | keine in Plugin-Zeilen. `ofd` **lehnt** eine `openldap`-Zeile mit geheimem Schlüssel (`password=`, `bind_pw=`, `credentials=`, …) ab, statt sie nur zu maskieren. |
| E-4 | Namen | Plugin-ID `io.onesimus.openldap`, Capabilities `openldap.read` und `openldap.write` (siehe unten); vor dem Ausstellen des G1-Signierzertifikats in `signer.cnf`. |
| E-6 | Form | **eine LDIF-Datei je Datenbank**, kein Eintragsbaum. cn=config bei slapd.d über `slapcat -n 0`, Datenbanken immer über `slapcat -b <suffix>` (die Nummern verschieben sich, L-11). |
| E-7 | Online-Restore | nur `slapadd` in ein **neues, leeres Ziel**, das der Host anlegt; der Host schreibt `olcDbDirectory`/`directory` auf das Ziel um. Nie in einen laufenden Produktivserver, kein modify/delete. |
| E-8 | Konfigurationsarten | Online-Konfiguration (slapd.d/cn=config) und Datei-Konfiguration (slapd.conf) gleichwertig, Erkennung automatisch. Restore stellt die gesicherte Art wieder her, keine Umwandlung. |
| E-9 | TLS-Schlüssel | gehören nicht zum Plugin. Ein normales `File =` im selben FileSet sichert sie (ofd als root); das Plugin **warnt**, wenn die von der Konfiguration genannten TLS-Dateien im FileSet fehlen. |
| E-10 | Broker allgemein | Vorphase: Host für mehrere Broker verallgemeinert (S-10), PostgreSQL unverändert. |
| — | Geheimnisse im Inhalt | `userPassword`, `olcRootPW`/`rootpw`, `credentials=` (syncrepl, idassert-bind), `olcDbACLPasswd`/`acl-passwd` stehen nur auf dem Volume, nie im Joblog, im Katalog oder in `_info.json` (dort nur die **Namen** der Geheimnisarten). Die Fehlerausgabe von slapcat/slapadd wird gefiltert. |
| — | Paket | `ofd-plugin-openldap` nach dem Muster von `ofd-plugin-postgresql`; postinst trägt `Plugin Names = "openldap"` ein, Neustart nur ohne laufenden Job, **kein Eingriff in slapd**. |

Bereits behoben vor diesem Plan (Mängelliste, E-11): Maskierung in
`FileSetText`/`show fileset` (S-3, `63b24ff`), Schlüssel mit `pw` (S-4),
Modus-Maske `& 0o777` für Plugin-Dateien (S-13, `e1ecd00`).

## Plugin-Zeile

```
FileSet {
  Name = "ldap-fs"
  Include {
    Plugin = "openldap:instance=main"
    File = /etc/ldap/tls            # TLS-Schlüssel (root-only), E-9
  }
}
```

| Option | Vorgabe | Bedeutung |
| --- | --- | --- |
| `instance` | `main` | Name im virtuellen Baum `/@openldap/<instance>/`, `[A-Za-z0-9_-]{1,64}`. |
| `databases` | `*` | Suffixe (Liste mit `;`, weil ein Suffix Kommas enthält; `*`-Muster, Groß-/Kleinschreibung egal). Nur `mdb` ist sicherbar. |
| `exclude` | leer | Suffix-Muster, die ausgelassen werden. |
| `config` | `yes` | Konfiguration mitsichern (cn=config bzw. Dateisatz). |
| `config_path` | – | Nur Auswahl, wenn **mehrere** slapd laufen: der Wert muss dem erkannten `-F`/`-f` eines Prozesses entsprechen. Wird nie als Pfad benutzt. |

Ein ausdrücklich genannter Suffix, den es nicht gibt oder der kein `mdb` ist,
ist ein Planfehler; ein Muster ohne Treffer nicht.

Restore-Optionen (Restore-Job, `Plugin Options`):
`openldap:restore_mode=online[:target=/var/lib/onesimus-ldap-restore/<name>]`.

## Virtueller Baum

```
/@openldap/main/                              Verzeichnis
/@openldap/main/_info.json                    zuerst: Art, Version, Datenbanken, TLS-Dateien, Geheimnisarten
/@openldap/main/config/cn=config.ldif         slapd.d: slapcat -n 0
/@openldap/main/config/files/etc/ldap/slapd.conf          slapd.conf: Hauptdatei
/@openldap/main/config/files/etc/ldap/schema/core.schema  … jede include-Datei (rekursiv)
/@openldap/main/config/files/etc/default/slapd            beide Arten
/@openldap/main/data/dc=example,dc=org.ldif   slapcat -b dc=example,dc=org
/@openldap/main/data/o=second,c=de.ldif
```

- Jede `data/<suffix>.ldif` ist **slapadd-fähig** (Entry-ID-Reihenfolge, Kind
  vor Elternteil möglich, L-16; slapadd nimmt das an). Der Suffix ist Dateiname,
  `/`, `%` und Steuerzeichen werden `%XX`-kodiert.
- `_info.json` steht **vorne**: Ein Online-Restore kennt damit Art und
  Hauptdatei, bevor die Konfiguration kommt. Inhalt: `kind` (`slapd.d`/
  `slapd.conf`), `config_path`, `server_version` (`slapd -VV`, L-13), je
  Datenbank Nummer, Backend, Suffix, Verzeichnis, Overlays, gesichert ja/nein
  mit Grund (L-6: `ldap`, `meta`, `monitor`, … „no local storage“), TLS-Dateien,
  `contains_secrets` (nur Namen), `consistency: "per-database"`.
- Modus aller Dateien 0600, Verzeichnisse 0700 (S-13, S-7.4).
- **Kein Eintragsbaum** (E-6, L-19): kein `@entry.ldif` je Eintrag, keine
  DN-Pfade im Katalog. Einzelne Einträge holt der Administrator aus der
  Datenbank-LDIF des Restores in Dateien (`ldapadd`).
- Konsistenz (L-3, gemessen): `slapcat` liest jede mdb-Datenbank in **einer**
  LMDB-Lesetransaktion – ein Schnappschuss je Datenbank, auch unter Schreiblast
  (20 000 Einträge Umbau, |p|+|q| immer N oder N+1). Zwischen Datenbanken und
  zwischen cn=config und Daten gibt es keinen gemeinsamen Zeitpunkt.
- Level: jede Sicherung ist vollständig (L-7); Incremental/Differential
  schreiben ebenfalls alles. `modifyTimestamp` taugt nicht als Änderungsmerkmal.

## Instanz erkennen (`ofd`, `ldap_instance::discover`, L-11)

1. In `/proc` jeden Prozess mit `comm == "slapd"` suchen. Genau einer: dieser.
   Mehrere: `config_path=` wählt; sonst Planfehler. Keiner: Planfehler
   (**Vorschlag, offen:** slapcat bräuchte keinen laufenden Server, L-11.5; ohne
   Prozess fehlt aber die gegengeprüfte Benutzer-ID, daher abgelehnt).
2. `/proc/<pid>/status`: reale = effektive UID und GID; **UID 0 wird abgelehnt**
   (slapd ohne `-u`).
3. `/proc/<pid>/cmdline`: `-F <dir>` → slapd.d, `-f <datei>` → slapd.conf.
   Ohne beides gilt die eingebaute Vorgabe wie bei der Debian-Unit:
   `/etc/ldap/slapd.d`, falls vorhanden, sonst `/etc/ldap/slapd.conf`
   (`SLAPD_CONF` aus `/etc/default/slapd` setzt die Unit nur als `-F`/`-f`).
   Relativer Pfad oder Symlink: abgelehnt.
4. Eigentümer: slapd.d gehört der Prozess-UID; slapd.conf gehört root oder der
   Prozess-UID und ist für Andere nicht schreibbar. Jedes `olcDbDirectory`/
   `directory` einer gesicherten Datenbank gehört der Prozess-UID (Prüfung im
   Host gegen die Antwort des Brokers, nicht im Broker).
5. Die Werkzeuge `/usr/sbin/slapcat`, `slapadd`, `slapd` müssen root gehören und
   dürfen für Gruppe/Andere nicht schreibbar sein (`check_trusted`).

## Broker `onesimus-ldap-broker`

Rust, nativ, im Plugin-Paket unter `openldap/onesimus-ldap-broker`, Hash im
signierten Manifest, Start per `fexecve` als Instanzbenutzer mit leerer
Umgebung (Vorbild PostgreSQL B). Argumente nur vom Host:

```text
onesimus-ldap-broker --sbin /usr/sbin (--config-dir <slapd.d> | --config-file <slapd.conf>) [--target <neues Ziel>]
```

Vertrag `proto/openldap_broker.proto` (Paket `onesimus.filed.broker.openldap.v1`):

| Operation | Broker führt aus | Prüfung im Host (`LdapPort`) |
| --- | --- | --- |
| `info` | `slapd -VV`; slapd.d: `slapcat -F <dir> -n 0` parsen; slapd.conf: Datei samt `include` (rekursiv) parsen. Ergebnis: Art, Version, Datenbanken (Nummer, Backend, Suffix, Verzeichnis, Overlays), Konfigurationsdateien **mit fester ID**, TLS-Dateien, Geheimnisarten | Auswahl aus `databases=`/`exclude=`/`config=` festlegen; nur `mdb`; Eigentümer jedes Datenbankverzeichnisses; Antwort an den Gast mit `selected` je Datenbank |
| `dump-open {config}` | `slapcat -F <dir> -n 0 -o ldif_wrap=no` (nur slapd.d; bei slapd.conf gibt es kein `-n 0`, L-10) | `config=yes` und Art slapd.d |
| `dump-open {suffix}` | `slapcat -F <dir>\|-f <datei> -b <suffix> -o ldif_wrap=no` | Suffix in der Auswahl |
| `config-file-open {id}` | liest die Datei mit dieser ID aus seiner eigenen `info`-Liste; nie ein Pfad vom Gast; nur unter festen Präfixen (Verzeichnis der Hauptdatei, `/etc/ldap`, `/etc/openldap`, `/usr/share/openldap`, `/etc/default/slapd`), nie eine TLS-Datei | ID aus der gefilterten Liste, `config=yes` |
| `dump-read`, `dump-close` | Strom lesen bzw. Prozess beenden | nur eigene Handles, höchstens ein Block |
| `load-open/-write/-close` (nur Restore, `openldap.write`) | `slapadd -F <ziel>/slapd.d -n 0` bzw. `slapadd -F\|-f <ziel…> -b <suffix> -q`, nur mit `--target`, Pfade nur unterhalb des Ziels | siehe Online-Restore |

- Feste Argumentlisten, `env_clear`, stdin `/dev/null` (slapcat) bzw. Pipe
  (slapadd). Ein Broker mit `--target` sichert nicht (keine dump-Operationen),
  einer ohne `--target` schreibt nicht.
- **stderr-Filter (S-7.2):** Diagnosen von slapcat/slapadd werden nie wörtlich
  weitergegeben. Der Broker meldet Exit-Status, die LDIF-Zeilennummer
  (`line=N`) und bekannte Fehlerklassen als eigene feste Texte. Eine
  slapadd-Meldung, die eine LDIF-Zeile zitiert, erreicht so nie das Joblog.
- Abbruch: Prozessgruppe (SIGTERM, dann SIGKILL), wie beim Vorbild.

## Gast ↔ Host: WIT und Capabilities

```wit
interface openldap {          // openldap.read
    info, dump-open, dump-read, dump-close, config-file-open
}
interface openldap-restore {  // openldap.write
    load-open, load-write, load-close
}
world openldap-plugin { include plugin; import openldap; import openldap-restore; }
```

| Capability | Wofür | Verknüpft |
| --- | --- | --- |
| `file.read` | Sicherung (NextFile/ReadFile) | Backup-Job |
| `file.write` | Online-Restore (WriteFile/CloseFile) | Restore-Job |
| `openldap.read` | Import `openldap` | wenn Manifest und Policy sie gewähren und ein openldap-Port läuft |
| `openldap.write` | Import `openldap-restore` mit echten Funktionen | **nur** im Restore-Job mit `restore_mode=online`; im Backup-Job antworten diese Funktionen nur mit Fehler (der Gast importiert beide Schnittstellen, eine Instanziierung braucht alle Importe) |

`signer.cnf` (Grants des G1-Signierzertifikats) bekommt:

```ini
[onesimus_plugin_ids]
p3 = UTF8String:io.onesimus.openldap
[onesimus_capabilities]
c4 = UTF8String:openldap.read
c5 = UTF8String:openldap.write
```

Grants sind ein Kreuzprodukt (S-11, E-5): ein signiertes PostgreSQL-Bundle
könnte `openldap.write` anfordern. **Vorschlag, offen:** vorerst akzeptiert,
Grants Version 2 (Capabilities je Plugin-ID) als offener Punkt. Ein Port
bedient nur seine eigene Schnittstelle: ein Plugin mit `openldap.read` bekommt
nie den Import `postgresql` und umgekehrt (Test).

## Sicherung

1. `Session::open` → `prepare_broker`: verifiziertes Manifest mit
   `openldap.read` → Zeile ohne Geheimnis (E-3) → `discover` → Werkzeuge
   prüfen → Broker als Instanzbenutzer starten → `LdapPort::new` holt `info`,
   prüft Eigentümer und legt die Auswahl fest.
2. TLS-Dateien aus `info`, die kein `File =` des FileSets abdeckt, ergeben je
   eine **Warnung** im Joblog (E-9); der Job läuft weiter.
3. Gast: `_info.json`, dann Konfiguration (cn=config bzw. Dateien), dann je
   ausgewählter Datenbank `data/<suffix>.ldif` als Strom ohne Zwischendatei.
4. Ein fehlgeschlagener slapcat (Exit ≠ 0) beendet den Job mit Fehler; die
   Diagnose ist gefiltert.

## Wiederherstellung

### In Dateien (Standard, `restore_mode=files`)

Ohne Plugin-Code (Regel Phase 1): `Where=/pfad` schreibt den Baum mit Modus
0600. Danach von Hand, geprüft im Container (L-9):

```sh
systemctl stop slapd                         # und warten, bis kein slapd mehr läuft
mv /etc/ldap/slapd.d /etc/ldap/slapd.d.alt; mv /var/lib/ldap /var/lib/ldap.alt
install -d -o openldap -g openldap -m 700 /etc/ldap/slapd.d /var/lib/ldap   # und jedes olcDbDirectory
R=/pfad/@openldap/main
runuser -u openldap -- slapadd -F /etc/ldap/slapd.d -n 0 -l $R/config/cn=config.ldif
runuser -u openldap -- slapadd -F /etc/ldap/slapd.d -b dc=example,dc=org -q -l $R/data/dc=example,dc=org.ldif
systemctl start slapd
```

slapd.conf: `config/files/…` an ihren Ort, TLS-Schlüssel aus dem `File =`-Teil,
dann `slapadd -f /etc/ldap/slapd.conf -b <suffix> -q -l …`. Vergleich mit
attributweise sortiertem LDIF (die Reihenfolge der Attribute in cn=config
verschiebt sich, L-9).

### Online, nur in ein neues Ziel (`restore_mode=online`)

1. Ziel: `target=` aus den Restore-Optionen, Vorgabe
   `/var/lib/onesimus-ldap-restore/<jobid>`. Der Host lehnt ab: relativer Pfad,
   `.`/`..`, Symlink, vorhandenes nicht leeres Verzeichnis, ein Elternpfad, der
   nicht root gehört oder für Andere schreibbar ist, und jede Überschneidung
   (gleich, darüber, darunter) mit dem Konfigurationspfad oder einem
   Datenbankverzeichnis der laufenden Instanz.
2. Der Host legt das Ziel an (root, 0755) und darin `slapd.d/` bzw. `conf/`,
   `db-<n>/`, `run/` (Instanzbenutzer, 0700).
3. **slapd.d:** Der Gast reicht `cn=config.ldif` über `load-*` an den Host; der
   Host puffert (höchstens 16 MiB), schreibt jedes `olcDbDirectory` auf
   `<ziel>/db-<n>` und `olcPidFile`/`olcArgsFile` auf `<ziel>/run/` um, legt die
   Verzeichnisse an und lässt den Broker `slapadd -n 0` ausführen (L-9:
   Verzeichnisse vor `-n 0`, sonst Teilzustand).
   **slapd.conf:** jede Datei geht an den Host, der sie unter
   `<ziel>/conf/<originalpfad>` ablegt und `include`, `directory` (fehlend:
   ergänzt), `pidfile`, `argsfile` umschreibt.
4. Danach je Datenbank-LDIF `slapadd -b <suffix> -q` gegen die Zielkonfiguration;
   der Suffix muss eine `mdb`-Datenbank der **zurückgespielten** Konfiguration
   sein, deren Verzeichnis im Ziel liegt. Daten ohne Konfiguration im selben
   Restore: abgelehnt (**Vorschlag, offen**).
5. Fehler: der Host löscht das Ziel. Erfolg: das Ziel bleibt; das Joblog nennt
   den Prüfbefehl `slapd -F <ziel>/slapd.d -h ldapi://<ziel>/run/ldapi -u openldap -g openldap`.
   Der Wechsel auf die neue Instanz (Dienst stoppen, Verzeichnisse tauschen)
   ist ein Schritt des Administrators; `ofd` führt ihn nie aus.
6. Am Produktivsystem ändert sich nichts, ein Rollback ist dort nicht nötig.

## Sicherheit und Betrieb

- Das Volume enthält Passwort-Hashes, Klartext-Proxy-Passwörter
  (`credentials=`, `acl-passwd`) und `olcRootPW`. Pool-Verschlüsselung wird
  empfohlen; `_info.json.contains_secrets` nennt die Arten.
- Joblog/Katalog: nur Pfade (keine DNs, da kein Eintragsbaum), gefilterte
  Diagnosen, die maskierte Plugin-Zeile. Lecktest siehe unten.
- `FileEntry.attributes` (E-12): nur Anzeigewerte (Suffix, Backend, Art); sie
  werden heute nicht gesichert (S-12) und enthalten nie Inhalte.
  **Vorschlag, offen:** Anzeige aus dem Pfad, kein Katalogfeld.
- Restore in Dateien als root: Dateien 0600, Eigentümer root (S-7.4).
- Broker: keine Zusatzgruppen (`setgroups(0)`, S-6); ein TLS-Schlüssel
  `root:ssl-cert 0640` ist für ihn nicht lesbar – deshalb E-9.

## Paket `ofd-plugin-openldap`

`packaging/ofd-plugin-openldap/build-deb.sh <openldap.pb> <onesimus-ldap-broker>`:
`/usr/lib/<triplet>/onesimus/plugins/openldap.pb` und
`…/plugins/openldap/onesimus-ldap-broker`, alles root, nicht schreibbar für
Gruppe/Andere. `Depends: ofd (= Version)`, `Recommends: slapd`,
`Enhances: slapd`. postinst (jede Installation, jedes Upgrade): markierte Zeile
`Plugin Names = "openldap"   # ofd-plugin-openldap` in die Client-Ressource,
`ofd -t` prüft, sonst Rückbau; Neustart von `ofd` nur ohne offene Verbindung
auf dem FD-Port. Kein Eingriff in slapd (keine ACL, keine Gruppe). postrm
`remove`/`purge`: entfernt die markierte Zeile.

## Tests

Unit (in `cargo test`, ohne slapd):
- Host: openldap-Port verknüpft nie `postgresql` und umgekehrt; `openldap.write`
  nur im Restore; Plugin ohne Capability instanziiert nicht.
- `LdapPort` mit gefälschtem Transport: fremder Suffix, Proxy-Datenbank,
  Config bei `config=no`, unbekannte Datei-ID, fremder Handle, Block-Grenze,
  `load-*` im Backup, Ziel unter/über/gleich Produktivpfad, Symlink, nicht leer,
  Umschreiben für slapd.d und slapd.conf (inkl. fehlendem `directory`).
- `discover` gegen ein gefälschtes `/proc`: fremder Prozess, slapd als root,
  Eigentümer ≠ UID, relatives `-F`, Vorgabe slapd.d vor slapd.conf, mehrere slapd.
- Broker: Parser für `slapcat -n 0` und slapd.conf (Fortsetzungszeilen,
  Anführungszeichen, rekursive includes), stderr-Filter gibt kein Geheimnis weiter.
- Geheimnisse: `openldap:…:password=x` abgelehnt.

Integration und Ende-zu-Ende in Podman (debian:trixie, slapd 2.6, `--init`;
Aufbau aus dem Mechanik-Gutachten) für **beide** Konfigurationsarten:
Backup unter Schreiblast konsistent; Restore in Dateien ohne Plugin-Code;
Online-Restore in ein neues Ziel (Daten, Overlays, ACLs, Hashes gleich,
Anmeldung mit altem Passwort an einem zweiten slapd auf dem Ziel); slapd.conf
mit include-Dateien; Lecktest (kein Geheimnis in Joblog, Attributen/Katalog
oder Fehlermeldungen); slapd als root abgelehnt; Produktivverzeichnis nie
überschrieben (`data.mdb` vor/nach gleich). Podman-Isolation: eigenes
`--root`, `--runroot`, `--tmpdir` und `XDG_RUNTIME_DIR` (0700), nie
`system reset`, Präfix `ldaptest-`.

## Offene Punkte

| # | Punkt | Vorschlag |
| --- | --- | --- |
| O-1 | Sicherung/Online-Restore ohne laufenden slapd | abgelehnt, die Benutzer-ID kommt nur vom Prozess (Vorschlag, offen) |
| O-2 | Grants v2 je Plugin-ID (E-5) | später |
| O-3 | include außerhalb der Präfixliste | Planfehler statt stilles Weglassen (Vorschlag, offen) |
| O-4 | Daten-Restore online ohne Konfiguration im selben Job | abgelehnt (Vorschlag, offen) |
| O-5 | Inkrementale (Hash je Eintrag statt Zeitstempel, L-7) | später |
| O-6 | `physical` über `mdb_copy` (L-18) | später |
| O-7 | Ende-zu-Ende mit `odir` + `bareos-sd` und Katalog | **erledigt 19.11.2026** mit Wegwerf-CA (Abschnitt „O-7: Ende zu Ende“); offen daraus: Audit-Einträge für Laden/Signatur/Broker (7.7), Lauf mit dem echten G1-Schlüssel |
| O-8 | Ein vorhandenes, leeres `target=` | nur, wenn es ofd (root) gehört und für Gruppe/Andere nicht schreibbar ist (Vorschlag, offen) |
| O-9 | slapd.d-Restore: Nebendateien (`/etc/default/slapd`) | landen unter `<ziel>/conf/…`, werden nicht angewendet (Vorschlag, offen) |

## Umsetzung (19.11.2026)

| Schritt | Commit | Stand |
| --- | --- | --- |
| Plan | `6b3c07c` | dieser Plan |
| S-10 Broker allgemein | `429dce4` | `BrokerFunction` je Broker, Tabelle Capability → Import → Funktionen (`BROKER_IMPORTS`, `import_plan`), Port nennt Familie und bediente Capabilities, `prepare_broker_for` verteilt nach Manifest und Jobart; PostgreSQL unverändert, Tests grün |
| Vertrag | `4b6032d` (proto `openldap` `1298e49`) | `proto/openldap_broker.proto`, WIT `openldap`, `openldap-restore`, Welt `openldap-plugin` |
| Broker | `7ac8939` | `crates/onesimus-ldap-broker`: info (slapd -VV, `slapcat -n 0` bzw. slapd.conf-Parser), dumps, Dateien nach ID, slapadd nur mit `--target`, stderr-Filter |
| Host | `03fc867` | `ldap_instance.rs`, `ldap_port.rs`, TLS-Warnungen, Ablehnung geheimer Optionen, Ziel anlegen/prüfen/umschreiben/löschen, Restore-Hinweise im Joblog |
| Gast-Laufzeit | `13c03d2` | Größenklassen statt Bump-Allokator (ein Strom wuchs je Block um zwei Puffer bis zur 32-MiB-Grenze); PostgreSQL/example-tree neu gebaut, grün |
| Gast | `2a50e31` | `plugins/openldap` (no_std) |
| Tests | `30e09b1` | `tests/openldap/` (Containerfile, Setup, `run-in-podman.sh`), `crates/ofd/src/ldap_e2e.rs` |
| Paket | `e08ff23` | `packaging/ofd-plugin-openldap` |

Container-Lauf (Podman 5.4.2 rootless, isoliert, debian:trixie, slapd
2.6.10), je `ldaptest-olc` und `ldaptest-conf` bestanden: Broker als
`openldap`; Daten-LDIF byte-gleich mit `slapcat -b`, cn=config byte-gleich mit
`slapcat -n 0` bzw. Dateisatz byte-gleich; unter Schreiblast p+q = 20 000
(Umbau lief während des Dumps); Restore in Dateien ohne Plugin-Code (0600);
Online-Restore in `/var/lib/onesimus-ldap-restore/<jobid>`, zweiter slapd auf
dem Ziel, Anmeldung `anna`/`annaPass1`, `memberOf`, Daten und cn=config
(ohne umgeschriebene Pfade) gleich; Ziel = Produktivverzeichnis abgelehnt,
`data.mdb` vorher/nachher gleich; slapadd-Fehler ohne Geheimnis/DN; Lecktest
(rootpw-Hash, userPassword, `credentials=`) nur in Dateidaten, nie in
Attribut-/Plugin-Namen-Strömen, Joblog oder Restore-Meldungen; slapd als root
abgelehnt.

Am echten System zu prüfen: Signieren mit dem G1-Schlüssel (TPM) mit den
Grants oben, BgLage (dort ofd auf dem LDAP-Host, Passwort im alten Job
ändern, S-3). Paket, `ofd --check-plugin` und der Lauf gegen `odir` +
`bareos-sd` mit Katalog sind mit einer Wegwerf-CA gemessen (nächster Abschnitt).

### O-7: Ende zu Ende mit odir, bareos-sd und Katalog (19.11.2026)

Test: `crates/onesimus-rig/tests/ldap_o7.rs` (ignoriert), Podman-Treiber
`tests/openldap/o7-podman.sh` (isolierter Speicher unter
`/tmp/ldaptest-o7-*`, eigenes `--root`/`--runroot`/`--tmpdir`/
`XDG_RUNTIME_DIR`, Dateisperren, cgroupfs, nie `system reset`). Start:
`cargo test -p onesimus-rig --test ldap_o7 -- --ignored --nocapture`
(Dauer 85 s ohne Image-Bau; das WASM-Plugin muss gebaut sein).

Aufbau: Wegwerf-Plugin-CA im Speicher (Ed25519-Wurzel und Generation
„O7 throwaway“, RSA-4096-Signierer mit den Grants `io.onesimus.openldap`,
`file.read`, `file.write`, `openldap.read`, `openldap.write`); nur die
Zertifikate liegen auf der Platte, kein privater Schlüssel. `ofd` wird damit
in `target/ldap-o7` gebaut (nie `target/debug`/`target/release`) und mit
`cargo deb --profile dev` verpackt, das Plugin-Paket mit
`packaging/ofd-plugin-openldap/build-deb.sh`. Das Testgerüst startet odir,
bareos-sd 25 und den Katalog auf dem Host (neu: `RigOptions::start_fd`,
`extra_director`, `auditing`, `Cluster::dump`); je Konfigurationsart ein
Container `ldaptest-o7-olc` / `ldaptest-o7-conf` mit `--network host`,
ofd auf `127.0.0.2:39461/39462`, slapd nur auf ldapi
(`LDAPTEST_SLAPD_SERVICES`, damit kein Port 389 im Host-Namensraum).

| Punkt | Ergebnis | Messung |
| --- | --- | --- |
| 7.1 Pakete | bestanden | `dpkg -i` ofd, dann ofd-plugin-openldap: postinst „added Plugin Names = "openldap" to …/client/myself.conf“, Zeile `Plugin Names = "openldap"   # ofd-plugin-openldap`, `ofd -t` gut; Bundle, Broker und jedes Verzeichnis bis `/` sowie Policy uid 0 und nicht gruppen-/weltschreibbar (644/755/555); `ofd --check-plugin`: „Plugin io.onesimus.openldap 1.0.0 (suite 0.2.0, Onesimus Plugin Signing O7 throwaway): X.509, signature, manifest and 1 native part(s) for x86_64-linux-gnu OK; not initialized (needs a broker)“ |
| 7.2 Daemon | bestanden | ofd mit `ExecStart` der Unit (+`-d 100`) im Container; `status client=ldap-olc-fd`: TLS 1.3, „ldap-olc-fd Version: 25.1.2+onesimus.0.2.0“ |
| 7.3 Backup | bestanden | über die Konsole `run job=…`: olc JobId 1 `T`, conf JobId 4 `T`, JobErrors 0 |
| 7.4 Katalog | bestanden | FileSetText enthält `Plugin = "openldap:instance=main"`; `list files`: olc 8 Einträge unter `/@openldap/main/` (`_info.json`, `config/cn=config.ldif`, `config/files/etc/default/slapd`, beide `data/*.ldif`, 3 Verzeichnisse), conf 13 (+7 aus `File = /etc/ldap/tls`); JobFiles 8 bzw. 20 = Einträge; JobBytes 143 271 bzw. 156 066 ≥ LDIF beider Datenbanken (97 580 bzw. 97 480) |
| 7.5 Lecktest | bestanden | rootpw-Hash (roh und Base64), `annaPass1` (roh und Base64), Proxy-Passwort (`admin123` bzw. `MetaSecret-4711`), `credentials=`: in keinem Path-/File-Namen, nicht in der Tabelle Log, nicht im ganzen `pg_dump --data-only` des Katalogs, nicht in job.log, audit.log, odir-Log und ofd-Log (`-d 100`, 1,8 MB). Gegenprobe: Proxy-Passwort und `credentials=` stehen auf dem Volume |
| 7.6a Restore in Dateien | bestanden | `restore … where=/root/o7-files all done yes` → `T`; beide Daten-LDIFs per `cmp` byte-gleich mit `slapcat -b`, cn=config mit `slapcat -n 0` (olc), slapd.conf/Schema/`/etc/default/slapd` mit den Originalen (conf); Modus 600, root |
| 7.6b Online-Restore | bestanden | `restore … pluginoptions=openldap:restore_mode=online all done yes` → `T`, Ziel `/var/lib/onesimus-ldap-restore/<JobId>`; zweiter slapd auf dem Ziel, `ldapwhoami` anna/`annaPass1` gelingt; beide Datenbanken (sortiert) gleich; `data.mdb` sha256 vorher = nachher |
| 7.7 Audit/Joblog | **nicht bestanden (Lücke)** | Das Backup-Joblog enthält **keine** Zeile zu Plugin-Laden, Signaturprüfung, Broker-Start oder -Ende; ebenso nicht das ofd-Log mit `-d 100`. Vorhanden nur beim Online-Restore: „restoring online through io.onesimus.openldap“ und „OpenLDAP restored online into …“. Das odir-Audit-Log (`Auditing = yes`) nennt nur Konsolenbefehle (`run job=…`, `restore … pluginoptions=…`). Fehlschläge erscheinen dagegen als Fatal error im Joblog (7.8) |
| 7.8 Negativ | bestanden | jeweils Job `f`, Fatal error im Joblog: manipulierter Broker → „…/onesimus-ldap-broker: SHA-256 does not match the signed manifest“; Signaturbyte gekippt → „cannot load: invalid plugin signature“; Bundle einer fremden CA → „cannot load: plugin key or identity is untrusted or revoked“; Zeile aus Plugin Names entfernt + Neustart → „plugin "openldap" is not in Plugin Names“. Gegenprobe: postinst erneut, Neustart, Backup `T` |

Befunde:

- **B-1 (7.7, Lücke):** ofd protokolliert ein erfolgreiches Laden nicht:
  weder Plugin-ID/Version/Signierer, noch Ergebnis der Signaturprüfung, noch
  Broker-Start/-Ende (Benutzer-ID, Exit-Status). Für ein signiertes Plugin,
  das als Instanzbenutzer Programme startet, fehlt damit die Spur im Joblog.
  **Vorschlag, offen:** je Job eine M_INFO-Zeile „Plugin io.onesimus.openldap
  1.0.0 verified (signer …, suite …), broker started as uid N“ und beim Ende
  der Exit-Status; ggf. zusätzlich M_AUDIT.
- **B-2 (Paket):** `ofd-plugin-openldap` hängt von `iproute2` ab (`ss` im
  postinst); `dpkg -i` ohne apt bricht ohne iproute2 ab (gemessen im ersten
  Lauf). Im Test-Image jetzt installiert; mit apt kein Problem.
- **B-3 (Paket, aus dem Code, nicht gemessen):** Der postinst trägt nur
  `Plugin Names` ein, keine `Plugin Policy`; ohne sie lädt ofd kein Plugin
  („no Plugin Policy and Plugin Directory configured“, `plugin.rs`
  `resolve_checked`). Der Test legt `/etc/onesimus/plugin-policy.json` (`{}`)
  und die Zeile von Hand an. **Vorschlag, offen:** Paket `ofd` liefert eine
  leere Policy und die Zeile in der Vorlage.
- **B-4 (Beobachtung):** Die Attribute der Plugin-Dateien tragen Größe 0 und
  mtime 1970-01-01 (Restore-Liste des FD); der Inhalt ist richtig. Folgen für
  Anzeige und Accurate nicht geprüft.
