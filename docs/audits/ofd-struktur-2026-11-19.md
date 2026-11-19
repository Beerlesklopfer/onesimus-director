# ofd-Struktur — Gutachten 2026-11-19

**Teilnehmer**: Trainee („Verstehe ich das ohne Vorwissen?“, kein Bareos-Wissen), Fachmann („Stimmt das?“, Quelltextvergleich mit Bareos 25 und Lauf am echten Gegenpart), Architekt („Trägt das als Ganzes?“, Abhängigkeiten und Cargo-Mechanik). Alle drei sind Agenten, Stand 19.11.2026, und haben getrennt voneinander gearbeitet, ohne die Berichte der anderen zu kennen. Diese Fassung führt die drei Berichte zusammen. Neue Befunde enthält sie nicht.
**Nordstern (Operator)**: ofd neu ordnen: `crates/ofd` als Schnittstelle (Protokoll, Typen, Client-Seite), `crates/ofd/daemon` als Implementierung, Plugins unter `crates/ofd/daemon/<name>`, dazu `crates/osd/daemon` als neuer Storage-Daemon nach demselben Muster, „damit sind die Schnittstellen entkoppelt“. odir und andere Nutzer hängen dann nur an Schnittstellen, nie an einem Daemon. Der osd-Daemon soll in der Nacht 20./21.11. entstehen (laut Architekt). Anlass: „11.500 Zeilen Rust, ein nicht mehr wartbarer Trümmer“.

Dies ist NICHT der Umsetzungsplan, sondern seine Grundlage.

**Messstände.** Trainee und Architekt haben bei Commit `c5e0218` gemessen. Beim Fachmann bewegte sich `development` während der Begutachtung: Er begann bei `c5e0218` und endete bei `7c77239`. Dazwischen liegen zehn Commits zum OpenLDAP-Plugin, +2412/−37 Zeilen in `crates/ofd`, im Kern davon nur `backup.rs` ±16 und `restore.rs` ±6. Jede Zahl unten nennt ihren Stand. Die Cargo-Mechanik hat der Architekt im Wegwerf-Workspace `/tmp/ofd-ws-probe` gemessen, das Gerüst der Fachmann im Wegwerf-Verzeichnis `/tmp/ofdrig`, beide außerhalb des Repositorys. Für Bareos-Funktionen ist der Vergleichsstand Branch `bareos-25` (lokal beim Fachmann unter `/tmp/bareos25-src/`).

**Lesehilfe.** **Mangel** heißt: Doku, Kommentar oder Plan sagen etwas Falsches oder Unvollständiges. **Defekt** heißt: Der Code hält nicht ein, was er soll (Bareos-25-Verhalten oder eigene Zusage). Defekte stehen gesammelt in §7. **Unbelegt** heißt: Die Berichte behaupten es, haben es aber nicht gemessen oder widersprechen sich, ohne dass sich die Frage klären ließ.

---

## §1 Anlass und Messung des Umfangs

**Befund zu der Zahl „11.500 Zeilen“** — Keine der drei Messungen trifft die Zahl, und ohne Datum und Zählweise lässt sie sich nicht nachprüfen.
**Fakt (gemessen 19.11.2026):**

| Stand | Zählweise | Ergebnis | Quelle |
|---|---|---|---|
| `c5e0218` | `wc -l crates/ofd/src/**/*.rs` | 12.503 Zeilen in 36 Dateien | Trainee |
| `c5e0218` | dasselbe plus `tests/plugin_check.rs` (38) und `build.rs` (48) | 12.589 | Fachmann, Architekt |
| `c5e0218` | Testcode ab dem ersten `#[cfg(test)]` je Datei (Näherung) / Kommentar- / Leerzeilen | 2.661 / 944 / 753, bleiben rund 8.100 Zeilen Code | Trainee |
| `7c77239` | `wc -l` über den Crate | 14.964 Zeilen in 41 Dateien | Fachmann |
| `c5e0218` → `7c77239` | Plugin-spezifischer Code (`plugin*`, `broker`, `pg_*`, `ldap_*`) | rund 2.500 Zeilen = 20 % → rund 4.650 Zeilen = 31 % | Fachmann |

Die größten Dateien bei `7c77239` (Fachmann): `backup.rs` 1233, `plugin.rs` 1216, `restore.rs` 1088, `ldap_port.rs` 1002, `job/backup_cmds.rs` 783, `compression.rs` 747, `job.rs` 713. Keine Datei hat mehr als 1250 Zeilen.
**Vermutung:** „11.500“ stammt aus einer früheren Messung oder einer anderen Zählweise (Trainee). Gewachsen ist vor allem das Plugin-System, nicht der FD-Kern (Fachmann).
**Impact:** Wer den Umbau begründet, sollte die Zahl mit Datum, Commit und Methode nennen. Der FD-Kern hat rund 10.000 Zeilen (Fachmann). Am schnellsten wächst der Plugin-Teil, und der liegt im selben Binary.
**NICHT die Ursache (ausgeschlossen):** „Gottdateien“ im Kern, denn keine Datei hat mehr als 1250 Zeilen (Fachmann). Nach dem Trainee erschwert auch die Größe allein das Verstehen nicht (§2).

**Befund zu Bau, Test und Lint** — ofd baut ohne Fehler und in Sekunden, und alle Tests sind grün.
**Fakt (gemessen 19.11.2026, Fachmann):**
- `cargo build -p ofd` inkrementell: bei `c5e0218` 7,5 s mit 5 Warnungen, bei `7c77239` 8,0 s mit 7 Warnungen. Alle Warnungen sind `dead_code` (`broker::launch`, `pg_port::…::allowed`, `plugin::…::open_resolved` und andere).
- `cargo test -p ofd`: bei `c5e0218` 70 bestanden, 4 ignoriert, dazu 1 Test in `tests/` (4,9 s Wandzeit). Bei `7c77239` 85 bestanden, 8 ignoriert, dazu 1 (6,0 s). Alle 8 ignorierten Tests betreffen Plugins, keiner den Datei-Backup/Restore-Kern. In drei Läufen fiel kein Test aus.
- `cargo clippy -p ofd`: bei `c5e0218` 38 Warnungen, bei `7c77239` 45, überwiegend Stil (17 × `collapsible_if`). 28 davon sind automatisch behebbar. Die 5 `missing_transmute_annotations` in den `dlsym`-Aufrufen (`compression::lzo_fn`, `acl_xattr`) sind fehlende Annotationen, keine falschen Typen.
- 11 Funktionen haben mehr als 100 Zeilen, 8 davon im Laufzeitcode. Die längsten: `restore::…::record` 198, `backup::…::save_file` 172, `main::main` 164, `fileset::set_options_and_flags` 144, `job::backup_cmds::…::backup_cmd` 124.

**Vermutung:** Die langen Funktionen sind 1:1-Übertragungen der gleichnamigen C++-Funktionen (`record` ↔ Hauptschleife in `restore.cc`, `save_file` ↔ `SaveFile`, `set_options_and_flags` ↔ `SetOptions`). Sie sind also bewusst bareos-treu und nicht verwildert (Fachmann).
**Impact:** Der Bau hält den Umbau nicht auf. Als Regressionsnetz taugen die Tests aber nur für das, was sie abdecken (§3).
**NICHT die Ursache (ausgeschlossen):** fehlschlagende oder instabile Tests. Ebenso wenig lässt sich „Trümmer“ mit Lint-Zahlen belegen.
Aufwand: Hausmeister (dead code), klein (Clippy).

**Widerspruch W1 (Befund, kein Streit): „Trümmer“.** Die drei Rollen bewerten den Anlass verschieden, und jede hat dafür Belege:
- Trainee: „Ohne Bareos-Vorwissen ist der Code nicht lesbar.“ „Nicht wartbar“ trifft aus seiner Sicht eher die Lesbarkeit als die Größe (§2).
- Fachmann: „‚Nicht mehr wartbarer Trümmer‘ stimmt nicht.“ Belege: Bau in 8 s, Tests grün, Backup und Restore gegen bareos-sd 25.1.3 bitgenau (§3).
- Architekt: Das größte Strukturproblem liegt nicht in ofd, sondern auf der Director-Seite: neun Handshake-Kopien, drei SD-Job-Formatierer (§4).
Zusammen ergibt das folgendes Bild. Fachlich ist der Kern funktionsfähig. Ohne Vorwissen ist er schwer lesbar. Und die Kopplung, die der Nordstern auflösen will, sitzt überwiegend außerhalb von ofd.

**Widerspruch W2 (Zählung, unbelegt):** Der Trainee zählt 31 `mod`-Zeilen in `main.rs`, der Architekt „alle 33 Module“, beide bei `c5e0218`. Bei den Tests zählt der Fachmann 85 + 8 ignoriert + 1 bei `7c77239`, der Architekt per grep „~94 `#[test]` + 8 `#[ignore]`“ bei angegebenem Stand `c5e0218`. Der Trainee kommt bei `c5e0218` auf 75 `#[test]`, was zu den 70 + 4 + 1 des Fachmanns passt. Woher die Abweichungen kommen (Zählweise, Untermodule, Arbeitsbaum), klären die Berichte nicht. Sie bleiben **unbelegt**.

### Verdikt §1
**Die Zahl „11.500“ ist nicht reproduzierbar (gemessen 12.503 bis 14.964 je nach Stand und Zählweise), und „nicht mehr wartbar“ lässt sich mit Bau-, Test-, Lint- und Größenzahlen nicht belegen.** Belegt ist ein schnell wachsender Plugin-Anteil (20 % → 31 % an einem Vormittag) im selben Binary wie der Kern.

---

## §2 Verständlichkeit (Trainee)

**Befund zu `main.rs` (Moduldoku)** — Im Crate gibt es keinen Einstieg. Module, Threads und Datenfluss sind nirgends zusammen dargestellt.
**Fakt (gemessen 19.11.2026, `c5e0218`):** Der `//!`-Kopf nennt drei Threads (`fd:accept`, `fd:session:N`, `fd:heartbeat`). Es fehlen `fd:grpc` aus `grpc::start()`, `spawn_blocking` je Verbindung in `grpc::Service::control()` und die Plugin-Worker aus `plugin::host()`. Danach folgen 31 `mod`-Zeilen, alphabetisch und ohne Gruppierung.
**Vermutung:** Ein Neuling kann die vier Arten von Modulen (Protokoll, Job-Ablauf, Dateisystem, Plugins) nicht auseinanderhalten.
**Impact:** Die Liste, welches Modul in die Schnittstelle und welches in den Daemon gehört, muss erst jemand erstellen. Der Architekt hat sie in §5 vorgelegt.
**NICHT die Ursache (ausgeschlossen):** fehlende Moduldoku. Alle 36 Dateien haben einen `//!`-Kopf. Es fehlt nur die Karte, die die Module zueinander in Beziehung setzt.
Aufwand: klein. Mangel.

