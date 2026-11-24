# Lizenzen je Crate

Stand 21.11.2026. Umsetzung der Entscheidung des Maintainers vom 21.11.2026
(duale Lizenz für eigene Teile, Hintergrund in
[plans/FREE-TIER-LIZENZ.md](plans/FREE-TIER-LIZENZ.md), Abschnitte
„Rechtlicher Vorbehalt“ und „Entscheidung des Nutzers“).

> **Keine Rechtsberatung.** Diese Einteilung ist eine technische
> Bestandsaufnahme aus dem Quelltext und der Git-Historie. Sie muss vor einer
> kommerziellen Lizenzvergabe von einem Anwalt für Open-Source-Lizenzen
> geprüft werden (offener Punkt aus dem Plan). Insbesondere offen: ob odir als
> enger, schnittstellenkompatibler Nachbau insgesamt ein abgeleitetes Werk von
> Bareos ist, und ob Code, der nur Bareos-Formate und -Protokolle *benutzt*,
> davon erfasst wird (siehe „Offene Rechtsfragen“).

## Zwei Lizenzen

| Angabe (SPDX) | Gilt für | Bedeutung |
| --- | --- | --- |
| `AGPL-3.0-only` | von Bareos abgeleitete Teile (Tabelle unten) | GNU AGPL v3, wie Bareos; nach Entscheidung des Besitzers vom 23.11.2026 tragen die Crates aber die Voreinstellung des Workspaces (`AGPL-3.0-or-later`), siehe „Offene Rechtsfragen“. Die Rechte der Bareos-Urheber (Bareos GmbH & Co. KG, Planets Communications B.V., Free Software Foundation Europe e.V. u. a.) bleiben bestehen; eine andere Lizenz ist nicht möglich. |
| `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | eigene Teile | Wahlweise AGPL v3 oder später oder eine kommerzielle Lizenz des Maintainers ([LICENSE-COMMERCIAL.md](../LICENSE-COMMERCIAL.md), Text folgt). |

Der AGPL-Text liegt unverändert in [LICENSE](../LICENSE). Die Angabe steht je
Crate im Feld `license` der `Cargo.toml`; die Voreinstellung des Workspaces
(`[workspace.package] license`) ist `AGPL-3.0-or-later`, eigene Crates
überschreiben sie.

## Was „eigen“ heißt

Ein Teil gilt als eigen, wenn alle drei Punkte zutreffen:

1. **Allein vom Maintainer geschrieben.** `git shortlog -sne` zeigt für das
   Repository und den Submodul `proto` nur Jörg Bernau (drei Adressen), keine
   Beiträge Dritter.
2. **Nichts aus Bareos übernommen:** keine portierte Logik aus den
   C++-Quellen (`core/src/…`), keine kopierten oder übersetzten Texte
   (Meldungen, Hilfetexte, Tabellen), keine aus Bareos-Dateien erzeugten Daten
   (DDL, Befehlstabellen).
3. **Neue Onesimus-Funktion** ohne Bareos-Gegenstück: im Quelltext meist
   ausdrücklich so markiert (`Bareos-Gegenstück: keins (Onesimus-Erweiterung)`,
   `Onesimus extension, not in Bareos`).

Code, der eine Bareos-Schnittstelle nur *benutzt* (z. B. BNET-Rahmen über
gRPC transportiert oder Daten im Bareos-Strom ablegt), zählt hier als eigen,
wenn er sonst die Punkte erfüllt. Ob das rechtlich trägt, ist Teil der
Anwaltsprüfung.

Ein Crate ist **gemischt**, wenn es überwiegend Bareos-Port ist, aber eigene
Module enthält. Gemischte Crates bleiben als Ganzes `AGPL-3.0-only`; ihre
eigenen Module sind unten genannt und unter „Offene Trennungen“ als Kandidaten
für ein eigenes Crate geführt.

## Bestandsaufnahme

Belege: Zahl der Zeilen mit Verweisen auf Bareos-Quelldateien (`*.cc`/`*.h`
in `crates/<crate>`, Stand `HEAD`), die Kennzeichnung `Bareos-Gegenstück` in
den Modulköpfen und die Git-Historie.

| Crate | Einordnung | Lizenz | Beleg |
| --- | --- | --- | --- |
| `bareos-bnet` | abgeleitet | `AGPL-3.0-only` | BNET-Protokoll, CRAM-MD5, TLS-PSK nach `lib/bsock.cc`, `lib/tls_openssl.cc`, `lib/cram_md5.cc`; byte-kompatibel zu Bareos. |
| `bareos-format` | abgeleitet | `AGPL-3.0-only` | LStat, Bareos-Base64, `STREAM_*`, `FT_*`, `M_*`, Datensatzköpfe; 61 Verweise (`stored/block.cc`, `stored/read.cc`, `filed/backup.cc`, `lib/util.cc`). |
| `bareos-config` | abgeleitet | `AGPL-3.0-only` | Parser der Bareos-Konfigurationssyntax mit den Ressourcentabellen; 29 Verweise (`dird_conf.cc`, `stored_conf.cc`, `run_conf.cc`, `lib/lex.cc`). |
| `bareos-catalog` | abgeleitet | `AGPL-3.0-only` | SQL-Abfragen des Katalogs, laut Modulkopf „mirror the queries in the C++ cats/dml/ directory“. |
| `bareos-orm` | gemischt | `AGPL-3.0-only` | Schema-Datei `schema/v2250_bareos_catalog.json` „Converted from bareos/core/src/cats/ddl/creates/postgresql.sql“ (abgeleitet). Eigen: die ORM-Maschine (`schema.rs`, `migration.rs`, `heal.rs`, `codegen.rs`, `error.rs`) und die Onesimus-Schritte `v2251`–`v2254` (Schema `onesimus`). |
| `odir` | gemischt, überwiegend abgeleitet | `AGPL-3.0-only` | Nachbau von `bareos-dir`; 981 Verweise auf Bareos-Quellen (`ua_cmds.cc`, `ua_restore.cc`, `dird/job.cc`, `catreq.cc` …), `ua_commands_table.rs` aus `ua_cmds.cc` erzeugt, Meldungstexte aus `dird/*.cc`. Eigene Module siehe unten. |
| `oconsole` | gemischt, überwiegend abgeleitet | `AGPL-3.0-only` | Ersatz für `bconsole`: Anmeldung und Ablauf nach `console_conf.cc`, `authenticate_console.cc`, `lib/bsock.cc` (`main.rs`). Eigene Module siehe unten. |
| `ofd` | gemischt, überwiegend abgeleitet | `AGPL-3.0-only` | Nachbau von `bareos-fd`; 56 Verweise (`filed/backup.cc`, `filed/restore.cc`, `findlib/*.cc`, `lib/attr.cc` …). Eigene Module siehe unten. |
| `osd` | abgeleitet | `AGPL-3.0-only` | Director-Seite des SD-Protokolls nach `dird/authenticate.cc`, `dird/msgchan.cc`, `dird/catreq.cc`, `stored/askdir.cc`. |
| `osd-daemon` (`crates/osd/daemon`) | gemischt, überwiegend abgeleitet | `AGPL-3.0-only` | Nachbau von `bareos-sd` (Sitzungen, Device, Leser/Schreiber; 45 Verweise auf `stored/*.cc`). Eigen: `grpc.rs`, `ringproto.rs` (beide „Bareos-Gegenstück: keins“). |
| `osd-ring` (`crates/osd/ring`) | eigen | `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | Blockring im gemeinsamen Speicher; Modulkopf „Bareos-Gegenstück: keins. bareos-sd schreibt aus dem Job-Thread direkt auf das Device“. Keine Abhängigkeit von anderen Workspace-Crates. |
| `daemon-crate` | abgeleitet | `AGPL-3.0-only` | Daemon-Rahmen nach `lib/daemon.cc`, `include/exit_codes.h`, `filed/filed.cc`/`stored/stored.cc` `main`, `filed/authenticate.cc`, `lib/message.cc`. Das einzige eigene Modul (gRPC-TLS) ist seit 21.11.2026 in `onesimus-bnet-grpc::tls`. |
| `onesimus-bnet-grpc` | eigen | `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | BNET-Rahmen über gRPC mit TLS, Modulkopf „Bareos-Gegenstück: keins (Onesimus-Erweiterung)“; transportiert Bareos-Rahmen, portiert keine Bareos-Logik. Dazu `tls` (aus `daemon-crate`). |
| `onesimus-proto-rs` | gemischt | `AGPL-3.0-only` | Aus allen Protos des Submoduls erzeugt, darunter `director_commands.proto` („Auto-generated from ua_cmds.cc by scrape_commands.py“). |
| `onesimus-plugins` | eigen | `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | Signierte WebAssembly-Plugins, Ed25519/X.509-Vertrauenskette (`docs/plans/SIGNIER-INFRASTRUKTUR.md`); ersetzt die Bareos-Plugins, ohne deren Code (keine Verweise auf Bareos-Quellen). |
| `onesimus-pg-broker` | eigen | `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | Broker des PostgreSQL-Plugins (`pg_dump`/`psql`); keine Bareos-Verweise. |
| `onesimus-ldap-broker` | eigen | `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | Broker des OpenLDAP-Plugins (`slapcat`/`slapadd`); keine Bareos-Verweise. |
| `onesimus-podman-core` | eigen | `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | Podman-Plugin: Richtlinien, ID-Maps, Journal; nennt den Bareos-Strom nur als Ziel. Keine Abhängigkeit von anderen Workspace-Crates. |
| `onesimus-podman-broker` | eigen | `AGPL-3.0-or-later OR LicenseRef-Onesimus-Commercial` | Broker des Podman-Plugins; keine Bareos-Verweise. |
| `onesimus-rig` | gemischt (Testgerüst) | `AGPL-3.0-only` | Nicht ausgeliefert. Eigenes Gerüst, aber `console.rs` bildet den `bconsole`-Ablauf nach (`console/console.cc`, `lib/cram_md5.cc`), `catalog.rs` lädt das Bareos-Schema `postgresql.sql`. Vorsichtshalber AGPL; kann nach Prüfung dual werden. |
| Submodul `proto` | gemischt, je Datei | `usage`, `podman_broker`, `postgresql_broker`, `openldap_broker`, `onesimus_options`: `AGPL-3.0-or-later` (Entscheidung des Maintainers 24.11.2026, nur Zeile Jörg Bernau); `plugin_ccc`, `filed_plugin`, `director_api`, `director_service`, `filed_service`, `stored_service`, `director_commands`: `AGPL-3.0-only` mit den Zeilen der Bareos-Rechteinhaber | Bareos-nahe Definitionen bleiben `-only`; `director_commands.proto` aus `ua_cmds.cc` erzeugt. Lizenzänderungen gehören in das Repository `onesimus-proto`. |

Außerhalb des Workspaces: die Gast-Plugins unter `plugins/`
(`onesimus-guest-runtime`, `onesimus-plugin-postgresql`,
`onesimus-plugin-openldap`, `onesimus-plugin-example-tree`, eigene
Cargo-Workspaces) sind nach denselben Kriterien eigen, tragen aber noch
`license = "AGPL-3.0-only"`. Nicht geändert, weil außerhalb dieses Auftrags;
Vorschlag: ebenfalls dual.

### Eigene Module in gemischten Crates

| Crate | Eigene Module (Onesimus-Erweiterungen) |
| --- | --- |
| `odir` | `grpc/` (gRPC-Dienst, Anmeldung, Push-Ereignisse, `DirectorApi`, TLS), `ovfs/` und `commands/ovfs.rs` (Wiederherstellungsbäume, Port der eigenen PostgreSQL-Erweiterung `ovfs`), `commands/acl.rs` und `catalog_acl.rs` (zweistufige ACL), `commands/purge_bulk.rs`, `commands/config_crud.rs` und `config_edit.rs` (Konfigurations-CRUD), `commands/grammar.rs`, `command_grammar.rs`, `grammar_docs.rs`, `manpages.rs` (Konsolengrammatik und Handbuchseiten; `grammar_table.rs` enthält Bareos-Syntax), `mdns.rs`, `fd_grpc.rs`, `sd_grpc.rs`, `fd_progress.rs`, `secrets.rs`, `logging.rs`, `kill.rs` (nicht dazu: `watchdog.rs`, Gegenstück `lib/watchdog.cc`) |
| `oconsole` | `mcp.rs` mit `mcp_*.rs`, `mcp_read/`, `mcp_write/`, `mcp_config/` (MCP-Server), `grpc_client.rs`, `mcp_token.rs`, `rate_guard.rs`, `grammar.rs` (Vervollständigung), `fishkeys.rs`, `color.rs` |
| `ofd` | `plugin.rs`, `plugin/`, `broker.rs`, `pg_instance.rs`, `pg_port.rs`, `ldap_instance.rs`, `ldap_port.rs`, `ldap_port/`, `ldap_e2e.rs`, `podman_port.rs`, `podman_port/`, `podman_rootful.rs`, `podman_scan.rs`, `podman_e2e.rs`, `grpc.rs`, `grpc_tls.rs` |
| `osd-daemon` | `grpc.rs`, `ringproto.rs` |
| `bareos-orm` | `schema.rs`, `migration.rs`, `heal.rs`, `codegen.rs`, `error.rs`, `schema/v2251`–`v2254` |

## Durchgeführte Trennung

- **gRPC-TLS** (21.11.2026): `daemon-crate/src/grpc_tls.rs` →
  `onesimus-bnet-grpc/src/tls.rs` (Aufrufer `ofd`, `osd-daemon` umgestellt,
  keine Verhaltensänderung). Danach ist `daemon-crate` rein abgeleitet und
  `onesimus-bnet-grpc` rein eigen.

## Offene Trennungen

Nicht umgesetzt, weil die Grenze nicht klar genug oder der Umbau zu groß für
einen risikoarmen Schritt ist. Vorschlag je Fall:

| Herkunft | Vorgeschlagenes Crate (dual) | Module | Warum noch nicht |
| --- | --- | --- | --- |
| `odir` | `onesimus-director-grpc` | `grpc/` | Hängt eng am Kern: `command_executor`, `acl`, `message_bus`, `config_edit`, `session_manager`, `command_queue` (je 5–13 Verwendungen). Braucht eine Schnittstelle (Trait) zwischen Dienst und Director-Kern. |
| `odir` | `onesimus-ovfs` | `ovfs/` (`tree.rs`, `lstat.rs` sind frei von `crate::`; `mod.rs`, `native.rs`, `fallback.rs`, `capability.rs` brauchen `catalog_worker::{CatalogConnection, SqlTable}`) | Katalogzugriff müsste hinter ein Trait; Testhelfer (`lstat::tests::encode`) werden crate-übergreifend genutzt; `lstat.rs` dekodiert das Bareos-Format `File.LStat` (Einordnung durch Anwalt). |
| `odir` | (im Kern belassen oder `onesimus-director-ext`) | `commands/acl.rs`, `catalog_acl.rs`, `purge_bulk.rs`, Konfigurations-CRUD, Grammatik/Handbuch, `mdns.rs` | Sind in die Befehlsverarbeitung des Bareos-Ports eingebettet (Registry, Dialoge, ACL-Prüfung von `PurgeCmd`). |
| `oconsole` | `onesimus-mcp` (Bibliothek) | `mcp*.rs`, `mcp_read/`, `mcp_write/`, `mcp_config/`, `mcp_token.rs`, `grpc_client.rs`, `rate_guard.rs` | Rund 7 400 Zeilen in einem Binär-Crate mit privaten Elementen; alles müsste `pub` werden und vollständige Rustdoc bekommen (`deny(missing_docs)`), dazu `CTRL_C` und der Verbindungsaufbau aus `main.rs`. |
| `ofd` | `ofd-plugin-host` | `plugin.rs`, `plugin/`, `broker.rs`, `pg_*`, `ldap_*`, `podman_*` | Nutzt die abgeleiteten FD-Module `restore`, `backup`, `jmsg`, `attribs`, `find`, `create_file`, `config`; braucht eine Schnittstelle zum FD-Kern. |
| `osd-daemon` | in `osd-ring` oder `osd-ringproto` | `ringproto.rs`, `grpc.rs` | `ringproto.rs` ist mit `writer`, `reader`, `device` und den Sitzungen verzahnt; `grpc.rs` ist klein, aber Teil des Serverstarts. |
| `bareos-orm` | `onesimus-orm` | `schema.rs`, `migration.rs`, `heal.rs`, `codegen.rs`, `error.rs`, Schritte `v2251`–`v2254` | Die Maschine kennt Bareos-Versionen und die Startmigration von odir (darf nicht wegfallen); Bareos-DDL (`v2250`) müsste getrennt im AGPL-Crate bleiben. |
| `onesimus-proto-rs` / `proto` | `onesimus-plugin-proto` | `filed_plugin.proto`, `plugin_ccc.proto`, `postgresql_broker.proto`, `openldap_broker.proto`, `podman_broker.proto` | Braucht eine Lizenzentscheidung im Repository `onesimus-proto` (heute `AGPL-3.0-only`, `packaging/copyright` nennt für `proto/*` die Bareos GmbH). Bis dahin linken die dualen Crates `onesimus-plugins`, die Broker und `onesimus-bnet-grpc` gegen das AGPL-only-Crate `onesimus-proto-rs`. |
| neu | `onesimus-licence` (Lizenz- und Limitprüfung) | noch nicht vorhanden | Plan FREE-TIER-LIZENZ.md: eigene Crate außerhalb des Bareos-Kerns; Lizenz (dual oder Source-available) ist dort noch offen. |

## Abhängigkeiten und Auslieferung

- Eine kommerzielle Lizenz kann nur die dual lizenzierten Crates erfassen.
  `onesimus-bnet-grpc` hängt von `bareos-bnet` und `onesimus-proto-rs` ab,
  `onesimus-plugins` und die drei Broker von `onesimus-proto-rs` (alle
  `AGPL-3.0-only`). Ohne Bareos-freie Abhängigkeiten nutzbar sind heute nur
  `osd-ring` und `onesimus-podman-core`.
- **Deb-Pakete** (unverändert): `odir`, `ofd`, `osd` und die Pakete
  `ofd-plugin-*` (Broker) enthalten Binärdateien, die dual lizenzierte Crates
  mit `AGPL-3.0-only`-Crates verbinden. Ausgeliefert ist jede dieser
  Binärdateien als Ganzes unter der AGPL; `packaging/copyright`
  (`License: AGPL-3.0-or-later`) bleibt daher zutreffend. Offen für die
  Paketpflege: `Files: *` nennt nur den Maintainer als Urheber; für die
  abgeleiteten Crates gehören die Bareos-Urheber dazu (bisher nur für
  `docs/wiki/commands/*` und `ua_commands_table.rs` eingetragen).
- `oconsole` enthält keine dual lizenzierten Crates.

## Beiträge Dritter

Beiträge zu dual lizenzierten Crates werden nur mit einer
Beitragsvereinbarung (CLA) angenommen, die dem Maintainer die kommerzielle
Lizenzierung erlaubt. Die CLA schreibt der Maintainer; sie liegt noch nicht
vor. Beiträge zu `AGPL-3.0-only`-Crates brauchen keine CLA.

## Offene Rechtsfragen (für die Anwaltsprüfung)

1. Ist odir (und sind ofd, osd, oconsole) als Nachbau mit byte-kompatiblen
   Protokollen, Formaten und Ausgaben ein abgeleitetes Werk von Bareos?
   Erfasst das auch eigene Module, die nur Bareos-Schnittstellen benutzen?
2. Reicht die Einteilung je Crate, oder braucht es Angaben je Datei
   (SPDX-Kopfzeilen)?
3. Sind die mit KI-Unterstützung geschriebenen Teile (Commits mit
   `Co-Authored-By: Claude …`) urheberrechtlich so dem Maintainer zuzuordnen,
   dass er sie kommerziell lizenzieren kann?
4. Lizenz der Limit-Crate (AGPL, dual oder Source-available), siehe
   FREE-TIER-LIZENZ.md.

Nachtrag 23.11.2026: Die Umstellung auf `AGPL-3.0-or-later` (Workspace, SPDX-Köpfe,
`proto`) gilt auch für die abgeleiteten Crates der Tabelle oben. Ob ein „or later“
für Teile zulässig ist, die von Bareos (AGPL-3.0-only) abgeleitet sind, ist für
die Anwaltsprüfung offen; die Zeilen der Bareos-Urheber bleiben unverändert.

## Bareos-Nähe je Crate und Datei

Stand 23.11.2026, Beleg-Auftrag für die Prüfung durch einen Juristen. Maintainer-Entscheidung vom
23.11.2026: Die Bareos-nahen Teile bleiben `AGPL-3.0-only` (Liste
[`scripts/ci/bareos-derived.txt`](../scripts/ci/bareos-derived.txt)); danach entscheidet ein Jurist
anhand von Belegen, was auf „or later“ darf. Dieser Abschnitt liefert die Belege. Er ändert weder
Köpfe noch `Cargo.toml` noch die Liste; Änderungsvorschläge stehen unten nur als Vorschlag.

> **Kein Rechtsrat.** Die Klassen unten sind eine technische Einordnung aus dem eigenen
> Quelltext, den Kommentaren und dem Vergleich mit installierten Bareos-Paketen. Ob eine Datei ein
> abgeleitetes Werk ist, entscheidet der Jurist, nicht diese Tabelle.

### Methode und Grenzen

**Ein Bareos-Quelltext war nicht erreichbar.** Gesucht wurde im Worktree, im Home-Verzeichnis, unter
`/usr/src`, `/opt` und `/tmp` (kein `ua_cmds.cc`, kein Klon; der im Kompatibilitätslog genannte
Checkout `/tmp/bareos-director-reference-18be77e6` besteht nicht mehr). Es wurde daher **kein
Quelltextvergleich** gemacht. Geurteilt wurde nur anhand

1. des eigenen Quelltexts und seiner Kommentare (Fundstellen auf Bareos-Dateien und -Funktionen,
   „port of“, „Bareos-Gegenstück“, Funktions- und Variablennamen nach dem C++-Vorbild),
2. eines **Vergleichs der Textkonstanten mit den installierten Bareos-Paketen** (Version
   `25.1.3~pre24.0e7742c39`: `/usr/sbin/bareos-dir`, `bareos-sd`, `bconsole`, `bareos-dbcheck`,
   `/usr/lib/bareos/libbareos*.so`; Zeichenketten per `strings -n 8`, 15 627 verschiedene). Gezählt
   wurden Zeichenkettenliterale des Rust-Quelltexts mit mindestens 20 Zeichen, mit Leerzeichen und
   ohne Formatplatzhalter (`{}`, `%`), die als Ganzes in diesen Binärstrings vorkommen. Das ist
   eine **Untergrenze** (Meldungen mit Platzhaltern und Mehrzeiler fallen heraus) und beweist nur
   gleichen Text, nicht Übernahme; Bareos 25.1.3 ist neuer als der Bezugsstand `18be77e6`
   (25.1.2~pre43) des Projekts. Der FD wurde nicht verglichen (das Paket `bareos-filedaemon`
   enthält hier `ofd`),
3. der installierten Dateien aus den Bareos-Paketen (`/usr/lib/bareos/scripts/ddl/creates/postgresql.sql`,
   `/usr/lib/bareos/defaultconfigs/`).

Die Kennzahlen je Datei stehen in den Tabellen: „Z<n>“ ist die Zeile des Belegs in der Datei (Stand
`HEAD` dieses Zweigs, gekürzte Zitate aus dem **eigenen** Quelltext), „Verweise“ die Zahl der Zeilen
mit Bareos-Dateinamen, `core/src`, „port of“ oder „C++“, „n von m Textkonstanten“ der Vergleich aus
Punkt 2.

### Klassen

| Klasse | Bedeutung | Maßstab in dieser Tabelle |
| --- | --- | --- |
| a | übersetzt oder wörtlich übernommen | Kommentar sagt „port of“/„exact port“/„GENERATED from Bareos“, der Quelltext führt C++-Zeilen mit, oder mindestens 8 und mindestens 40 % der vergleichbaren Textkonstanten stehen auch in den Bareos-Binärdateien, oder die Datei ist eine Kopie |
| b | Struktur übernommen, beim Lesen des Bareos-Quelltexts geschrieben | Fundstellen mit Datei und Funktionsname von Bareos, gleiche Reihenfolge der Schritte, gleiche Namen; keine Zeilenübersetzung belegt |
| c | eigenständig nach Protokoll/Format oder eigene Erweiterung | nur mit Gegenbeleg im Quelltext: „Bareos-Gegenstück: keins“, „Onesimus extension, not in Bareos“, eigene Maschine ohne Bareos-Entsprechung (dann mit Begründung). Die Gegenbelege sind Selbstauskünfte der Köpfe, nicht unabhängig geprüft |
| ungeklärt | weder a/b noch c belegt | Ohne Beleg gilt vorsichtig: **nicht** c. In der Spalte „Beleg“ steht dann die Tendenz („Tendenz c“ heißt: nichts spricht für eine Übernahme, aber nichts belegt das Gegenteil) |

### Ergebnis in Zahlen

Die Liste enthält 7 ganze Crates (74 Dateien) und 168 Einzeldateien, zusammen **242 Dateien** (Rust,
Python, Shell):

| Klasse | Dateien |
| --- | ---: |
| a (übersetzt/übernommen) | 38 |
| b (Struktur übernommen) | 130 |
| c (eigen, mit Gegenbeleg) | 23 |
| ungeklärt | 51 |

Von den 168 Einzeldateien sind 7 Skripte/Tests unter `scripts/` und `tests/` (alle ungeklärt, Tendenz c).
Nicht mitgezählt sind die Nicht-Code-Dateien der ganzen Crates (in `bareos-orm`: 7 JSON-Schemadateien, die Testdaten unter `tests/data/`; sie stehen unter „bareos-orm“ unten). Die Zahlen zählen Dateien, nicht Zeilen; die großen Befehlsdateien von `odir` (a) wiegen schwerer als
die vielen kleinen (c).


### Übersicht

| Crate (Gruppe) | Dateien | Klasse (a / b / c / ungeklärt) | Kernbeleg | Empfehlung für den Juristen |
| --- | ---: | --- | --- | --- |
| `bareos-bnet` | 11 | a 3 / b 2 / c 0 / ungeklärt 6 | Z218 in `cram_md5.rs` „exact port of C++ BinToBase64“; Z155 in `hello.rs` „Port of C++ GetNameAndResourceTypeAndVersionFromHello (lib/util.cc:207)“; `tls_psk.rs` 13 Fundstellen mit Zeilennummern | Bis zur Entscheidung `-only` lassen. Fragen: Fallen die Protokollteile ohne Quellenangabe (`wire.rs`, `stream.rs`, `listen.rs`, `tls.rs`) allein wegen ihrer Lage im Crate darunter? |
| `bareos-catalog` | 3 | a 0 / b 0 / c 0 / ungeklärt 3 | `queries.rs` Z8 „mirror the queries in the C++ cats/dml/ directory“ (Selbstauskunft); 9 einfache SELECT; das Crate wird von `odir` nicht benutzt | Prüfen, ob das Crate überhaupt gebraucht wird; sonst entfernen statt einstufen |
| `bareos-config` | 10 | a 1 / b 6 / c 0 / ungeklärt 3 | `parser.rs` Z686 „port of C++ dird/run_conf.cc“, Z130/138 mit `dird_conf.cc`-Zeilen; Fehlertext „Expected a Authentication Type keyword“ steht auch in `bareos-dir`; `sd.rs` mit `stored_conf.cc` | `-only` belassen (Ressourcentabellen, Vorgaben und Meldungen aus Bareos); `onesimus.rs` und `secrets.rs` getrennt prüfen |
| `bareos-format` | 17 | a 3 / b 11 / c 1 / ungeklärt 2 | `base64.rs` Z99 „C++: reg \|= (int8_t)bin[i];“; `compression.rs` „block decoders, ported to Rust“; `bsr.rs` Z53-58 mit 16 Bareos-Funktionsnamen und Bezugsstand `18be77e6` | `-only` belassen; Fremdcode (FastLZ, LZ4, CRC32) mit eigener Lizenz beachten. Frage: Sind reine Formatkonstanten (`stream.rs`, `filetype.rs`, `msg.rs`) schutzfähig? |
| `bareos-orm` | 10 | a 0 / b 0 / c 7 / ungeklärt 3 | Schema `v2250_bareos_catalog.json` „Converted from bareos/core/src/cats/ddl/creates/postgresql.sql“; `tests/data/bareos-18be77e6-creates-postgresql.sql` ist inhaltsgleich mit der Datei des Pakets (0 Abweichungen ohne Kommentare); die ORM-Maschine hat kein Bareos-Gegenstück | Aufteilen: Schema `v2250` und Testdaten sind Kopien (a), die Maschine (`schema.rs`, `heal.rs`, `codegen.rs`, `error.rs`, `lib.rs`) ist mit Vorbehalt als eigen eingestuft (c: kein Bareos-Gegenstück bekannt, nicht am Quelltext geprüft). Aufteilung nur nach Entscheidung (siehe LIZENZEN.md „Offene Trennungen“) |
| `daemon-crate` | 15 | a 0 / b 13 / c 1 / ungeklärt 1 | Jede Datei nennt „Bareos-Gegenstück“ mit Datei und Funktion (`lib/daemon.cc daemon_start`, `lib/bsys.cc CreatePidFile`, `include/exit_codes.h`, `filed/authenticate.cc` …) | `-only` belassen; kleine Systemhilfen, Frage nach der Schöpfungshöhe |
| `outils` | 8 | a 0 / b 0 / c 0 / ungeklärt 8 | Ersatz für `bls`/`bscan`/`bextract`; **keine** Fundstelle, keine identische Textkonstante, baut auf `bareos-format`/`bareos-config` auf | Ohne Beleg nicht als eigen werten; Entstehung der Werkzeuge beim Maintainer erfragen (wurde der Bareos-Quelltext von `bls.cc`/`bscan.cc`/`bextract.cc` gelesen?) |
| `odir` (Einzeldateien) | 98 | a 29 / b 58 / c 1 / ungeklärt 10 | Befehle, Dialoge, Jobabläufe mit „port of“ und Zeilenfundstellen (`catreq.rs` 85 Verweise, 24 von 27 Meldungen identisch; `ua_commands_table.rs` 100 von 100); `ua_commands_table.rs`, `show_items.rs`, `configure_usage.rs` sind aus Bareos-Quelltext erzeugt | `-only` belassen. Für die 10 ungeklärten und `volume_damaged.rs` (c) Einzelentscheidung |
| `ofd` (Einzeldateien) | 27 | a 1 / b 23 / c 2 / ungeklärt 1 | Modulköpfe nennen `filed/*.cc`, `findlib/*.cc`, `lib/*.cc` mit Funktionsnamen; `fnmatch.rs` „fnmatch() of Bareos (OpenBSD 1.6) in Rust“; kaum identische Meldungen (der FD wurde nicht verglichen) | `-only` belassen (b); `first_start.rs`, `trust_acl.rs` (c) könnten aus der Liste |
| `osd` (`crates/osd/`) | 18 | a 1 / b 15 / c 2 / ungeklärt 0 | Köpfe „Bareos-Gegenstück: stored/*.cc (Funktionen)“; `fd_session.rs` 22 von 24 Meldungen identisch; `grpc.rs`, `ringproto.rs`, `ring/src/lib.rs` ausdrücklich „keins“ | `-only` belassen (b); die drei c-Dateien könnten aus der Liste |
| `oconsole` | 3 | a 0 / b 1 / c 0 / ungeklärt 2 | `main.rs` Z286/484/511 mit `console_conf.cc`, `lib/bsock.cc:348`; MCP-Dateien sind Onesimus-Erweiterungen mit Bareos-Befehlsnamen | `main.rs` `-only`; MCP-Dateien Einzelentscheidung |
| `onesimus-rig` | 10 | a 0 / b 1 / c 7 / ungeklärt 2 | Köpfe: Analogie zu den `systemtests`, Vorlage ist das eigene Gerüst `ofdrig.py`; `console.rs` ahmt `bconsole` nach | Test-Gerüst, nicht ausgeliefert; Einstufung als eigen vertretbar, aber Selbstauskunft |
| `onesimus-bnet-grpc` | 5 | a 0 / b 0 / c 2 / ungeklärt 3 | `lib.rs` Z44 „Bareos-Gegenstück: keins (Onesimus-Erweiterung)“; nur Rahmenform entspricht `bsock_tcp.cc` | Fünf Dateien in der Liste, obwohl LIZENZEN.md das Crate als eigen führt: Liste prüfen |
| Skripte und Tests | 7 | a 0 / b 0 / c 0 / ungeklärt 7 | Werkzeuge, die Bareos-Quelltext lesen oder gegen ein Original-Bareos messen; kein Bareos-Text enthalten | als eigen (c) vertretbar, aber hier nicht belegt |


### Gegenprobe: Dateien, die nicht in der Liste stehen

Geprüft wurden alle 169 Rust-, Python- und Shell-Dateien unter `crates/`, `scripts/` und `tests/`, die
**nicht** in `bareos-derived.txt` stehen. 59 davon zeigten ein Signal (Verweis auf `.cc`/`.h`/`core/src`/
„C++“, mindestens drei Nennungen von „Bareos“, ein Marker „Onesimus extension“ oder identische
Textkonstanten) und wurden gelesen. Dazu kommen die Nicht-Code-Dateien (Konfigurationen, Handbuch,
Mitschnitte, Proto). Kein Treffer in dieser Gruppe ist im Sinn der Tabelle oben als a belegt, außer wo
ausdrücklich „Kopie“ steht.

| Datei(en) | Befund | Klasse | Empfehlung |
| --- | --- | --- | --- |
| `tests/bareos-dir.d/fileset/{Catalog,SelfTest,Windows All Drives}.conf`, `job/{BackupCatalog,RestoreFiles,backup-bareos-fd}.conf`, `jobdefs/DefaultJob.conf`, `messages/Standard.conf`, `pool/{Differential,Full,Incremental,Scratch}.conf`, `profile/{operator,webui-admin,webui-readonly}.conf`, `schedule/{WeeklyCycle,WeeklyCycleAfterBackup}.conf` (17 Dateien) | **inhaltsgleich** (ohne Kommentar- und Leerzeilen) mit `/usr/lib/bareos/defaultconfigs/bareos-dir.d/…` aus dem Paket `bareos-director` 25.1.3; ohne SPDX-/Copyright-Kopf (die Konfigurationsdateien prüft `spdx.py` nicht) | a (Kopie) | **fehlt in der Liste und in `packaging/copyright`.** Mit Bareos-Vermerk versehen oder durch eigene Beispiele ersetzen; Jurist: Schöpfungshöhe von Beispielkonfigurationen |
| weitere 23 `.conf` in `config/` und `tests/` (`config/bareos-dir.d`, `config/bareos-sd.d`, `tests/bareos-dir.d`, `tests/bareos-sd.d`, `tests/bconsole.conf`) | gleiche Dateinamen wie die Paketvorlagen, 1 bis 13 abweichende Zeilen | b (angepasste Kopie) | wie vorige Zeile |
| `docs/wiki/commands/*.md` (104 Dateien) und `bareos-director-commands.json`, `console-grammar.json`, `audit-notes.json`, Schema | 89 Dateien enthalten den Block „Aus den Bareos-Originalquellen“: **Originalbeschreibung** (Hilfetext) und **Originalsyntax** aus `ua_cmds.cc` mit Datei, Zeile und Link auf Bareos `18be77e6`. `bareos-director-commands.json` (330 KB, 88 Befehle) führt je Befehl Hilfetext, Syntaxzeile, Handler und Quellzeile von Bareos. Der übrige Text („Zweck“, „Syntax“, Beispiele) ist eigene Dokumentation; seine Herkunft wurde nicht geprüft | a (die Bareos-Blöcke), Rest ungeklärt | steht schon in `packaging/copyright`; Jurist: Hilfetexte als Zitat |
| `crates/odir/src/ua_commands_table.rs` (Liste) | siehe Tabelle `odir`: 100 von 100 Zeichenketten (Hilfe, Syntax) identisch mit den Binärstrings von `bareos-dir`; Kopf „Generated from Bareos core/src/dird/ua_cmds.cc commands[] (25.1.2)“ | a | bleibt `-only` |
| `crates/odir/src/command_grammar.rs`, `grammar_docs.rs`, `manpages.rs`, `commands/grammar.rs`, `packaging/man/*.7` (105 Dateien, erzeugt) | Grammatik und Handbuchseiten bauen auf `grammar_table.rs` (b, Liste) und `ua_commands_table.rs` auf. `grammar_docs.rs`: 1 801 Zeilen eigene Beschreibungen mit 14 Markern „Onesimus extension“, 2 von 680 Textkonstanten identisch (`status subscriptions`, `status configuration`); `command_grammar.rs` 2 von 79 (Syntaxfragmente). Herkunft der Beschreibungstexte (eigene Worte oder Bareos-Handbuch) wurde nicht geprüft | ungeklärt | dem Juristen mit `grammar_table.rs` vorlegen; eigene Worte beim Maintainer erfragen |
| `crates/odir/src/state_file.rs` | Kopf: „State file of the Director (`WriteStateFile` / `ReadStateFile`, dird/dir_plugins and lib/bstat of the original)“; eigenes JSON-Format statt der Binärdatei von Bareos | ungeklärt (Tendenz b/c) | prüfen, ob in die Liste |
| `crates/odir/src/grpc/api.rs` (919 Z), `grpc/service.rs` (`KNOWN_COMMANDS` Z281-312 mit Hilfe-/Syntaxkurztexten), `grpc/api_config.rs`, `api_dev.rs`, `auth.rs`, `push_events.rs`, `fd_grpc.rs`, `sd_grpc.rs`, `fd_progress.rs` | gRPC-Dienst (Onesimus-Erweiterung). `api.rs` zerlegt die Textausgabe von Bareos-Befehlen (Z222-226 „The next Volume to be used by Job …“), 4 von 15 Textkonstanten identisch; `service.rs` 2 von 32 („Print Director version“, „Truncate purged volumes“). 0 Verweise auf Bareos-Quelltext | ungeklärt (Tendenz c) | wie `grpc/mod.rs` (Liste) behandeln |
| `crates/odir/src/commands/{acl,config_crud,grammar,ovfs,purge_bulk}.rs` | Köpfe: „Onesimus extension (no Bareos counterpart)“, „without a bareos-dir counterpart“, „not in Bareos“. `purge_bulk.rs` ruft aber `purge::purge_jobs_from_volume` und `volume_purge_refused` (Liste, a) | c, `purge_bulk.rs` mit Abhängigkeit | zu Recht außerhalb der Liste; Abhängigkeit von `purge.rs` dem Juristen nennen |
| `crates/odir/src/{storage_health,usage,kill,mdns,secrets,logging,command_executor,command_worker,fsm,sd_protocol}.rs`, `commands/{catalog,mod}.rs`, `ovfs/{mod,capability,native,fallback}.rs` | eigene Funktionen (Köpfe nennen Pläne unter `docs/plans/`), 0 bis 2 Verweise, keine identischen Textkonstanten. `ovfs/mod.rs` Z14 nennt „a port of“ des eigenen OVFS-Kerns | ungeklärt (Tendenz c); Köpfe von `storage_health`, `usage`, `kill`, `mdns`, `secrets` ohne Bareos-Bezug | zu Recht außerhalb der Liste; „c“ nur nach Bestätigung durch den Maintainer |
| `crates/ofd/src/streams.rs` (7 Z), `stmode.rs`, `paths.rs`, `overlay.rs`, `vss.rs`, `win_trust.rs`, `plugin*`, `broker*`, `pg_*`, `ldap_*`, `podman_*`, `grpc*.rs` | `streams.rs` Z3 „Stream ids of include/streams.h: one definition in bareos-format“ (Wiedergabe der Konstanten aus dem Bareos-Crate); `stmode.rs` Z6-8 POSIX-Modebits „as Bareos defines them for Win32“; `paths.rs` Z3-5 Bareos-Verzeichnislayout; übrige Dateien Onesimus-Erweiterungen (Plugins, Windows) ohne Bareos-Fundstelle | `streams.rs`, `stmode.rs`, `paths.rs`: ungeklärt (Tendenz b, kleine Konstanten); Rest ungeklärt (Tendenz c) | `streams.rs` in die Liste aufnehmen oder entfernen |
| `crates/oconsole/src/grammar.rs`, `mcp*.rs` (außer den drei in der Liste), `grpc_client.rs`, `rate_guard.rs`, `fishkeys.rs`, `color.rs`, `mcp_token.rs` | Vervollständigung und MCP-Server (Onesimus); `mcp_read.rs` 3 von 169 Textkonstanten identisch (`status subscriptions`, `status configuration`), `grammar.rs` Z84 unterscheidet „bareos or onesimus“-Herkunft von Knoten | ungeklärt (Tendenz c) | Einstufung wie `mcp_tree.rs`/`mcp_write.rs` (Liste) |
| `crates/onesimus-bnet-grpc/src/tls.rs` | aus `daemon-crate` verschoben (21.11.2026), Kopf: 2 Marker „Onesimus extension“ | c (Selbstauskunft) | zu Recht außerhalb der Liste |
| `crates/onesimus-rig/tests/{osd,osd_copy,osd_grpc,osd_health,osd_read_wait,ldap_o7}.rs`, `src/glossary.rs` | Tests, die `bareos-sd` als Gegenstelle starten; nennen „bareos“ oft, aber keine Fundstellen auf Bareos-Quelltext; `ldap_o7.rs` hat 2 SQL-Zeilen auf Bareos-Tabellen (`SELECT Path FROM Path` …) | ungeklärt (Tendenz c) | zu Recht außerhalb der Liste |
| `docs/upstream/U-001-getorcreatepoolrecord-nextpool.patch` | Patch gegen `core/src/dird/job.cc` von Bareos (21 Diffzeilen mit Kontext und Hunk-Kopf, also Bareos-Quelltext); bestimmt für einen Pull Request an Bareos | a (Bareos-Quelltext als Kontext) | der Patch bleibt unter der Lizenz von Bareos (AGPL-3.0-only); in `packaging/copyright` vermerken |
| `tests/parity/` (`cases/*.jsonl`: Befehlszeilen, `baseline.json`: Ergebnisse „match“) und `tests/results/` (207 Dateien aus Läufen ab dem 21.03.2026: Konfigurationen, Logs, Ausgaben, Diffs) | enthalten Ausgaben von `bareos-dir` und Testkonfigurationen; keine Quelltexte | ungeklärt | Jurist: Programmausgaben und Mitschnitte als Testdaten; Größe 4,3 MB |
| `scripts/ci/spdx.py`, `scripts/compare_command_responses.py`, `scripts/test_oconsole_rate_guard.py`, `tests/generate_test_data.py`, `tests/mcp/mcp_rig.py`, `tests/parity/parity.py`, `tests/test_commands.sh`, `tests/verify/rig.py` | Prüf- und Testwerkzeuge; `spdx.py` enthält nur die Copyright-Zeile von Bareos als Text | c bzw. ungeklärt (Tendenz c) | zu Recht außerhalb der Liste |
| `proto/` (Submodul, eigenes Repository) | siehe Abschnitt „Befehlstexte und Protokolldefinitionen“ unten | gemischt | Entscheidung gehört in das Repository `onesimus-proto` |

Dateien, die ich für Bareos-nah halte und die **nicht** in der Liste stehen: die 17 identischen
`tests/bareos-dir.d/*.conf` (a), die 23 abweichenden Konfigurationen in `config/` und `tests/` (b),
`docs/wiki/commands/*` (a für die Bareos-Blöcke, steht aber in `packaging/copyright`),
`docs/upstream/U-001-…patch` (a) und, als Grenzfälle, `crates/odir/src/state_file.rs`,
`crates/ofd/src/streams.rs`, `command_grammar.rs`/`grammar_docs.rs`/`manpages.rs`. Die Muster, die der
Auftrag nannte (`commands/*.rs` als Nachbau der Konsolenbefehle, `ofd/restore.rs`, `ofd/backup.rs`,
`osd/daemon/src/director.rs`), **stehen alle in der Liste**; die nicht gelisteten Dateien unter
`commands/` sind die Onesimus-Befehle `acl`, `config_crud`, `grammar`, `ovfs`, `purge_bulk`, `catalog`.


### Einzelnachweise je Crate

Die Tabellen führen **jede** Datei der Liste. Beleg-Zitate sind Auszüge aus dem eigenen Quelltext
(Kommentare), keine Bareos-Texte.

#### bareos-bnet (BNET-Protokoll, TLS, CRAM-MD5)

Das Crate setzt das Netzprotokoll von Bareos byte-kompatibel um. Belege für Übersetzung: `cram_md5.rs`
Z218 („exact port of C++ BinToBase64 from base64.cc“) und Z103, `hello.rs` Z155 („Port of C++
GetNameAndResourceTypeAndVersionFromHello (lib/util.cc:207)“), `signal.rs` Z9 („map 1:1 to the C++
constants in lib/bsock.h“), `debug.rs` Z6-15 (C++-Makro `Dmsg` und Pegelkonvention im Kopf zitiert),
`tls_psk.rs` mit 13 Fundstellen samt Zeilennummern in `lib/tls_openssl.cc` und `lib/bsock.cc`.
Gegenbelege gibt es nicht; `wire.rs` (Länge als 4-Byte-Big-Endian-Zahl vor den Nutzdaten),
`stream.rs`, `tls.rs` und `listen.rs` nennen keine Quelle und sind deshalb ungeklärt. `listen.rs`
beschreibt eine Windows-Messung (`[::]` nimmt kein IPv4) und ist eine Eigenlösung, nennt Bareos aber
als Vergleich (Z15).

Fragen an den Juristen: Ist ein Protokoll (Rahmen, Signalwerte, Hello-Zeilen) schutzfähig oder nur
sein Text? Gilt `wire.rs` als Schnittstellenbeschreibung? Fallen die Dateien ohne Quellenangabe wegen
des Crates unter Bareos?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `examples/bnet_capture.rs` | 128 | ungeklärt | Prüfwerkzeug für die Paritätstests (tests/parity), keine Fundstelle; Tendenz c |
| `src/cram_md5.rs` | 441 | a | Z218 "exact port of C++ BinToBase64 from base64.cc"; Z103 "like C++ CramMd5Challenge does"; Protokollablauf im Kopf |
| `src/debug.rs` | 277 | b | Z6-15 "1:1 kompatibel mit C++ d_msg/Dmsg", Makro und Pegelkonvention aus baconfig.h im Kopf zitiert |
| `src/hello.rs` | 310 | a | Z155 "Port of C++ GetNameAndResourceTypeAndVersionFromHello (lib/util.cc:207)"; Z132 lib/bsock.cc:348; 2 von 15 Textkonstanten identisch |
| `src/lib.rs` | 40 | ungeklärt | nur Modulübersicht und Re-Exports, keine Fundstelle; Tendenz b |
| `src/listen.rs` | 64 | ungeklärt | Z15 "Bareos-Gegenstück: lib/bnet_network.cc" nur als Vergleich; Eigenlösung (socket2, auf Windows 11 gemessen); Tendenz c, nicht belegt |
| `src/signal.rs` | 166 | a | Z9 "map 1:1 to the C++ constants in lib/bsock.h" (Signalwerte der Leitung; Namen und Werte übernommen) |
| `src/stream.rs` | 106 | ungeklärt | Eigenbau (ein Strom-Typ über Klartext/TLS/PSK/gRPC), keine Fundstelle; Tendenz c, nicht belegt |
| `src/tls.rs` | 177 | ungeklärt | rustls-Anbindung, Z156 "matches C++ TlsPolicy enum" (Werte); sonst keine Fundstelle; Tendenz c |
| `src/tls_psk.rs` | 481 | b | 13 Fundstellen mit Datei:Zeile, z. B. Z69 "TlsCipherGetName, lib/tls_openssl.cc:89", Z80 "GetCipherMessageString, lib/bsock.cc:755", Z127/149 Verhalten des C++-Directors nachgebildet |
| `src/wire.rs` | 290 | ungeklärt | Wire-Format "4 Byte Länge, Big Endian, Nutzdaten" (Protokoll), aber keine Quelle genannt; Tendenz c, Quelle fehlt |

#### bareos-catalog

Drei Dateien (314 Zeilen). `queries.rs` Z8 sagt „These mirror the queries in the C++ cats/dml/
directory“; die neun Abfragen sind einfache `SELECT`-Anweisungen auf die Bareos-Tabellen `Job`,
`Client`, `Pool`, `Media`. Ob sie aus `cats/dml/` stammen, konnte ohne den Quelltext nicht geprüft
werden. **Das Crate wird von `odir/src` nicht benutzt** (nur `bareos-catalog.workspace = true` in
`crates/odir/Cargo.toml:27`; keine Zeile `use bareos_catalog` in `crates/odir/src`).

Frage an den Maintainer: Wird das Crate gebraucht? Wenn nicht, erübrigt sich die Einstufung.

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/lib.rs` | 168 | ungeklärt | sqlx-Anbindung (Z40-144), keine Fundstelle; die Crate wird von odir/src nicht benutzt (nur Abhängigkeit in Cargo.toml) |
| `src/queries.rs` | 62 | ungeklärt | Z8 "These mirror the queries in the C++ cats/dml/ directory" (Selbstauskunft); 9 einfache SELECT auf Bareos-Tabellen; nicht gegen Bareos geprüft |
| `src/records.rs` | 84 | ungeklärt | Strukturen mit den Spaltennamen der Bareos-Tabellen (Schema-Tatsachen), keine Fundstelle |

#### bareos-config (Konfigurationssyntax)

Übersetzt: `parser.rs` Z686 „port of C++ dird/run_conf.cc“ (Schedule-`Run`), Z749/778 mit den
Schlüsselwort-Tabellen `run_conf.cc keyw[]` und `dird_conf.cc joblevels[]`, Z130/138 mit Zeilen
(`dird_conf.cc:2613`, `:2640`). Die Meldung „Expected a Authentication Type keyword“ (Z138) steht
auch im Binärstring von `bareos-dir` (13 von 35 Textkonstanten der Datei sind identisch). `resources.rs`
übernimmt Direktiven, Vorgaben und Auswahlwörter mit `dird_conf.cc`-Zeilen (Z49 `:142`, Z85 `:202`),
`sd.rs` die `store_items` und Vorgaben von `stored_conf.cc`. Eigenständig und nicht belegt:
`onesimus.rs` (Verzeichnis `/etc/onesimus`) und `secrets.rs` (Maskierung von Zugangsdaten); beide
ohne Fundstelle, Tendenz c.

Fragen an den Juristen: Sind Direktivennamen, Vorgabewerte und Fehlermeldungen als Schnittstelle frei
benutzbar? Dürfen `onesimus.rs` und `secrets.rs` wegen ihrer Lage im Crate getrennt werden?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/edit.rs` | 573 | b | Z14 "Lexer rules as in raw.rs / C++ lib/lex.cc", Z402 "Run directive (C++ StoreRun() ...)"; Onesimus-Befehl configure update setzt darauf auf |
| `src/jobdefs.rs` | 414 | b | Z6 "JobDefs inheritance, as dird_conf.cc PopulateJobdefaults() does it", Z7 PropagateJobdefs; Reihenfolge der Ressourcenkette nachgebildet |
| `src/lib.rs` | 24 | ungeklärt | Modulübersicht |
| `src/onesimus.rs` | 58 | ungeklärt | Verzeichnis /etc/onesimus (Onesimus-Overlay), keine Fundstelle; Tendenz c, nicht belegt |
| `src/parser.rs` | 1580 | a | Z686 "port of C++ dird/run_conf.cc"; Z749 "run_conf.cc keyw[]", Z778 "dird_conf.cc joblevels[]"; Z130/138 StoreAuthprotocoltype (dird_conf.cc:2613) und StoreAuthtype (:2640); Meldungen wie "Expected a Authentication Type keyword" auch im Binärstring von bareos-dir; 13 von 35 Textkonstanten identisch |
| `src/raw.rs` | 489 | b | Z6 "Raw view ... as the C++ lexer sees it", Z14 "lexer semantics of lib/lex.cc", Z17 Ressourcenreihenfolge des C++-Directors |
| `src/resources.rs` | 441 | b | Z49 "Key Encryption Key (dird_conf.cc:142)", Z85 "Protocol (dird_conf.cc:202)", Z97 tls_resource_items.h, Z125 include/auth_protocol_types.h: Direktiven, Vorgabewerte und Auswahlwörter aus Bareos |
| `src/sd.rs` | 1152 | b | Z50 "Bareos-Gegenstück: core/src/stored/stored_conf.cc (store_items ...)", Z58-70 Vorgabewerte je "stored_conf.cc"; 52 Nennungen von Bareos im Text |
| `src/secrets.rs` | 98 | ungeklärt | Maskierung von Zugangsdaten in Plugin-Optionen (Onesimus-Funktion), keine Fundstelle; Tendenz c |
| `src/values.rs` | 191 | b | Z19 "Time modifiers of lib/edit.cc DurationToUtime"; Wert-Parser für yes/no, Zeiten, Größen |

#### bareos-format (Strom- und Volume-Format)

Alle Dateien nennen „Bareos-Gegenstück“ mit Datei und Funktion. Zeilengenau mitgeführt: `base64.rs`
Z99 („C++: reg |= (int8_t)bin[i];“) und Z50. Übersetzt: `compression.rs` Z11-12 („block decoders,
ported to Rust“ für FastLZ und LZ4, enthält damit Fremdcode mit eigener Lizenz) und Z6/340
(`DecompressData`/`CompressData`). Funktionsnamen von Bareos stehen in `bsr.rs` Z53-58 (u. a.
`MatchBsr`, `MatchVoladdr`, `find_next_bsr`, `CreateRestoreVolumeList`, mit Bezugsstand `18be77e6`) und
dort stehen auch „Abweichungen“ (Z59 ff.). `stream.rs` bildet die `STREAM_*`-Tabelle ab (10 von 13
Zeichenketten identisch). `build.rs` ist der gemeinsame rustdoc-Hilfscode (c). `posix_regex.rs` und
`glossary.rs` sind Eigenbau ohne Fundstelle (ungeklärt, Tendenz c).

Fragen an den Juristen: Sind Zahlenwerte und Namen von Datei-, Strom- und Meldungstypen
(`filetype.rs`, `stream.rs`, `msg.rs`) Tatsachen oder geschützter Ausdruck? Wie ist Fremdcode
(FastLZ, LZ4, CRC-32) im Bareos-Baum einzuordnen?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `build.rs` | 14 | c | Z1-14: bindet nur scripts/doc/module_tree.rs ein (rustdoc-Modulbaum), kein Bareos-Inhalt |
| `src/base64.rs` | 318 | a | Z23 "Bareos-Gegenstück: core/src/lib/base64.cc (ToBase64, FromBase64 ...)"; Z99 "// C++: reg /= (int8_t)bin[i];" (Zeile für Zeile mit dem C++-Code mitgeführt), Z50 "wraps like the C++ uint64_t arithmetic" |
| `src/bash.rs` | 99 | b | Z13-22 Tabelle "Bareos-Gegenstück: lib/util.cc BashSpaces, UnbashSpaces" (Leerzeichen <-> 0x01) |
| `src/breg.rs` | 198 | b | Z21 "Bareos-Gegenstück: core/src/lib/breg.cc (NewBregexp ...)", Z178 "(breg.cc keeps search on the copied separator)" |
| `src/bsr.rs` | 909 | b | Z53-58 "Bareos-Gegenstück: lib/parse_bsr.cc (parse_bsr, store_*, IsFastRejectionOk ...), stored/bsr.cc (MatchBsr, MatchAll, MatchVolume ... find_next_bsr ...), Bareos 18be77e6"; Funktionsnamen werden übernommen (Tendenz a); Z59 listet "Abweichungen" |
| `src/compression.rs` | 768 | a | Z6 "lib/compression.cc DecompressData()", Z11-12 "fastlzlib block stream (core/src/fastlz/src/fastlzlib.c) ... block decoders, ported to Rust", Z340 CompressData; enthält Fremdcode (FastLZ, LZ4) mit eigener Lizenz |
| `src/filetype.rs` | 100 | b | Z17 "Bareos-Gegenstück: core/src/include/filetypes.h", Z79 "filetypes.h:84": FT_*-Werte und Namen |
| `src/glossary.rs` | 34 | ungeklärt | Glossar (kein Code), erklärt Bareos-Begriffe mit eigenen Worten; Tendenz c |
| `src/label.rs` | 406 | b | Z30 "Bareos-Gegenstück: core/src/stored/label.cc (CreateVolumeLabel, ... UnserVolumeLabel, UnserSessionLabel)", Z15 lib/serial.cc |
| `src/lib.rs` | 78 | b | Z26-28 "Bareos-Gegenstück: include/streams.h, filetypes.h, lib/message_severity.h, lib/attribs.cc (EncodeStat, DecodeStat) ..."; 15 Verweise |
| `src/lstat.rs` | 342 | b | Z33 "Bareos-Gegenstück: core/src/lib/attribs.cc (EncodeStat, DecodeStat)", Z133 "Reads like the C++ code: thirteen numbers, each followed by one skipped ..." |
| `src/msg.rs` | 71 | b | Z16 "Bareos-Gegenstück: core/src/lib/message_severity.h": M_*-Werte |
| `src/posix_regex.rs` | 196 | ungeklärt | Z12-16 Plattform-Anbindung (libc regcomp unter Unix, regex-Crate unter Windows) und Vergleich mit lib/bregex.h; Eigenlösung, Tendenz c |
| `src/record.rs` | 168 | b | Z14 "Struktur laut Bareos core/src/stored/record.h und block.h", Z26 Konstanten PRE_LABEL ... SOS_LABEL; Z37 Tabelle filed/backup.cc fsend-Formate |
| `src/sd_job.rs` | 190 | b | Z26 "Bareos-Gegenstück: dird/msgchan.cc StartStorageDaemonJob() (Formatzeichenkette jobcmd), stored/job.cc"; Z14 "Regeln (wie msgchan.cc)" |
| `src/stream.rs` | 235 | a | Z17 "Bareos-Gegenstück: core/src/include/streams.h"; STREAM_*-Tabelle mit Werten; 10 von 13 Textkonstanten identisch mit Binärstrings |
| `src/volume.rs` | 822 | b | Z28 "SerBlockHeader", Z42 "stored/block.cc (SerBlockHeader, unSerBlockHeader, WriteBlockToDev), stored/record.cc (WriteHeaderToBlock)", Z81 crc32_fast von stored/crc32/crc32.cc (zlib-CRC) |

#### bareos-orm (Katalogschema und Migration)

Kopien: `schema/v2250_bareos_catalog.json` (Z5 „Converted from
bareos/core/src/cats/ddl/creates/postgresql.sql“; `tests/upstream_ddl.rs` prüft Spalte für Spalte gegen
die Original-DDL des Bezugsstands) und `tests/data/bareos-18be77e6-creates-postgresql.sql`, 641 Zeilen:
**ohne Kommentarzeilen und Leerraum identisch** mit `/usr/lib/bareos/scripts/ddl/creates/postgresql.sql`
des Pakets 25.1.3 (0 abweichende Zeilen im Vergleich; Kommentare wurden nicht verglichen). Die Datei ist
also eine wörtliche Kopie der Bareos-DDL (a). Die Onesimus-Schritte `v2251`–`v2256` (JSON, nicht in der Zahlentabelle) sind eigene SQL-
Objekte im Schema `onesimus` (Ansichten, Funktionen; `v2251_views_and_functions.json` hat einen
Kommentar „same as C++“ in der Funktion `sp_create_jobmedia`: dieser Teil ist b, die übrigen Schritte ungeklärt, Tendenz c), die Maschine (`schema.rs`, `migration.rs`,
`heal.rs`, `codegen.rs`) liest JSON und gleicht die Live-DB ab. Bareos pflegt SQL-Skripte
(Paketstruktur `scripts/ddl/{creates,updates,drops,grants}`) und hat keine solche Maschine; das ist ein
Gegenbeleg aus der Paketstruktur, nicht aus dem Quelltext. `migration.rs` ist ungeklärt, weil es die
Bareos-Tabelle `version` (VersionId 2250) liest und schreibt (Z143, Z240-242, Z719).

Fragen an den Juristen: Trägt die Trennung „Maschine eigen, Schema und Daten Bareos“? Sind SQL-
Schemata (Tabellen-, Spaltennamen) Tatsachen oder Ausdruck? Die Dateien `v2250` und `tests/data` sind
Kopien und bleiben auf jeden Fall `-only`.

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/codegen.rs` | 131 | c | Erzeugt Rust-Structs aus dem JSON-Schema (131 Zeilen); Bareos hat keine JSON-Schema-Maschine (Bareos pflegt SQL-Skripte, Paketstruktur /usr/lib/bareos/scripts/ddl/{creates,updates,drops,grants}); nicht gegen den Quelltext geprüft |
| `src/error.rs` | 25 | c | 25 Zeilen, Fehler-Enum des ORM, gelesen: kein Bareos-Bezug |
| `src/heal.rs` | 402 | c | Z1-12 Selbstheilung der Onesimus-Katalogobjekte (Schema onesimus), Z68 "Onesimus extension files only"; Bareos hat kein Gegenstück |
| `src/lib.rs` | 61 | c | 61 Zeilen Modulübersicht des JSON-Schema-ORM (Z1-10), gelesen: kein Bareos-Bezug außer dem Katalognamen |
| `src/migration.rs` | 1067 | ungeklärt | Maschine eigen (JSON-Schritte, Vergleich mit der Live-DB), aber Z143/240/719 behandeln die Bareos-Tabelle version (VersionId 2250) und halten den C++-Director lauffähig; Gegenbeleg nur teilweise |
| `src/schema.rs` | 763 | c | Z1-30 Strukturen CatalogSchema/Table/Column für JSON-Dateien (763 Zeilen); keine Fundstelle; Bareos-Gegenstück nicht vorhanden (siehe codegen.rs) |
| `tests/catalog_sequences.rs` | 63 | ungeklärt | Test: Onesimus-Schritte dürfen VersionId 2250 nicht ändern (Z33); keine Fundstelle |
| `tests/common/mod.rs` | 92 | c | Testhilfen (92 Zeilen), gelesen: kein Bareos-Bezug |
| `tests/onesimus_schema_migration.rs` | 690 | c | Test der Schritte v2251-v2254 (eigenes Schema onesimus); 0 Fundstellen |
| `tests/upstream_ddl.rs` | 583 | ungeklärt | Z7-11 vergleicht schema/v2250 mit "core/src/cats/ddl/creates/postgresql.sql of the reference commit"; liest die Bareos-Datei zur Laufzeit, enthält sie nicht |

#### daemon-crate (Daemon-Rahmen)

Jede der 13 Quelldateien nennt „Bareos-Gegenstück: <Datei> <Funktion>“ (siehe Tabelle); es sind kleine
Systemhilfen (PID-Datei, Rechteabgabe, Signale, Exit-Codes, Windows-Dienst, Handshake). Die Namen
folgen Bareos (`BEXIT_SUCCESS` in `exit.rs` Z15, `daemon_start` in `background.rs` Z15,
`AuthenticateDirector` in `handshake.rs` Z18-20). Gegenbelege fehlen. Zwei Dateien sind Hilfsdateien
(`build.rs` c, `glossary.rs` ungeklärt).

Frage an den Juristen: Reichen funktionale Entsprechungen kleiner Systemaufrufe (`fork`, `setuid`,
PID-Datei) für eine Ableitung?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `build.rs` | 30 | c | Z1-30: bindet scripts/doc/module_tree.rs ein (rustdoc-Modulbaum), kein Bareos-Inhalt |
| `src/accept.rs` | 200 | b | Z18-20 "Bareos-Gegenstück: lib/bnet_server_tcp.cc (BnetThreadServerTcp), lib/tls_openssl_private.cc (PSK-Server-Callback), filed/socket_server.cc" |
| `src/background.rs` | 58 | b | Z15 "Bareos-Gegenstück: lib/daemon.cc daemon_start" |
| `src/bwlimit.rs` | 162 | b | Z19 "Bareos-Gegenstück: core/src/lib/bsock.cc (SetBwlimit ...)" |
| `src/config_check.rs` | 49 | b | Z15 "Bareos-Gegenstück: filed/filed.cc und stored/stored.cc main" (Option -t) |
| `src/exit.rs` | 39 | b | Z15 "Bareos-Gegenstück: core/src/include/exit_codes.h (BEXIT_SUCCESS ...)": Exit-Codes |
| `src/glossary.rs` | 29 | ungeklärt | Glossar (kein Code), erklärt die Begriffe "ohne Bareos-Vorwissen"; Tendenz c |
| `src/handshake.rs` | 145 | b | Z18-20 "Bareos-Gegenstück: filed/authenticate.cc (AuthenticateDirector), stored/authenticate.cc, lib/util.cc" |
| `src/lib.rs` | 104 | b | Z50-52 "Bareos-Gegenstück: lib/daemon.cc, lib/bsys.cc (drop, CreatePidFile ...), lib/signal.cc, include/exit_codes.h, filed/filed.cc"; 10 Verweise |
| `src/logging.rs` | 145 | b | Z17 "Bareos-Gegenstück: lib/message.cc (InitMsg, debug_level)" mit Abweichung (tracing statt Bareos-Meldungssystem) |
| `src/pidfile.rs` | 61 | b | Z15 "Bareos-Gegenstück: lib/bsys.cc CreatePidFile, DeletePidFile" |
| `src/privileges.rs` | 181 | b | Z22 "Bareos-Gegenstück: lib/bsys.cc drop()", Z44 "(C++: pw_gid)" |
| `src/signals.rs` | 130 | b | Z29-30 "Bareos-Gegenstück: lib/signal.cc InitSignals, dird/dird.cc ReloadConfig an SIGHUP" |
| `src/version.rs` | 62 | b | Z14 "Bareos-Gegenstück: lib/version.cc (kBareosVersionStrings), lib/bsys.cc (GetOsInfo)"; 1 von 2 Textkonstanten identisch |
| `src/winservice.rs` | 187 | b | Z32 "Bareos-Gegenstück: core/src/win32/generic/service.cc (ReportStatus, bareosWorkerThread), win32/generic/main.cc" |

#### outils (ols, oscan, oextract)

Ersatz für `bls`, `bscan`, `bextract` (Kopf von `lib.rs` Z9). Der Quelltext nennt **keine** Bareos-
Datei, keine Funktion, keine identische Meldung (0 Verweise in 1 464 Zeilen; 0 identische Textkonstanten
unter 13 vergleichbaren im Quelltext); er liest Volumes über `bareos-format` (b) und die Konfiguration über
`bareos-config`. Es gibt keinen Gegenbeleg („Bareos-Gegenstück: keins“ fehlt). Deshalb ungeklärt mit
Tendenz c; als eigenständig darf das **nicht** gelten, bevor der Maintainer die Entstehung bestätigt.

Fragen: Wurde beim Schreiben der Quelltext von `bls.cc`, `bscan.cc`, `bextract.cc` gelesen (Ablauf,
Optionen, Ausgabeformat)? Ist die Abhängigkeit von abgeleiteten Crates allein ein Ableitungsgrund?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/bin/oextract.rs` | 101 | ungeklärt | Ersatz für bls/bscan/bextract (Kopf); 0 Fundstellen auf Bareos-Quelltext, 0 identische Textkonstanten; baut auf bareos-format/bareos-config (b) auf; Tendenz c, kein Gegenbeleg im Quelltext |
| `src/bin/ols.rs` | 202 | ungeklärt | Ersatz für bls/bscan/bextract (Kopf); 0 Fundstellen auf Bareos-Quelltext, 0 identische Textkonstanten; baut auf bareos-format/bareos-config (b) auf; Tendenz c, kein Gegenbeleg im Quelltext |
| `src/bin/oscan.rs` | 158 | ungeklärt | Ersatz für bls/bscan/bextract (Kopf); 0 Fundstellen auf Bareos-Quelltext, 0 identische Textkonstanten; baut auf bareos-format/bareos-config (b) auf; Tendenz c, kein Gegenbeleg im Quelltext |
| `src/extract.rs` | 372 | ungeklärt | Ersatz für bls/bscan/bextract (Kopf); 0 Fundstellen auf Bareos-Quelltext, 0 identische Textkonstanten; baut auf bareos-format/bareos-config (b) auf; Tendenz c, kein Gegenbeleg im Quelltext |
| `src/lib.rs` | 118 | ungeklärt | Ersatz für bls/bscan/bextract (Kopf); 0 Fundstellen auf Bareos-Quelltext, 0 identische Textkonstanten; baut auf bareos-format/bareos-config (b) auf; Tendenz c, kein Gegenbeleg im Quelltext |
| `src/scan.rs` | 305 | ungeklärt | Ersatz für bls/bscan/bextract (Kopf); 0 Fundstellen auf Bareos-Quelltext, 0 identische Textkonstanten; baut auf bareos-format/bareos-config (b) auf; Tendenz c, kein Gegenbeleg im Quelltext |
| `tests/oextract.rs` | 116 | ungeklärt | Test der Werkzeuge, 0 Fundstellen; Tendenz c |
| `tests/ols.rs` | 92 | ungeklärt | Test der Werkzeuge, 0 Fundstellen; Tendenz c |

#### odir (Director)

Auf 98 Dateien (70 669 Zeilen) kommen 29 Dateien der Klasse a. Muster: Der Kopf nennt „port of“ mit
der C++-Datei und Funktion (`catreq.rs` Z8 → `dird/catreq.cc`, 85 Verweise; `ua_select.rs` Z6 →
`ua_input.cc`; `restore.rs` Z6 → `ua_restore.cc`; `expand.rs` Z6 → `lib/var.cc` (OSSP-var, Fremdcode);
`verify.rs`, `bvfs.rs`, `configure.rs`, `estimate.rs` …), und die Meldungen stimmen mit den
Binärstrings von `bareos-dir` überein (`bvfs.rs` 78 von 100, `job.rs` 75 von 110, `admin.rs` 64 von 220,
`restore.rs` 63 von 146, `status.rs` 44 von 77, `expand.rs` 39 von 49, `restore_tree.rs` 36 von 39,
`catreq.rs` 24 von 27, `ua_commands_table.rs` 100 von 100). Drei Dateien sind aus Bareos-Quelltext
**erzeugt**: `ua_commands_table.rs` (aus `ua_cmds.cc`), `commands/show_items.rs` (aus `dird_conf.cc`,
`tls_resource_items.h`), `commands/configure_usage.rs` (mit SHA-256 der Quelldateien). Dazu sind
`scripts/generate_configure_usage.py` und `scripts/generate_director_reference.py` Werkzeuge, die den
Bareos-Quelltext einlesen. Das Verhalten wurde im Paritätstest gegen `bareos-dir` 25.1.2~pre43 Byte für
Byte abgeglichen (`docs/COMPATIBILITY-LOG.md`, Bezugsstand `18be77e6`); das Log nennt den Checkout des
Bareos-Quelltexts als Referenz, **der Quelltext wurde also beim Schreiben gelesen**.

Klasse b (58 Dateien): gleiche Abläufe und Namen mit Fundstellen (`scheduler.rs`, `session.rs`,
`job_manager.rs` …). Ungeklärt (10) oder c (1, `commands/volume_damaged.rs`, Z3 „Onesimus extension,
not in Bareos“): Onesimus-Funktionen, die nur Bareos als Vergleich nennen (`watchdog.rs`,
`grpc/mod.rs`, `grpc/coverage.rs`, `grpc/api_write.rs`, `config_edit.rs`, `catalog_acl.rs`,
`command_ast.rs`, `command_registry.rs`) und zwei Tests. Besonderheit: `ovfs/lstat.rs` und
`ovfs/tree.rs` sind Ports der **eigenen** PostgreSQL-Erweiterung OVFS (Kopf Z6, Commit `4041335` im
Repository `oenesimus-postgres-plugin`), lösen aber Jobketten „as Bareos does for a restore“ auf
(`tree.rs` Z27, `GetFileList`, `ua_tree.cc`) und dekodieren das Bareos-Format `File.LStat`
(`lstat.rs` Z11). Die Entscheidung, das Verhalten byte-gleich zu halten, ist Grundlage des Projekts
(Fehler von Bareos werden bewusst berichtigt: `docs/upstream/README.md`).

Fragen an den Juristen: Ist ein Nachbau mit identischem Verhalten und identischen Meldungen, der den
Quelltext als Referenz hatte, insgesamt abgeleitet (dann auch die Dateien der Klasse b und die
ungeklärten)? Dürfen die Onesimus-Dateien der Liste (grpc, `config_edit`, `catalog_acl`, `watchdog`,
`volume_damaged`) getrennt werden, obwohl sie in den Befehlsablauf eingebettet sind (siehe „Offene
Trennungen“ oben)? Wie sind Fremdcode-Teile (OSSP var) einzuordnen?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `build.rs` | 42 | b | Z6 "Build-time version strings (C++ lib/version.cc kBareosVersionStrings and"; 1 Verweise |
| `src/accept.rs` | 479 | b | Z10 "Supports optional TLS (required for C++ bconsole compatibility)."; 7 Verweise |
| `src/acl.rs` | 965 | b | Z6 "Console ACL semantics from Bareos ua_acl.cc; policies are read after reload."; 6 Verweise |
| `src/audit.rs` | 426 | b | Z6 "Console auditing (dird/ua_audit.cc) and the daemon message delivery it"; 12 Verweise |
| `src/backup.rs` | 838 | a | Z7 "job.cc SetupJob for JT_BACKUP (GetOrCreateClientRecord,"; 32 Verweise; 15 von 26 Textkonstanten identisch |
| `src/backup_worker.rs` | 840 | a | Z7 "Compatibility log C-058): backup.cc DoNativeBackup() in its order —"; 16 Verweise; 13 von 21 Textkonstanten identisch |
| `src/btime.rs` | 272 | b | Z6 "lib/btime.cc StrToUtime() — the one parser for catalog and console"; 7 Verweise |
| `src/catalog_acl.rs` | 587 | ungeklärt | zweite ACL-Stufe (Onesimus-Erweiterung, docs/plans/ACL-ZWEISTUFIG.md); Z10 nennt ua_acl.cc als Grundlage der ersten Stufe; Tendenz c |
| `src/catalog_worker.rs` | 1146 | b | Z32 "SQL query returning every value as libpq text (like the C++ catalog)."; 14 Verweise |
| `src/catreq.rs` | 1990 | a | Z8 "Port of dird/catreq.cc (CatalogRequest, CatalogUpdate/UpdateAttribute,"; 85 Verweise; 24 von 27 Textkonstanten identisch |
| `src/catreq/tests.rs` | 741 | b | Z6 "Requests in the exact stored/askdir.cc formats against a catalog double"; 5 Verweise; 17 von 114 Textkonstanten identisch |
| `src/check_catalog.rs` | 336 | b | Z6 "Startup part of C++ CheckCatalog() (dird/check_catalog.cc:118-162):"; 10 Verweise |
| `src/command_ast.rs` | 48 | ungeklärt | 48 Zeilen, eigene Befehlsdarstellung (Z44 nur "C++ ua->cmd" als Vergleich); Tendenz c |
| `src/command_queue.rs` | 351 | b | Z18 "Message kinds of UaContext::ErrorMsg/WarningMsg/InfoMsg (C++"; 8 Verweise |
| `src/command_registry.rs` | 288 | ungeklärt | Registry der Befehle (Onesimus-Aufbau), Z164/179/182 markieren Onesimus-Erweiterungen; die Befehlsnamen sind die von Bareos; 2 von 58 Textkonstanten identisch |
| `src/commands/admin.rs` | 2827 | b | Z35 "A complete label/relabel request (ua_label.cc do_label)."; 64 Verweise; 64 von 220 Textkonstanten identisch |
| `src/commands/autoprune.rs` | 274 | b | Z6 "Auto prune of Jobs and Files at the end of a job (dird/autoprune.cc"; 10 Verweise; 3 von 11 Textkonstanten identisch |
| `src/commands/bandwidth.rs` | 374 | b | Z6 "`setbandwidth` — C++ ua_cmds.cc SetbwlimitCmd (:935) with"; 6 Verweise; 5 von 30 Textkonstanten identisch |
| `src/commands/basic.rs` | 320 | b | Z16 "C++ ua_cmds.cc VersionCmd: "<name> Version: <Full> (<Date>) <OsInfo>"; 5 Verweise; 6 von 17 Textkonstanten identisch |
| `src/commands/bvfs.rs` | 1311 | a | Z8 "Port of C++ dird/ua_dotcmds.cc (DotBvfs*Cmd, BvfsParseArg,"; 52 Verweise; 78 von 100 Textkonstanten identisch |
| `src/commands/cancel.rs` | 633 | b | Z6 "`cancel` — C++ ua_cmds.cc CancelCmd (:803) with CancelStorageDaemonJob"; 14 Verweise; 9 von 41 Textkonstanten identisch |
| `src/commands/configure.rs` | 1568 | a | Z6 "`configure` — port of dird/ua_configure.cc (Bareos 25.1.2)."; 14 Verweise; 14 von 92 Textkonstanten identisch |
| `src/commands/configure_usage.rs` | 79 | a | Z6-9 "GENERATED by scripts/generate_configure_usage.py", Z8-9 SHA256 von dird_conf.cc und tls_resource_items.h |
| `src/commands/debug.rs` | 606 | b | Z63 "C++ atoi(): optional sign and leading digits, 0 otherwise."; 7 Verweise; 6 von 26 Textkonstanten identisch |
| `src/commands/defaults.rs` | 674 | b | Z6 "`.defaults` field order and types from Bareos ua_dotcmds.cc::DotDefaultsCmd."; 3 Verweise |
| `src/commands/dot.rs` | 728 | b | Z8 "Compatible with C++ ua_dotcmds.cc output format."; 30 Verweise; 3 von 16 Textkonstanten identisch |
| `src/commands/estimate.rs` | 657 | a | Z6 "`estimate` — port of C++ ua_cmds.cc EstimateCmd()."; 21 Verweise; 11 von 11 Textkonstanten identisch |
| `src/commands/extra.rs` | 1585 | b | Z16 "C++ FindArgKeyword(ua, {"on", "off"}): first argument keyword (case"; 33 Verweise; 24 von 90 Textkonstanten identisch |
| `src/commands/job.rs` | 2736 | a | Z26 "ua_run.cc: jcr->dir_impl->IgnoreDuplicateJobChecking (S-22)."; 41 Verweise; 75 von 110 Textkonstanten identisch |
| `src/commands/label.rs` | 508 | a | Z6 "The Storage daemon and catalog part of `label`/`relabel` (ua_label.cc):"; 8 Verweise; 10 von 21 Textkonstanten identisch |
| `src/commands/list.rs` | 1802 | a | Z8 "Port of C++ ua_output.cc DoListCmd() and cats/sql_list.cc: the same SQL"; 34 Verweise; 16 von 24 Textkonstanten identisch |
| `src/commands/pool_db.rs` | 331 | b | Z6 "Pool and Media catalog records from the Pool resource: ua_db.cc"; 16 Verweise |
| `src/commands/prompt.rs` | 26 | b | Z10 "`SendCmdUsage` (ua_output.cc:1923): "<message>\nUSAGE: <cmd> <usage>\n"."; 1 Verweise |
| `src/commands/purge.rs` | 2107 | b | Z6 "Catalog work of `prune` and `purge` (ua_prune.cc, ua_purge.cc and the"; 42 Verweise; 37 von 128 Textkonstanten identisch |
| `src/commands/resolve.rs` | 278 | b | Z7 "(C++ ua_cmds.cc ResolveCmd, fd_cmds.cc DoClientResolve, sd_cmds.cc DoStorageResolve)."; 4 Verweise; 4 von 25 Textkonstanten identisch |
| `src/commands/restore.rs` | 2467 | a | Z6 "restore: port of C++ ua_restore.cc RestoreCmd and UserSelectJobidsOrFiles"; 53 Verweise; 63 von 146 Textkonstanten identisch |
| `src/commands/restore_bsr.rs` | 581 | a | Z6 "Bootstrap of a console restore (compatibility log C-047): port of"; 7 Verweise; 5 von 5 Textkonstanten identisch |
| `src/commands/restore_tree.rs` | 1117 | a | Z6 "The restore file selection tree (compatibility log C-047): port of"; 17 Verweise; 36 von 39 Textkonstanten identisch |
| `src/commands/show.rs` | 2330 | a | Z8 "Port of dird/ua_output.cc show_cmd(), lib/res.cc PrintResourceItem(),"; 32 Verweise |
| `src/commands/show_items.rs` | 451 | a | Z6-8 "Directive tables of the Bareos 25.1.2 Director (C++ ResourceItem arrays) ... GENERATED from bareos core/src/dird/dird_conf.cc, lib/tls_resource_items.h" |
| `src/commands/sql.rs` | 558 | b | Z9 "are rendered like C++ cats BareosDb::ListSqlQuery (see sql_list.rs)."; 14 Verweise; 6 von 23 Textkonstanten identisch |
| `src/commands/status.rs` | 2088 | a | Z6 "Status commands: status, .status (C++ dird/ua_status.cc)"; 45 Verweise; 44 von 77 Textkonstanten identisch |
| `src/commands/update_db.rs` | 337 | b | Z6 "The catalog work of `update` (ua_update.cc) beyond single Media"; 7 Verweise; 3 von 4 Textkonstanten identisch |
| `src/commands/volume_damaged.rs` | 355 | c | Z3 "Onesimus extension, not in Bareos" (Ablauf bei beschädigtem Volume); nutzt aber Katalogfunktionen mit Bareos-Herkunft (Z32 UpgradeCopies, sql_update.cc) und purge.rs |
| `src/commands/wait.rs` | 272 | b | Z15 "C++ wait_cmd OpenClientDb(); Ok carries the catalog messages."; 2 Verweise |
| `src/config_check.rs` | 438 | b | Z6 "Configuration checks the C++ Director makes while parsing and in"; 17 Verweise |
| `src/config_edit.rs` | 1180 | ungeklärt | Konfigurations-CRUD als Rohtext (Onesimus-Erweiterung laut docs/plans/CONFIG-CRUD-GRPC.md); Z501/518 nennen lib/parse_conf.cc nur als Fehlerherkunft; Tendenz c |
| `src/config_edit/tests.rs` | 545 | ungeklärt | Tests von config_edit.rs (Z349 "reverse glob order (C++ resource chain)"); Tendenz c |
| `src/config_test.rs` | 443 | b | Z6 "`odir -t` like `bareos-dir -t` (dird.cc main): ParseConfigOrExit,"; 18 Verweise |
| `src/console_msg.rs` | 331 | b | Z6 "Console message queue (lib/message.cc InitConsoleMsg / `con_fd`,"; 8 Verweise |
| `src/consolidate.rs` | 264 | b | Z6 "ConsolidateJobs() (dird/consolidate.cc), Compatibility log C-058, S-06:"; 1 Verweise |
| `src/director_config.rs` | 534 | b | Z18 "Every vector keeps the C++ resource-chain order (config files in reverse"; 15 Verweise |
| `src/expand.rs` | 1526 | a | Z6 "Variable expansion: port of the OSSP var library (C++ lib/var.cc) and of"; 12 Verweise; 39 von 49 Textkonstanten identisch |
| `src/fd_protocol.rs` | 465 | b | Z233 "(C++ ClientConnectionHandshakeMode, dird/fd_cmds.cc)."; 5 Verweise; 3 von 9 Textkonstanten identisch |
| `src/grammar_table.rs` | 776 | b | Z12 "strings of ua_cmds.cc ([ua_commands_table]) and the keywords the odir handlers read", Z330 "the keywords of ua_run.cc"; Onesimus-Erweiterungen markiert (4); enthält die Bareos-Syntax je Befehl |
| `src/grpc/api_write.rs` | 1454 | ungeklärt | Schreib-RPCs; baut Bareos-Befehlszeilen aus Typen, Z767 "(ua_purge.cc), a dialog a typed RPC cannot answer"; 1 von 93 Textkonstanten identisch; Tendenz c |
| `src/grpc/coverage.rs` | 584 | ungeklärt | Zuordnung Konsolenbefehl -> RPC, Z244 "Onesimus extensions"; nennt Bareos-Befehle (Z274 ua_tree.cc); Tendenz c |
| `src/grpc/mod.rs` | 169 | ungeklärt | gRPC-Server (Onesimus-Erweiterung); Z9 "Unlike the C++ per-client model"; Tendenz c |
| `src/internal_jobs.rs` | 652 | a | Z7 "C-058): the selection of a Copy/Migrate control job (migrate.cc"; 2 Verweise; 11 von 24 Textkonstanten identisch |
| `src/job_manager.rs` | 1973 | b | Z42 "(C++ jcr->dir_impl->res.client / write_storage)."; 45 Verweise; 4 von 15 Textkonstanten identisch |
| `src/job_manager/limits.rs` | 710 | b | Z8 "- AllowDuplicateJob() (dird/job.cc:883), done in SetupJob() through the"; 9 Verweise; 5 von 7 Textkonstanten identisch |
| `src/job_messages.rs` | 1018 | a | Z6-8 "Job message texts of the Director (C++ dird/job.cc, backup.cc, restore.cc, ...)"; 20 von 53 Textkonstanten identisch mit Binärstrings von bareos-dir |
| `src/job_worker.rs` | 1915 | b | Z44 "admin.cc: no daemon contact; the worker only reports the start and"; 45 Verweise; 6 von 46 Textkonstanten identisch |
| `src/json_output.rs` | 243 | b | Z6-14 "Like C++ OutputFormatter (lib/output_formatter.cc, JsonFinalizeResult)"; Z14 "Like C++, plain text output is not part of the JSON result" |
| `src/listen.rs` | 296 | b | Z9 "As Bareos 25 (lib/res.cc StoreAddresses*, lib/address_conf.cc AddAddress,"; 3 Verweise |
| `src/localtime.rs` | 96 | b | Z6 "Local time formatting like C++ bstrftime()" (96 Zeilen) |
| `src/mail.rs` | 622 | b | Z7 "destinations (Bareos 25 lib/message.cc)."; 26 Verweise |
| `src/main.rs` | 883 | b | Z261/304/457 Verweise auf C++ (Dmsg-Referenz, "bareos-dir" für BNET, CheckCatalog() at start (dird.cc)); Kopf "Drop-in replacement for the Bareos Director daemon" |
| `src/message_bus.rs` | 243 | b | Z15 "Maps 1:1 to the C++ BusMessage variant in message_bus.h", Z35 "maps to C++ M_INFO ..." |
| `src/migrate.rs` | 747 | a | Z7 "dird/migrate.cc after the selection — StartNewMigrationJob() (the"; 7 Verweise; 9 von 21 Textkonstanten identisch |
| `src/ndmp.rs` | 133 | b | Z9-20 "NDMP branches of upstream (dird/ndmp_dma_storage.cc, ndmp_dma_generic.cc)"; ohne NDMP-Schicht, nur die Meldungen der Zweige; 3 von 5 Textkonstanten identisch |
| `src/ovfs/lstat.rs` | 141 | b | Z6 "Port of ovfs-core/src/lstat.rs of the OVFS extension (commit 4041335)" = Port einer eigenen Erweiterung; Z11 dekodiert das Bareos-Format (attribs.cc EncodeStat, base64.cc); keine eigenen Textkonstanten |
| `src/ovfs/tree.rs` | 1060 | b | Z6 "Port of ovfs-core/src/tree.rs" (eigene Erweiterung), Z27 löst Jobketten "as Bareos does for a restore (GetFileList with select_recent_version_with_basejob_and_delta, ua_tree.cc InsertTreeHandler)" |
| `src/pam_auth.rs` | 303 | b | Z7 "`dird/auth_pam.cc` and `console/auth_pam.cc` (docs/STUBS.md S-43)."; 1 Verweise |
| `src/privileges.rs` | 13 | b | Z6 "drop root privileges (like C++ lib/bsys drop())" (13 Zeilen) |
| `src/protect.rs` | 354 | b | Z6-12 bewusste Abweichung von bareos-dir (Entscheidung des Maintainers 19.11.2026), nennt ua_prune.cc:1077-1114 und AccurateGetJobids als Vorbild der Kettenbildung; Tendenz c für den Schutzgedanken, b für die Kettenlogik |
| `src/reload.rs` | 275 | b | Z58 "Configuration path the Director was started with (C++"; 3 Verweise |
| `src/run_time.rs` | 267 | a | Z6 "When a Schedule Run directive triggers — port of C++ dird/date_time.cc,"; 8 Verweise |
| `src/runscript.rs` | 711 | a | Z6 "Director-side RunScripts: lib/runscript.cc RunScripts() and"; 17 Verweise; 10 von 21 Textkonstanten identisch |
| `src/scan.rs` | 154 | b | Z6-8 "lib/scan.cc bsscanf(): what sscanf means in the C++ Director": das Verhalten von bsscanf nachgebaut |
| `src/scheduled_run.rs` | 430 | b | Z7 "(dird/scheduler_private.cc TryCreateJobControlRecord, SetJcrFromRunResource;"; 7 Verweise |
| `src/scheduler.rs` | 571 | b | Z6 "Job scheduler — dir:scheduler thread (dird/scheduler.cc,"; 7 Verweise |
| `src/sd_copy.rs` | 512 | b | Z8 "Copy/Migrate worker jobs (migrate.cc DoActualMigration, local replicate)"; 7 Verweise |
| `src/sd_label.rs` | 108 | b | Z9 "Used by `purge volume action=truncate` (ua_purge.cc:604"; 3 Verweise |
| `src/session.rs` | 1065 | b | Z29 "Console version from the Hello (C++ BareosVersionNumber, major * 100 + minor)."; 24 Verweise; 6 von 24 Textkonstanten identisch |
| `src/session_manager.rs` | 493 | b | Z31 "Catalog state of the console session (C++ UaContext catalog/db)."; 16 Verweise |
| `src/sql_list.rs` | 356 | b | Z6 "Rendering of catalog query results like C++ `BareosDb::ListResult`"; 13 Verweise |
| `src/ua_commands_table.rs` | 94 | a | Z1 "Generated from Bareos core/src/dird/ua_cmds.cc commands[] (25.1.2)"; 100 von 100 Textkonstanten (Hilfe, Syntax) identisch mit Binärstrings von bareos-dir |
| `src/ua_db.rs` | 479 | a | Z8 "Port of C++ dird/ua_db.cc OpenClientDb()/OpenDb()/CloseDb() and"; 12 Verweise; 4 von 13 Textkonstanten identisch |
| `src/ua_impexp.rs` | 729 | a | Z6 "`import`, `export` and `move` (dird/ua_impexp.cc PerformMoveOperation):"; 3 Verweise; 12 von 28 Textkonstanten identisch |
| `src/ua_select.rs` | 1472 | a | Z6 "Console dialogs: port of C++ dird/ua_input.cc (GetCmd, GetPint,"; 64 Verweise; 27 von 42 Textkonstanten identisch |
| `src/ua_storage_select.rs` | 1100 | a | Z8 "Port of C++ dird/ua_select.cc get_storage_resource() (:1320),"; 34 Verweise; 9 von 51 Textkonstanten identisch |
| `src/vbackup.rs` | 529 | a | Z6 "VirtualFull backups (dird/vbackup.cc), Compatibility log C-058, S-06."; 6 Verweise; 16 von 19 Textkonstanten identisch |
| `src/verify.rs` | 1151 | a | Z6 "Verify jobs: port of dird/verify.cc (DoVerifyInit, DoVerify,"; 14 Verweise; 8 von 11 Textkonstanten identisch |
| `src/version_strings.rs` | 191 | b | Z6 "Central version strings, C++ lib/version.cc `kBareosVersionStrings` and"; 5 Verweise |
| `src/volume_key.rs` | 104 | b | Z6 "Volume encryption keys (dird/volume_key.cc, lib/passphrase.cc,"; 4 Verweise |
| `src/watchdog.rs` | 201 | ungeklärt | Z6 "Watchdog - dir:watchdog thread (Onesimus)"; Z8 lib/watchdog.cc nur als Vergleich: eigenes Prozessmodell (fork, Waisen); Tendenz c |
| `tests/integration_bnet.rs` | 237 | ungeklärt | Integrationstest (Z59 "C++ ua_server.cc: no reply, socket closed" als erwartetes Verhalten); Tendenz c |

#### ofd (File Daemon)

Köpfe nennen `filed/*.cc`, `findlib/*.cc` und `lib/*.cc` mit Funktionsnamen (`backup.rs` Z6
`BlastDataToStorageDaemon`, `SaveFile`; `restore.rs` Z6 `DoRestore`; `find.rs` `FindFiles`/`OurCallback`;
`create_file.rs` `CreateFile`/`makepath`; `jmsg.rs` `Jmsg`/`Qmsg`; `fnmatch.rs` „fnmatch() of Bareos
(OpenBSD 1.6) in Rust“). Die Textkonstanten stimmen kaum überein (der FD wurde nicht verglichen, das
Paket `bareos-filedaemon` ist hier `ofd`); daher ist hier nur b belegt, a wegen „in Rust“ nur für
`fnmatch.rs`. Zwei Dateien sind laut Kopf ohne Quelltext-Gegenstück: `first_start.rs` Z27 („Bareos-
Gegenstück: der NSIS-Installer“) und `trust_acl.rs` Z44 („Bareos-Gegenstück: keines“).
`platform.rs` ist ungeklärt (Hilfsdatei).

Fragen: Wie ist der Nachbau des Datei-Durchlaufs (`find`, `find_one`) einzuordnen, wenn der
Algorithmus durch das Dateisystem und das Format der Attribute (LStat) weitgehend vorgegeben ist?
Fremdcode: `fnmatch.rs` stammt aus OpenBSD (BSD-Lizenz) über Bareos.

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/accept.rs` | 143 | b | Z6 "TCP accept loop - fd:accept thread (filed/socket_server.cc)", Z39 "filed/authenticate.cc OK_hello" |
| `src/accurate.rs` | 152 | b | Z6 "Accurate mode: filed/accurate.cc (AccurateCmd, AccurateCheckFile,"; 3 Verweise |
| `src/acl_xattr.rs` | 672 | b | Z6 "ACL and extended attribute restore on Linux: findlib/acl.cc"; 9 Verweise |
| `src/attribs.rs` | 409 | b | Z6 "Attribute records of a restore: lib/attr.cc (UnpackAttributesRecord,"; 6 Verweise |
| `src/backup.rs` | 1389 | b | Z6 "Backup — FD side: filed/backup.cc (BlastDataToStorageDaemon, SaveFile,"; 5 Verweise |
| `src/berrno.rs` | 97 | b | Z6 "lib/berrno.cc BErrNo::bstrerror(): the plain strerror() text in the"; 2 Verweise |
| `src/config.rs` | 563 | b | Z7 "(filed/filed_conf.cc: Client/FileDaemon, Director, Messages)."; 2 Verweise |
| `src/create_file.rs` | 599 | b | Z6 "findlib/create_file.cc CreateFile() and findlib/mkpath.cc makepath()."; 2 Verweise |
| `src/digest.rs` | 99 | b | Z6 "File digests (lib/crypto.cc crypto_digest_new/Update/Finalize,"; 2 Verweise |
| `src/fileset.rs` | 674 | b | Z6 "The FileSet the Director sends (`fileset` command): filed/fileset.cc"; 5 Verweise |
| `src/find.rs` | 586 | b | Z6 "Walking the FileSet: findlib/find.cc FindFiles()/OurCallback() and"; 3 Verweise |
| `src/first_start.rs` | 194 | c | Z27 "Bareos-Gegenstück: der NSIS-Installer des Windows-Clients" (kein Quelltext-Gegenstück): Vorlage der Konfiguration beim ersten Start unter Windows |
| `src/fnmatch.rs` | 277 | a | Z6 "fnmatch() of Bareos (core/src/lib/fnmatch.cc, OpenBSD 1.6) in Rust"; Fremdcode (BSD) mit eigener Lizenz |
| `src/jmsg.rs` | 184 | b | Z6 "Job messages of the File Daemon (lib/message.cc Jmsg()/Qmsg()) and the"; 4 Verweise |
| `src/job.rs` | 743 | b | Z6 "Director command loop of the File Daemon: filed/dir_cmd.cc"; 3 Verweise; 4 von 24 Textkonstanten identisch |
| `src/job/backup_cmds.rs` | 816 | b | Z6 "The backup side of the Director commands (filed/dir_cmd.cc LevelCmd,"; 2 Verweise; 4 von 16 Textkonstanten identisch |
| `src/main.rs` | 514 | b | Z9 "Drop-in replacement for the C++ bareos-fd"; 1 Fundstelle; Programmrahmen |
| `src/platform.rs` | 41 | ungeklärt | 41 Zeilen Plattformhilfen; Z22 nennt win32/compat/compat.cc wchar_2_UTF8 nur als Vergleich |
| `src/restore.rs` | 1115 | b | Z6 "Restore — FD side: filed/restore.cc DoRestore()."; 2 Verweise |
| `src/runscript.rs` | 256 | b | Z6 "Client run scripts: lib/runscript.cc (RunScripts, RunScript::Run),"; 4 Verweise |
| `src/scan.rs` | 157 | b | Z6 "lib/scan.cc bsscanf(): the `sscanf` of all Bareos daemons"; 2 Verweise |
| `src/sd.rs` | 323 | b | Z6 "Storage daemon connections of a job: filed/dir_cmd.cc StorageCmd()"; 2 Verweise |
| `src/status.rs` | 122 | b | Z8 "Compatible with C++ bareos-fd status output format", Z12/19 "same format as C++ FD" |
| `src/trust_acl.rs` | 361 | c | Z44 "Bareos-Gegenstück: keines; Bareos lädt keine signierten Plugins" |
| `src/version.rs` | 31 | b | Z6 "Version strings of the File Daemon (lib/version.cc kBareosVersionStrings)."; 1 Verweise |
| `src/winattr.rs` | 130 | b | Z20 "(`findlib/attribs.cc`, `encode_attribsEx`/`set_win32_attributes`)"; 1 Verweise |
| `src/winfs.rs` | 481 | b | Z10 "dort aus `WIN32_FILE_ATTRIBUTE_DATA` (`win32/compat/compat.cc`,"; 1 Verweise |

#### osd (Storage Daemon)

`crates/osd/src` (Director-Seite des SD-Protokolls) und `crates/osd/daemon` nennen in jedem Kopf
„Bareos-Gegenstück: stored/<Datei>.cc (<Funktionen>)“, z. B. `director.rs` Z21-26
(`dir_cmd.cc`, `AuthenticateDirector`, `job_cmd`, `use_cmd`), `writer.rs` Z28-31 (`AcquireDeviceForAppend`,
`WriteBlockToDev`), `reader.rs` Z30-32, `fd_session.rs` Z28-32 mit 22 von 24 identischen Meldungen.
Gegenbelege: `grpc.rs` Z31 („Bareos-Gegenstück: keins (Onesimus-Erweiterung)“), `ringproto.rs` Z28 („Bareos-Gegenstück: keins (bareos-sd schreibt im Job-Thread selbst)“) und `osd/ring/src/lib.rs` Z62-63 („keins. bareos-sd schreibt aus
dem Job-Thread direkt auf das Device“): die Bareos-Verweise dort grenzen nur ab. `osd-ring` ist laut
LIZENZEN.md ein eigenes, duales Crate, steht aber mit `lib.rs` in der Liste: ein Fehltreffer der
Suche nach `.cc`-Verweisen.

Fragen: Gilt der SD-Nachbau insgesamt als abgeleitet? Dürfen die drei Dateien mit „keins“ (und das
Crate `osd-ring`) aus der Liste, wenn ihr Quelltext nur auf Bareos-Dateien zur Abgrenzung verweist?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `osd/daemon/src/daemon.rs` | 476 | b | Z14 "Bareos-Gegenstück: `core/src/stored/stored.cc` (globale Daten,"; 3 Verweise |
| `osd/daemon/src/device.rs` | 246 | b | Z13 "Bareos-Gegenstück: `core/src/stored/backends/unix_file_device.cc`,"; 2 Verweise |
| `osd/daemon/src/director.rs` | 841 | b | Z21 "Bareos-Gegenstück: `core/src/stored/dir_cmd.cc`"; 7 Verweise; 6 von 17 Textkonstanten identisch |
| `osd/daemon/src/fd_session.rs` | 788 | a | Z28 "Bareos-Gegenstück: `core/src/stored/fd_cmds.cc` (`RunJob`,"; 5 Verweise; 22 von 24 Textkonstanten identisch |
| `osd/daemon/src/grpc.rs` | 128 | c | Z31 "Bareos-Gegenstück: keins (Onesimus-Erweiterung); die Sitzungen sind die von stored/socket_server.cc" |
| `osd/daemon/src/jobs.rs` | 394 | b | Z15 "Bareos-Gegenstück: `core/src/stored/job.cc` (`job_cmd`, `WaitFD`,"; 4 Verweise |
| `osd/daemon/src/main.rs` | 263 | b | Z15 "Argumente (wie `stored/stored.cc`):"; 2 Verweise |
| `osd/daemon/src/read_session.rs` | 673 | b | Z26 "Bareos-Gegenstück: `core/src/stored/read.cc` (`DoReadData`,"; 7 Verweise; 4 von 7 Textkonstanten identisch |
| `osd/daemon/src/reader.rs` | 601 | b | Z30 "Bareos-Gegenstück: `core/src/stored/acquire.cc`"; 6 Verweise |
| `osd/daemon/src/server.rs` | 281 | b | Z22 "Bareos-Gegenstück: `core/src/stored/socket_server.cc`"; 5 Verweise |
| `osd/daemon/src/status.rs` | 151 | b | Z13 "Bareos-Gegenstück: `core/src/stored/status.cc` (`OutputStatus`,"; 1 Verweise |
| `osd/daemon/src/transport.rs` | 109 | b | Z18 "Bareos-Gegenstück: `core/src/lib/bsock.h` (`BareosSocket` mit `recv`,"; 1 Verweise |
| `osd/daemon/src/writer.rs` | 894 | b | Z28 "Bareos-Gegenstück: `core/src/stored/acquire.cc`"; 7 Verweise; 5 von 14 Textkonstanten identisch |
| `osd/ring/src/lib.rs` | 826 | c | Z62-63 "Bareos-Gegenstück: keins. bareos-sd schreibt aus dem Job-Thread direkt auf das Device (stored/append.cc DoAppendData, stored/block.cc)": Verweise nur als Abgrenzung; Fehltreffer der Liste |
| `osd/src/askdir.rs` | 347 | b | Z13 "`odir/src/catreq.rs` (`dird/catreq.cc`)."; 5 Verweise |
| `osd/src/connect.rs` | 312 | b | Z14 "Ablauf (Bareos `dird/authenticate.cc` `AuthenticateWithStorageDaemon`):"; 1 Verweise |
| `osd/src/job.rs` | 260 | b | Z60 "The Job command of `p` (msgchan.cc jobcmd), formatted by"; 2 Verweise |
| `osd/src/lib.rs` | 57 | b | Z44 "Bareos-Gegenstück: `core/src/dird/authenticate.cc`"; 3 Verweise |

#### oconsole

Drei Dateien in der Liste. `main.rs` ist der Bareos-Konsolenablauf (Z286 `console_conf.cc`-Semantik,
Z484 `lib/bsock.cc:348`, Z511 „C++ Director sends NO signal after init“) mit 3 von 35 identischen
Textkonstanten: b. `mcp_tree.rs` und `mcp_write.rs` sind MCP-Werkzeuge der Onesimus-Erweiterung, die
Bareos-Befehle (`ua_tree.cc`, `ua_purge.cc`) nur als Vergleich nennen; `mcp_write.rs` trägt drei
Marker „Onesimus extension, not in Bareos“. Ungeklärt, Tendenz c.

Frage: Sind die MCP-Dateien als Teil der Konsole abgeleitet, weil sie Bareos-Befehle als Parameter
führen, oder als eigene Schnittstelle trennbar?

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/main.rs` | 2066 | b | Z286 "console_conf.cc semantics: `Director { Name Address DirPort Password }`"; 11 Verweise; 3 von 35 Textkonstanten identisch |
| `src/mcp_tree.rs` | 528 | ungeklärt | Z6-8 "the 26 commands of the Director restore tree (ua_tree.cc) behind restore_tree_*" (MCP-Werkzeuge der Onesimus-Erweiterung, Befehlsnamen von Bareos); 2 von 52 Textkonstanten identisch |
| `src/mcp_write.rs` | 1727 | ungeklärt | MCP-Schreibwerkzeuge (Onesimus-Erweiterung, 3 Markierungen "Onesimus extension, not in Bareos"); Z1157 nennt ua_purge.cc nur im Kommentar; 5 von 196 Textkonstanten identisch; Tendenz c |

#### onesimus-rig (Testgerüst, nicht ausgeliefert)

Sieben Dateien belegen sich als eigen: Z17-27 der Köpfe nennt als „Bareos-Gegenstück“ die `systemtests`
von Bareos als Analogie und als Vorlage das **eigene** Wegwerf-Gerüst `ofdrig.py` aus dem Gutachten
`docs/audits/ofd-struktur-2026-11-19.md` §3. Das ist eine Selbstauskunft. Das Gerüst enthält weder DDL
noch Konfigurationsvorlagen von Bareos im Quelltext; `catalog.rs` lädt das Schema **zur Laufzeit** aus
`/usr/lib/bareos/scripts/ddl/creates/postgresql.sql` (`prereq.rs` Z27, `catalog.rs` Z81). `console.rs` ahmt den `bconsole`-Ablauf nach
(b). `tests/pam_capture.rs` ist ein Schwarzkasten-Mitschnitt des installierten Original-`bconsole`.

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/catalog.rs` | 154 | c | Kopf: "Bareos-Gegenstück" ist die Analogie zu den systemtests von Bareos, die Vorlage ist das eigene Wegwerf-Gerüst ofdrig.py (docs/audits/ofd-struktur-2026-11-19.md §3); Selbstauskunft, nicht unabhängig geprüft |
| `src/config.rs` | 521 | c | Kopf: "Bareos-Gegenstück" ist die Analogie zu den systemtests von Bareos, die Vorlage ist das eigene Wegwerf-Gerüst ofdrig.py (docs/audits/ofd-struktur-2026-11-19.md §3); Selbstauskunft, nicht unabhängig geprüft |
| `src/console.rs` | 143 | b | Z17-18 "Bareos-Gegenstück: console/console.cc (bconsole) mit lib/bsock.cc und lib/cram_md5.cc": Anmeldeablauf wie bconsole |
| `src/lib.rs` | 48 | c | Kopf: "Bareos-Gegenstück" ist die Analogie zu den systemtests von Bareos, die Vorlage ist das eigene Wegwerf-Gerüst ofdrig.py (docs/audits/ofd-struktur-2026-11-19.md §3); Selbstauskunft, nicht unabhängig geprüft |
| `src/prereq.rs` | 168 | c | Kopf: "Bareos-Gegenstück" ist die Analogie zu den systemtests von Bareos, die Vorlage ist das eigene Wegwerf-Gerüst ofdrig.py (docs/audits/ofd-struktur-2026-11-19.md §3); Selbstauskunft, nicht unabhängig geprüft |
| `src/process.rs` | 125 | c | Kopf: "Bareos-Gegenstück" ist die Analogie zu den systemtests von Bareos, die Vorlage ist das eigene Wegwerf-Gerüst ofdrig.py (docs/audits/ofd-struktur-2026-11-19.md §3); Selbstauskunft, nicht unabhängig geprüft |
| `src/rig.rs` | 613 | c | Kopf: "Bareos-Gegenstück" ist die Analogie zu den systemtests von Bareos, die Vorlage ist das eigene Wegwerf-Gerüst ofdrig.py (docs/audits/ofd-struktur-2026-11-19.md §3); Selbstauskunft, nicht unabhängig geprüft |
| `src/tree.rs` | 202 | c | Kopf: "Bareos-Gegenstück" ist die Analogie zu den systemtests von Bareos, die Vorlage ist das eigene Wegwerf-Gerüst ofdrig.py (docs/audits/ofd-struktur-2026-11-19.md §3); Selbstauskunft, nicht unabhängig geprüft |
| `tests/pam_capture.rs` | 96 | ungeklärt | Z4-9 Mitschnitt des Dialogs vom installierten Original-bconsole (Schwarzkasten-Methode), nennt dird/auth_pam.cc; Tendenz c |
| `tests/rig.rs` | 186 | ungeklärt | Ende-zu-Ende-Läufe, Z168 nennt filed/dir_cmd.cc nur im Kommentar; Tendenz c |

#### onesimus-bnet-grpc

Fünf Dateien. `lib.rs` Z44 („Bareos-Gegenstück: keins (Onesimus-Erweiterung)“) und `client.rs` Z22 („keins;
der Aufbau ersetzt `BareosSocketTCP::connect`“) belegen c. `frame.rs`, `io.rs`, `server.rs` nennen
`lib/bsock_tcp.cc` und `lib/bnet_server_tcp.cc` als Gegenstück zur Rahmenform; sie transportieren
BNET-Rahmen, ohne Bareos-Logik zu enthalten. LIZENZEN.md führt das Crate als **eigen** (dual); die Liste
enthält es trotzdem mit allen fünf Dateien (Fehltreffer durch die Verweise). Abhängigkeit:
`onesimus-bnet-grpc` hängt von `bareos-bnet` (-only) ab.

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `src/client.rs` | 258 | c | Z22 "Bareos-Gegenstück: keins; der Aufbau ersetzt BareosSocketTCP::connect und den TLS-PSK-Handshake (lib/bsock.cc)" |
| `src/frame.rs` | 90 | ungeklärt | Z14 "Bareos-Gegenstück: der Längenkopf von lib/bsock_tcp.cc (positiv = Nutzdaten, negativ = Signal)": Rahmen ohne Längenkopf, Eigenaufbau über Proto; Tendenz c |
| `src/io.rs` | 298 | ungeklärt | Z29 "Bareos-Gegenstück: lib/bsock_tcp.cc (recv, send)": Adapter gRPC-Strom -> Read/Write, Eigenaufbau; Tendenz c |
| `src/lib.rs` | 75 | c | Z44 "Bareos-Gegenstück: keins (Onesimus-Erweiterung); die Rahmen entsprechen lib/bsock_tcp.cc (send, recv, signal)" |
| `src/server.rs` | 143 | ungeklärt | Z19 "Bareos-Gegenstück: lib/bnet_server_tcp.cc (BnetThreadServerTcp) für den BNET-Port" nur als Vergleich; gRPC-Server; Tendenz c |

#### Skripte und Tests (Einzeldateien)

`scripts/generate_configure_usage.py`, `scripts/generate_director_reference.py` lesen den Bareos-
Quelltext (`generate_configure_usage.py` Z23 `SOURCES = ('core/src/dird/dird_conf.cc', …)`, `generate_director_reference.py` Z107 liest `core/src/dird/*.cc`) und
erzeugen `configure_usage.rs`, `show_items.rs` und `docs/wiki/commands/*` (Klasse a); die Skripte selbst
enthalten keinen Bareos-Text. Die Tests unter `tests/` messen gegen ein laufendes Bareos
(`BAREOS_BUILD`, „C++ Director“). Alle sieben sind ungeklärt mit Tendenz c.

| Datei | Zeilen | Klasse | Beleg |
| --- | ---: | --- | --- |
| `scripts/generate_configure_usage.py` | 129 | ungeklärt | Werkzeug, das den Bareos-Quelltext einliest und Dateien der Klasse a erzeugt (liest core/src/dird/*.cc); das Skript selbst enthält keinen Bareos-Text; Tendenz c |
| `scripts/generate_director_reference.py` | 441 | ungeklärt | Werkzeug, das den Bareos-Quelltext einliest und Dateien der Klasse a erzeugt (liest core/src/dird/*.cc); das Skript selbst enthält keinen Bareos-Text; Tendenz c |
| `tests/benchmark_backup.py` | 516 | ungeklärt | Prüf-/Messwerkzeug gegen ein Bareos-Original (nennt "C++ Director", BAREOS_BUILD); 0 identische Textkonstanten; Tendenz c |
| `tests/compare_directors.py` | 679 | ungeklärt | Prüf-/Messwerkzeug gegen ein Bareos-Original (nennt "C++ Director", BAREOS_BUILD); 0 identische Textkonstanten; Tendenz c |
| `tests/stress/stress_escalation.sh` | 92 | ungeklärt | Prüf-/Messwerkzeug gegen ein Bareos-Original (nennt "C++ Director", BAREOS_BUILD); 0 identische Textkonstanten; Tendenz c |
| `tests/stress/stress_mixed.sh` | 84 | ungeklärt | Prüf-/Messwerkzeug gegen ein Bareos-Original (nennt "C++ Director", BAREOS_BUILD); 0 identische Textkonstanten; Tendenz c |
| `tests/volrecycle/rig.py` | 267 | ungeklärt | Prüf-/Messwerkzeug gegen ein Bareos-Original (nennt "C++ Director", BAREOS_BUILD); 0 identische Textkonstanten; Tendenz c |


### Befehlstexte und Protokolldefinitionen: Herkunft

| Bestand | Herkunft | Beleg |
| --- | --- | --- |
| `crates/odir/src/ua_commands_table.rs` | erzeugt aus `core/src/dird/ua_cmds.cc` `commands[]` (25.1.2) | Z1-2 „Generated from Bareos core/src/dird/ua_cmds.cc commands[] (25.1.2) by scratchpad gen_ua_commands.py“; 100 von 100 Zeichenketten (Hilfe und Syntax je Befehl) stehen so in `bareos-dir`; das Erzeugungsskript liegt nicht im Repository |
| `docs/wiki/commands/*` | je Befehl ein Block „Aus den Bareos-Originalquellen“ mit Originalbeschreibung und Originalsyntax, Quelle `ua_cmds.cc` mit Zeile, erzeugt von `scripts/generate_director_reference.py` (Bezugsstand `18be77e6`); `bareos-director-commands.json` mit allen 88 Befehlen | 89 von 104 Markdown-Dateien tragen den Block; Rest und Beispiele eigener Text (Herkunft nicht geprüft). Ist in `packaging/copyright` den Bareos-Urhebern zugeordnet |
| `proto/director_commands.proto` (Submodul, 1 240 Zeilen, 106 Nachrichten) | Z8-18: „Alle 88+ Commands aus ua_cmds.cc sind hier definiert“; je Befehl `help_text` und `usage` aus `ua_cmds.cc` | 85 von 105 `help_text`-Zeichenketten stehen so in `bareos-dir`; die `usage`-Zeichenketten (über mehrere Literale verteilt) konnten mit dem Verfahren nicht verglichen werden (0 von 61, vermutlich Verfahrensgrenze). Nach Herkunft a |
| `proto/director_api.proto` (2 539 Zeilen, 184 Nachrichten, 188 `rpc`) | typisierte Schnittstelle über den Konsolenbefehlen; eigener Aufbau (Onesimus), nennt Bareos nur in Kommentaren (ein Verweis, `ua_tree.cc`) | ungeklärt, Tendenz c; die Befehlsnamen sind die von Bareos |
| `proto/{director_service,stored_service,filed_service,usage,onesimus_options}.proto`, `*_broker.proto`, `plugin_ccc.proto`, `filed_plugin.proto` | gRPC-Dienste, Plugin- und Broker-Nachrichten, Onesimus-Erweiterungen; 0 bis 2 Verweise auf Bareos | ungeklärt, Tendenz c; LIZENZEN.md führt sie als eigen |
| `crates/bareos-orm/tests/data/bareos-18be77e6-creates-postgresql.sql` | Kopie der Bareos-DDL | ohne Kommentare und Leerraum gleich der Datei des Pakets 25.1.3 (`diff`: 0 Zeilen); a |
| `docs/upstream/U-001-…patch` | Patch gegen `core/src/dird/job.cc` | a (enthält Bareos-Zeilen als Kontext) |

Zu `proto`: Das Submodul ist im Worktree nicht ausgecheckt; gelesen wurde der Stand im Hauptverzeichnis
des Projekts (nicht notwendig der gepinnte Commit). Dort tragen **alle 12** `.proto`-Dateien
`AGPL-3.0-only` und die Zeile „2012-2026 Bareos GmbH & Co. KG“ im Kopf, auch die rein eigenen (z. B.
`filed_service.proto`, `*_broker.proto`). LIZENZEN.md nennt das Submodul `AGPL-3.0-or-later` seit
23.11.2026; der Stand hier zeigt das nicht. Die Einstufung je Datei (nur `director_commands.proto` ist
nach Belegen a, die anderen ungeklärt mit Tendenz c) gehört in das Repository `onesimus-proto`.

### Vorschläge zur Änderung der Einstufung (nur Vorschlag, nicht umgesetzt)

Die Liste, die Köpfe und die `Cargo.toml` bleiben unverändert. Vorschläge für den Maintainer und den
Juristen:

1. **In die Liste aufnehmen** (nach Prüfung): die 17 inhaltsgleichen `tests/bareos-dir.d/**/*.conf`
   und die 23 abweichenden Konfigurationen (a bzw. b), den Patch `docs/upstream/U-001-…`
   und prüfen: `crates/odir/src/state_file.rs`, `crates/ofd/src/streams.rs`.
2. **Aus der Liste nehmen** kämen nach Bestätigung der Selbstauskünfte in Frage (Klasse c mit
   Gegenbeleg im Kopf): `crates/osd/ring/src/lib.rs`, `crates/osd/daemon/src/grpc.rs`,
   `crates/osd/daemon/src/ringproto.rs`, `crates/onesimus-bnet-grpc/src/lib.rs`, `client.rs`,
   `crates/ofd/src/first_start.rs`, `trust_acl.rs`, `crates/odir/src/commands/volume_damaged.rs`
   (dort mit der Abhängigkeit von `purge.rs`) und die sieben Dateien von `onesimus-rig/src/` (c,
   Selbstauskunft). **Nicht** ohne Beleg: die ungeklärten Dateien mit Tendenz c
   (`odir/src/grpc/{mod,coverage,api_write}.rs`, `config_edit.rs` mit `config_edit/tests.rs`,
   `catalog_acl.rs`, `watchdog.rs`, `command_ast.rs`, `command_registry.rs`, `onesimus-bnet-grpc`
   `frame.rs`/`io.rs`/`server.rs`, `bareos-config/src/{onesimus,secrets}.rs`, `outils`): hier fehlt der
   Gegenbeleg; ihn kann nur der Maintainer liefern (Entstehung, „Bareos-Gegenstück: keins“ im Kopf).
3. **`bareos-catalog`** prüfen: nicht benutzt (siehe oben); Entfernen erspart die Einstufung.
4. **`bareos-orm` aufteilen**, wie LIZENZEN.md „Offene Trennungen“ vorschlägt: Schema und Testdaten
   (a) bleiben, Maschine (`schema.rs`, `heal.rs`, `codegen.rs`, `error.rs`, `lib.rs`; c mit
   Vorbehalt) nur nach Entscheidung des Juristen.
5. Die Tabelle „Bestandsaufnahme“ weiter oben in dieser Datei führt `onesimus-bnet-grpc`, `osd-ring`
   und die Dateien `grpc.rs`, `ringproto.rs` als eigen; die Liste führt sie als Bareos-nah. Diese
   Widersprüche sollten vor der Weitergabe an den Juristen geklärt werden.

### Was nicht belegbar war

- **Kein Bareos-Quelltext erreichbar.** Alle Einstufungen beruhen auf eigenem Quelltext,
  Kommentaren, installierten Bareos-Paketen (Version 25.1.3) und Dateinamen. Ob eine Datei Zeile für
  Zeile übersetzt wurde, ist nur dort belegt, wo der Kommentar es sagt oder C++-Ausdrücke
  mitgeführt sind (z. B. `base64.rs` Z99).
- **Kommentare sind Selbstauskünfte.** „port of“ und „Bareos-Gegenstück: keins“ stehen im Quelltext
  des Maintainers (und KI-unterstützt entstandener Teile, vgl. Rechtsfrage 3 oben); sie wurden nicht
  unabhängig geprüft.
- **Textvergleich nur gegen `bareos-dir`, `bareos-sd`, `bconsole`, `bareos-dbcheck` und die Bibliotheken
  25.1.3**, nicht gegen den FD und nicht gegen den Bezugsstand 25.1.2~pre43. Meldungen mit
  Platzhaltern, mehrzeilige und kurze Texte (< 20 Zeichen) fehlen; die Zahlen sind Untergrenzen.
  Gleicher Text beweist nicht, dass der Quelltext abgeschrieben wurde (das Verhalten ist Teil der
  Schnittstelle, die byte-genau nachgebaut werden sollte).
- **Herkunft der eigenen Beschreibungstexte** (`grammar_docs.rs`, Handbuchseiten, `docs/wiki/*.md`
  außerhalb der Bareos-Blöcke) wurde nicht geprüft.
- **Fremdcode in Bareos** (FastLZ, LZ4, CRC-32, OSSP var, OpenBSD fnmatch) und dessen Lizenzen wurde nur
  benannt, nicht untersucht.
- Die Gast-Plugins unter `plugins/` und die Crates `onesimus-plugins`, `-pg-broker`, `-ldap-broker`,
  `-podman-*`, `smartread` stehen nicht in der Liste und wurden nur auf Bareos-Verweise durchsucht
  (keine, außer allgemeinen Bezügen auf Bareos-Datenströme).
- Die Einstufungsschwellen (a: mindestens 8 Treffer und 40 %) sind eine Arbeitsregel dieser Tabelle,
  keine rechtliche Grenze.

## Nachtrag 23.11.2026: Umsetzung der Vorschläge 1 bis 3

Auf Freigabe des Maintainers (Weitergabe durch die GUI-Sitzung), Vorsicht vor Großzügigkeit:

- **Als `AGPL-3.0-only` mit den Bareos-Zeilen gekennzeichnet:** die 17 inhaltsgleichen
  `tests/bareos-dir.d/**/*.conf` (Kopf als `#`-Kommentar), `crates/odir/src/state_file.rs`,
  `crates/ofd/src/streams.rs`; in `scripts/ci/bareos-derived.txt` und `packaging/copyright` eingetragen.
  `docs/wiki/commands/*` und `docs/upstream/U-001-*.patch` stehen unter den Bareos-Urhebern in
  `packaging/copyright` (Markdown und Patch tragen keinen Kopf). Die 23 abweichenden `.conf` (b) bleiben
  ohne Kopf und sind weiter für den Juristen offen.
