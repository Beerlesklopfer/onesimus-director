# Planung: Phase 1 – Plugins in ofd ausführen

Stand: 18.11.2026. Status: **Freigegeben (18.11.2026); Schritte 1 bis 7 umgesetzt. Offen: Lauf gegen einen echten Bareos-SD und Director.**

Gemeinsame Voraussetzung für [PostgreSQL](POSTGRESQL-PLUGIN.md) (zuerst),
[OpenLDAP](OPENLDAP-PLUGIN.md), [Docker](DOCKER-PLUGIN.md) und
[Podman](PODMAN-PLUGIN.md). Umfasst Phase 0 (Vertragserweiterung) und Phase 1
(Anschluss an die Job-Engine). Ein Broker gehört **nicht** dazu; das Testplugin
liefert seine Dateien aus eigenem Speicher.

## Ist-Stand im Code

| Teil | Stand |
| --- | --- |
| `onesimus-plugins::verify`, `bundle`, `wasm::load_signed`, `executor` | fertig, getestet |
| `ofd --check-plugin` | lädt und initialisiert ein Bundle, führt keinen Job aus |
| `ofd/src/find.rs` | `Plugin = …` im Include → `M_FATAL "Plugin … not found"` |
| `ofd/src/restore.rs` | `STREAM_PLUGIN_NAME` wird übergangen |
| `wit/filed-plugin.wit` | Exporte `initialize` … `shutdown`, keine Importe |

## Konfiguration des File Daemons

Namen wie im Bareos-FD, damit vorhandene Konfigurationen lesbar bleiben:

```
Client {
  Plugin Directory = /usr/lib/x86_64-linux-gnu/onesimus/plugins  # Vorgabe: Multiarch-Pfad
  Plugin Names = postgresql                         # Positivliste
  Plugin Policy = /etc/onesimus/plugin-policy.json  # öffentliche Trust-Policy
}
```

- `Plugin = "postgresql:…"` → Name vor dem ersten `:` → Bundle
  `<Plugin Directory>/postgresql.pb`. Der Name muss in `Plugin Names` stehen.
- Jede Plugin-Zeile wird **bei jedem Job** neu geprüft (Signatur, Policy,
  Widerruf), wie im Architekturvertrag verlangt. Kein Cache von Freigaben.
- Ohne `Plugin Policy` bleibt das heutige Verhalten: `M_FATAL … not found`.
- Optionen nach dem Namen werden als `key=value` (Trenner `:`, `\:` als Literal)
  zerlegt und als `InitializeRequest.options` übergeben; in Meldungen maskiert.

## Signatur ist die Ausführungsbedingung

Der Unterschied zum Python-Plugin des Bareos-FD: Dort führt der FD jede Datei
aus, die im Plugin-Verzeichnis liegt. Wer dort schreiben kann, ein Mensch, ein
Skript oder ein LLM-Agent mit Shellzugriff, bringt eigenen Code in einen root-Prozess
mit Zugriff auf alle Daten. Bei `ofd` läuft nur, was mit einem Schlüssel aus
dem Build-System signiert ist; dieser Schlüssel liegt auf keinem Host, den ein
Agent bedient.

Damit die Signatur nicht umgangen werden kann, gilt zusätzlich:

- **Kein unsignierter Pfad, auch nicht zum Testen.** Es gibt keine Option
  „Signatur überspringen“, keinen Entwicklermodus und keinen Fallback. Tests
  signieren mit einer Test-CA, die nur in der Test-Policy steht.
- **Policy und Plugin-Verzeichnis sind root-eigen** (wie `StrictModes` bei
  sshd): `ofd` verweigert `Plugin Policy`, `Plugin Directory` und jedes Bundle,
  wenn Eigentümer nicht root ist oder Gruppe/Andere Schreibrecht haben, auch
  für jedes übergeordnete Verzeichnis. Sonst könnte jemand eine eigene CA in die
  Policy schreiben und sich selbst freigeben.
- **Geprüfte Bytes werden ausgeführt** (`VerifiedComponent`), die Datei wird
  nach der Prüfung nicht erneut geöffnet.
- **Was ein Plugin darf, steht in der Signatur:** Capabilities aus dem Manifest
  und der Herausgeberfreigabe; `Plugin Names` kann nur weiter einschränken.
- **Wiederherstellung in Dateien führt keinen Plugin-Code aus** und braucht
  deshalb keine Signatur. Online-Wiederherstellung lädt und prüft wie die
  Sicherung.
- **Übergangsskripte sind die Ausnahme, die wir abbauen.** `RunScript`-Skripte
  wie `pg-dump-all.sh` sind unsigniert; sie bleiben nur bis zur Abnahme des
  jeweiligen Plugins und liegen in einem root-eigenen Verzeichnis aus
  `Allowed Script Dir`.