**Befund zu den Modulköpfen** — Die Doku erklärt fast nur über Verweise auf den C++-Quelltext von Bareos, und der liegt nicht im Repository.
**Fakt (gemessen 19.11.2026, `c5e0218`):** 169 Kommentarzeilen haben C++-Bezug. 36 von 538 `///`-Zeilen bestehen nur aus einem C++-Namen, etwa `Session::backup_cmd()` („BackupCmd()“), `fileset.rs` `FOpts` („findFOPTS“) oder `restore.rs` `Rctx` („r_ctx“).
**Vermutung:** Wer Bareos kennt, bekommt eine präzise Zuordnung. Ein Neuling erfährt daraus nichts.
**Impact:** Wozu eine Funktion da ist, erschließt sich nur mit dem Bareos-Quelltext daneben.
**Widerspruch W3 (Befund):** Der Fachmann wertet genau diese Zuordnung als Stärke: „die Schnittstelle zur Bareos-Welt ist im Code lokal sichtbar und mit Funktionsnamen der C++-Vorlage dokumentiert“ (§3). Was für den Kenner ein Vorteil ist, versperrt dem Neuling den Zugang. Ob die Bareos-Spiegelung als Doku-Prinzip bleiben soll, entscheidet der Nutzer (§8).
Aufwand: mittel. Mangel.

**Befund zu Begriffen** — Die tragenden Bareos-Begriffe werden hundertfach benutzt und nirgends erklärt. Ein Glossar gibt es nicht.
**Fakt (gemessen 19.11.2026, `c5e0218`):** Treffer in `crates/ofd/src` gegen die beiden Wiki-Seiten `file-daemon.md` und `plugin-architektur.md`: `FO_*` 132/0, `statp` 78/0, `FfPkt` 53/0, `DIRBEGIN`/`DIREND` 40/0, `jcr` 25/0 („Job control record“ steht nirgends im Repository), `ticket` 20/1, `passive` 17/0, `Accurate` 12/0, `rechdr` 11/0. `grep -rli 'glossar|glossary' docs` liefert 0 Treffer. `job.rs` `Session` („The jcr of one Director connection“) erklärt also ein Unbekanntes durch ein anderes.
**NICHT die Ursache (ausgeschlossen):** BNET, TLS-PSK und Hello/CRAM erklärt das Wiki wenigstens im Groben.
Aufwand: klein (Glossar), mittel (Doc-Kommentare). Mangel.

**Befund zu Mehrdeutigkeiten** — Die Namensgebung hilft dem Neuling nicht weiter.
**Fakt (gemessen 19.11.2026, `c5e0218`):**
- „Session“ (195 Treffer) steht für sechs Dinge: `job.rs` `Session` (Job-Zustand), `plugin.rs` `Session` (Plugin-Instanz), SD-„append/read open session“, `vol_session_id`/`_time`, `session_id` (TCP-Nummer in `accept.rs`) und `grpc.rs` `sessions`. Dazu kommt das Director-Kommando `session`. `backup.rs` importiert `plugin::Session`, während `backup_cmds.rs` in `impl Session<'_>` aus job.rs arbeitet.
- „Handler“: `find.rs` `trait Handler` (Rückruf je Datei) und `job.rs` `type Handler` (Kommando-Handler).
- „Stream“: `BnetStream`/`SdStream` (Verbindung) gegen `streams.rs` (Satzart, etwa `STREAM_UNIX_ATTRIBUTES`).
- Optionen: `fileset.rs` `FOpts`, `fileset.rs` `FileOpts` und im Director `FileOptions` (`odir/src/commands/show.rs` `parse_options()`).
- „Include“ hat drei Lesarten (Block, Code `I`, Antwort `"2000 OK include\n"` für das ganze FileSet).
- Rückgabewerte im C-Stil haben drei Bedeutungen. `find::Handler::handle()` liefert „1 ok, 0 error, −1 ignore“, `Backup::save_file()` „1 ok, 0 error“, die Kommando-Handler `bool` mit `false` = „Schleife beenden“. Ein **erfolgreiches** `Session::backup_cmd()` endet deshalb mit `false`.
- `jmsg.rs` `JobMsgs::set_status()` überschreibt nur nach Priorität (`status_priority()`). Die Prüfung direkt nach `set_status(JS_TERMINATED)` in `backup_cmd()` ergibt nur so einen Sinn, und am Aufruf steht dazu kein Kommentar.
- „S-13“ hat zwei Bedeutungen. In `backup.rs` (Test `plugin_entry_mode_keeps_only_permission_bits`) steht es für „a plugin cannot create setuid/setgid/sticky files“, in `docs/STUBS.md` für „Eigener FD … Rest“. Woher das Sicherheits-S-13 stammt, ist **unbelegt**.

**Vermutung:** Wer die Status-Prüfung für überflüssig hält, entfernt sie. Das wäre eine stille Fehlsteuerung.
**NICHT die Ursache (ausgeschlossen):** ein Fehler im Code bei den Rückgabewerten. Die Semantik steht am Typ-Alias, nur nicht am Aufruf.
Aufwand: klein bis mittel (Umbenennen im Umbau). Mangel.

**Befund zum Nachvollzug eines Backups** — Den Ablauf kann man von `accept.rs` `accept_loop()` bis `bareos_bnet::BnetStream::send()` geradlinig verfolgen, aber nur gegen die Beschriftung.
**Fakt (gemessen 19.11.2026, `c5e0218`):** Verfolgte Kette: `accept_loop()` → `handle_connection()` (erstes Byte `0x16` = TLS-PSK) → `handle_session()` (gRPC tunnelt über `grpc.rs` `Service::control()` in **dieselbe** `handle_session()`) → `Session::run()` → `job_cmd()` → `level_cmd()` → `fileset_cmd()` → **`job.rs` `storage_cmd()`** → `backup_cmd()` → `blast_data()` → `Finder::find_files()` → `BackupHandler::handle()` → `save_file()` → `encode_and_send_attributes()` → `SdSender::send()`. Stolperstellen:
- Der Kopf von `job.rs` sagt, das Modul enthalte die Restore-Kommandos, die Backup-Seite liege in `job/backup_cmds.rs`. Das Backup braucht aber `job_cmd()` und `storage_cmd()` aus job.rs.
- Die Köpfe von `backup.rs` und `restore.rs` sagen, Plugins gebe es in diesem FD nicht. Dieselben Dateien enthalten `save_plugin()`, `run_plugin()`, `start_online()` und `write_online()`.
- `backup.rs` enthält auch `EstimateHandler` und `VerifyHandler`.
- `backup-protocol.md` zeigt `session VOLNAME …` im Backup und sagt für den Restore „NICHT gesendet“. Der Kopf von `job.rs` führt `session` dagegen unter Restore, und `open_sd_read_session()` liest aus `session_cmd()`. Welches stimmt, bleibt **unbelegt**: Der Trainee hat es an den Fachmann verwiesen, und dessen Bericht behandelt die Frage nicht.
- Was nach `BnetStream::send()` auf der Leitung steht (Rahmung in `bareos-bnet`), hat der Trainee nicht gelesen. Das ist **unbelegt**.

**Impact:** Der erste Satz, den man in `backup.rs` liest, ist falsch, und danach misstraut man jedem Kommentar.
**NICHT die Ursache (ausgeschlossen):** ein Fehler im Code. Falsch ist nur die Beschriftung.
Aufwand: klein. Mangel.

**Befund zur Aufgabenprobe** — Eine neue Ja/Nein-Option kann ein Neuling bauen. An einer Option mit Wert und an einer neuen Plugin-Operation scheitert er ohne fremde Hilfe.
**Fakt (gemessen 19.11.2026, `c5e0218`):**
- Option: `odir/src/commands/show.rs` `parse_options()` → `odir/src/commands/estimate.rs` `emit_item()` (`"O {fo.opts}"`) → `fileset.rs` `FileSet::add()` → `set_options_and_flags()` (Buchstabe → `FO_*`-Bit, Bareos-Nummerierung mit Lücken) → `accept_file()`. `FileOpts` reicht nur `flags`, `compress_algo` und `compress_level` weiter. Werte wie `strip_path` und `size_match` gehen über `find.rs` `FfPkt` einen anderen Weg. Welcher Weg für eine neue Option der richtige ist, steht nirgends. Der Buchstabenkatalog ist nicht dokumentiert. Ein unbekannter Buchstabe erzeugt nur `tracing::error!` und keine Job-Meldung.
- Plugin-Operation: Sie berührt `proto/filed_plugin.proto` (8 RPCs), `crates/onesimus-plugins/wit/filed-plugin.wit` (7 Exports, ohne `cancel`, begründet), `onesimus-plugins/src/wasm.rs` `enum Operation`/`enum Reply` und `ofd/src/plugin.rs` `Session`, dazu den Aufrufer und das Gast-SDK. Das Wiki nennt nur den Protobuf-Vertrag. Den WIT-Vertrag erwähnt es nicht (`grep` ergibt 0 Treffer).

**Vermutung:** Schickt ein neuerer odir eine neue Option an einen älteren ofd, wird sie dort still ignoriert und taucht nur im Daemon-Log auf.
**NICHT die Ursache (ausgeschlossen):** Die Lücken in der `FO_*`-Liste sind Bareos-Nummerierung und kein Fehler. Ob ein freies Bit bareos-kompatibel belegt werden darf, ist **unbelegt**. Auch dazu sagt der Fachmann-Bericht nichts.
Aufwand: mittel (Option), klein (Doku) bzw. groß (Plugin-Operation über drei Crates). Mangel.