- **Aus der Liste genommen** (Kopf nennt „Bareos-Gegenstück: keins“, die Verweise grenzen nur ab):
  `crates/osd/ring/src/lib.rs` (damit `osd-ring` als Crate wieder rein `AGPL-3.0-or-later OR
  LicenseRef-Onesimus-Commercial`), `crates/onesimus-bnet-grpc/src/lib.rs` und `client.rs`.
  `frame.rs`, `io.rs`, `server.rs` von `onesimus-bnet-grpc` bleiben `-only`: ihre Köpfe nennen
  `bsock_tcp.cc` / `bnet_server_tcp.cc` als Gegenstück. Das Crate ist damit gemischt und behält den
  Ausdruck `(… OR …) AND AGPL-3.0-only`.
- **`bareos-catalog`:** nur vermerkt, nicht gelöscht; das Crate wird weiter von `odir` nicht benutzt
  (`Cargo.toml:27`), bleibt `-only`.
- **Fremdcode** (FastLZ, LZ4, OpenBSD fnmatch, OSSP var): siehe `THIRD-PARTY-NOTICES`.


## Nachtrag 24.11.2026: Submodul `proto`

Der Maintainer hat entschieden: Die fünf eigenen Definitionen `usage`, `podman_broker`, `postgresql_broker`, `openldap_broker` und `onesimus_options` stehen unter `AGPL-3.0-or-later` und tragen nur die Zeile von Jörg Bernau (Commit 47c5e7e in `onesimus-proto`). Alle anderen Dateien bleiben `AGPL-3.0-only` mit den Bareos-Zeilen. `onesimus-proto-rs` ist damit gemischt; solange es auch die `-only`-Dateien enthält, bleibt das Crate `AGPL-3.0-only`. Die Aufteilung für die dualen Crates (Abschnitt „Aufteilung“, Zeile `onesimus-proto-rs` / `proto`) ist offen: `filed_plugin.proto` und `plugin_ccc.proto` bleiben `-only`.
