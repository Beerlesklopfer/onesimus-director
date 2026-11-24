# Betrieb des Onesimus Directors (odir)

Nachschlageseite für Administratoren: wie `odir` als Dienst läuft, welche
Einstellungen es außerhalb von `/etc/bareos/bareos-dir.d` gibt und wo es sich
bewusst anders verhält als `bareos-dir`. Die Kommandozeilenparameter stehen in
[kommandozeile.md](kommandozeile.md).

`odir` ist ein Drop-in-Ersatz für `bareos-dir` 25: dieselbe Konfiguration,
derselbe Katalog, dieselben Clients und Storage-Daemons. Der Rückweg auf
`bareos-dir` muss jederzeit offen bleiben. Deshalb stehen in `bareos-dir.d` keine
Direktiven, die `bareos-dir` nicht kennt. Einstellungen von Onesimus liegen
im Overlay `/etc/onesimus/odir.d/` (Option `--overlay`; `configure update …
overlay` schreibt es, Näheres in [osd.md](osd.md#der-director-wertet-es-aus))
oder laufen über Umgebungsvariablen (siehe unten). Dasselbe gilt für
`osd.d/` und `ofd.d/` im selben Verzeichnis. Ältere Einträge dieser
Direktiven in `bareos-dir.d` (gRPC, mDNS, DB Driver) nimmt `odir` noch an,
warnt aber.

## Dienst

| | |
| --- | --- |
| Unit | `odir.service` (`/usr/lib/systemd/system/odir.service`) |
| Benutzer | `bareos:bareos`, Arbeitsverzeichnis `/var/lib/bareos` |
| Start | `/usr/sbin/odir -c /etc/bareos/bareos-dir.d -f` |
| Reload | `systemctl reload odir`: erst `odir -t`, nur bei Erfolg SIGHUP |
| Neustart bei Fehler | nach 30 s, höchstens 10-mal am Tag (`systemctl reset-failed odir` setzt zurück) |
| Kein Neustart | Exit 41 (falsche Argumente) und 42 (Konfigurationsfehler) |
| Gegenseitiger Ausschluss | `Conflicts=bareos-director.service`: startet einer, stoppt der andere |
| Paket-Upgrade | die postinst startet den Dienst sofort neu (`dpkg -i`) |

Exit-Codes beim Start:

| Code | Bedeutung |
| --- | --- |
| 0 | regulär beendet (SIGTERM) |
| 1 | Laufzeitfehler, z. B. Katalog nicht erreichbar, Adresse nicht bindbar |
| 42 | Konfigurationsfehler, auch fehlende oder unlesbare Konfiguration |

Vor einem Neustart die Konfiguration prüfen (liest den Katalog nur, ändert nichts):

```sh
sudo -u bareos /usr/sbin/odir -t -c /etc/bareos/bareos-dir.d; echo $?
```

## Adressen und Ports

Wie `bareos-dir` 25:

- `DirAddresses` (Blockform): jede Zeile wird gebunden. `ip` und `ipv4` sind
  IPv4, `ipv6` ist IPv6. Werte ohne Anführungszeichen schreiben
  (`addr = ::1`, nicht `addr = "::1"`; `bareos-dir` lehnt die zitierte Form ab).
  Eine Zeile ohne `port` nimmt 9101, nicht `DirPort`. `DirAddresses` darf nicht
  mit `DirAddress` oder `DirPort` gemischt werden.
- `DirAddress`: die erste Adresse, auf die ein Name auflöst.
- ohne Angabe: `0.0.0.0` und `::`.
- Eine Adresse, die sich nicht binden lässt, wird dreimal im Abstand von 5 s
  versucht; danach endet der Start (kein Lauschen auf einer Teilmenge).
- gRPC (TLS) lauscht auf denselben Adressen auf `DirPort + 1000` (Vorgabe
  10101) bzw. auf `gRPC Port`.

Prüfen: `ss -ltnp | grep -E '9101|10101'` und die Zeilen `listening on …` im Journal.

## Katalog beim Start

1. Verbindung wie `CheckCatalog()`: jeder Katalog muss sich öffnen lassen
   (sechs Versuche, 5 s Abstand), sonst endet der Start mit Exit 1.
   Verbunden wird mit allen Angaben der Catalog-Ressource (Adresse oder
   Socket, Port, Benutzer, Passwort), genauso wie bei `odir -t`.
2. Schema-Erweiterung (gewollte Funktion): der Migrator spielt die
   Onesimus-Stufen aus `/usr/lib/bareos/scripts/ddl/v2251_*.json` bis
   `v2254_*.json` ein, wenn sie noch fehlen. Alle
   Onesimus-Objekte liegen im eigenen PostgreSQL-Schema `onesimus`,
   `public` bleibt reines Bareos (Abschnitt [Katalog-Schema
   `onesimus`](#katalog-schema-onesimus)). Die Bareos-Tabelle `version`
   bleibt auf der Bareos-Version (2250), die Onesimus-Stufen stehen in
   `public.bareos_schema_version`; `bareos-dir` öffnet den Katalog also
   weiterhin. Die Migrationen löschen keine Daten (kein DROP TABLE, kein
   TRUNCATE). Journal: `applied N schema migration(s)` bzw. `schema up to
   date`; scheitert eine Stufe, `catalog migration failed: …` (der Director
   läuft ohne die Erweiterung weiter und versucht es beim nächsten Start
   erneut).
   Danach prüft die **Selbstheilung** jedes Onesimus-Objekt und spielt ein
   fehlendes oder kaputtes neu ein (Abschnitt [Selbstheilung beim
   Start](#selbstheilung-beim-start)). Journal: `N Onesimus catalog objects
   intact`, sonst `self-heal re-created …` bzw. `… missing or broken and not
   repairable …`; der Start endet daran nie.
3. Jobs eines früheren Laufs, die noch auf `C` oder `R` stehen, enden als `f`
   (wie `CheckCatalog(UPDATE_AND_FIX)`); nur beim Start, nicht bei `reload`.
4. Fällt der Katalog im Betrieb aus, scheitern die Anfragen mit einer Meldung;
   ist er wieder da, verbindet der Director von selbst neu.

## Katalog-Schema `onesimus`

Entscheidung des Maintainers vom 20.11.2026: Onesimus-Objekte bekommen ein
eigenes PostgreSQL-Schema, `public` bleibt reines Bareos.

| Objekt | Ort | Stufe |
| --- | --- | --- |
| Bareos-Tabellen, `version` (2250), Bareos-Funktionen `decode_lstat`, `bareos_frombase64` | `public` | 2250 (Bareos-DDL) |
| Views `vw_job_summary`, `vw_volume_status`, `vw_last_backup_per_client`, `vw_file_list`; Funktionen `sp_create_job_record`, `sp_update_job_end`, `fn_find_appendable_volume`, `fn_find_latest_backup`, `sp_create_jobmedia`, `fn_since_utime` | `onesimus` | 2251 |
| Views `vw_running_jobs`, `vw_media_list`; Funktionen `sp_prune_expired_volumes`, `sp_prune_expired_jobs`, `sp_purge_volume`, `fn_director_status` | `onesimus` | 2252 |
| Tabelle `console_acl` (Katalog-ACL, [ACL-ZWEISTUFIG.md](../plans/ACL-ZWEISTUFIG.md)) | `onesimus` | 2253 |
| Funktionen `fn_vw_job_summary`, `fn_vw_volume_status`, `fn_vw_last_backup_per_client`, `fn_vw_file_list`, `fn_vw_running_jobs`, `fn_vw_media_list`; die sechs Views lesen nur noch diese (spätgebunden) | `onesimus` | 2254 |
| `bareos_schema_version` (Onesimus-Stufen) | `public` (**Ausnahme**, siehe unten) | – |

Stufen:

- **2251/2252** legen ihre Objekte direkt in `onesimus` an (die Namen in den
  JSON-Dateien sind qualifiziert, `onesimus.vw_job_summary`). Das gilt für
  neue Kataloge und für Kataloge, die nur die Bareos-DDL haben.
- **2253** legt das Schema an (falls es fehlt), verschiebt die Views und
  Funktionen, die ein odir vor dieser Stufe unqualifiziert im Standardschema
  (`current_schema()`, normalerweise `public`) angelegt hat, mit
  `ALTER VIEW/FUNCTION … SET SCHEMA onesimus` (OID, Eigentümer, Rechte und
  davon abhängige eigene Views des Betreibers bleiben erhalten), spielt
  danach ihre ausgelieferte Definition erneut ein (`CREATE OR REPLACE`; ein
  fehlendes Objekt entsteht so neu) und legt `onesimus.console_acl` an.
  Liegt ein Objekt in beiden Schemas (z. B. abgebrochener Lauf, von Hand
  neu angelegt), wird die Kopie in `public` ohne CASCADE gelöscht. Auf einem
  Katalog auf Stufe 2252 (wie dem Produktionskatalog) verschiebt sie also
  alles; auf einem neuen Katalog ist sie ein No-op bis auf die Tabelle. Ein
  zweiter Start ändert nichts.
- **2254** macht die Views spätgebunden (Abschnitt [Bareos-Upgrade](#bareos-upgrade)):
  sie legt je View eine Funktion `onesimus.fn_<view>()` an und ersetzt die
  View mit `CREATE OR REPLACE VIEW` durch `SELECT … FROM
  onesimus.fn_<view>()`. Name, Spalten, Spaltentypen (auch `character(1)`)
  und Reihenfolge bleiben gleich; OID, Rechte und eigene Views des
  Betreibers auf den Views bleiben erhalten.
- Schema-Eigentümer ist der verbundene Katalog-Benutzer (`CREATE SCHEMA`
  ohne `AUTHORIZATION`). Dafür braucht er das Recht `CREATE` auf der
  Datenbank. Fehlt es, endet die Migration mit `cannot create schema
  onesimus`, die Stufe wird **nicht** eingetragen, die Objekte bleiben in
  `public`, odir läuft weiter (Katalog-ACL nur aus Dateien,
  `fn_find_latest_backup` über die eingebaute Abfrage) und versucht es beim
  nächsten Start erneut. Die Meldung nennt die Abhilfe mit den Werten
  **dieses** Katalogs (Datenbank und Benutzer der Verbindung, also
  `DbName`/`DbUser` der Catalog-Ressource), eine von beiden:
  `GRANT CREATE ON DATABASE <DbName> TO <DbUser>;` oder als Superuser in
  der Datenbank `<DbName>`: `CREATE SCHEMA onesimus AUTHORIZATION <DbUser>;`
  (ein vorhandenes Schema legt odir nicht neu an und braucht dann das
  Datenbankrecht nicht).
- **Mehrere Kataloge:** odir migriert und heilt beim Start **jede**
  Catalog-Ressource, jede in ihrer eigenen Datenbank; Ressourcen auf
  dieselbe Datenbank (Adresse, Port, Name) nur einmal. Die Journalzeilen
  nennen Katalog und Datenbank. Die Katalog-ACL (`onesimus.console_acl`)
  liest odir aus dem ersten Katalog, wie alle Director-weiten Daten.

Zugriff aus odir: Onesimus-Objekte werden **qualifiziert** angesprochen
(`onesimus.console_acl`, `onesimus.fn_find_latest_backup(…)`); der
`search_path` der Katalogverbindungen bleibt unverändert. Unqualifizierte
Namen lösen damit genau wie vorher in die Bareos-Tabellen von `public` auf.
`onesimus` steht nie im `search_path`.

Für `.sql`, `sqlquery` und eigene Skripte: Bareos-Tabellen weiter
unqualifiziert (`SELECT … FROM job`). Onesimus-Objekte nur noch qualifiziert
(`SELECT * FROM onesimus.vw_job_summary`); wer bisher `vw_…` oder
`sp_…` unqualifiziert aufgerufen hat, muss `onesimus.` voranstellen.
`.sqltables`/`.sqlschema` zeigen wie bei Bareos nur `public` (die Tabelle
der Katalog-ACL also nicht mehr).

**Ausnahme `bareos_schema_version` in `public`:** Die Versionstabelle der
Onesimus-Stufen bleibt in `public`. Jeder bisherige odir sucht sie dort
(unqualifiziert). Läge sie in `onesimus`, hielte ein zurückgerollter älterer
odir den Katalog für Stufe 2250 und legte die Views und Funktionen von
2251/2252 erneut in `public` an. Sie ist die einzige Onesimus-Tabelle in
`public`.

### Bareos-Upgrade

Ein Bareos-Update-Skript (`update_bareos_tables`) darf die Bareos-Tabellen
in `public` ändern wie ohne Onesimus: `ALTER TABLE … ALTER COLUMN … TYPE`,
`DROP COLUMN`, `DROP TABLE` (ohne CASCADE). Kein Onesimus-Objekt hängt in
`pg_depend` von einer Tabelle in `public` ab (seit Stufe 2254, Entscheidung
des Maintainers vom 20.11.2026):

- **Funktionen** haben Rümpfe als Zeichenkette (PL/pgSQL oder `LANGUAGE
  sql` mit `$$…$$`, kein SQL-Standard-Rumpf `BEGIN ATOMIC`, der
  Abhängigkeiten einträgt). PostgreSQL löst die Tabellen darin erst beim
  Aufruf auf.
- **Views** lesen seit 2254 nicht mehr die Tabellen, sondern je eine solche
  Funktion: `onesimus.vw_job_summary` ist `SELECT … FROM
  onesimus.fn_vw_job_summary()`. Die View hängt nur von der Funktion ab.
  Die Funktionen sind `STABLE`, `LANGUAGE sql`, ohne `SET`-Klauseln; der
  Planer setzt ihren Rumpf in die Abfrage ein, Bedingungen wie `WHERE
  jobid = …` wirken also weiter auf die Indizes der Tabellen. Jede Spalte
  ist im Rumpf auf den bisherigen Typ gecastet: ändert Bareos z. B.
  `job.jobfiles` auf `bigint`, liefern die Views weiter `integer`.

Nach dem Upgrade: Fällt dabei eine Spalte weg, die eine Funktion liest,
scheitert nur diese Funktion (und ihre View) beim Aufruf; die Selbstheilung
meldet sie beim nächsten Start (`not repairable`), bis eine Onesimus-Version
mit angepasster Definition läuft. Bareos-Tabellen und `bareos-dir` sind
davon nicht berührt.

Rückfall (nur falls ein Objekt doch im Weg steht, z. B. eine eigene View des
Betreibers auf einer Bareos-Tabelle, die PostgreSQL ebenso schützt): das
Objekt vor dem Upgrade löschen. Die Onesimus-Views und -Funktionen legt odir
beim nächsten Start selbst neu an (Selbstheilung); von Hand ist nichts mehr
nötig.

### Selbstheilung beim Start

Nach der Migration prüft odir bei jedem Start jedes ausgelieferte
Onesimus-Objekt gegen die neueste Definition aus den JSON-Dateien
(`bareos_orm::Migrator::heal`):

| Objekt | gilt als kaputt, wenn | Prüfung (führt nichts aus) |
| --- | --- | --- |
| Schema `onesimus` | es fehlt | `pg_namespace` |
| Tabelle `console_acl` | sie fehlt | `to_regclass` |
| Funktion | sie fehlt, ihr Rumpf (`prosrc`), Rückgabetyp oder ihre Sprache von der ausgelieferten abweicht, oder der Validator der Sprache sie ablehnt | `pg_proc`; `fmgr_sql_validator`/`plpgsql_validator` (parst den Rumpf, bei SQL auch gegen die aktuellen Tabellen); keine Funktion wird aufgerufen, mehrere schreiben |
| View | sie fehlt, eine Relation außerhalb von `onesimus` direkt liest (alte Form vor 2254) oder `EXPLAIN SELECT * FROM <view>` scheitert | `pg_class`, `pg_depend`; `EXPLAIN` plant nur |

Ein kaputtes Objekt wird mit `CREATE OR REPLACE` neu eingespielt. Lehnt
PostgreSQL das ab (geänderter Rückgabetyp, geänderte Spalten), wird es
**ohne CASCADE** gelöscht und neu angelegt, in einer Transaktion (scheitert
das Anlegen, bleibt das alte Objekt). Hängen ausgelieferte Views an einer
solchen Funktion, werden sie in derselben Transaktion gelöscht und neu
angelegt; hängt ein eigenes Objekt des Betreibers daran, unterbleibt die
Reparatur dieses Objekts mit Meldung (`… (not shipped) depends on it`).
Jede Reparatur wird danach erneut geprüft.

Journal: je Objekt `catalog self-heal: re-created <art> <name>: <grund>`
bzw. `cannot repair …`, dann eine Zusammenfassung. Der Start endet daran
nie. Die Selbstheilung läuft nur, wenn der Katalog genau auf der neuesten
mitgelieferten Stufe steht: darunter ist die Migration unvollständig (sie
läuft beim nächsten Start erneut), darüber gehören die Objekte einem
neueren odir und werden nicht angefasst. Sie braucht keine weiteren Rechte
als die Migration (Eigentümer der Objekte).

### Versionsführung: jeder Schritt einzeln

Bareos-Basisversionen (2250, künftig z. B. 2260) und Onesimus-Schritte
(2251–2254) teilen einen Nummernraum, werden aber getrennt geführt
(`bareos_orm::migration::AppliedSteps`, seit 20.11.2026):

- Ein **Onesimus-Schritt** gilt nur dann als ausgeführt, wenn genau seine
  Nummer in `public.bareos_schema_version` steht. Die Bareos-Zahl in
  `version` sagt darüber nichts. Bisher entschied die höchste Zahl beider
  Tabellen. Hätte Bareos den Katalog mit seinem eigenen Update-Skript auf
  2260 gehoben, bevor odir 2253/2254 ausführte, wären beide Schritte
  stillschweigend übersprungen worden (nachgestellt im Test
  `bareos_upgrade_before_odir_never_skips_onesimus_steps`; mit dem alten
  Migrator: 0 Schritte).
- Ein **Bareos-Basisschritt** gilt als ausgeführt, wenn er eingetragen ist
  oder `version` mindestens seine Nummer hat. Ihn führt odir nie ein zweites
  Mal aus.
- Fehlt ein Onesimus-Schritt mitten in der Reihe, führt odir ihn aus und
  danach alle späteren Onesimus-Schritte noch einmal, ohne sie doppelt
  einzutragen. Sonst stünde die ältere Definition eines Objekts über der
  neueren.
- Jeder Schritt wird gegen den **Stand aller früheren Schritte** abgeglichen,
  nicht nur gegen die vorige Datei. Erweiterungsdateien enthalten nur ihre
  eigenen Objekte; eine neue Spalte an `onesimus.console_acl` in einem
  späteren Schritt wäre sonst nie angelegt worden.
- Zwei Schemadateien mit derselben Nummer lehnt odir beim Laden ab. Was der
  Abgleich nicht ausdrücken kann (geänderte Spaltendefinition, neue
  Fremdschlüssel oder Constraints an bestehender Tabelle, umdefinierter
  Index), meldet odir als Warnung `not migrated automatically`.
  Ein Unit-Test (`every_shipped_ddl_item_is_migrated`) stellt sicher, dass
  keine ausgelieferte Datei so etwas enthält und jedes Element jeder Datei
  in den Anweisungen landet.
- Der Datenbanktest `every_migration_path_ends_with_the_same_catalog`
  vergleicht drei Wege Objekt für Objekt (Spalten, Funktionsrumpf,
  View-Definition, Constraints, Indizes): Neuinstallation, ein Katalog aus
  dem Bareos-DDL und der Produktionsweg (Stand vor 2253, dann 2253/2254).
  Alle drei enden gleich.
- Die Selbstheilung prüft ebenfalls Schritt für Schritt: Eine Bareos-Zahl
  über den ausgelieferten Schritten ist kein „neuerer odir“. Nur
  eingetragene Onesimus-Schritte, die dieser odir nicht kennt, lassen sie
  aussetzen.

### Rückweg auf einen älteren odir

Ein odir auf Stufe 2253 (Stand `c27ad72`) auf einem Katalog auf 2254: er
migriert nichts (`database already at v2254 (latest: v2253)`), die
spätgebundenen Views liefern dieselben Spalten. Er hat keine Selbstheilung.

Ein odir vor dieser Stufe (z. B. die Version auf Stufe 2252) auf einem
migrierten Katalog:

- Er liest `public.bareos_schema_version` = 2253, kennt selbst nur bis 2252
  und migriert nichts (`database already at v2253 (latest: v2252)`). Er
  legt nichts in `public` neu an. (Geprüft am 20.11.2026 mit dem Stand
  `ca365ab` auf einem Wegwerf-Katalog.)
- Er ruft `fn_find_latest_backup` unqualifiziert auf; der Aufruf scheitert
  und er nimmt die eingebaute Abfrage mit demselben Ergebnis. Sonst benutzt
  sein Code keine Onesimus-Objekte.
- Die Katalog-ACL (`onesimus.console_acl`) wertet ein odir vor den
  zweistufigen ACLs nicht aus: es gelten nur die Datei-ACLs. Ein
  Entwicklungsstand `76d7c4c` (nie veröffentlicht) sucht
  `onesimus_console_acl`, findet sie nicht und fällt ebenso auf die
  Datei-ACLs zurück. Einschränkungen per `.acl` gehen beim Rückweg also
  verloren, bis wieder ein aktueller odir läuft; die Einträge selbst bleiben
  erhalten.
- Eigene Abfragen auf `vw_…` ohne Schema scheitern auch unter dem alten odir.
- Wer dauerhaft zurück will und die Objekte wieder in `public` braucht:
  `ALTER VIEW onesimus.<view> SET SCHEMA public;` bzw. `ALTER FUNCTION
  onesimus.<funktion>(…) SET SCHEMA public;` für jedes Objekt der Tabelle
  oben, dann `DELETE FROM bareos_schema_version WHERE version = 2253;`. Die
  Tabelle `onesimus.console_acl` stört einen alten odir nicht.

## Umgebungsvariablen

Vollständige Liste der Variablen, die `odir` im Betrieb liest. Gesetzt werden
sie mit einem systemd-Drop-in:

```sh
sudo systemctl edit odir
#   [Service]
#   Environment=ONESIMUS_DEFAULT_MEDIA_WAIT=3600
sudo systemctl restart odir
```

| Variable | Wirkung | Werte | Vorgabe |
| --- | --- | --- | --- |
| `ONESIMUS_DEFAULT_MEDIA_WAIT` | Wartezeit auf ein Volume für Jobs ohne `Max Wait Time`; danach wird der Job abgebrochen | ganze Sekunden > 0 | 600 |
| `PGTZ` | Zeitzone der Katalogsitzung (wie bei libpq); sonst die Einstellung von Rolle/Datenbank, sonst `log_timezone` des Servers | Zeitzonenname | – |
| `RUST_LOG` | Filter der Log-Ausgabe (tracing `EnvFilter`), ersetzt die Vorgabe | z. B. `info`, `debug,hyper=info` | `info` (mit `-d`: `debug`) |

Nur für Tests, nicht für den Betrieb: `ONESIMUS_CATREQ_DB`,
`OVFS_EQUIV_TEMPLATE`, `OVFS_EQUIV_URL`, `OVFS_FALLBACK_URL`,
`UPDATE_GRPC_API_DOC`. Beim Bau: `SOURCE_DATE_EPOCH` (Build-Datum in der
Versionsangabe, für reproduzierbare Pakete).

### ONESIMUS_DEFAULT_MEDIA_WAIT

- Steuert, wie lange ein Job ohne `Max Wait Time` auf ein beschreibbares
  Volume wartet (z. B. bei vollem Pool oder fehlendem Medium). Danach bricht
  der Watchdog den Job ab; die Meldung nennt den „Onesimus default“ und geht
  wie jede Job-Meldung an die Messages-Ziele (auch Mail).
- Einheit: ganze Sekunden. Vorgabe: 600 (Onesimus-Entscheidung C-085).
- Ungültige Werte (leer, 0, negativ, keine Zahl) fallen auf 600 zurück; im
  Journal steht dann eine Warnung.
- Es gibt keinen Wert für „unbegrenzt“.
- `Max Wait Time` im Job (oder in seinen JobDefs) hat immer Vorrang.
- Wird einmal beim Start gelesen: wirksam erst nach `systemctl restart odir`
  (nicht nach `reload`).
- Wirksamen Wert prüfen:

  ```sh
  sudo journalctl -u odir -b | grep "volume wait"
  # volume wait for jobs without Max Wait Time: 3600 s (ONESIMUS_DEFAULT_MEDIA_WAIT)
  systemctl show odir -p Environment
  ```

**Unterschied zu bareos-dir:** `bareos-dir` wartet ohne `Max Wait Time`
unbegrenzt. `odir` begrenzt die Wartezeit, damit ein nächtlicher Job bei vollem
Pool nicht tagelang hängt. Eine eigene Direktive dafür gibt es bewusst nicht:
`bareos-dir` würde die Konfiguration mit einer unbekannten Direktive nicht mehr
lesen, und der Rückweg wäre versperrt. Wer das Verhalten von `bareos-dir` auch
unter `odir` will, setzt `Max Wait Time` im Job; das verstehen beide Directors.

## Meldungen per Mail und an den Operator

`Mail`, `MailOnError`, `MailOnSuccess` und `Operator` der Messages-Ressource
werden wie in `bareos-dir` bedient: Mail sammelt die Meldungen eines Jobs und
ruft am Job-Ende einmal `MailCommand` auf (Meldungen auf stdin, Job-Codes wie
`%r`, `%j`, `%e` ersetzt); `Operator` ruft `OperatorCommand` je Meldung sofort
auf. Ohne `MailCommand` nimmt `odir` wie `bareos-dir` `/usr/lib/sendmail -F
BAREOS <Empfänger>`. Daemon-Meldungen ohne Job gehen am Ende jedes Jobs, jeder
Konsolensitzung und beim Beenden hinaus. Fehler des Mail-Programms stehen als
„Message delivery ERROR“ im Log.

## AutoPrune am Job-Ende

Nach jedem Job prunt der Director Jobs und Files des Clients nach `Job
Retention` und `File Retention` (Pool-Retention vor Client-Retention), wenn
`AutoPrune = yes` gesetzt ist (wie `bareos-dir`). Der erste Job nach dem
Wechsel von einer Version ohne AutoPrune prunt den ganzen Rückstand; vorher den
Katalog sichern.

Strenger als `bareos-dir` (Entscheidung des Nutzers vom 19.11.2026): Die
jüngste erfolgreiche Kette je Client und FileSet-Name im Katalog wird nie
gepurgt, auch wenn sie älter als die Retention ist:

- Kette: der letzte Full mit JobStatus `T`/`W`, die letzte Differential danach
  (mit Dateien) und die Incrementals danach, wie `AccurateGetJobids` (Level
  Incremental); dazu Base-Jobs der Kette. Das FileSet zählt nach Namen über
  alle FileSetIds (eine geänderte Include-Liste ergibt eine neue FileSetId).
- Gilt unabhängig davon, ob Job, FileSet oder Client noch konfiguriert sind
  (umbenannter Job, entfernter Client).
- Weder die Jobs noch ihre File-Zeilen werden gepurgt: `prune jobs`, `prune
  files`, AutoPrune, `purge jobs`/`purge files` überspringen sie
  (`PurgedFiles` bleibt 0; der Restore mit Dateiauswahl bleibt möglich).
- Ein Volume mit Daten (JobMedia) eines Jobs dieser Ketten wird nicht gepruned,
  gepurgt, recycelt, `truncate`d oder gelöscht: weder durch `Prune Volumes`,
  `Recycle Oldest Volume`/`Purge Oldest Volume` bei der Volume-Suche eines Jobs
  noch durch `prune volume`, `purge volume`, `truncate` oder `delete volume` auf
  der Konsole (dort hilft auch `yes` nicht).
- Ist die Kette nicht ermittelbar (Katalogfehler), wird nichts gepurgt.
- Meldungen nennen, was geschützt wurde, z. B. `Keeping the latest Full chain of
  client "fd1" FileSet "fs" (JobIds 3,4,5); its jobs are not pruned.` oder
  `Volume "Full-0007" holds the latest Full chain of client "fd1" FileSet "fs"
  (JobIds 3); not pruned.`

Auswirkung auf die Volume-Wahl: Findet ein Job außer solchen Volumes kein
freies, wartet er auf ein Volume wie bei einem vollen Pool (Meldung an den Job:
„the job waits for a volume as with a full pool“). Abhilfe: ein Volume labeln
oder hinzufügen bzw. `Maximum Volumes` erhöhen. Ein Volume wird erst wieder frei,
wenn eine neuere Kette die geschützte ablöst (nächster Full). Ausweg für den
Einzelfall bleibt `delete jobid=…` auf der Konsole.

## Abweichungen von Bareos

Bewusst entschieden (Stand 20.11.2026):

| Thema | bareos-dir | odir | Grund |
| --- | --- | --- | --- |
| Schema-Erweiterung | keine | eigenes PostgreSQL-Schema `onesimus`: Views und Funktionen (v2251/v2252), Tabelle `onesimus.console_acl` (v2253, verschiebt ältere Objekte aus `public`), spätgebundene Views (v2254: keine Abhängigkeit von Bareos-Tabellen, Bareos-Upgrades nie blockiert); Selbstheilung fehlender/kaputter Objekte bei jedem Start; `public` bleibt Bareos, einzige Ausnahme `public.bareos_schema_version`; `version` bleibt 2250 | Logik in der Datenbank änderbar ohne neuen Code; Katalog-ACL; Trennung von Bareos (Entscheidung des Maintainers 20.11.2026) |
| Wartezeit auf ein Volume | unbegrenzt ohne `Max Wait Time` | 600 s, über `ONESIMUS_DEFAULT_MEDIA_WAIT` | kein hängender Nachtlauf; keine Direktive wegen Rückweg |
| Gescheiterter Batch-Insert der File-Zeilen | Block wird ignoriert, Job `T`, Dateien fehlen im Katalog | Job wird fatal (`f`), „Attribute create error“ | kein stilles Fehlen von Dateien im Restore-Baum |
| Purge (PurgeFiles/PurgeJobs) | ohne Transaktion, Fehler ignoriert | eine Transaktion, Fehler werden gemeldet | kein halb gelöschter Job |
| `prune jobs`, Behalten-Liste nicht ermittelbar | Fehler ignoriert, löscht trotzdem | bricht vor dem Löschen ab | die letzte Full-Kette darf nie verloren gehen |
| Schutz der jüngsten Kette | nur bei `prune jobs`/AutoPrune, nur für konfigurierte Job/FileSet/Client, FileSet nach FileSetId; File-Zeilen fallen nach `File Retention` weg; Volumes werden ohne Rücksicht gepruned/recycelt | je Client und FileSet-Name immer, auch nicht konfiguriert; Jobs und File-Zeilen bleiben; Volumes mit diesen Jobs werden weder gepruned, gepurgt, recycelt, `truncate`d noch gelöscht; Katalogfehler purgt nichts | Entscheidung des Nutzers 19.11.2026: der letzte Stand jedes Clients muss restaurierbar bleiben; ein Job wartet dafür ggf. auf ein Volume |
| Mehrere Volumes purgen | kein Befehl; `purge volume pool=…` öffnet die Volume-Auswahl | `purge volume all pool=<p>\|allpools [volstatus=…] dryrun\|yes` (Onesimus-Erweiterung): Vorschau mit `dryrun` ändert nichts; `yes` purgt jedes Volume über denselben Weg wie `purge volume=<name> yes` mit allen Prüfungen, abgelehnte Volumes werden mit Grund übersprungen; nie interaktiv; Vorgabe-VolStatus `Full`, `Used`, `Error`, nie `Purged`/`Recycle`. Die Bareos-Formen bleiben unverändert ([purge](commands/purge.md#mehrere-volumes-auf-einmal-onesimus-erweiterung)) | Entscheidung des Nutzers 20.11.2026 („Ja, mit Vorschau“): Sicherheit vor Bequemlichkeit, Purge ignoriert die Retention |
| ACLs in Dateien ändern | `configure add console/profile/user` legt Dateien mit ACLs an | `configure add profile/user` wird abgelehnt (Profile und User nur im Editor); `configure add console` nur als `name=… password=… profile=… [profile=…] [description=…]`, ohne ACL- oder andere Direktiven, und nur mit Profilen, die nicht mehr erlauben als die anlegende Console selbst (Datei- ∩ Katalog-ACL, je ACL-Typ; gesperrte Console nie; Profile mit `.acl` nur ohne eigene Katalog-Einträge); Passwort in der Antwort `***`; `configure delete console` bleibt abgelehnt ([configure](commands/configure.md#configure-add-console--enge-form-onesimus-abweichung)) | Entscheidung des Nutzers 19.11.2026 (zweistufige ACLs): sonst kann sich eine Console mit `configure` selbst mehr Rechte geben; Nachtrag 20.11.2026: Consoles für die GUI anlegen ohne Editor, ohne Weg zur Selbstausweitung |
| Zweistufige ACLs | nur Datei-ACLs | wirksam ist Datei-ACL **und** Katalog-ACL (`onesimus.console_acl`, je Console); `.acl set/delete/clear/copy` ändern nur die Katalog-ACL (braucht `.acl` und `configure`; die eigene Console nur enger). Ohne Katalog-Einträge wie Bareos. Katalog-ACL nicht lesbar: Consoles mit (möglichen) Einträgen dürfen nur `status`, `.status`, `version`, `.api`; Journal `catalog ACL not readable`, neuer Versuch alle 30 s; sonst neu geladen alle 60 s (Änderungen per SQL wirken ohne `reload`); jede Katalogabfrage der ACL höchstens 5 s | Entscheidung des Nutzers 19.11.2026 ([ACL-ZWEISTUFIG.md](../plans/ACL-ZWEISTUFIG.md)): Einschränken auch per MCP, ohne die Dateien anzufassen |
| Geheimnisse in Befehlszeilen | RunScript-Befehle stehen ungekürzt im Job-Log (`shell command: run ClientBeforeJob "…"`) und in `show` | odir ersetzt Werte von `--password=…`, `--password …`, `PGPASSWORD=…`, `MYSQL_PWD=…`, Namen auf `…password/passwd/pwd/passphrase/secret/token/apikey` und Zugangsdaten in URLs (`://user:…@`) durch `***`: in jeder Job- und Daemon-Meldung vor allen Zielen (Katalog-`Log`, Journal, Mail, Datei, Console, gRPC-Push) und in `show` für jede Console. Ungekürzt bleibt der Text nur über `.config read` / `configure export` (Recht `configure`). Einträge, die vor dieser Version im Katalog gespeichert wurden, bleiben unverändert | Entscheidung des Nutzers 20.11.2026: rückwärtskompatibel, bestehende Konfigurationen bleiben gültig (`crates/odir/src/secrets.rs`) |
| `list jobs count` | zählt alle Jobs, auch die, die Job-/Client-ACL der Console verbirgt | zählt bei eingeschränkter Job- oder Client-ACL nur die Jobs, die `list jobs` zeigen würde (derselbe ACL-Filter); ohne Einschränkung unverändert | ACL-Grenze: eine eingeschränkte Console soll nichts über gesperrte Jobs erfahren (G-2, 20.11.2026) |
| gRPC | gibt es nicht | TLS auf `DirPort + 1000`, gleiche Adressen wie BNET | Schnittstelle der GUI |
| Mail-Sammlung | Dateien `<WorkingDirectory>/*.mail` | im Speicher | – |
| Paketversion | – | bleibt `25.1.2+onesimus.0.2.0`, Installation mit `dpkg -i` | Entscheidung des Nutzers; neue Builds an Prüfsumme und Build-Datum erkennbar |

Vor dem Rückweg auf `bareos-dir` müssen Onesimus-eigene Direktiven (`gRPC Port`,
`gRPC TLS Certificate`, `gRPC TLS Key`, `mDNS Advertise`) aus der Konfiguration
entfernt werden; `bareos-dir` kennt sie nicht.

## Volume-Werkzeuge (ols, oscan, oextract)

Das Paket `osd` (nicht `odir`) bringt die Rust-Ersatzwerkzeuge für `bls`, `bscan` und `bextract` mit (Crate `crates/outils`, installiert nach `/usr/bin`). Sie lesen Datei-Volumes (auch die von bareos-sd und osd geschriebenen, Blockformat BB01/BB02) ohne Director und Storage Daemon, Block für Block (Speicherbedarf: ein Block). Bandgeräte lesen sie nicht.

| Werkzeug | Aufruf | Zweck |
|---|---|---|
| `ols` | `ols [-j] [-l] [-k] [-i <JobId>] <Volume>...` | Label, Jobs (SOS/EOS) und Dateien; `-j` nur Label und Jobs, `-l` lange Zeilen, `-i` ein Job, `-k` ohne Prüfsummen |
| `oextract` | `oextract [-i <JobId>] [-p <Präfix>]... [-n] [-O] [-k] <Ziel> <Volume>...` | Dateien zurückschreiben: Dateien, Verzeichnisse, Symlinks, Hartlinks, spärliche und komprimierte Daten (gzip, lz4, fastlz, lzo), Zeit und Rechte; `-n` Trockenlauf, `-O` Eigentümer (Root). Nicht: Windows-Datenströme, ACLs, erweiterte Attribute, Geräte, verschlüsselte Daten (gemeldet). Pfade mit `..` werden abgewiesen |
| `oscan` | `oscan [-s] [-m] [-p <Pool>] [-x] [-c <Verz.>] [-C <Catalog>] [-k] <Volume>...` | Katalog aus Volumes aufbauen: Pool, Client, FileSet, Media, Job, JobMedia und mit `-s` die File-Einträge samt Prüfsumme. Ohne `-x` gibt es das SQL-Skript aus (ändert nichts), mit `-x` führt es `psql` in einer Transaktion aus (Verbindung aus der Catalog-Ressource von `-c`, Voreinstellung `/etc/bareos/bareos-dir.d`) |

`oscan` ist wiederholbar: was im Katalog steht (JobId, Volume, Dateien eines Jobs), wird nicht noch einmal angelegt; steht eine JobId unter einem anderen Job-Namen im Katalog, bricht das Skript ab. Ein neu angelegtes Volume bekommt den Status `Full` (`update volume volstatus=Append` gibt es frei), ein neuer Pool die Art `Backup`. JobBytes kommt aus dem End-Label (was der SD schrieb), nicht vom FD. Start und Ende im JobMedia sind Byte-Positionen in der Datei, wie osd und bareos-sd sie führen. Eigene Angaben (Herkunft eines gescannten Jobs) gehören ins Schema `onesimus`, nicht in die Standardtabellen: `oscan` schreibt deshalb nichts in `Job.Comment`.

Getestet: `crates/outils/tests/` gegen ein von `bareos-format` aufgebautes Volume; im Gerüst (`onesimus-rig`, Test `outils`) gegen ein Volume, das osd schrieb: Job, Dateien und Volume werden gelöscht, `oscan -s` baut sie auf (Job, JobMedia gleich dem Original), die Rücksicherung durch den Director und `oextract` ohne Director ergeben beide denselben Baum wie die Quelle. Nicht verglichen: Volumes von bareos-sd, Mehr-Volume-Jobs mit `oscan`.

## Ausschluss nur für einen Options-Block (Onesimus-Erweiterung)

In Bareos gilt ein `Exclude` im FileSet für den ganzen FileSet. Ein `Exclude { … }` *innerhalb* von
`Options { … }` kennt Bareos nicht: `bareos-dir -t` lehnt es ab (`dird/inc_conf.cc:813`, „expected an
equals, got: {“), und `odir -t` tut es seit c497421 genauso. Damit `bareos-dir.d` Standard bleibt, steht
die Erweiterung im Overlay `/etc/onesimus/odir.d/fileset/<Name>.conf`; der FileSet gleichen Namens aus
`bareos-dir.d` wird erweitert:

```
FileSet {
  Name = "LinuxAll"
  Include {            # der n-te Include des Originals (Index, ab 1; ohne Angabe 1)
    Index = 1
    Options {          # der m-te Options-Block dieses Includes (ab 1; ohne Angabe 1)
      Index = 1
      Exclude {
        File = /var/lib/docker
        File = /home/alice/.cache
      }
    }
  }
}
```

Die Pfade (und alles darunter, nur ganze Pfadteile: `/var/lib/docker` trifft nicht `/var/lib/dockerx`)
sind nur für die Dateien ausgeschlossen, die dieser Options-Block erfasst, also mit `Wild`/`Regex`; ein
Options-Block ohne Muster gilt für alle Dateien des Includes. Gezählt wird in der Reihenfolge der
Originaldatei; ändert sich dort die Reihenfolge, verrutscht der Index (beim Laden meldet der Director
einen Index außerhalb des Bereichs; bleibt der Index gültig, zeigt aber nach dem Reload auf einen anderen Options-Block, warnt der Director im Log: „the Options block … to which the Exclude of the overlay refers, differs from the one before the reload“). In `odir.d` sind in diesem FileSet nur `Include`, `Options`,
`Index`, `Exclude` und `File` erlaubt.

Der Director schickt dem FD je Pfad eine Zeile `Y <Pfad>` im Options-Block (nach den `W`/`R`-Zeilen,
vor dem `N`). Nur `ofd` versteht sie. Ein Bareos-FD antwortet auf die Include-Liste mit einem Fehler;
der Job endet dann mit „The FileSet excludes paths inside Options (odir.d); that needs an Onesimus ofd,
a Bareos file daemon does not understand it.“ statt Daten zu sichern, die ausgeschlossen sein sollten.

Geprüft im Gerüst mit dem echten `ofd` (`crates/onesimus-rig/tests/options_exclude.rs`, 24.11.2026): Das Backup vor dem Overlay sichert alle 13 Einträge des Testbaums, nach `reload` mit `Exclude { File = …/data/sub }` in `Options` fehlen `sub`, `inner.txt` und `hard2` (10 Einträge), und `estimate listing` zeigt sie ebenfalls nicht. Mit einem echten Bareos-FD (25.1.3, aus dem Paket entpackt, nicht installiert) geprüft (`bareos_fd_exclude.rs`, `ONESIMUS_BAREOS_FD=<Pfad zu bareos-fd>`): Das Backup ohne Overlay läuft, mit dem Overlay endet der Job mit der Meldung „that needs an Onesimus ofd“ und sichert nichts.


## Client-initiierte Verbindung (`Connection From Client To Director`)

Ein Client, den der Director nicht erreicht (NAT, Firewall), verbindet sich selbst und wartet:

- Director, Client-Resource: `Connection From Client To Director = yes`.
- FD (ofd oder bareos-fd), Director-Resource: `Connection From Client To Director = yes`, `Address` und
  `Port` des Directors; `TLS Enable` wie beim Director (TLS-PSK mit dem gemeinsamen Passwort).

odir hält die Verbindung (`status dir`, Abschnitt „Client Initiated Connections“) und nimmt sie für den
nächsten Job des Clients und für `status client`, `estimate`, `setdebug`, `setbandwidth`, `resolve`; der
FD verbindet danach sofort neu. Mit `Run On Incoming Connect Interval = <Zeit>` in einem Job startet
odir den Job, wenn der Client sich verbindet und der letzte Lauf (Status T, W, C oder R) länger als die
Zeit zurückliegt oder es noch keinen gab (Bericht: „Job triggered by: Client“). Anders als bareos-dir
startet odir ihn sofort statt beim nächsten Scheduler-Durchlauf.

Noch nicht umgesetzt: `Connection From Director To Client = no` (odir versucht ohne wartende Verbindung
weiter, selbst zu verbinden) und das Warten auf eine Verbindung. Plan und Mitschnitt:
[CLIENT-INITIIERT.md](../plans/CLIENT-INITIIERT.md).