**Befund zu Doku gegen Code und laufenden Dienst** — Die Doku beschreibt einen älteren ofd.
**Fakt (gemessen 19.11.2026, `c5e0218`):**
- `file-daemon.md` sagt „Ohne `--grpc-listen` startet nur der bisherige BNET-Listener“, `kommandozeile.md` sagt, `--grpc-cert` gelte „nur zusammen mit `--grpc-listen`“. `main()` startet gRPC dagegen standardmäßig auf Port+1000 („gRPC is the default control channel“, `grpc_tls::load_or_generate()`). `--disable-grpc` fehlt in `kommandozeile.md`. Am laufenden System lautet `ExecStart=/usr/sbin/ofd -f -c /etc/bareos/bareos-fd.d`, und `ss -tln` zeigt `*:10102` neben `*:9102`.
- `plugin-architektur.md` sagt, ein Broker „fehlt noch“, und der WASM-Host sei „noch umzusetzen“. Im Code stehen `broker.rs` (376 Zeilen), `pg_port.rs` und `pg_instance.rs`. Das Diagramm zeigt einen „administrierten Trust Store“, der Text sagt „einkompiliert“, und das Feld `ed25519_signature` trägt auch RSA-PSS.
- Der Kopf von `broker.rs` nennt „schema 2“. Laut Wiki laden Schema 1 und 2 nicht mehr, und der Test setzt `schema_version: 3`.
- `docs/STUBS.md` S-13 und `architektur-vergleich.md` führen Plugins in ofd als fehlend.
- `docs/backup-protocol.md` (398 Zeilen) ist die einzige Ablaufbeschreibung eines Backups, und keine Seite verlinkt sie (`grep` ergibt 0 Treffer).

**Impact:** Ein Betreiber glaubt, nur Port 9102 sei offen. Ein Neuling weiß nicht, was fertig ist, und erzeugt womöglich ein Manifest, das nicht lädt.
**NICHT die Ursache (ausgeschlossen):** ein Fehler im Code beim gRPC-Standard, denn das Verhalten ist beabsichtigt. Ob der Broker produktiv freigegeben ist, hat der Trainee nicht geprüft.
Aufwand: klein. Mangel.

### Verdikt §2
**Ohne Bareos-Vorwissen ist der Code nicht lesbar.** Es fehlen ein Einstieg, ein Glossar und eindeutige Namen, und die Doku widerspricht dem Code beim gRPC-Standard, bei den Plugins, beim Broker und bei der Schema-Version. Der Ablauf selbst ist geradlinig. Woran der Trainee scheitert, sind Begriffe, Doppelnamen und veraltete Köpfe, nicht die Größe.

---

## §3 Fachliche Richtigkeit und Testabdeckung (Fachmann)

**Befund zur Testabdeckung von `job::backup_cmds`** — Die Befehlsseite von Backup, Estimate und Verify ist praktisch ungetestet.
**Fakt (gemessen 19.11.2026, `cargo llvm-cov -p ofd --summary-only`, `7c77239`):** `job/backup_cmds.rs` 5,33 % (604 von 638 Zeilen nie ausgeführt), einziger Test dort ist `tests::commas`. `backup.rs` 37,4 %, `sd.rs` 41,8 %, `streams.rs` 18,0 %, `accept.rs` 24,5 %, `heartbeat.rs` und `grpc_tls.rs` 0 %. Die Gesamtzahl von 61,0 % ist geschönt, weil die eingebetteten Testmodule mitzählen. Es gibt genau einen Ende-zu-Ende-Test des Kerns, `job::tests::restore_session_with_simulated_director_and_sd`, und der läuft gegen einen nachgebauten SD (`fake_sd`). `tests/plugin_check.rs` prüft nur das Laden ohne CA. Der Trainee kommt bei `c5e0218` unabhängig zum selben Ergebnis: Kein Test durchläuft `backup_cmd()` mit simuliertem SD.
**Vermutung:** `backup_cmd`, `blast_data`, `estimate_cmd`, `verify_cmd`, `do_verify`, `do_verify_volume`, `level_cmd`, `fileset_cmd`, `accurate_cmd` und `run_scripts` erreicht kein Test.
**Impact:** Ein Umbau, der diese Pfade verschiebt, arbeitet ohne Sicherheitsnetz. Genau hier sitzen die Defekte D1 bis D3 (§7).
**NICHT die Ursache (ausgeschlossen):** fehlende Testinfrastruktur, denn `job::tests::fake_sd` existiert und könnte auch die Backup-Seite bedienen.
Aufwand: mittel.

**Befund zum Protokoll gegen Bareos 25** — Der Protokollkern ist eine sorgfältige, bareos-treue Übertragung mit benennbaren Lücken.
**Fakt (gemessen 19.11.2026, Quelltextvergleich, `7c77239`):**
- `job::Session::select` hat 24 Einträge, `dir_cmd.cc` `s_cmds[]` ebenfalls 24. In ofd fehlt `"resolve "`, und `".status"` liegt auf `status_cmd` statt auf `QstatusCmd` (D6, D7).
- Kernlogik: `backup::…::send_data` ↔ `SendDataToSd`/`SendPlainData` (gleiche Sparse-Bedingung, Nullblöcke nicht in der Prüfsumme), `accurate_cmd` ↔ `AccurateCmd`, `level_cmd` ↔ `LevelCmd` (10 BTIME-Abfragen, Warnung ab 600 s), `jmsg::status_priority` ↔ `jcr.cc::GetStatusPriority` identisch.
- Es fehlen Heartbeat (D4), die client-initiierte Verbindung, `Maximum Concurrent Jobs` (D9) und PKI (§7, Funktionslücken).
- `status::fd_status` ist teilweise Attrappe (D8).

**Impact:** Keine dieser Lücken ist durch einen Test als Lücke markiert.
**NICHT die Ursache (ausgeschlossen):** Reihenfolge und Präfixlogik der Befehlstabelle, die sind identisch zu Bareos.

**Befund zum Gerüst-Lauf odir + ofd + bareos-sd 25.1.3** — Gegen einen echten bareos-sd sichert ofd und stellt bitgenau wieder her. Das ist die erste reproduzierbare Messung dafür.
**Fakt (gemessen 19.11.2026, drei Läufe zu je ~27 s, `target/debug/ofd` gegen 05:20 aus dem Arbeitsbaum gebaut):** Wegwerf-Gerüst `ofdrig.py` im Scratchpad des Fachmanns, abgeleitet aus `tests/verify/rig.py`. Darin `target/debug/ofd --disable-grpc`, `target/debug/odir`, `/usr/sbin/bareos-sd` 25.1.3, Konsole `target/debug/examples/bnet_capture`, Ports 39391/39392/39393 auf 127.0.0.1, Katalog als Wegwerf-Datenbank `ofdrig_39391` aus `/usr/lib/bareos/scripts/ddl/creates/postgresql.sql`, ohne root. Danach hat der Fachmann alles abgeräumt (Prozesse, Datenbank, `/tmp/ofdrig`, Ports). Job-Tabelle von Lauf 3:

| JobId | Job | Typ/Level | Status | Files | JobBytes | ReadBytes |
|---|---|---|---|---|---|---|
| 1 | Backup (MD5, GZIP, Sparse, 8 Einträge inkl. Symlink und 300-KB-Nulldatei) | B/F | T | 8 | 239 | 37 934 |
| 2 | Verify VolumeToCatalog | V/O | T | 8 | 0 | 0 |
| 3 | Verify DiskToCatalog | V/d | **D** | 8 | 37 926 | 37 926 |
| 4 | Restore nach `/tmp/ofdrig/restore` | R/F | T | 8 | 37 934 | 0 |
| 5 | Backup 8 MiB, 200 kb/s, nach 4 s abgebrochen | B/F | A | 0 | 0 | 0 |

`diff -r --no-dereference` zwischen Quelle und Restore ergibt rc=0. Modus, Größe, mtime und Symlink-Ziel stimmen.
**Vermutung:** Als Skript unter `tests/` oder als `--ignored`-Test wäre das der fehlende Beleg für den Backup-Pfad und die erste Messlatte für osd. Der SD und der Director lassen sich je mit einer Zeile austauschen (`/usr/sbin/bareos-sd` → später `target/debug/osd`, `odir` ↔ `/usr/sbin/bareos-dir`).
**Impact:** Der Umbau kann mit einem Ende-zu-Ende-Netz beginnen statt blind.
**NICHT die Ursache (ausgeschlossen):**
- Die abweichende mtime des wiederhergestellten Symlinks ist bareos-konform (`findlib/attribs.cc::SetAttributes` setzt bei `FT_LNK` nur `lchown`, ebenso `attribs::set_attributes`).
- JobId 3, „MD5 digest differs“ bei `zeros.bin`, ist geerbtes Bareos-Verhalten und kein ofd-Defekt. Bei `Sparse = yes` lässt `send_data` die Nullblöcke aus der Prüfsumme weg, Verify rechnet die ganze Datei, und `backup.cc` tut dasselbe. Empirisch nicht gegengeprüft, weil bareos-fd nicht installiert ist. Das ist ein Mangel der Doku: Die Eigenheit gehört dokumentiert.
- Rechte: Alles lief als normaler Benutzer, und das installierte `ofd.service` blieb unangetastet.

Gemessene Abweichungen im Lauf: Abbruchstatus E statt A (D1), still verworfene Bandbreitenangabe `200k` (D5). Den Estimate-Pfad hat der Lauf nicht erreicht, weil odir den FD bei `estimate job=VBackup listing` gar nicht ansprach. `estimate_cmd` bleibt damit **unbelegt**, weder durch einen Test noch durch einen Lauf. Die Ursache liegt in odir und ist ungeklärt.
Aufwand: klein (Gerüst als Skript ins Repository).

**Befund zur vorhandenen Prüfbank und zum Stand-Papier** — Im Repository gibt es keinen lauffähigen Director+FD+SD-Test, und eine Erfolgsmeldung im Plan ist nicht reproduzierbar.
**Fakt (gemessen 19.11.2026, Fachmann und Architekt übereinstimmend):** `tests/verify/rig.py::run` und `tests/volrecycle/rig.py` starten hart `/usr/sbin/bareos-fd`. Das Binary fehlt (`bareos-filedaemon` hat Status `ic`, installiert ist `ofd 25.1.2+onesimus.0.2.0-1`). `docs/plans/STAND-2026-11-19.md` sagt „Echter Lauf gegen `odir` + `bareos-sd` 25.1.3 bestanden“, und dafür gibt es im Repository weder ein Skript noch ein Protokoll.
**Widerspruch W4 (Befund):** Der Architekt (Stand `c5e0218`) schreibt, es gebe „keinen lauffähigen Test“, und ob ofd die Rig-Konfiguration annimmt, sei „nicht gemessen“. Der Fachmann hat das danach gemessen: Mit seinem Wegwerf-Gerüst lief es dreimal. Beides stimmt. Im **Repository** gibt es weiterhin kein Gerüst. Dass es sich bauen lässt, ist jetzt belegt.
**Impact:** Wie sich ofd gegen einen echten SD verhält, war bis zum Lauf des Fachmanns nicht reproduzierbar belegt. Die Aussage im Stand-Papier ist ein Mangel.
**NICHT die Ursache (ausgeschlossen):** fehlende Programme. `bareos-sd` und `bareos-dir` 25.1.3 sind installiert, PostgreSQL 17 ist erreichbar.

