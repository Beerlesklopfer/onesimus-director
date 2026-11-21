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
| `configure add console`/`profile`/`user` (Bareos-Befehl) | ~~legt eine Datei mit ACL-Direktiven an~~ seit 19.11.2026 abgelehnt (F-1) | `profile`/`user` bleiben abgelehnt; `console` seit 20.11.2026 in enger Form ([Nachtrag F-1](#nachtrag-20112026-f-1-configure-add-console-in-enger-form)) |
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
| ~~F-1~~ | **Nachtrag 20.11.2026:** `configure add console` in enger Form erlaubt, siehe [Nachtrag F-1](#nachtrag-20112026-f-1-configure-add-console-in-enger-form); `profile`, `user` und `configure delete console` bleiben abgelehnt. **Entschieden und umgesetzt 19.11.2026:** `configure add console`, `profile` und `user` werden immer abgelehnt (nicht nur mit ACL-Zeilen); Consoles legt nur der Editor an. Ursprüngliche Frage: `configure add console`/`profile` ist ein Bareos-Befehl und schreibt ACL-Direktiven in eine Datei. Regel 3 schließt das aus. Vorschlag: ablehnen, wenn ACL-Direktiven (`*Acl`, `Profile`) im Aufruf stehen; eine Console ohne ACL-Zeilen hat in Bareos keine Rechte und darf angelegt werden. Abweichung von Bareos, kommt in betrieb-odir.md. | Maintainer |
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

### Nachtrag 20.11.2026: F-1, `configure add console` in enger Form

Entscheidung des Nutzers 20.11.2026: Consoles (etwa für GUI-Benutzer) sollen
ohne Editor angelegt werden können, ohne einen Weg zur Selbstausweitung zu
öffnen. Variante „Profile ≤ eigene Rechte“. Profile und User bleiben reine
Editor-Sache, `configure delete console` bleibt abgelehnt (Meldung
unverändert).

**Form.** `configure add console name=<n> password=<p> profile=<P1>
[profile=<P2> …] [description=<d>]`; Schlüssel wie Bareos-Schlüsselwörter
normalisiert, `profile=a,b` wie zwei Profile. Abgelehnt: jede `…acl`-Direktive,
jede andere Direktive, fehlendes Passwort, kein oder ein unbekanntes Profil,
vorhandener Name, `*UserAgent*`, ungültige Namen, Werte mit Steuerzeichen,
`"`, `\`, `;`, `@`. Die neue Console hat keine eigenen ACL-Zeilen; ihre
effektive ACL ist die Datei-ACL ihrer Profile, die Katalog-ACL (`.acl set`)
kann sie danach weiter einschränken. Schreiben wie jedes `configure add`
(Datei `bareos-dir.d/console/<n>.conf`, sofort im laufenden Director);
danach prüft odir, dass die eingelesene Console genau die verlangten
Direktiven und Profile hat, sonst werden Datei und Ressource entfernt.
Das Passwort steht nie in einer Antwort (Text und `content`:
`Password = "***"`); Audit-Zeile und gRPC-Feld `command` maskieren
`password=`.

**Regel „Profile ≤ eigene Rechte“** (`crates/odir/src/acl.rs`,
`grant_error`). Die anlegende Console C darf nur Profile vergeben, deren
Rechte ihre eigenen effektiven Rechte (Datei ∩ Katalog, dieselbe Auswertung
wie `acl::allowed`) nicht übersteigen, je ACL-Typ (`Kind::ALL`):

- *CommandAcl:* Für jeden Befehlsnamen, den eine Prüfung treffen kann
  (Bareos-Befehlstabelle, registrierte Befehle und Aliasse; auch die festen
  Prüfungen wie `configure`, `.sql`, `messages` verwenden diese Namen) gilt:
  erlauben die Profile ihn (erster Treffer über die Profile), muss
  `acl::allowed(C, Command, name)` gelten. Punktbefehle wie `.api` sind in
  Bareos Regexe; über die endliche Namensmenge werden sie genau entschieden.
- *Übrige Typen* (Namen offen, auch künftige Ressourcen zählen): Die von den
  Profilen erlaubten Namen werden nach oben abgeschätzt, die von C erlaubten
  nach unten, beide als „genau diese Namen“ oder „alle außer diesen“; die
  obere Schätzung muss in der unteren liegen. Für Listen aus Namen,
  `!`-Einträgen und `*all*` ist das genau. Regexe: in einem Profil zählt ein
  erlaubender Regex wie `*all*`, ein `!`-Regex wird übergangen; bei C wird
  ein erlaubender Regex übergangen, ein `!`-Regex zählt wie `!*all*`. Jede
  dieser Ersetzungen verschiebt die Menge nur in die sichere Richtung
  (erster Treffer entscheidet: einen erlaubenden Eintrag weglassen oder die
  Liste mit einem Verbot beenden kann nur weniger erlauben, und umgekehrt).
  Katalog-Einträge von C werden genauso als zweite Liste geschnitten.
- `!`-Einträge in Profilen schränken nur ein; ein leerer Typ erlaubt nichts.
- Eine gesperrte C (F-4) legt nichts an; ohne angemeldete Console auch nicht.
- Erlauben die Profile `.acl` (oder den Alias `.profile`), muss C frei von
  Katalog-Einträgen sein. Grund: F-2 begrenzt nur Änderungen an der
  *eigenen* Console; die neue Console dürfte die Katalog-Einträge von C
  löschen und C damit über die Rechte heben, die C beim Anlegen hatte.

**Warum das keine Selbstausweitung öffnet.** C kennt das Passwort der neuen
Console N und kann sich als N anmelden. Die Regel stellt sicher, dass N je
Typ nur Namen erlaubt, die C beim Anlegen selbst durfte; über N gewinnt C
also kein Recht dazu. Ohne Katalog-Einträge von C kann N mit `.acl` an C
nur rückgängig machen, was später hinzukommt, nicht über den Stand beim
Anlegen hinaus.

**Was ein Mensch erwarten könnte, aber abgelehnt wird.** Eine Console wie
die MCP-Console auf PDC (`CommandAcl = "!.acl", "!.profile", "configure",
".config"; Profile = operator`) kann ein Profil mit `*all*` in CommandAcl
nicht vergeben, auch nicht das Bareos-Standardprofil `operator`
(`!…, *all*`): es erlaubt `.acl`, das ihr selbst verboten ist. Vergeben
lassen sich nur Profile, die genau innerhalb ihrer Rechte liegen. Ein
erlaubender Regex in einem Profil (außer CommandAcl) zählt wie `*all*`; er
lässt sich nur vergeben, wenn C für den Typ alle Namen außer höchstens
denen darf, die das Profil vorher mit `!` verbietet (etwa C `!geheim,
*all*`, Profil `!geheim, backup-.*`), auch wenn C den Regex selbst in der
Liste hat. Pfade in WhereAcl mit `.` sind Regexe und fallen ebenfalls
darunter.

**Bleibende Eigenschaften.** N ist eine eigene Identität: wird C später
eingeschränkt (Datei oder Katalog), betrifft das N nicht; N muss dann
eigens eingeschränkt oder im Editor gelöscht werden. Eine künftige
odir-Version mit neuen Befehlen wertet CommandAcl-Einträge `*all*` oder
Regexe der Profile von N auch für die neuen Befehle aus; die Prüfung
geschah über die Befehle der Version beim Anlegen. Katalog-Einträge, die
unter dem Namen von N aus einer früheren, gelöschten Console stehen,
gelten weiter; sie schränken nur ein.

**Schnittstellen.** gRPC `ConfigureAdd` (`CONFIGURE_RESOURCE_CONSOLE`):
`directives` ist `map<string, string>`, mehrere Profile als Kommaliste;
JSON-Anfragen dürfen eine Liste angeben. MCP `configure_add`
(`resource: "console"`): `profile` als Zeichenkette oder Liste. Die
Prüfung liegt nur in odir.

Tests: `commands::configure::tests::unrestricted_creator_adds_console_with_all_profile`,
`restricted_creator_cannot_escalate`, `catalog_tier_limits_the_creator`,
`console_form_refusals`, `configure_add_of_acl_resources_is_refused`,
`acl::tests::name_sets_approximate_in_the_safe_direction`,
`grpc::api_write::tests::admin_lines` (Plan), oconsole
`mcp_write::tests::configuration_and_daemon_lines`, `mcp_typed` (Kommaliste).

### Offen

- Die Katalog-Einträge stehen im typisierten gRPC-Ergebnis nur in
  `extra_fields` (JSON-Text); eigene Proto-Felder (`catalog`, `effective`,
  `catalog_state`, `locked` in `ConsoleAclInfo`) brauchen die GUI-Seite und
  sind bewusst nicht umgesetzt.

## Assistentenkonsolen (`*-mcp`, 21.11.2026)

Eine dritte Stufe vor den beiden ACL-Stufen: `acl::allowed` fragt zuerst `assistant_allowed`. Für eine Console, deren Name auf `-mcp` endet, sind `.acl`, `.profile`, `sqlquery`, `.sql` und `query` immer verboten, gleich was Datei-ACL und Katalog-ACL sagen (`ASSISTANT_REFUSED`). Ein Assistent kann damit weder seine eigenen Grenzen lösen noch rohes SQL lesen. `grant_error` verweigert ihm außerdem `configure add console`. Das Profil `assistant` gibt Lesen und `configure`. Außerdem speichert ein Assistent kein Programm in der Konfiguration (S-55): `RunScript`, die Run-Before/After-Direktiven, `Plugin`, `File = "|…"`, `Write Bootstrap = "|…"` werden in `config_edit::assistant_refusal` für `*-mcp` abgelehnt, auf allen Wegen (BNET `configure add/update`, `.config`, gRPC). Gefunden beim Gegenlesen der Webseite am 21.11.2026: Ein Assistent konnte einen Job mit `Run Before Job` anlegen, den ein Zeitplan startet. Dokumentation für Anwender: [mcp.md](../wiki/mcp.md). Tests: `acl::tests::assistant_consoles_never_change_acls_or_read_sql`, `restricted_creator_cannot_escalate` (Creator-Console heißt dort `pdc-ops`, weil ein `*-mcp`-Creator keine Consolen mehr anlegt).