## Verteilung: ein Debian-Paket je Plugin

Entscheidung vom 18.11.2026: Jedes Plugin ist ein eigenes Debian-Paket
(`onesimus-plugin-<name>`).

- **Multiarch (Entscheidung 18.11.2026):** Ein Plugin bringt Binärteile je
  Architektur mit (z. B. seinen Broker), das Paket ist also
  `Architecture: any` und `Multi-Arch: same`. Das Bundle liegt unter
  `/usr/lib/<triplet>/onesimus/plugins/<name>.pb`; das ist die Vorgabe von
  `Plugin Directory` (Triplet der Architektur, für die `ofd` gebaut ist).
  dpkg legt die Dateien als `root:root`, Modus `0644` an; das erfüllt die
  Rechteprüfung ohne Zutun.
- **Das Paket gibt sich nicht selbst frei.** Es ändert weder `Plugin Names` noch
  die Policy. Freigabe ist ein Schritt des Administrators (oder seiner
  Konfigurationsverwaltung). Ein installiertes Paket allein führt keinen Code aus.
- **Die Trust-Policy gehört nicht in ein Plugin-Paket.** Sie kommt aus einem
  eigenen Paket (Vorschlag `onesimus-plugin-trust`, Conffile
  `/etc/onesimus/plugin-policy.json` mit den CA-Zertifikaten) oder vom
  Administrator. Sonst könnte jedes Paket seine eigene CA mitbringen.
- `Depends: ofd (>= <Version mit Phase 1>)`; das Paket prüft im `postinst` das
  Bundle mit `ofd --check-plugin` und meldet einen Fehler, bricht die
  Installation aber nicht ab (die Policy kann später kommen).
- Das Bundle ist bereits signiert, wenn es ins Paket kommt; der Paketbau
  braucht keinen Signierschlüssel. Updates ersetzen die Datei atomar
  (dpkg schreibt neu und benennt um); ein laufender Job behält die bereits
  geprüften Bytes.

### Signatur der Binärteile (Entscheidung 18.11.2026, umzusetzen mit dem ersten Broker)

Der WASM-Teil ist signiert und wird bei jedem Laden geprüft. Ein nativer Teil
eines Plugin-Pakets (Broker) läuft dagegen als echter Prozess, nicht in der
Sandbox. Ohne eigene Prüfung wäre er genau der unsignierte Ausführungspfad,
den die Architektur ausschließt. Deshalb:

1. **Echtheit über X.509:** Das signierte Manifest (Schemaversion 2) nennt je
   Triplet den SHA-256 jedes Binärteils. Ein ausgetauschter Broker hat einen
   anderen Hash und startet nicht, gleich wem die Datei gehört.
2. **Dateisystem als zweite Schicht:** Binärteil und jedes Verzeichnis darüber
   gehören root und sind für Gruppe und Andere nicht schreibbar (dieselbe
   Prüfung wie für Policy und Bundle). Das meldet Fehlkonfigurationen früh.
3. **Kein Austausch zwischen Prüfung und Start:** `ofd` öffnet die Datei, prüft
   Hash und `fstat` auf dem offenen Deskriptor und startet genau ihn
   (`fexecve`), nicht erneut über den Pfad.

Die Trust-Policy selbst kann nicht per X.509 geschützt werden, denn sie legt
fest, welchen CAs vertraut wird; für sie ist die Eigentümer- und Rechteprüfung
die einzige Schutzschicht.

## Phase 0: Vertragserweiterung

`FileEntry` bekommt rückwärtsverträglich `map<string,string> attributes = 7;`
und `FILE_KIND_VIRTUAL = 4` (regulärer Inhalt ohne Dateisystem-Gegenstück).
Alte Gäste senden die Felder nicht; der Host behandelt fehlende Attribute als leer.
`attributes` werden im Katalog nicht als eigene Spalte geführt, sondern als
`_info.json` des Plugins geliefert; die Oberfläche liest dort (Entscheidung
aus dem OpenLDAP-Plan bleibt offen bis Phase 4).

## Sicherung

Je Plugin-Zeile eine Worker-Instanz (`executor::Host::spawn`), gebunden an
Job-ID, Instanz-ID `<job>-<incexe>-<n>`, Device `plugin:<name>`.

1. `Initialize` mit Optionen und Job-Kontext, dann `HandleEvent(JobStarted)`.
2. Schleife `NextFile` bis `ScanComplete`. Für jeden Eintrag:
   - `STREAM_PLUGIN_NAME`-Satz wie im Bareos-FD (`<FileIndex> 1 <Plugin-Zeile>`),
     damit Original-Bareos-SD und -Director den Satz unverändert speichern.
   - Attribute als `FT_REG` (bzw. `FT_DIRBEGIN`/`FT_DIREND`) mit dem virtuellen
     Pfad, Größe, Zeit und Modus aus `FileEntry`; UID/GID des FD-Prozesses.
   - Daten über `ReadFile` in Blöcken von `Maximum Network Buffer Size`
     durch den vorhandenen Sendepfad (Kompression, Prüfsumme) in den SD-Strom.