**Befund zu Fehlerbehandlung und Nebenläufigkeit** — Die Fehlerbehandlung ist für einen Daemon solide.
**Fakt (gemessen 19.11.2026, `7c77239`):** Im Laufzeitcode stehen nur unfehlbare `try_into().unwrap()` auf Teilstücken fester Länge, `expect` beim Start und `expect("job registered"|"sd open")` auf einer Invariante. Vergiftete Mutexe behandelt der Code durchgehend mit `unwrap_or_else(|e| e.into_inner())`. `sd::JOBS` wird nur kurz gehalten, und verschachtelte Sperren gibt es nicht. `BwState::control` drosselt Lese- und Schreibrichtung gemeinsam (wie Bareos `ControlBwlimit`).
**Vermutung:** Panikt eine Sitzung, stirbt nur ihr Thread, aber der Eintrag in `sd::JOBS` bleibt liegen. `Builder::spawn(...).ok()` in `accept.rs` verwirft Fehler beim Thread-Start stillschweigend (D10).
**NICHT die Ursache (ausgeschlossen):** Lock-Vergiftung, `unwrap` auf Netzwerkdaten, Verklemmungen.

### Verdikt §3
**Fachlich ist der Kern korrekt: Gegen bareos-sd 25.1.3 laufen Backup und Restore bitgenau, und Verify VolumeToCatalog stimmt. Die Tests belegen das aber nicht (Backup/Estimate/Verify 5 % Abdeckung, kein Gerüst im Repository).** Es gibt benennbare Defekte und Lücken gegenüber Bareos 25 (§7).

---

## §4 Architektur heute: Abhängigkeiten, Parallelstrukturen (Architekt)

**Befund zum Workspace-Graph** — ofd hat keinen einzigen Abnehmer, und odir hängt an osd, aber nicht an ofd.
**Fakt (gemessen 19.11.2026, `cargo tree … --offline`, `c5e0218`):** ofd → bareos-bnet, bareos-config, onesimus-plugins, onesimus-proto-rs (240 Pakete transitiv). osd → bareos-bnet, bareos-config (128). odir → u. a. **osd** (228). bareos-bnet → **onesimus-proto-rs**, tonic, tokio (123). `cargo tree -i ofd -e all` findet keinen Abnehmer, `cargo tree -i osd` nur odir. `crates/ofd` hat kein `lib.rs`, und `Cargo.toml` enthält nur `[[bin]]`. Trainee und Fachmann haben das unabhängig ebenso gemessen.
**Vermutung:** Der Satz „odir hängt nur an der Schnittstelle“ ist für den FD schon heute erfüllt, aber nur deshalb, weil odir das FD-Protokoll selbst nachbaut.
**Impact:** Die FD-Schnittstelle muss **neu geschnitten** werden. Füllen lässt sie sich aus odir und bareos-bnet, nicht aus ofd.
**NICHT die Ursache (ausgeschlossen):** dev-dependencies. Mit `-e all` nachgemessen, auch dort gibt es keinen Abnehmer.

**Befund zu `bareos_bnet::fd_grpc` / `stream::AnyStream::Grpc`** — Die Wire-Bibliothek zieht den gesamten gRPC-Stapel und alle Protos in jeden Nutzer, auch in osd.
**Fakt (gemessen 19.11.2026, `c5e0218`):** tonic kommt in osd nur über bareos-bnet (osd 128 Pakete, bareos-bnet allein 123). `onesimus-proto-rs` (`build.rs`) kompiliert director_service, director_commands, director_api, plugin_ccc, filed_service, filed_plugin und postgresql_broker in **ein** Crate.
**Impact:** Die Entkopplung wird an der untersten Schicht unterlaufen. Jede Protoänderung am DirectorApi baut ofd, osd und die Broker neu.
**NICHT die Ursache (ausgeschlossen):** osd selbst, denn es nutzt kein gRPC.
Aufwand: mittel.

**Befund zu odir → osd** — odir nutzt osd an 8 Stellen, und der Modulkommentar in osd sagt das Gegenteil.
**Fakt (gemessen 19.11.2026, `c5e0218`):** `osd::connect::connect_sd` (u. a. `commands/status.rs` `native_storage_status`), `osd::command::sd_control_command`, `sd_query_commands`, `osd::job::{sd_job_setup, sd_use_device, sd_send_bootstrap}`. Für `osd::catrec` und `osd::status::query_sd_status` gibt es keinen Aufrufer. `crates/osd/src/lib.rs` behauptet „`connect` … nicht von odir genutzt“, und das ist falsch (Mangel). Die Abhängigkeit `bareos-config` in `crates/osd/Cargo.toml` ist tot.
Aufwand: klein / Hausmeister.

**Befund zu Formatpaaren, die odir als Text kennt** — odir kennt keine Interna von ofd, wohl aber dessen Text-Ausgaben, und zwar ohne gemeinsamen Typ.
**Fakt (gemessen 19.11.2026, `c5e0218`):** `odir/src/fd_progress.rs::parse` gegen `ofd/src/status.rs::fd_status`, `backup_worker.rs::parse_end_job` gegen `2800 End Job`, `job_worker.rs::wait_for_fd_end_job` gegen `2000 OK storage end`. Auf der ofd-Seite steckt das Director-Protokoll als 25 `scan()`-Aufrufe und 80 Antwortliterale in den Handlern (Trainee).
**Vermutung:** An diesen Stellen bricht ein ofd-Umbau odir unbemerkt.
**Impact:** Diese Formatpaare gehören in die Schnittstelle: Erzeuger und Parser im selben Crate, mit einem Test.

**Befund zu Parallelstrukturen** — Oberhalb von bareos-bnet existiert fast alles als Kopie.
**Fakt (gemessen 19.11.2026, `grep -rn "fn <name>"`, `c5e0218`):**
- Director-Client-Handshake **neunmal**. FD: `odir/src/fd_protocol.rs::do_fd_status`, `::connect_fd_with_timeout`, `::connect_fd_mode`, `job_worker.rs::connect_fd_mode_on`, `::connect_fd_on`, `fd_grpc.rs::try_connect`. SD: `osd/src/connect.rs::connect_sd`, `osd/src/status.rs::query_sd_status` (ungenutzt), `job_worker.rs::connect_sd_on`. `connect_sd` prüft `SD_OK_HELLO`, `connect_sd_on` verwirft die Antwort und setzt keinen Lese-Timeout (D12).
- SD-Job-Kommando **dreimal**: `osd/src/job.rs::sd_job_setup`, `odir/src/sd_copy.rs::format_job_command`, inline in `odir/src/verify.rs`. Die osd-Fassung ist fehlerhaft (D11).
- BashSpaces mit zwei Semantiken: `bareos_bnet::cram_md5::bash_spaces` (`' '` → `'_'`, im Hello) gegen `odir/src/catreq.rs::bash_spaces`, `sd_label.rs::bash_spaces` und `sd_copy.rs::bash` (`0x01`). Dazu kommen `ofd/src/job.rs::unbash` und `odir/src/job_messages.rs::unbash_spaces` (D13).
- Konfigurationsparser: `ofd/src/config.rs` (`FdConfig::parse`, `strip_comments`, `walk_conf_dir`) neben `bareos-config/src/parser.rs` und `raw.rs::load`. Von bareos-config nutzt ofd nur `secrets`.
- Formatkopien: `ofd/src/breg.rs` („Same code as odir/src/breg.rs, on bytes“), LStat-Decoder dreimal, Bareos-Base64 viermal oder öfter, Stream-IDs in `ofd/src/streams.rs` gegen `odir/src/verify.rs`, `M_FATAL` sechsmal (5 × in odir als `u32`, 1 × in ofd als `i32`).
- gRPC-TLS-Zertifikat zweimal (`ofd/src/grpc_tls.rs::load_or_generate`, `odir/src/grpc/tls.rs`), Daemonize zweimal.
- Plugin-Protos in Gästen von Hand nachgebaut (`plugins/postgresql/src/lib.rs`: „filed_plugin.proto (subset)“ mit eigenen `#[derive(prost::Message)]`).

**Vermutung:** Feldnummern der Plugin-Protos können auseinanderlaufen, ohne dass ein Build bricht. Ohne gemeinsame Crates bekommt der osd-Daemon einen dritten Parser, eine dritte Accept-Schleife und eine vierte LStat-/Base64-Kopie.
**Impact:** Diese neun Handshakes in die beiden Schnittstellen zusammenzuführen, ist der Kern des Umbaus. Ohne das bleibt „odir hängt nur an der Schnittstelle“ ein Etikett.
**NICHT die Ursache (ausgeschlossen):**
- Der Konsolen-Handshake in `oconsole/session.rs` hat eine andere Rolle und ist bewusst nicht mitgezählt.
- Doppelte TLS-Wege (openssl-PSK in `bareos_bnet::tls_psk` neben rustls/tonic) sind gewollt, weil rustls kein PSK kann. Zusammenzulegen ist nur die rcgen-Zertifikatserzeugung.

Aufwand: mittel (Handshakes, Parser, Proto-Crate), klein (Formatkopien, SD-Job-Kommando).

### Verdikt §4
**Der Graph ist flach und ohne Zyklus, aber die Schnittstellen existieren nicht als Crates. Die FD-Seite hat 0 Zeilen Schnittstelle, die SD-Seite 548 Zeilen Client-Bibliothek, davon die Hälfte ungenutzt.** Das größte Strukturproblem liegt auf der Director-Seite: neun Handshake-Kopien, drei SD-Job-Formatierer (einer fehlerhaft), zwei BashSpaces-Semantiken. Dazu zieht bareos-bnet über `fd_grpc` tonic und alle Protos in jeden Nutzer.

---

## §5 Zielbild Schnittstelle/Daemon/Plugins/osd und Cargo-Mechanik

**Befund zur Zuordnung `crates/ofd` ↔ `crates/ofd/daemon`** — Die Zuordnung ist eindeutig machbar. Etwa 95 % von ofd gehören in den Daemon.
**Fakt (gemessen 19.11.2026, Architekt, `c5e0218`):** Die Grundregel lautet: In die Schnittstelle gehört jedes Formatpaar, dessen eine Seite in odir und dessen andere in ofd liegt, dazu der Client-Verbindungsaufbau. Was ofd **und** osd brauchen, kommt in ein neutrales Crate.

