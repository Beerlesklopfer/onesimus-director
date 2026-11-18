# Planung: OpenLDAP-Plugin für ofd

Stand: 27.10.2026. Status: **Entwurf zur Freigabe; keine Umsetzung begonnen.**

Das Plugin ersetzt das Python-Plugin `bareos-fd-ldap` (siehe Job `BackupBgLage`):
Sicherung **und** Wiederherstellung eines OpenLDAP-Servers (`slapd`), die
Datenbank und die Online-Konfiguration (`cn=config`) einschließen. Der Inhalt
einer Sicherung soll später in der Oberfläche **durchsuchbar** sein, wie jedes
andere FileSet. Daraus folgt: kein LDIF-Klumpen, sondern ein Baum virtueller
Dateien mit Metadaten.

Es gelten [Plugin-Architektur](../wiki/plugin-architektur.md) und
[Protobuf-Vertrag](../../proto/filed_plugin.proto): signiertes Rust/WASM, kein
Python-/Shell-Plugin, eigene Instanz je Job, begrenzte Queues, außerbandiges
Cancel. DIR↔FD über gRPC mit BNET-Fallback, FD↔SD bleibt BNET.

## Stand der Grundlage

Vorhanden: Verifier, Wasmtime-Host (`wasm::load_signed`), Executor, Vertrag
`FileDaemonPlugin`, `ofd --check-plugin`. **Es fehlen** (Voraussetzungen):

| Lücke | Wirkung |
| --- | --- |
| `ofd` führt `Plugin = …` im `Include` nicht aus (heute: „not found“) | Ohne Anschluss an die Job-Engine läuft kein Plugin. |
| Der WASM-Gast hat keine Netz-, Datei- oder Prozess-Importe | Der Zugriff auf `slapd` braucht einen Broker auf dem Host. |
| `FileEntry` kennt nur Pfad, Art, Größe, Zeit, Modus, Linkziel | Für Durchsuchen und Inkrementale fehlen Metadaten (siehe unten). |

Diese drei Punkte sind Phase 0 und 1; das eigentliche Plugin ist Phase 2.

## Ein Include, zwei Plugin-Zeilen

```
Include {
  Plugin = "openldap:instance=main:content=data:uri=ldaps\://ldap.example.org:bind_dn=cn=admin,dc=example,dc=org:password=…"
  Plugin = "openldap:instance=main:content=config:uri=ldapi\:///"
}
```

