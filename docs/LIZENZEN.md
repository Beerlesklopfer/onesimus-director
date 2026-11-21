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
| `AGPL-3.0-only` | von Bareos abgeleitete Teile | GNU AGPL v3, wie Bareos. Die Rechte der Bareos-Urheber (Bareos GmbH & Co. KG, Planets Communications B.V., Free Software Foundation Europe e.V. u. a.) bleiben bestehen; eine andere Lizenz ist nicht möglich. |
| `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | eigene Teile | Wahlweise AGPL v3 oder eine kommerzielle Lizenz des Maintainers ([LICENSE-COMMERCIAL.md](../LICENSE-COMMERCIAL.md), Text folgt). |

Der AGPL-Text liegt unverändert in [LICENSE](../LICENSE). Die Angabe steht je
Crate im Feld `license` der `Cargo.toml`; die Voreinstellung des Workspaces
(`[workspace.package] license`) bleibt `AGPL-3.0-only`, eigene Crates
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
| `osd-ring` (`crates/osd/ring`) | eigen | `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | Blockring im gemeinsamen Speicher; Modulkopf „Bareos-Gegenstück: keins. bareos-sd schreibt aus dem Job-Thread direkt auf das Device“. Keine Abhängigkeit von anderen Workspace-Crates. |
| `daemon-crate` | abgeleitet | `AGPL-3.0-only` | Daemon-Rahmen nach `lib/daemon.cc`, `include/exit_codes.h`, `filed/filed.cc`/`stored/stored.cc` `main`, `filed/authenticate.cc`, `lib/message.cc`. Das einzige eigene Modul (gRPC-TLS) ist seit 21.11.2026 in `onesimus-bnet-grpc::tls`. |
| `onesimus-bnet-grpc` | eigen | `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | BNET-Rahmen über gRPC mit TLS, Modulkopf „Bareos-Gegenstück: keins (Onesimus-Erweiterung)“; transportiert Bareos-Rahmen, portiert keine Bareos-Logik. Dazu `tls` (aus `daemon-crate`). |
| `onesimus-proto-rs` | gemischt | `AGPL-3.0-only` | Aus allen Protos des Submoduls erzeugt, darunter `director_commands.proto` („Auto-generated from ua_cmds.cc by scrape_commands.py“). |
| `onesimus-plugins` | eigen | `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | Signierte WebAssembly-Plugins, Ed25519/X.509-Vertrauenskette (`docs/plans/SIGNIER-INFRASTRUKTUR.md`); ersetzt die Bareos-Plugins, ohne deren Code (keine Verweise auf Bareos-Quellen). |
| `onesimus-pg-broker` | eigen | `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | Broker des PostgreSQL-Plugins (`pg_dump`/`psql`); keine Bareos-Verweise. |
| `onesimus-ldap-broker` | eigen | `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | Broker des OpenLDAP-Plugins (`slapcat`/`slapadd`); keine Bareos-Verweise. |
| `onesimus-podman-core` | eigen | `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | Podman-Plugin: Richtlinien, ID-Maps, Journal; nennt den Bareos-Strom nur als Ziel. Keine Abhängigkeit von anderen Workspace-Crates. |
| `onesimus-podman-broker` | eigen | `AGPL-3.0-only OR LicenseRef-Onesimus-Commercial` | Broker des Podman-Plugins; keine Bareos-Verweise. |
| `onesimus-rig` | gemischt (Testgerüst) | `AGPL-3.0-only` | Nicht ausgeliefert. Eigenes Gerüst, aber `console.rs` bildet den `bconsole`-Ablauf nach (`console/console.cc`, `lib/cram_md5.cc`), `catalog.rs` lädt das Bareos-Schema `postgresql.sql`. Vorsichtshalber AGPL; kann nach Prüfung dual werden. |
| Submodul `proto` | gemischt | `AGPL-3.0-only` (eigenes Repository, unverändert) | `director_commands.proto` aus `ua_cmds.cc` erzeugt; eigen sind u. a. `filed_plugin.proto`, `plugin_ccc.proto`, `*_broker.proto`, `onesimus_options.proto`. Lizenzänderungen gehören in das Repository `onesimus-proto`. |

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
  (`License: AGPL-3.0-only`) bleibt daher zutreffend. Offen für die
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