| heute | Ziel |
|---|---|
| `main.rs`, `version.rs`, `heartbeat.rs`, `grpc.rs`, `backup.rs`, `restore.rs`, `find.rs`, `scan.rs`, `create_file.rs`, `acl_xattr.rs`, `accurate.rs`, `compression.rs`, `runscript.rs`, `berrno.rs`, `breg.rs`, `plugin*.rs`, `broker.rs`, `build.rs`, `pg_*.rs`, `tests/plugin_check.rs` | Daemon |
| `job.rs`, `job/backup_cmds.rs` | Daemon. Die **Befehlsgrammatik** (`JobId=… Job=… SDid=…`, `2000 OK Job`) kommt in die Schnittstelle |
| `fileset.rs` | Daemon. Das Include/Exclude-Zeilenformat kommt in die Schnittstelle (Vermutung) |
| `status.rs` | Daemon. Ausgabeformat und Parser (`odir/src/fd_progress.rs::parse`) kommen in die Schnittstelle |
| `streams.rs`, `attribs.rs`, `digest.rs`, `M_*` aus `jmsg.rs` | neutral (`bareos-format`) |
| `accept.rs`, `grpc_tls.rs`, `BwState`/`Limited` aus `sd.rs` | Daemon-Gerüst (`onesimus-daemon`) |
| `config.rs` | Parser nach `bareos-config` |
| `sd.rs` (`connect_storage`, `handle_stored_connection`) | Daemon, nutzt die **osd-Schnittstelle** |
| aus bareos-bnet: `fd_grpc` (`connect`, `pair`, `ControlIo`) | Schnittstelle ofd, damit bareos-bnet tonic-frei wird |
| aus odir: `fd_protocol.rs` (Connect-Varianten, `do_fd_status`, `fd_query_commands`), `fd_grpc.rs::try_connect`, `job_worker.rs` (`fd_job_setup`, `fd_restore_setup`, `end_job_of`), `backup_worker.rs::parse_end_job`, `fd_progress.rs::parse` | Schnittstelle ofd. Director-Zustand (`SUCCESSFUL_HANDSHAKE`, Zielregister, Retry-Texte) bleibt in odir |

Trainee und Fachmann kommen unabhängig zum selben Schluss: Eine Schnittstelle zum Herauslösen gibt es heute nicht. Der gRPC-Kanal ist ein Transport für dieselben BNET-Nachrichten und keine typisierte Schnittstelle (Trainee).
**Impact:** Der Umbau ist überwiegend **Arbeit in odir und bareos-bnet**, nicht in ofd. Das Verschieben von ofd selbst ist mechanisch billig.

**Befund zum FD→SD-Datenprotokoll** — Es gehört weder allein in die ofd- noch allein in die osd-Schnittstelle.
**Fakt (gemessen 19.11.2026, Architekt):** Sender ist `ofd/src/backup.rs` (Records mit Stream-IDs aus `streams.rs`), Empfänger beim Restore `restore.rs`. Der Verbindungsaufbau läuft über `sd.rs::connect_storage` (PSK-Identität `R_JOB…`) bzw. passiv über `handle_stored_connection`.
**Vermutung:** Die Record-Ebene ist neutral (`bareos-format`), der Verbindungsaufbau „Client zum SD“ Teil der osd-Schnittstelle. Der Fachmann schlägt unabhängig davon ein gemeinsames Stromformat-Crate für ofd und osd vor (Stream-IDs, `rechdr`, LStat, Sparse-Header, Kompressionsrahmen), als „natürlicheren Schnittstellen-Teil als eine Trennung `crates/ofd` ↔ `crates/ofd/daemon`“. Hier stimmen die beiden Rollen überein.
**Impact:** ofd-daemon wird an der osd-Schnittstelle hängen. Das entspricht dem Nordstern, muss aber bewusst entschieden werden.
Aufwand: mittel.

**Befund zu gemeinsamen Crates (Vorschlag des Architekten, Namen sind Vorschläge)**:
1. `bareos-bnet`: nur Rahmen, Signale, TLS-PSK/Zertifikat, CRAM-MD5 und Hello, **ohne** tonic/proto. `fd_grpc` wandert hinaus, `AnyStream::Grpc` wird über Trait/Box gelöst statt als Enum-Variante.
2. `bareos-format`: Stream-IDs, Record-Header, LStat, Bareos-Base64, Digest-Kennungen, `M_*`, BashSpaces, später Volume-/Blockformat. Ohne IO.
3. `bareos-config`: Director, FD, SD, mit `raw.rs` als Unterbau.
4. `onesimus-daemon`: Accept-Schleife mit PSK-Auflösung, Server-Handshake, gRPC-TLS, Daemonize/Signale/Logging, Bandbreitenbegrenzer.
5. Protos aufgeteilt: `onesimus-proto-director`, `-filed`, `-plugin`, später `-stored`.

**Impact:** Liegen `bareos-format` und `onesimus-daemon` vor dem osd-Bau (20./21.11.) nicht wenigstens als Hülle vor, verdreifacht sich die Parallelstruktur.

**Befund zu Plugins unter `crates/ofd/daemon/<name>`** — Ein Plugin hat drei Teile mit drei Build-Welten. Als **Ordner** passt `<name>`, als ein Crate nicht.
**Fakt (gemessen 19.11.2026, Architekt):** Der Gast hat ein anderes Ziel (wasm32-wasip2) und eine andere Toolchain (`cargo +1.99.1`). Er hat ein eigenes `[workspace]`, `cdylib` und ein eigenes `Cargo.lock`. Der Broker ist ein natives Binary im Haupt-Workspace (`crates/onesimus-pg-broker`). Der Host-Port (`pg_port.rs`, `pg_instance.rs`, `ldap_port.rs`) ist Teil des Daemon-Binarys und stützt sich auf `pub(crate)`-APIs. Das Paket `ofd-plugin-postgresql` bündelt Gast und Broker, aber nicht den Host-Port. WIT ist die Schnittstelle zwischen Daemon und Gast, und die Gäste lesen sie über einen relativen Pfad (`wit_bindgen::generate!({ path: "../../crates/onesimus-plugins/wit" })`).
**Vorschlag:** `crates/ofd/daemon/<name>/{guest (eigener Workspace, exclude), broker (Mitglied), packaging, tests}`. Der Host-Port bleibt Modul im Daemon (Vermutung). `guest-runtime` wandert nach `crates/ofd/daemon/plugin-sdk/`. `onesimus-plugins` bleibt eigenes Crate. Für den Plugin-Vertrag gibt es ein eigenes Proto-Crate, ob es no_std-tauglich ist, ist **unbelegt**.
**Widerspruch W5 (Befund): erster Schnitt.** Der Fachmann empfiehlt als ersten Schnitt „Plugin-/Broker-Code ↔ FD-Kern“, weil dort das Wachstum sitzt (§1). Der Architekt legt das Verschieben der Plugins in seiner Reihenfolge ganz ans Ende (Schritt 7, nach dem OpenLDAP-Merge) und setzt die Schnittstellenarbeit in odir und bareos-bnet davor. Beide begründen das. Der Nutzer entscheidet (§8).

**Befund zur Cargo-Mechanik** — Ein Crate im Unterordner eines anderen ist als Workspace-Mitglied sauber. Die gefährlichste Folge ist aber still: `-p ofd` meint danach die Schnittstelle.
**Fakt (gemessen 19.11.2026 im Wegwerf-Workspace `/tmp/ofd-ws-probe`, cargo 1.99.0, cargo-deb 3.7.0):**

| # | Messung | Ergebnis |
|---|---|---|
| M1 | `cargo build` mit lib `ofd` + Paket `ofd-daemon` (bin `ofd`), ebenso osd | baut |
| M2 | Daemon-Paket ebenfalls `name = "ofd"` | Fehler „two packages named `ofd`“: Der Paketname muss abweichen, der Binary-Name darf bleiben |
| M3 | `cargo test -p ofd` | läuft **nur** die Schnittstellen-Tests, **ohne Warnung** |
| M4 | `cargo build --release -p ofd` | „Finished“, aber **kein** `target/release/ofd`, ohne Warnung |
| M5 | `… -p ofd --bin ofd` | Fehler mit Hinweis auf `ofd-daemon` |
| M6 | `cargo deb -p ofd` | Fehler „No binaries or cdylibs found“ (laut) |
| M7/M8 | `cargo deb -p ofd-daemon` mit `../../../` statt `../../` | korrekt bzw. laut fehlerhaft. `target/…` ist relativ zum Workspace, alles andere relativ zum Paketordner |
| M9 | `cargo doc --workspace` | Warnung „output filename collision“ (lib `ofd` gegen bin `ofd`), ebenso osd |
| M10 | `cargo package -p ofd --list` | schließt `daemon/` aus |
| M11 | Gast mit eigenem `[workspace]`, nicht in `members` | Haupt-Build unberührt, Gastbau erzeugt `.wasm` |
| M12–M13 | Gast in `members` bzw. Glob `plugins/*` | Fehler „multiple workspace roots“ bzw. „failed to read Cargo.toml“ |
| M14 | `members = [… "…/plugins/*/broker"]` + `exclude` für Gäste | baut |

Von `-p ofd` betroffene Stellen: `.github/workflows/package.yml` (Tests, Release-Build, `cargo deb`), `README.md`, `docs/plans/STAND-2026-11-19.md`, `docs/plans/SIGNIER-INFRASTRUKTUR.md`, `plugins/*/README.md` und der Doc-Kommentar in `ofd/src/plugin.rs`. Pfade, die an der Tiefe hängen: `[package.metadata.deb]` in `crates/ofd/Cargo.toml`, die Tests `guest_wasm` und `pg_setup` in `ofd/src/plugin.rs` (`../../target/plugins`), `wit_bindgen`-Pfade der Gäste, `packaging/ofd-plugin-postgresql/build-deb.sh` (`top=$here/../..`), `scripts/generate_director_reference.py` und `docs/wiki/commands/bareos-director-commands.json` (Pfad auf `crates/osd/src/command.rs`). Dazu kommt `onesimus-plugins` per Pfad statt über `workspace.dependencies`. `build.rs` (`ONESIMUS_PLUGIN_CA`, gelesen von `plugin/trust.rs::builtin`) muss mit dem Daemon wandern, und im Unterordner wirkt es unverändert (M1).
**Vermutung:** Die CI bliebe grün, während die Daemon-Tests nicht mehr laufen. Erst `cargo deb` bräche. Die Testpfade brechen nur in `#[ignore]`-Tests, also unbemerkt. Bliebe `build.rs` in der Schnittstelle, würde jede Änderung von `ONESIMUS_PLUGIN_CA` odir neu bauen. Eine Abhilfe für die Pfade wäre `.cargo/config.toml` `[env]` mit `relative = true`, das ist aber nicht gemessen und **unbelegt**.
**Impact:** Ohne gleichzeitige Anpassung aller `-p ofd` verliert das Projekt still seine FD-Tests.
**NICHT die Ursache (ausgeschlossen):** cargo-deb, denn es bricht laut (M6/M8).
Aufwand: klein.