Jede Zeile ist eine Plugin-Instanz mit eigenem Inhaltsbereich (`content=data`
oder `content=config`). Passwörter in den Optionen werden in Meldungen
maskiert (`ofd/src/secrets.rs`). Fehlt ein Plugin, meldet `ofd` jede Zeile
einzeln (Issue #9).

## Virtueller Baum und Durchsuchen

Jeder LDAP-Eintrag wird eine virtuelle Datei; die Verzeichnishierarchie des
DIT wird zum Verzeichnisbaum. Dadurch erscheint er im Katalog (Tabellen
`File`, `Path`) und ist über die vorhandene Pfadsuche (bvfs/OVFS) durchsuchbar.

```
/@openldap/main/_info.json                              Instanz-Metadaten
/@openldap/main/data/dc=de/dc=example/@entry.ldif       der Eintrag dc=example,dc=de
/@openldap/main/data/dc=de/dc=example/ou=people/@entry.ldif
/@openldap/main/data/dc=de/dc=example/ou=people/uid=anna/@entry.ldif
/@openldap/main/config/@entry.ldif                      cn=config
/@openldap/main/config/olcDatabase={1}mdb/@entry.ldif
```

**Jeder Eintrag ist ein Verzeichnis**, das seinen Inhalt in `@entry.ldif`
trägt; Kinder sind Unterverzeichnisse. Das gilt einheitlich für Blätter und
innere Knoten. Der Pfad eines Eintrags ändert sich nie, wenn er später Kinder
bekommt oder verliert; Versionen und Inkrementale bleiben stabil. In der
Baumansicht ist jeder Knoten genau ein Knoten, so wie im DIT.

- **Pfad:** Relative DNs in umgekehrter Reihenfolge, Sonderzeichen nach RFC 4514
  und zusätzlich für `/` und Nicht-UTF-8 escaped; Pfadbytes bleiben Bytes.
- **Datei `@entry.ldif`:** Der Eintrag als LDIF (RFC 2849), Binärattribute
  base64. Eine Datei je Eintrag ermöglicht den **Restore einzelner Einträge**
  (die Datei) **oder Teilbäume** (das Verzeichnis).
- **Zeitstempel:** `modifyTimestamp` des Eintrags als Änderungszeit; Größe =
  LDIF-Länge. Daraus rechnet das FileSet-Verhalten (Accurate/Incremental) wie
  bei normalen Dateien.
- **`_info.json`** (Datenbankinformationen für die Oberfläche): Instanz, Server-
  Version, Backend (`mdb`, `hdb`), Suffix, Anzahl Einträge, `contextCSN`,
  Schemaliste, Overlays, Zeitpunkt der Sicherung, Sicherungsmodus. Die
  Oberfläche liest dies, ohne den Baum zu durchlaufen.

### Datensätze auswählen

Einträge sind einzeln und als Teilbaum auswählbar, sowohl beim Wiederherstellen
als auch beim Sichern.

- **Restore:** Die Auswahl läuft über die vorhandene Dateiauswahl: nur der
  Eintrag ist seine `@entry.ldif`, der Teilbaum das Verzeichnis (`ou=people/`
  mit Schrägstrich). Die Oberfläche zeigt DN und Objektklasse
  (`FileEntry.attributes`), die Auswahl selbst bleibt eine Pfadauswahl im
  Katalog. `fileregex` und `files`/`directories` des Restore-Befehls gelten
  unverändert.
- **Eltern fehlen:** Wer `uid=anna` ohne `ou=people` auswählt, bekommt im Plan
  den Hinweis „Elterneintrag fehlt im Ziel“. Der Online-Import legt Eltern nicht
  stillschweigend an; `with_parents=yes` nimmt die Elternkette ausdrücklich mit.
- **Verweise:** Einträge, die auf andere verweisen (`member`, `manager`,
  `seeAlso`, Overlays wie `memberof`), werden nicht automatisch ergänzt. Der
  Plan listet DNs, die im Ziel nicht existieren.
- **Backup:** Eine Plugin-Zeile kann den Umfang begrenzen: `base=<DN>`
  (Teilbaum), `scope=sub|one`, `filter=<Filter>`. Die zulässigen Basen und
  Filter legt die signierte Freigabe fest, nicht das Plugin; ein Filter ist
  kein freier Suchausdruck des Gasts. Mehrere Zeilen im selben `Include`
  sichern unterschiedliche Teile, derselbe DN zweimal ist ein Planfehler.
- **Durchsuchen vs. Zeitstand:** Jede Sicherung ist ein Stand; die Oberfläche
  zeigt je Eintrag die Versionen über die vorhandenen Versions-Sichten
  (`ovfs_versions`, `bvfs_versions`).

### Erweiterung des Vertrags (Voraussetzung)

`FileEntry` bekommt, versioniert und rückwärtsverträglich, ein Feld
`map<string,string> attributes` (z. B. `dn`, `entryUUID`, `objectClass`) und
eine Art `FILE_KIND_VIRTUAL`, damit die Oberfläche den DN statt des
maskierten Pfads zeigen und nach Objektklasse filtern kann. Ohne diese
Änderung stünden DN und Klasse nur im LDIF-Inhalt.

## Zugriff: Broker mit Capability `ldap.read` / `ldap.write`

Der Gast bleibt ohne Netzwerk. Ein Broker auf dem Host (Teil von `ofd`,
nicht des Plugins) bietet typisierte, jobgebundene Operationen:

| Operation | Zweck |
| --- | --- |
| `ldap.info` | Root-DSE, `namingContexts`, Datenbankliste, Zähler, `contextCSN` |
| `ldap.search` | Eintragsliste (DN, `modifyTimestamp`, `entryUUID`) eines Suffixes, seitenweise |
| `ldap.read_entry` | Ein Eintrag als LDIF, inkl. Operational Attributes (`+`) |
| `ldap.add_entry`, `ldap.modify`, `ldap.delete` | Nur Restore, nur nach Plan-Freigabe |

Zwei freigegebene Verbindungsarten:

- **`ldapi:///` mit SASL EXTERNAL.** Der FD läuft als root; damit sind
  `cn=config` und alle Datenbanken ohne Passwort im Optionstext erreichbar.
  Bevorzugt, wenn `slapd` auf demselben Host läuft.
- **`ldaps://` mit Bind-DN und Passwort.** Für einen entfernten `slapd`
  (heutiger Zustand von `BgLage-fs`). Zertifikat wird geprüft; kein
  Fallback auf unverschlüsselt. Passwort nur aus den Plugin-Optionen, nie im Katalog.

Der Broker erlaubt dem Gast keine freien Filter, Base-DNs oder URIs: Instanz,
Suffix und URI kommen aus der Konfiguration und der signierten Freigabe
(`allowed_capabilities` der Trust-Policy), nicht vom Plugin.

## Sicherung

1. `Initialize`; `ldap.info` prüft Erreichbarkeit, Backend und Suffixe.
2. `content=data`: je Suffix paginiert alle DNs lesen; Baum in
   Elternreihenfolge (Eltern vor Kindern) als `NextFile` liefern.
   `content=config`: `cn=config` und alle `olc…`-Unterbäume.
3. `ReadFile` liefert das LDIF des Eintrags, `ldap.read_entry` mit
   Operational Attributes (`entryUUID`, `createTimestamp`, `creatorsName`,
   `modifyTimestamp`, …), damit eine Wiederherstellung die Identität erhält.
4. `_info.json` zuletzt, mit der Zahl tatsächlich gelesener Einträge.
5. **Inkremental:** Einträge mit `modifyTimestamp` seit der letzten Sicherung;
   gelöschte Einträge erkennt die Accurate-Liste (Pfad fehlt). Ändert sich
   `contextCSN` während der Sicherung, endet der Lauf mit Warnung, nicht
   stillschweigend inkonsistent. Konsistenzklasse: **fuzzy** (kein
   Snapshot); ein konsistenter Modus (`slapcat` auf einem
   freigegebenen Dateisystemsnapshot) ist eine spätere Klasse.

## Wiederherstellung

1. Plan anzeigen: Ziel (`ldapi`/`ldaps`), Suffix, Zahl der Einträge, Konflikte
   (vorhandene DNs), Umgang mit Operational Attributes.
2. **Standard: Wiederherstellung in Dateien** (`where=/pfad`): Der Restore
   schreibt die LDIF-Dateien; nichts am laufenden `slapd` ändert sich.
3. **Online-Import** nur ausdrücklich (`restore_mode=online`): Einträge in
   Elternreihenfolge per `ldap.add_entry`. Online lassen sich `entryUUID`,
   `createTimestamp` und `creatorsName` nur mit der Relax-Kontrolle und
   passenden Rechten setzen; sonst erzeugt `slapd` neue Werte. Das Ergebnis wird
   im Joblog genannt, kein stilles Abweichen.
4. **Offline-Import** (`slapadd`) bleibt ein getrennter, manueller Schritt
   (Dienst anhalten); `ofd` startet ihn nicht.
5. `replace=never` ist Vorgabe: vorhandene DNs werden gemeldet, nicht überschrieben.
   `cn=config` wird nie ohne ausdrückliche Freigabe zurückgeschrieben; ein
   fehlerhaftes `olcDatabase` kann `slapd` am Start hindern.

## Sicherheit und Betrieb

- Kein Passwort im Joblog, im Katalog oder in `_info.json`
  (`mask_plugin_secrets`; Testfall mit `password=` und `bind_pw=`).
- `userPassword`-Hashes stehen im LDIF. Das Volume ist entsprechend zu
  behandeln (Verschlüsselung der Sicherung, Zugriff auf den Pool).
- Fehlerbehandlung: Trap, Timeout und Cancel betreffen nur diese Instanz
  (Executor-Vertrag); der Broker schließt die LDAP-Sitzung.
- Eine einzelne Plugin-Zeile darf nicht stillschweigend weggelassen werden:
  Fehlt `content=config`, endet der Job mit Fehler.

## Phasen und Abnahme

| Phase | Liefergegenstand | Exitkriterium |
| --- | --- | --- |
| 0 | Vertragserweiterung (`FileEntry.attributes`, `FILE_KIND_VIRTUAL`), Entscheidung zum Katalog (Pfad ↔ DN) | Roundtrip in Protobuf und Katalog getestet |
| 1 | Plugin-Anschluss in `ofd`: `Plugin =` laden, `NextFile`/`ReadFile` in den SD-Strom, `WriteFile` beim Restore, Cancel | Testplugin sichert und stellt wieder her; zwei Plugin-Zeilen je Include |
| 2 | Broker `ldap.*` (ldapi, ldaps), Plugin `openldap` (Backup) | Echter `slapd` (Wegwerfinstanz): alle Einträge, Reihenfolge, Operational Attributes, `_info.json` |
| 3 | Restore in Dateien, danach Online-Import | Roundtrip: sichern, löschen, wiederherstellen, `slapcat`-Vergleich |
| 4 | Inkrementale, Durchsuchen in der Oberfläche | Änderungen und Löschungen erscheinen; DN-Anzeige statt Pfad |

Pflichttests: Sonderzeichen und Nicht-UTF-8 im DN; Binärattribute;
Mehrfach-Suffixe; sehr große Einträge (Gruppen mit vielen `member`-Werten);
`slapd` fällt während der Sicherung aus; Cancel bei blockiertem Broker;
falsches Passwort; abgelaufenes Zertifikat; Plugin A hält Device B nicht an.

## Offene Entscheidungen

1. Reicht der Katalogpfad aus DN-Segmenten, oder braucht die Oberfläche ein
   eigenes Feld (`FileEntry.attributes`)? Der Plan nimmt Letzteres an.
2. Ist **fuzzy** als erste Konsistenzklasse akzeptabel, oder ist ein
   `slapcat`-Modus auf einem Snapshot Pflicht für Phase 2?
3. Soll der Online-Import (Phase 3) das Setzen von Operational Attributes über
   die Relax-Kontrolle verlangen, oder reicht Wiederherstellung in Dateien als
   Standard?
4. Welche Verbindungsart ist für BGLAGE maßgeblich: `ldapi:///` (wenn `slapd` auf
   demselben Host liegt) oder `ldaps://`?