3. Abschließender `STREAM_PLUGIN_NAME`-Satz `<FileIndex> 0` (Ende).
4. `HandleEvent(JobFinished|JobCancelled)`, `Shutdown`.
5. Cancel des Jobs → `Worker::cancel()`, außerbandig (vorhandener Interrupt).

Accurate/Inkremental: In Phase 1 werden Plugin-Dateien **immer** gesendet.
Die Auswertung von `modified_unix_nanos` für Inkrementale folgt mit den
Plugins, die sie brauchen (OpenLDAP Phase 4).

## Wiederherstellung

`STREAM_PLUGIN_NAME` setzt den Plugin-Kontext für die folgenden Dateien.

- **In Dateien (Vorgabe, `Where` gesetzt):** `ofd` schreibt die virtuellen
  Dateien selbst nach `<Where>/<virtueller Pfad>`, ohne das Plugin zu laden.
  Damit ist jede Plugin-Sicherung auch dann wiederherstellbar, wenn das Plugin
  fehlt, gesperrt oder abgelaufen ist. Das ist die wichtigste Eigenschaft für
  den Notfall.
- **Online (`restore_mode=online` in der Plugin-Zeile des Restore-Jobs):**
  Plugin laden, `WriteFile` je Block, `CloseFile` je Datei. Ohne geladenes
  Plugin bricht der Restore mit Fehler ab, kein stilles Schreiben auf die Platte.

## Testplugin und Abnahme

`crates/onesimus-plugins/examples/` bekommt einen Gast `example-tree`
(wasm32-wasip2, ohne Importe), der einen festen Baum liefert: Verzeichnis,
leere Datei, kleine Datei, Datei größer als ein Netzwerkblock, Pfad mit
Nicht-UTF-8-Byte. Online-Restore schreibt er in seinen Speicher und meldet
Prüfsummen in `CloseFile`.

| Test | Nachweis |
| --- | --- |
| Backup mit Testplugin gegen Bareos-SD | Job `T`, Dateien im Katalog unter dem virtuellen Pfad |
| Restore in Dateien | Inhalt byte-gleich, ohne installiertes Plugin |
| Online-Restore | Prüfsummen des Gasts stimmen |
| zwei Plugin-Zeilen in einem Include | getrennte Instanzen, beide vollständig |
| Plugin nicht in `Plugin Names` / Signatur ungültig / widerrufen | `M_FATAL`, Job `f`, kein Gastcode ausgeführt |
| Trap / Timeout im Gast | nur dieser Job endet mit Fehler |
| Cancel während `ReadFile` | Job `A`, Worker beendet, kein hängender Thread |
| Passwort in der Plugin-Zeile | maskiert in Joblog und Debuglog |

## Schritte

1. Proto/WIT: `attributes`, `FILE_KIND_VIRTUAL`; Roundtrip-Test.
2. FD-Konfiguration: `Plugin Directory`, `Plugin Names`, `Plugin Policy`;
   Optionszerlegung mit Maskierung.
3. `ofd/src/plugin.rs`: Laden je Zeile, Worker-Verwaltung je Job, Cancel.
4. Backup-Pfad in `find.rs`/`backup.rs`: Plugin-Dateien in den SD-Strom.
5. Restore-Pfad in `restore.rs`: Dateien-Modus, dann Online-Modus.
6. Gast `example-tree`, Signatur mit Test-CA, Ende-zu-Ende-Tests.
7. Doku: `docs/wiki/plugin-architektur.md` (Ist-Stand), `file-daemon.md`.

## Ende-zu-Ende-Lauf (18.11.2026)

`odir` + `bareos-sd` 25.1.3 + `ofd` mit dem signierten Gast `example-tree`:
Sicherung (5 Dateien), Wiederherstellung in Dateien ohne Plugin (Inhalt
identisch, Modus und Zeit der Dateien wie geliefert), Online-Wiederherstellung
über das Plugin, Abweisung eines Bundles einer fremden CA und eines Bundles mit
verändertem WASM-Code (Fatal Error, 0 Dateien). Kein Plugin-Passwort auf dem
Volume oder im Joblog.

Behoben dabei: Prüfung der CA-Dateien der Policy (`ofd`), Weitergabe von
`pluginoptions` beim Restore (`odir`).

Danach behoben: Verzeichniszeit von Plugin-Verzeichnissen (Attribute nach dem
Inhalt), `FD Address` und `DirAddress` werden beachtet, kein mDNS für einen
Director auf Loopback.

Offen: Der Job-Bericht zeigt „Software Compression: None“, obwohl LZ4 aktiv war.