**Verworfen (bleibt mit Begründung stehen):**
- **Ein Proto-Crate mit Cargo-Features je Proto** (Architekt). Features sind additiv und werden im Workspace vereinigt, also bliebe die Kopplung beim gemeinsamen Build bestehen. Die Begründung ist eine Vermutung, nicht gemessen.
- **Daemon-Paket ebenfalls `ofd`** (M2). Cargo lehnt zwei gleichnamige Pakete ab.
- **Plugin `<name>` als ein Crate** (Architekt). Es scheitert an der Toolchain-Trennung zwischen Gast (wasm32-wasip2, `+1.99.1`) und Broker/Host.
- **Gäste in `members` oder per Glob** (M12, M13). Das führt zu „multiple workspace roots“ bzw. Lesefehlern.
- **„Erst trennen, dann testen“** (Fachmann §7). Dabei würde Code mit 5 % Abdeckung ohne Netz verschoben.
- **Als verworfene Variante markiert, bleibt aber als Option stehen:** Die Schnittstelle heißt `ofd-proto`/`ofd-api`, der Daemon behält den Paketnamen `ofd`. Dann bliebe `-p ofd` richtig, und M3/M4 entfielen vollständig. Gegen die Vorgabe „`crates/ofd` = Schnittstelle“ verstößt das nur im Paketnamen, nicht im Pfad. Die Entscheidung liegt beim Nutzer (§8).

### Verdikt §5
**Das Zielbild trägt, aber als Neubau der Schnittstelle, nicht als Verschieben: `crates/ofd` muss aus odir und bareos-bnet zusammengeführt werden, und bareos-bnet muss tonic-frei werden, sonst sind die Schnittstellen nur dem Pfad nach entkoppelt.** Die Cargo-Mechanik ist gemessen unkritisch, bis auf eine stille Falle (`-p ofd`), die sich mit einer Namensentscheidung beheben lässt. Plugins passen als Ordner mit getrennten Workspaces.

---

## §6 Empfohlene Reihenfolge

Grundsatz, in dem Fachmann und Architekt übereinstimmen: erst das Netz, dann reine Funktionen, dann Verbindungscode, zuletzt die Ordner verschieben. Jeder Schritt soll für sich baubar und testbar sein.

| # | Schritt | testbar durch | Kollision mit laufender Arbeit | Aufwand |
|---|---|---|---|---|
| 0 | Gerüst Director+FD+SD ins Repository (Fachmanns `ofdrig.py` als Vorlage, SD und Director als austauschbare Parameter: bareos-sd jetzt, osd später) | sich selbst, vor und nach jedem Schritt | keine | klein |
| 0a | Tests für `job/backup_cmds.rs` mit `job::tests::fake_sd` (Backup, Estimate, Verify) | `cargo test` | keine | mittel |
| 1 | Hausmeister: osd-Kommentar, tote `bareos-config`-Abhängigkeit, `onesimus-plugins` über `workspace.dependencies`, Testpfade über die Workspace-Wurzel | `cargo test` | `plugin.rs`-Tests | Hausmeister |
| 2 | `bareos-format` anlegen, LStat/Base64/Stream-IDs/`M_*`/BashSpaces/breg dorthin verschieben | `attribs.rs::decode_stat_like_original`, bvfs/ovfs-Tests, Gerüst | `backup.rs`/`restore.rs` | klein |
| 3 | `fd_grpc` aus bareos-bnet in das neue `crates/ofd` (lib), `AnyStream::Grpc` ersetzen | `cargo tree -p bareos-bnet -i tonic` leer, gRPC-Tests in ofd und odir | keine | mittel |
| 4 | Schnittstelle ofd füllen: ein Client-Connect (6 → 1), Formatierer/Parser (`fd_job_setup`, `parse_end_job`, `fd_progress::parse`) | Unit-Tests je Formatpaar, Gerüst | odir-Worktree `agent-abbb161c62bdb14ac` (uncommitted Änderungen in `commands/purge.rs`, `catreq.rs`, `main.rs`) | mittel |
| 5 | Schnittstelle osd: `connect_sd` (3 → 1), ein Job-Kommando-Formatierer (D11 beheben), FD→SD-Client (`R_JOB`) | Gerüst (Restore-Pfad) | **osd-Neubau 20./21.11.** | mittel |
| 6 | ofd nach `crates/ofd/daemon` verschieben, alle `-p ofd` und deb-Pfade anpassen | M1–M10 im echten Repository, CI muss die Testzahl des Daemons zeigen | OpenLDAP (siehe unten) | mittel |
| 7 | Plugins verschieben (`plugins/<name>` → `…/<name>/guest`, Broker, Packaging), in einem Zug für postgresql, openldap und example-tree | Gast-Builds `+1.99.1`, `#[ignore]`-E2E | OpenLDAP (siehe unten) | mittel |
| 8 | `bareos-config` um FD/SD erweitern, `ofd/config.rs` ablösen, `onesimus-daemon`-Gerüst | Konfig-Tests, Gerüst | osd-Daemon braucht es, idealerweise vor 20./21.11. als Hülle | mittel |

Schritt 0a kommt aus dem Fachmann-Bericht („Erst das Gerüst und Tests für `job/backup_cmds.rs`, dann trennen“). In der Reihenfolge des Architekten steht er nicht als eigener Schritt.

**Befund zum osd-Neubau** — Der Neubau kann sofort als `crates/osd/daemon` beginnen (Paket `osd-daemon`, bin `osd`). M1 zeigt, dass lib `osd` und bin `osd` nebeneinander bauen. `bareos-format` und `onesimus-daemon` sollten vorher wenigstens als leere Crates angelegt sein, damit der Neubau dort hineinschreibt, statt zu kopieren (Architekt).

**Widerspruch W6 (Stände, unbelegt): Ist OpenLDAP schon gemergt?** Der Architekt (Stand `c5e0218`) fand OpenLDAP im gesperrten Worktree `.claude/worktrees/agent-a30204bcfe6928be5` (HEAD `30e09b1`, 42 Dateien, +9681/−269, Merge-Basis `5dae029`) und fordert, die Schritte 6 und 7 strikt erst nach dessen Merge auszuführen. Ein Verschieben vorher erzeugt Rename-Konflikte, weil die neuen `ldap_*.rs` am alten Ort landen. Der Fachmann sah, wie `development` während der Begutachtung bis `7c77239` um zehn OpenLDAP-Commits wuchs (+2412/−37 in `crates/ofd`, u. a. `ldap_port.rs`). Ob damit der gesamte Worktree-Stand gemergt ist, sagt keiner der Berichte. Die Vorbedingung für die Schritte 6 und 7 ist deshalb **unbelegt** und vor Schritt 6 neu zu messen.

### Verdikt §6
**Zuerst das Gerüst und Tests für den Backup-Pfad, dann reine Format-Funktionen, dann Client-Code in die Schnittstellen. Ofd und Plugins werden zuletzt verschoben, und erst wenn OpenLDAP nachweislich vollständig gemergt ist.** Der osd-Neubau kann sofort beginnen, sollte aber in vorher angelegte gemeinsame Crates schreiben.

---

## §7 Defekte (Liste)

Defekt heißt: Der Code hält Bareos-25-Verhalten oder eine eigene Zusage nicht ein. Zur Herkunft: F = Fachmann, A = Architekt, T = Trainee.

