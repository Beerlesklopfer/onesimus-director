# Zweistufige ACLs: Dateien als Rahmen, PostgreSQL als innere Einschränkung

Stand: 20.11.2026. Status: **Entschieden (F-1 bis F-6) und umgesetzt**, siehe
[Umsetzung](#umsetzung-20112026).

## Vorgabe (Maintainer, 19.11.2026)

> Wir werden ein zweistufiges Konzept haben: files ist der äußere Rahmen.
> PSQL der innere. Der innere kann nur weiter einschränken. file-ACLs können
> nur per Editor gesetzt werden.

## Anlass

Seit 19.11.2026 sind alle Schreib-RPCs über gRPC in jedem Build da
(`grpc-development` als Standard-Feature). Die Console-ACL ist damit die
einzige Grenze, was eine Sitzung darf. Auf PDC meldet sich die einzige
oconsole (und mit ihr das MCP jeder Claude-Sitzung) als Console `bconsole`
mit `*all*` an. Die BgLage-Sitzung konnte deshalb `delete`, `purge` und
`.sql` ausführen.

## Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Datei-ACL** (äußere Stufe) | Die ACL-Direktiven der Console- und Profile-Ressourcen in `bareos-dir.d` (`JobAcl`, `CommandAcl`, …), ausgewertet wie in Bareos (`dird/ua_acl.cc`: der erste passende Eintrag entscheidet, Console vor Profilen). |
| **Katalog-ACL** (innere Stufe) | Einträge in einer Onesimus-Tabelle im Katalog (PostgreSQL), je Console und ACL-Typ. |
| **Effektive ACL** | Erlaubt ist nur, was die Datei-ACL **und** die Katalog-ACL erlauben. |

## Regeln

1. **Effektiv = Datei ∩ Katalog.** Die Katalog-ACL kann nur einschränken.
   Ein Eintrag im Katalog, der mehr erlaubt als die Datei, bleibt wirkungslos.
   Er wird gespeichert und als wirkungslos gemeldet (F-3).
2. **Ohne Katalog-Einträge gilt die Datei-ACL unverändert.** Ein Director ohne
   Onesimus-Tabelle verhält sich also wie Bareos.
3. **Datei-ACLs nur im Editor.** Kein Befehl und kein RPC schreibt
   ACL-Direktiven in die Dateien.
4. **Eine zentrale Entscheidungsfunktion.** Jede ACL-Prüfung (Befehle,
   typisierte RPCs, CRUD, Push-Ereignisse, `GetConfigCapabilities`) läuft
   durch `crate::acl::allowed` bzw. `any_allowed`. Dort wird die innere Stufe
   eingehängt.
5. **Katalog nicht erreichbar:** konservativ. Hat die Console Katalog-Einträge,
   die gerade nicht lesbar sind, gilt für sie „nichts erlaubt“ außer `status`,
   `.status` und `version` (F-4) sowie `.api` (nur Ausgabeformat, Nachtrag
   20.11.2026 unter Umsetzung). Nie stillschweigend nur die Datei-ACL, denn
   das wäre eine Rechteausweitung.

## Was sich am Bestand ändert

| Stelle | heute | künftig |
| --- | --- | --- |
| `.acl set/delete/clear/copy` (Onesimus-Erweiterung, `commands/acl.rs`) | ~~schreibt die Console-Datei um~~ ~~19.11.2026 abgelehnt~~ | **umgesetzt 20.11.2026:** schreibt die **Katalog-ACL**; die Datei bleibt unberührt |
| `.acl list` | zeigt Datei-ACLs | **umgesetzt:** zeigt Datei-, Katalog- und effektive ACL |
| `configure add console`/`profile`/`user` (Bareos-Befehl) | ~~legt eine Datei mit ACL-Direktiven an~~ seit 19.11.2026 abgelehnt (F-1) | bleibt abgelehnt |
| `configure delete console` | seit `233a6f3` abgelehnt | bleibt abgelehnt |
| Konfigurations-CRUD (gRPC, `.config`) | Console/Profile/User nicht editierbar | bleibt so (Regel 3) |
| Push-Ereignisse gRPC | ungefiltert (K-19) | gefiltert über die zentrale Funktion, **beide Stufen** |

## Speicher der inneren Stufe

Neue Tabelle in der Onesimus-Schemaerweiterung des Katalogs, Stufe **v2253**
(`crates/bareos-orm/schema/v2253_onesimus_schema.json`, siehe Umsetzung zur
Nummer). Umgesetzt wie vorgeschlagen, dazu eine CHECK-Bedingung auf `acl_type`.
Ort (Entscheidung des Maintainers 20.11.2026): die Tabelle heißt
`onesimus.console_acl` und liegt im eigenen PostgreSQL-Schema `onesimus`
aller Onesimus-Objekte, `public` bleibt reines Bareos; dieselbe Stufe
verschiebt die Views und Funktionen von 2251/2252 dorthin
([betrieb-odir.md](../wiki/betrieb-odir.md#katalog-schema-onesimus)).
Zuerst (Commit `76d7c4c`, nie veröffentlicht) hieß sie
`public.onesimus_console_acl`.

```sql
-- eine Zeile je Console, ACL-Typ und Eintrag, Reihenfolge wie in Bareos
CREATE TABLE onesimus.console_acl (
    console   text     NOT NULL,   -- Name der Console-Ressource
    acl_type  text     NOT NULL,   -- job, client, storage, schedule, pool,
                                   -- command, fileset, catalog, where, pluginoptions
    position  integer  NOT NULL,   -- Auswertungsreihenfolge
    entry     text     NOT NULL,   -- wie in der Datei: Name, Regex, !Negation, *all*
    changed_by text    NOT NULL,   -- Console, die den Eintrag gesetzt hat
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (console, acl_type, position),
    CONSTRAINT console_acl_type_check CHECK (acl_type IN ('job', 'client', …))
);
```

Semantik eines Typs mit Katalog-Einträgen: Die Liste wird wie eine
Bareos-ACL ausgewertet; erlaubt ist ein Name nur, wenn Datei-Liste und
Katalog-Liste ihn beide erlauben. Ein Typ ohne Katalog-Einträge schränkt
nicht ein.

## Offene Fragen

| # | Frage | Entscheider |
| --- | --- | --- |
| ~~F-1~~ | **Entschieden und umgesetzt 19.11.2026:** `configure add console`, `profile` und `user` werden immer abgelehnt (nicht nur mit ACL-Zeilen); Consoles legt nur der Editor an. Ursprüngliche Frage: `configure add console`/`profile` ist ein Bareos-Befehl und schreibt ACL-Direktiven in eine Datei. Regel 3 schließt das aus. Vorschlag: ablehnen, wenn ACL-Direktiven (`*Acl`, `Profile`) im Aufruf stehen; eine Console ohne ACL-Zeilen hat in Bareos keine Rechte und darf angelegt werden. Abweichung von Bareos, kommt in betrieb-odir.md. | Maintainer |
| ~~F-2~~ | **Entschieden 19.11.2026 wie vorgeschlagen, umgesetzt 20.11.2026** (genaue Regel unter Umsetzung). Wer darf die Katalog-ACL ändern? Vorschlag: Command-ACL `.acl` **und** `configure` (wie heute), und nur für **andere** Consoles; die eigene Console kann sich nur selbst einschränken, nie lockern. | Maintainer |
| ~~F-3~~ | **Entschieden wie vorgeschlagen, umgesetzt 20.11.2026.** Ein Katalog-Eintrag, der mehr erlaubt als die Datei: ablehnen oder speichern und als wirkungslos melden? Vorschlag: speichern und melden, weil sich die Datei später ändern kann. | Maintainer |
| ~~F-4~~ | **Entschieden wie vorgeschlagen (`status`/`version`), umgesetzt 20.11.2026; `.api` dazu (Nachtrag unter Umsetzung).** Katalog nicht erreichbar bei einer Console mit Katalog-Einträgen: nur `status`/`version` (Vorschlag) oder gar nichts? | Maintainer |
| ~~F-5~~ | **Entschieden wie vorgeschlagen (nur je Console), umgesetzt 20.11.2026.** Gilt die innere Stufe auch für Profile (Einträge je Profil) oder nur je Console? Vorschlag: nur je Console, Profile bleiben reine Datei-Sache. | Maintainer |
| ~~F-6~~ | ~~Sitzungen des MCP~~ **Entschieden 19.11.2026:** Der MCP-Weg MUSS eine eigene Console in der Konfiguration haben, Suffix `-mcp` (`<console>-mcp`). oconsole meldet sich im MCP-Modus nur so an und startet ohne sie nicht. Die innere Stufe bleibt damit je Console. | Maintainer |

## Umsetzungsschritte (nach Abstimmung, alle erledigt 20.11.2026)

1. Zentrale Entscheidungsfunktion prüfen (alle Aufrufer über
   `acl::allowed`/`any_allowed`); Push-Filter (K-19) läuft bereits darüber.
2. Tabelle in der Schemaerweiterung, Lesen beim Start und nach jeder
   Änderung (Cache je Console, Invalidierung bei `.acl`-Änderung und Reload).
3. `.acl set/delete/clear/copy` auf die Katalog-ACL umstellen; Datei-Schreiben
   entfernen.
4. `.acl list` mit drei Sichten; gRPC-Gegenstücke (DotAcl*) entsprechend.
5. F-1 umsetzen, Doku (betrieb-odir.md: Abweichung von Bareos, commands/dot-acl.md).
6. Tests: Schnittmenge je Typ inkl. Negation und Regex, Katalog leer/fehlend,
   Selbst-Einschränkung, Push-Filter mit innerer Stufe.

## Umsetzung (20.11.2026)

Commit `76d7c4c` (`feat(odir): zweistufige ACLs – Katalog-ACL als innere Stufe`)
(Branch `development`, Nachfolger von `ca365ab`).

### Bausteine

| Stelle | Inhalt |
| --- | --- |
| `crates/bareos-orm/schema/v2253_onesimus_schema.json` | Tabelle `onesimus.console_acl` (Schlüssel `console, acl_type, position`, CHECK auf die zehn Typnamen); dieselbe Stufe legt das Schema `onesimus` an und verschiebt die Objekte von 2251/2252 dorthin (zuerst `v2253_console_acl.json` mit `public.onesimus_console_acl`, Commit `76d7c4c`) |
| `crates/odir/src/catalog_acl.rs` | Cache `CatalogAcl` (Ladezustand, Einträge, F-4-Logik), Laden, Schreiben in einer impliziten Transaktion, Thread `dir:catalog-acl` |
| `crates/odir/src/acl.rs` | `allowed` = `file_allowed` ∧ `catalog_allowed`; `any_allowed`, `has_restrictions`, `no_restrictions` berücksichtigen die Katalog-ACL |
| `crates/odir/src/director_config.rs` | Feld `catalog_acl: Arc<CatalogAcl>`; `reload::install_config` reicht denselben Speicher an die neue Konfiguration weiter |
| `crates/odir/src/main.rs` | Katalog anhängen und laden **vor** dem Start der Listener |
| `crates/odir/src/commands/acl.rs` | `.acl list` mit drei Sichten, `.acl set/delete/clear/copy` auf die Katalog-ACL, Regel F-2, Meldung F-3, Protokollzeile |

### Schemanummer v2253

Die Erweiterungen zählen fortlaufend weiter: 2250 ist die Bareos-Basis
(`author: "Bareos"`), 2251 und 2252 sind Onesimus-Stufen (`author:
"Onesimus"`, `min_compatible_version` = Vorgänger). Der Migrator trägt jede
Stufe in `bareos_schema_version.version` ein (die Tabelle hat nur `version`
und `applied_at`, eine eigene Spalte für die odir-Version gibt es nicht) und
setzt die Bareos-Tabelle `version` nur für Bareos-Stufen; `version` bleibt
also 2250. Die nächste Stufe dieses Schemas ist 2253 (so auch schon in
`docs/TESTPLAN.md` als nächste Stufe vorgesehen). Ein künftiges Bareos-Basis-
schema (voraussichtlich 2260) liegt außerhalb.

### Zugriffsregel F-2, genau

Ändern braucht die wirksame CommandACL `.acl` (der Befehl selbst) **und**
`configure`. Für eine **andere** Console ist jede Änderung erlaubt. Für die
**eigene** Console gilt je geändertem ACL-Typ (alte Liste `alt`, neue Liste
`neu`), sonst wird die ganze Änderung abgelehnt:

1. `alt` ist leer (Typ ohne Katalog-Einträge, also ohne innere Einschränkung):
   jede `neu` ist erlaubt; oder
2. `neu` ist nicht leer und entsteht aus `alt` nur durch **Einfügen von
   `!`-Einträgen** (an beliebiger Stelle) und **Entfernen von Einträgen ohne
   `!`**: die übrigen Einträge behalten ihre Reihenfolge, jeder `!`-Eintrag von
   `alt` bleibt. Einträge werden als Text genau verglichen.

Warum das nie erweitert (für beliebige Einträge, auch Regex und `*all*`): die
Liste entscheidet nach dem ersten Treffer, ohne Treffer verboten. Ein
eingefügtes `!x` kann eine Entscheidung nur auf „verboten“ ändern. Ein
entfernter erlaubender Eintrag `p` ändert nur Namen, deren erster Treffer `p`
war (also erlaubt); sie fallen auf einen späteren Eintrag oder auf
„verboten“. Damit ist Katalog(neu) ⊆ Katalog(alt) und, weil die Datei-Stufe
gleich bleibt, auch wirksam(neu) ⊆ wirksam(alt). Alles andere (`!`-Eintrag
entfernen, erlaubenden Eintrag hinzufügen, umstellen, Typ ganz löschen,
`clear`) wird für die eigene Console abgelehnt, auch wenn es im Einzelfall
nicht erweitern würde: die Regel ist syntaktisch und damit entscheidbar;
Enthaltensein von Regex-Listen wird nicht geprüft. Prüfung: gierige
Einbettung (`commands/acl.rs`, `narrower`).

### Wirkungslos, F-3

Gemeldet (Text `no effect`, JSON `ineffective`) wird ein Katalog-Eintrag ohne
`!`, wenn: ein Name, den die Datei verbietet; `*all*`, wenn die Datei für den
Typ nichts erlaubt; ein Regex, der mindestens einen konfigurierten Namen bzw.
registrierten Befehl trifft und dessen Treffer die Datei alle verbietet
(Where/PluginOptions: Regex nie). Der Eintrag wird trotzdem gespeichert.

### Verhalten bei unlesbarem Katalog, F-4

- Geladen wird beim Start (vor den Listenern), nach jedem erfolgreichen
  `reload`, nach jeder `.acl`-Änderung, alle 30 s, solange das letzte Laden
  scheiterte, und alle 60 s, solange es gelang (Thread `dir:catalog-acl`,
  `catalog_acl::refresh_tick`; Nachtrag 20.11.2026, siehe unten). Keine
  DB-Abfrage je ACL-Prüfung.
- Scheitert das Laden, ist eine Console **gesperrt** (nur `status`,
  `.status`, `version`, `.api`; daneben wie immer `quit`/`exit`/`whoami`; alle anderen
  ACL-Typen nichts), außer sie ist als einträgefrei **bekannt**: das letzte
  erfolgreiche Laden fand keine Einträge für sie, und seitdem wurde keine
  `.acl`-Änderung an ihr begonnen (eine Änderung markiert die Console vor dem
  Schreiben; erst das nächste erfolgreiche Laden löscht die Marke). Vor dem
  ersten erfolgreichen Laden ist keine Console bekannt, also alle gesperrt.
  `*UserAgent*` ist keine Console-Ressource, kann keine Einträge haben und
  wird nie gesperrt.
- Gesperrte Consoles verwenden nicht die alten (eventuell zu weiten)
  Einträge, sondern nur die Sperrliste.
- Fehlt die Tabelle, gilt das als erfolgreiches Laden ohne Einträge (wie
  Bareos), solange nie Einträge gelesen wurden; sonst als Fehler.
- `.acl set/delete/clear/copy` lädt vor der Entscheidung neu und lehnt ab,
  wenn das nicht gelingt.
- Jede Katalogabfrage der Katalog-ACL (Laden, Schreiben) ist auf 5 s
  begrenzt (`catalog_acl::CATALOG_TIMEOUT`, `CatalogHandle::query_rows_within`
  / `execute_within`): bei einem Katalogausfall kommt die Ablehnung nach
  höchstens etwa 6 s statt nach dem Acquire-Timeout des Pools (30 s).
- Ohne Katalog (kein `Catalog`) ist die innere Stufe aus.

### Zentrale Funktion, geprüfte Aufrufer

Alle ACL-Entscheidungen laufen über `acl::allowed`/`any_allowed` (direkt oder
über `access`, `access_locked`, `access_audited`, `GrpcAuth::allowed`,
`GrpcAuth::any_allowed`, `push_events::visible_to`). Gefundene Umgehung:
`acl::has_restrictions` und `acl::no_restrictions` lasen nur die Datei-Listen;
Listen und Auswahlen (`list`, `status`, Pool-/Client-Auswahl in
`commands/admin.rs`, `setdevice`, OVFS) überspringen danach die Prüfung je
Zeile, ein katalog-eingeschränkter Typ wäre ungefiltert sichtbar gewesen. Beide zählen jetzt Katalog-Einträge des Typs
(und eine Sperre) als Einschränkung. Weitere Stellen, die Console-ACLs selbst
auswerten, gibt es nicht (`show console` zeigt nur an).

### Tests

`acl::tests::catalog_tier_intersects_and_counts_as_restriction`,
`catalog_acl::tests::*`, `commands::acl::tests::*` (Schnittmenge je Typ mit
Negation, Regex, `*all*`; Typ ohne Einträge; F-2 eigene/andere Console; F-3;
F-4; Cache nach Änderung und Reload; typisierte DotAclList-Antwort),
`grpc::push_events::tests::catalog_acl_restricts_push_events`,
`bareos-orm/tests/onesimus_schema_migration.rs` (frischer Katalog, Katalog auf
Stufe 2252 mit den Objekten in `public`, reiner Bareos-Katalog, halb
verschobener Katalog, fehlendes `CREATE`-Recht; braucht `BAREOS_ORM_TEST_DB`).

### Nachtrag 20.11.2026: offene Punkte erledigt

- **Regelmäßiges Neuladen:** Der Thread `dir:catalog-acl` lädt die
  Katalog-ACL auch im gesunden Zustand alle 60 s neu
  (`catalog_acl::REFRESH_INTERVAL`, `refresh_due`/`refresh_tick`). Änderungen
  direkt per SQL an `onesimus.console_acl` wirken damit spätestens nach
  einer Minute ohne `reload`; Journal `catalog ACL changed in the catalog,
  reloaded (N console(s) with entries)`. Änderungen über `.acl` laden sofort
  und erscheinen dort nicht.
- **`.api` auf der Sperrliste (F-4), `.help` nicht:** `.api` schaltet nur das
  Ausgabeformat der Sitzung um (API 0/1/2, JSON) und gibt nichts preis.
  API-Clients (bconsole `-api`, WebUI, GUI, MCP) senden es als Erstes; ohne
  `.api` könnte ein gesperrter API-Client nicht einmal die Antworten auf
  `status`/`version` auswerten. `.help` listet die Befehle (Konfiguration)
  und bleibt konservativ gesperrt; für die Fehlersuche reicht `status`.
- **Kurzer Timeout:** Laden und Schreiben der Katalog-ACL warten höchstens
  5 s (`CATALOG_TIMEOUT`). Neue Anfrageart `CatalogRequest::Within` im
  Katalog-Worker: Er beginnt eine Anfrage nach Ablauf der Frist nicht mehr
  (sie kann hinter langsamen Anfragen in der Warteschlange gestanden haben)
  und bricht Verbindungsaufbau bzw. Ausführung an der Frist ab; der Aufrufer
  wartet Frist + 1 s. Eine abgelehnte `.acl`-Änderung wird also nicht später
  doch noch ausgeführt. Restfall: Läuft die Frist ab, während PostgreSQL
  das COMMIT schon ausgeführt, die Antwort aber noch nicht geschickt hat,
  meldet `.acl` einen Fehler, obwohl die Änderung gilt; das anschließende
  Laden bzw. `.acl list` zeigt den tatsächlichen Stand.
- Tests: `catalog_acl::tests::refresh_schedule`,
  `periodic_refresh_picks_up_rows_changed_by_sql`,
  `outage_answers_within_the_catalog_timeout`,
  `catalog_worker::within_tests::*`,
  `acl::tests::catalog_tier_intersects_and_counts_as_restriction` (`.api`
  erlaubt, `.help` gesperrt).

### Offen

- Die Katalog-Einträge stehen im typisierten gRPC-Ergebnis nur in
  `extra_fields` (JSON-Text); eigene Proto-Felder (`catalog`, `effective`,
  `catalog_state`, `locked` in `ConsoleAclInfo`) brauchen die GUI-Seite und
  sind bewusst nicht umgesetzt.