| # | Defekt | Fundstelle | Beleg | Aufwand | Entscheider |
|---|---|---|---|---|---|
| D1 | Ein abgebrochener Backup-Job endet im FD mit `TermCode=69` (E) statt A. Bareos setzt `JS_Canceled` sofort in `CancelCmd`, ofd setzt nur ein `AtomicBool`, danach greift `set_status(JS_CANCELED)` wegen gleicher Priorität (9) nicht mehr | `sd.rs` `JobShared::cancel`, `job.rs` `Session::run`, `backup_cmds.rs` `backup_cmd` | F, gemessen 19.11.2026 im Gerüst (FD-Log 04:25:41) | klein | Entwickler |
| D2 | Beim Abbruch `ReadBytes=0` bei `JobBytes=524288`. In der Pipeline-Variante kommt `read_bytes` erst nach der Schleife dazu | `backup.rs` `send_data` | F, gemessen 19.11.2026 | klein | Entwickler |
| D3 | Ein Abbruch während Estimate oder Verify (DiskToCatalog/Catalog) wird nie gesehen: lokales `AtomicBool::new(false)` bzw. einmalige Kopie. Bareos prüft `IsJobCanceled()` je Datei | `backup_cmds.rs` `estimate_cmd`, `do_verify`; `backup.rs` `EstimateHandler`, `VerifyHandler` | F, gelesen 19.11.2026, im Lauf nicht nachgemessen | klein | Entwickler |
| D4 | Heartbeat ist nicht implementiert. `heartbeat_loop` schläft nur, `Heartbeat Interval` wird gelesen und verworfen. Bareos startet `MakeHeartbeatMonitor` und `MakeDirHeartbeat` | `heartbeat.rs` `heartbeat_loop`, `config.rs` `heartbeat_interval_secs` | F gelesen, T gemessen (12 Zeilen, kein Socket-Zugriff). T ließ offen, ob Keepalives anderswo entstehen. F belegt aus dem Quelltext, dass das Intervall nirgends genutzt wird | mittel | Entwickler |
| D5 | Eine ungültige Bandbreitenangabe (`200k`) wird still zu „unbegrenzt“. Dasselbe `unwrap_or(default)`-Muster gilt für `maximumconcurrentjobs`, `maximumworkersperjob`, `heartbeatinterval`, `sdconnecttimeout`, `maximumnetworkbuffersize` | `config.rs` `parse_speed`, Client-Parser | F, gemessen 19.11.2026 (Lauf 2: 8 MiB in 1 s). Dass bareos-fd beim Start ablehnt, ist **nicht gegengeprüft** | klein | Entwickler |
| D6 | `resolve` fehlt in der Befehlstabelle. Die Konsole bekommt „2999 Invalid command“, und die Verbindung endet | `job.rs` `Session::select` | F, belegt im Quelltext, nicht im Lauf | klein | Entwickler |
| D7 | `.status` liegt auf `status_cmd` statt auf dem maschinenlesbaren `QstatusCmd`-Format | `job.rs` `Session::select` | F, belegt im Quelltext, nicht im Lauf | klein | Entwickler |
| D8 | Die Statusausgabe ist teilweise Attrappe: `Local::now()` als Versions- und Startdatum, `run=0`, `bwlimit=0kB/s` und „Terminated Jobs“ fest verdrahtet. Zwei Versionsdaten in einem Binary (17. Oktober gegen 19. November 2026) | `status.rs` `fd_status`, `version.rs` `DATE` | F, gemessen im Gerüst 19.11.2026 | klein | Entwickler |
| D9 | `Maximum Concurrent Jobs` wird gelesen und nicht durchgesetzt. `accept.rs` startet je Verbindung einen Thread ohne Obergrenze | `config.rs` `max_concurrent_jobs`, `accept.rs` Accept-Schleife | F, gelesen 19.11.2026 | mittel | Nutzer (ob gebraucht) |
| D10 | Verdacht: Panikt eine Sitzung, bleibt ihr Eintrag in `sd::JOBS` liegen. `Builder::spawn(...).ok()` verwirft Fehler beim Thread-Start | `accept.rs`, `sd.rs` `register`/`unregister` | F, Vermutung, Impact gering | klein | Entwickler |
| D11 | Das SD-Job-Kommando von `osd::job::sd_job_setup` schreibt die Client-**Adresse** als `client_name`, ein festes `FileSet=LinuxAll` und keine BashSpaces. Genutzt wird es u. a. für Restore (`job_worker.rs::run_worker`) | `osd/src/job.rs` `sd_job_setup`, `odir/src/job_worker.rs` `to_sd_params` | A, gemessen 19.11.2026. Die Folgen am C++-SD sind **unbelegt**. Für den osd-Neubau wäre es ein Vertragsbruch | klein | Nutzer (Mängelliste odir) |
| D12 | Verdacht: `connect_sd_on` verwirft die OK-Hello-Antwort und hat keinen Lese-Timeout. Ein falsches SD-Passwort fällt im Job-Pfad erst beim nächsten Befehl auf | `odir/src/job_worker.rs` `connect_sd_on` (gegen `osd/src/connect.rs` `connect_sd`) | A, Codeunterschied gemessen, Folge Vermutung | klein | Entwickler |
| D13 | Verdacht: BashSpaces hat zwei Semantiken (`'_'` im Hello gegen `0x01`). Ein Director-Name mit Leerzeichen könnte die Authentifizierung scheitern lassen | `bareos_bnet::cram_md5::bash_spaces`, `hello::make_director_hello` gegen `odir/src/catreq.rs::bash_spaces`, `ofd/src/job.rs::unbash` | A. Gegen das C++-Original ist das **unbelegt**: Der Architekt hatte keinen C++-Quelltext lokal und hat nur odir's Zitate von `lib/util.cc` herangezogen | klein | Fachmann |

**Funktionslücken gegenüber Bareos 25.** Hier fehlt eine Funktion ganz. Ob das ein Defekt ist, hängt davon ab, ob ofd die Funktion zusagt.
- **PKI-Verschlüsselung und -Signatur** fehlen in beide Richtungen, absichtlich und mit sauberer Fehlermeldung (`backup_cmds::backup_cmd`, `restore::…::record`, Endmeldung fest `Encrypt=0`). Mit bareos-fd verschlüsselt gesicherte Daten lassen sich mit ofd nicht zurückholen. Dass `provides = "bareos-filedaemon"` in der Paketbeschreibung das nicht erwähnt, ist ein **Mangel** (F). Aufwand groß, Entscheider Nutzer.
- **Client-initiierte Verbindung** („Connection From Client To Director“, Bareos `StartConnectToDirectorThreads`) fehlt. In `accept.rs` steht nur ein Kommentar (F). Aufwand mittel, Entscheider Nutzer.

**Mängel (Doku/Plan), gesammelt:** fehlende Modulkarte in `main.rs`; Modulköpfe nur mit C++-Bezug; kein Glossar; `backup-protocol.md` verwaist; Kopf von `job.rs` (Restore/Backup-Aufteilung); Köpfe von `backup.rs`/`restore.rs` („no plugins“); `session`-Kommando in `backup-protocol.md` gegen `job.rs` (welches stimmt, ist **unbelegt**); `file-daemon.md`/`kommandozeile.md` zum gRPC-Standard und zu `--disable-grpc`; `plugin-architektur.md` zu Broker, WASM-Host, Trust Store und WIT; Kopf von `broker.rs` „schema 2“; `STUBS.md` S-13 und `architektur-vergleich.md` („ohne Plugins“); doppeltes „S-13“; Buchstabenkatalog der FileSet-Optionen undokumentiert; `crates/osd/src/lib.rs` (Aussage zu `connect` falsch); `docs/plans/STAND-2026-11-19.md` („Echter Lauf … bestanden“ ohne Skript); Verify-Sparse-Eigenheit undokumentiert; PKI-Ausschluss nicht in der Paketbeschreibung; Modulname und Doc von `heartbeat.rs` versprechen eine Funktion (siehe D4).

**Kein Defekt (ausgeschlossen):** „MD5 digest differs“ bei Nulldateien in Verify DiskToCatalog (geerbt von Bareos, F); mtime des Symlinks nach Restore (bareos-konform, F); `false` am Ende eines erfolgreichen `backup_cmd()` (T); Lücken in der `FO_*`-Liste (T); `transmute`-Warnungen (F); doppelte TLS-Wege (A).

**Ungeklärt in odir (kein ofd-Befund):** `estimate job=VBackup listing` hat den FD nicht kontaktiert (F). odir sendete beim Job-Wert `200k` kein `setbandwidth` (F, „gehört in das Gutachten zu odir“). Der Director meldete nach dem Abbruch „No Job status returned from FD“, obwohl ofd `2800 End Job` gesendet hatte. Ob das an odir oder ofd liegt, ist **unbelegt**, weil ein Mitschnitt fehlt.

### Verdikt §7
**Dreizehn Defekte bzw. Defektverdachte, davon zehn in ofd (D1 bis D10) und drei auf der Director/osd-Seite (D11 bis D13). Gemessen sind D1, D2, D5, D8 und D11, die übrigen sind aus dem Quelltext gelesen oder vermutet.** Hinzu kommen zwei Funktionslücken gegenüber Bareos 25 (PKI, client-initiierte Verbindung) und eine lange Liste von Doku-Mängeln. Keiner dieser Punkte ist durch einen Test markiert.

---

## §8 Offene Entscheidungen des Nutzers

| # | Frage | Entscheider | Aufwand |
|---|---|---|---|
| 1 | Paketnamen: Schnittstelle `ofd` und Daemon `ofd-daemon` (dann alle `-p ofd` gleichzeitig anpassen, sonst gehen die FD-Tests still verloren) **oder** Schnittstelle `ofd-api`/`ofd-proto` und Daemon behält `ofd` (M3/M4 entfallen)? Gleiche Frage für osd | Nutzer | klein |
| 2 | Gerüst Director+FD+SD und Tests für `job/backup_cmds.rs` verbindlich **vor** jedem Umbauschritt? | Nutzer, Entwickler | klein (Gerüst), mittel (Tests) |
| 3 | Erster Schnitt: Plugin/Broker ↔ Kern (Fachmann) oder zuerst die Schnittstellen aus odir und bareos-bnet, Plugins zuletzt (Architekt)? (W5) | Nutzer | mittel |
| 4 | `bareos-format` und `onesimus-daemon` als Hüllen **vor** dem osd-Neubau 20./21.11. anlegen? | Nutzer | klein |
| 5 | bareos-bnet tonic-frei machen (`fd_grpc` in die ofd-Schnittstelle, `AnyStream::Grpc` über Trait)? | Nutzer | mittel |
| 6 | Protos in mehrere Crates aufteilen (director, filed, plugin, später stored)? Die Feature-Variante ist verworfen | Nutzer | mittel |
| 7 | Ort des FD↔SD-Datenprotokolls: Record-Ebene neutral in `bareos-format`, Verbindungsaufbau in der osd-Schnittstelle, ofd-daemon hängt an der osd-Schnittstelle? | Nutzer | mittel |
| 8 | Plugins: `crates/ofd/daemon/<name>` als Ordner mit Gast (eigener Workspace) und Broker (Mitglied); Host-Port als Modul im Daemon oder eigenes Crate? Proto-Crate „Plugin-Vertrag“ für Gäste (no_std unbelegt)? | Nutzer | mittel |
| 9 | Konfigurationsparser vereinheitlichen (`bareos-config` für FD und SD, `ofd/config.rs` ablösen)? | Nutzer | mittel |
| 10 | Bleibt die Bareos-Spiegelung (C++-Funktionsnamen) das Doku-Prinzip, oder kommen Glossar und eigene Erklärungen dazu? (W3) | Nutzer | mittel |
| 11 | Kompatibilitätsregel für eigene FileSet-Optionen: Dürfen freie `FO_*`-Bits belegt werden? Soll ein unbekannter Buchstabe eine Job-Meldung erzeugen? | Nutzer | mittel |
| 12 | PKI nachbauen oder den Ersatz-Ausschluss in der Paketbeschreibung (`provides = "bareos-filedaemon"`) dokumentieren? | Nutzer | groß (Nachbau) · klein (Doku) |
| 13 | Client-initiierte Verbindung und `Maximum Concurrent Jobs` (D9): gebraucht? | Nutzer | mittel |
| 14 | D11 (`client_name` = Adresse, `FileSet=LinuxAll`) auf die Mängelliste odir, vor Übernahme in die osd-Schnittstelle beheben? | Nutzer | klein |
| 15 | Verify-Sparse-Eigenheit als bekannte Bareos-Eigenschaft dokumentieren? | Nutzer | Hausmeister |
| 16 | Die Anlass-Zahl „11.500“ mit Datum, Commit und Zählweise neu fassen? | Nutzer | klein |
| 17 | 60 liegengebliebene Agent-Worktrees (222 GB, `git worktree list` 61 Einträge, je eine Kopie von `crates/ofd`) aufräumen? Laut Fachmann kann ein Teil des „Trümmer“-Eindrucks daher stammen, weil `grep -r` jede Datei 61-mal findet | Nutzer | Hausmeister |
| 18 | Vor den Schritten 6 und 7 neu messen, ob der OpenLDAP-Worktree vollständig in `development` gemergt ist (W6) | Nutzer, Entwickler | klein |

**Unbelegt geblieben (zusammengefasst):** Woher die Zahl „11.500“ stammt. Die Abweichungen bei Modul- und Testzählungen (W2). Ob `session` im Backup oder im Restore gesendet wird. Ob freie `FO_*`-Bits bareos-kompatibel sind. Die Rahmung hinter `BnetStream::send()`. Ob Keepalives außerhalb von `heartbeat.rs` entstehen (laut D4 wird das Intervall nirgends genutzt). Was bareos-fd mit `200k` macht. Ob bareos-fd die Sparse-Prüfsumme genauso berechnet (empirisch). Woher „No Job status returned from FD“ kommt. Die C++-Semantik von BashSpaces im Hello. Was `client_name=<Adresse>` am C++-SD bewirkt. Ob ein Plugin-Proto-Crate no_std-tauglich ist. `[env] relative = true` als Pfadlösung. Ob die Feature-Unification die Proto-Kopplung tatsächlich erhält. Woher das Sicherheits-„S-13“ stammt. Ob der OpenLDAP-Merge vollständig ist (W6).

### Verdikt §8
**Achtzehn Entscheidungen liegen beim Nutzer. Die dringendsten vor dem osd-Neubau am 20./21.11. sind Nr. 1 (Paketnamen), Nr. 2 (Gerüst zuerst) und Nr. 4 (gemeinsame Crates als Hülle).**

---

## §Referenzen

Quelltext ofd (Stand `c5e0218` bzw. `7c77239`, wie im jeweiligen Befund):
- `crates/ofd/Cargo.toml` — nur `[[bin]]`, `[package.metadata.deb]` mit tiefenabhängigen Pfaden, `onesimus-plugins` per Pfad, `provides`/`conflicts bareos-filedaemon`
- `crates/ofd/build.rs`, `crates/ofd/src/plugin/trust.rs` `builtin` — `ONESIMUS_PLUGIN_CA`, muss mit dem Daemon wandern
- `crates/ofd/src/main.rs` `main()`, `check_plugin()`, Moduldoku — Thread-Modell unvollständig, gRPC standardmäßig an, alle Module privat
- `crates/ofd/src/accept.rs` `accept_loop()`, `handle_connection()`, `handle_session()`, `psk_for_identity()` — Server-Annahme, ein Thread je Verbindung ohne Obergrenze
- `crates/ofd/src/grpc.rs` `start()`, `Service::control()` — gRPC als Transport derselben BNET-Nachrichten; `grpc_tls.rs` `load_or_generate()` — Parallele zu odir
- `crates/ofd/src/job.rs` `Session`, `Session::select()`, `Session::run()`, `cancel_cmd`, `storage_cmd()`, `session_cmd()`, `open_sd_read_session()`, `unbash`; Tests `restore_session_with_simulated_director_and_sd`, `fake_sd` — Befehlstabelle (D6, D7), Endstatus (D1), einziger E2E-Test des Kerns
- `crates/ofd/src/job/backup_cmds.rs` `backup_cmd`, `blast_data`, `fileset_cmd`, `level_cmd`, `accurate_cmd`, `estimate_cmd`, `verify_cmd`, `do_verify`, `do_verify_volume`, `setbandwidth_cmd`, `run_scripts` — 5,33 % Abdeckung, D3
- `crates/ofd/src/backup.rs` `send_data`, `save_file`, `encode_and_send_attributes`, `save_plugin`, `run_plugin`, `EstimateHandler`, `VerifyHandler`, Modulkopf — Sparse/Prüfsumme, D2, „no plugins“-Kopf
- `crates/ofd/src/restore.rs` `record`, `Rctx`, `start_online`, Modulkopf — verschlüsselte Ströme, „no plugins“-Kopf
- `crates/ofd/src/sd.rs` `connect_storage`, `handle_stored_connection`, `JobShared::cancel`, `BwState::control`, `JOBS`, `register`/`unregister` — FD↔SD, D1, D10
- `crates/ofd/src/jmsg.rs` `JobMsgs::set_status`, `status_priority` — Prioritätslogik
- `crates/ofd/src/heartbeat.rs` `heartbeat_loop` — D4
- `crates/ofd/src/status.rs` `fd_status`; `crates/ofd/src/version.rs` `DATE` — D8, Formatpaar mit odir
- `crates/ofd/src/config.rs` `FdConfig::parse`, `strip_comments`, `walk_conf_dir`, `parse_speed`, `heartbeat_interval_secs`, `max_concurrent_jobs` — eigener Parser, D5, D9
- `crates/ofd/src/fileset.rs` `FileSet::add()`, `set_options_and_flags()`, `accept_file()`, `FOpts`, `FileOpts` — Aufgabenprobe Option
- `crates/ofd/src/find.rs` `FfPkt`, `trait Handler` — Begriffe, Rückgabewerte
- `crates/ofd/src/attribs.rs` `decode_stat`, `from_base64`, `set_attributes`; `streams.rs`; `digest.rs`; `breg.rs` — Kandidaten für `bareos-format`
- `crates/ofd/src/broker.rs` Modulkopf; `plugin.rs` `Session`, Tests `guest_wasm`, `pg_setup` — Schema-Version, tiefenabhängige Testpfade
- `crates/ofd/tests/plugin_check.rs` — einziger Test unter `tests/`

Andere Crates:
- `crates/bareos-bnet/src/fd_grpc.rs` `connect`, `pair`, `ControlIo`; `stream.rs` `AnyStream::Grpc` — tonic in jedem Nutzer
- `crates/bareos-bnet/src/cram_md5.rs` `bash_spaces`; `hello.rs` `make_director_hello` — D13
- `crates/onesimus-proto-rs/build.rs` — ein Crate für alle Protos
- `crates/onesimus-plugins/wit/filed-plugin.wit` `world plugin`; `src/wasm.rs` `enum Operation`, `enum Reply` — Plugin-Vertrag Daemon↔Gast
- `proto/filed_plugin.proto` `service FileDaemonPlugin` — Protobuf-Vertrag
- `crates/osd/src/lib.rs` Modulkommentar; `connect.rs` `connect_sd`; `status.rs` `query_sd_status`; `job.rs` `sd_job_setup` — falscher Kommentar, ungenutzter Code, D11
- `crates/odir/src/fd_protocol.rs` `do_fd_status`, `connect_fd_with_timeout`, `connect_fd_mode`, `connect_fd_console` — Handshake-Kopien
- `crates/odir/src/fd_grpc.rs` `try_connect`, `try_worker` — Handshake im gRPC-Strom
- `crates/odir/src/job_worker.rs` `connect_fd_mode_on`, `connect_fd_on`, `connect_sd_on`, `to_sd_params`, `run_worker`, `wait_for_fd_end_job` — Handshake-Kopien, D11, D12
- `crates/odir/src/backup_worker.rs` `connect_file_daemon`, `start_sd_job`, `parse_end_job`, `setup_fd` — Formatpaare
- `crates/odir/src/sd_copy.rs` `format_job_command`, `bash`; `verify.rs` — SD-Job-Kommando, `STREAM_*`
- `crates/odir/src/catreq.rs` `bash_spaces`; `sd_label.rs` `bash_spaces`; `job_messages.rs` `unbash_spaces` — BashSpaces mit `0x01`
- `crates/odir/src/fd_progress.rs` `parse` — Status-Parser
- `crates/odir/src/commands/show.rs` `parse_options()`; `commands/estimate.rs` `emit_item()`, `include_exclude_messages()` — FileSet-Optionen
- `crates/odir/src/commands/bvfs.rs` `decode_stat`; `ovfs/lstat.rs`; `grpc/tls.rs` — Formatkopien, TLS-Parallele
- `plugins/postgresql/src/lib.rs` — `wit_bindgen::generate!` mit relativem Pfad, handkopierte Proto-Typen
- `packaging/ofd-plugin-postgresql/build-deb.sh` — `top=$here/../..`

Doku und Pläne:
- `docs/wiki/file-daemon.md`, `docs/wiki/kommandozeile.md` §ofd — gRPC-Standard falsch beschrieben
- `docs/wiki/plugin-architektur.md` — Broker, WASM-Host, Trust Store, WIT
- `docs/backup-protocol.md` — verwaist, `session`-Kommando
- `docs/STUBS.md` S-13, `docs/wiki/architektur-vergleich.md` — „ohne Plugins“
- `docs/plans/STAND-2026-11-19.md` — „Echter Lauf … bestanden“, Bau-/Testbefehle mit `-p ofd`
- `.github/workflows/package.yml`, `README.md`, `docs/plans/SIGNIER-INFRASTRUKTUR.md` — `-p ofd`

Prüfbank und Lauf:
- `tests/verify/rig.py` `run`, `tests/volrecycle/rig.py` — starten `/usr/sbin/bareos-fd` (nicht installiert)
- Gerüst `ofdrig.py`, Messprotokolle `rig-out*/`, `build*.log`, `test*.log`, `clippy*.log`, `cov*.log` — Scratchpad der Fachmann-Sitzung vom 19.11.2026, nicht im Repository
- `/tmp/ofd-ws-probe` — Wegwerf-Workspace der Cargo-Messungen M1–M14
- Laufzustand: `systemctl cat ofd`, `ss -tln` (9102, 10102), `ofd --help`
- Worktrees `.claude/worktrees/agent-a30204bcfe6928be5` (OpenLDAP), `.claude/worktrees/agent-abbb161c62bdb14ac` (uncommitted odir-Änderungen)

Bareos 25 (Branch `bareos-25`, lokal beim Fachmann `/tmp/bareos25-src/`):
- `filed/dir_cmd.cc` `s_cmds[]`, `CancelCmd`, `ResolveCmd`, `AreMaxConcurrentJobsExceeded`, `RestoreCmd` — Befehlstabelle, D1, D6, D9
- `filed/filed.cc` `StartConnectToDirectorThreads` — client-initiierte Verbindung
- `filed/backup.cc` `SendDataToSd`, `SendPlainData`, `MakeHeartbeatMonitor`; `filed/heartbeat.cc` `MakeDirHeartbeat` — Sparse/Prüfsumme, D4
- `filed/estimate.cc`, `filed/verify.cc` `IsJobCanceled()` — D3
- `filed/status.cc` `QstatusCmd` — D7
- `filed/accurate.cc` `AccurateCmd`; `lib/jcr.cc` `GetStatusPriority`; `findlib/attribs.cc` `SetAttributes` — Treue des Kerns
