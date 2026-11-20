# .acl — Konsolen-ACLs anzeigen und einschränken (Onesimus)

## Zweck

`.acl` zeigt die ACLs einer Console und ändert ihre **Katalog-ACL**, die innere Stufe des zweistufigen ACL-Modells ([ACL-ZWEISTUFIG.md](../../plans/ACL-ZWEISTUFIG.md)):

- **Datei-ACL** (äußerer Rahmen): die ACL-Direktiven der Console und ihrer Profile in `bareos-dir.d`, ausgewertet wie in Bareos. Sie werden **nur im Editor** geändert; `.acl` schreibt nie in eine Datei.
- **Katalog-ACL** (innere Stufe): Einträge je Console und ACL-Typ in der Katalogtabelle `onesimus.console_acl` (eigenes Schema `onesimus`, siehe [betrieb-odir.md](../betrieb-odir.md#katalog-schema-onesimus)). Sie kann nur weiter einschränken.
- **Wirksam** ist, was Datei-ACL **und** Katalog-ACL erlauben. Ein ACL-Typ ohne Katalog-Einträge schränkt nicht ein; ohne jede Katalog-Einträge verhält sich der Director wie Bareos.

Die Katalog-Liste wird wie eine Bareos-Liste ausgewertet: der erste passende Eintrag entscheidet, `!` verbietet, reguläre Ausdrücke (POSIX ERE, ganzer Name, ohne Groß-/Kleinschreibung) und `*all*` gelten wie in der Datei, passt kein Eintrag, ist der Name verboten. Eine Änderung wirkt sofort (der Director lädt die Katalog-ACL nach jeder Änderung, nach `reload` und beim Start neu).

**Onesimus-Erweiterung, nicht im Original (C-074, Entscheidung in [#4](https://github.com/Beerlesklopfer/onesimus-director/issues/4)).** Das Bareos-Original kennt den Befehl nicht und antwortet `.acl: is an invalid command.`.

## Berechtigungen

- `.acl` selbst muss in der (wirksamen) `Command ACL` erlaubt sein.
- `list` ohne `configure`-Recht zeigt nur die eigene Konsole; die ACLs anderer Konsolen sieht nur eine Konsole, die `configure` darf.
- `set`, `delete`, `clear` und `copy` brauchen `.acl` **und** `configure` (Entscheidung F-2).
- Die Katalog-ACL einer **anderen** Console darf beliebig geändert werden (sie kann nie über deren Datei-ACL hinaus).
- Die **eigene** Console darf ihre Katalog-ACL nur enger machen. Erlaubt ist je ACL-Typ nur:
  1. eine erste Liste für einen Typ, der noch keine Katalog-Einträge hat (jede Liste ist enger als „keine Einschränkung“), oder
  2. eine nicht leere neue Liste, die aus der alten nur durch **Einfügen von `!`-Einträgen** (an beliebiger Stelle) und **Entfernen von Einträgen ohne `!`** entsteht; die übrigen Einträge behalten ihre Reihenfolge, alle `!`-Einträge bleiben. Verglichen wird der Text genau.

  Das erweitert nie, gleich was die Einträge treffen: ein eingefügtes `!x` kann eine Entscheidung nur zu „verboten“ machen; ein entfernter erlaubender Eintrag betrifft nur Namen, die er erlaubt hat, sie fallen auf einen späteren Eintrag oder auf „verboten“. Abgelehnt werden das Entfernen eines `!`-Eintrags, ein neuer erlaubender Eintrag, Umstellen, das Löschen eines ganzen Typs und `clear` – auch dann, wenn es im Einzelfall nicht erweitern würde. Die Regel ist bewusst rein syntaktisch und damit entscheidbar; ob eine Liste regulärer Ausdrücke in einer anderen enthalten ist, prüft der Director nicht.

## Syntax

```text
.acl [list] [console=<name>]
.acl set console=<name> <typ>acl=<eintrag>[,<eintrag> …] …
.acl delete console=<name> <typ>acl[=<eintrag>[,<eintrag> …]] …
.acl clear console=<name>
.acl copy from=<console> to=<console>
```

`<typ>` ist einer von `job`, `client`, `storage`, `schedule`, `pool`, `command`, `fileset`, `catalog`, `where`, `pluginoptions` (also `jobacl=`, `commandacl=` usw., Groß-/Kleinschreibung egal).

## Argumente

| Argument | Bedeutung |
|---|---|
| `list` (oder kein Unterbefehl) | Je ACL-Typ drei Zeilen: die Datei-Einträge (erst die der Konsole, dann die ihrer Profile, mit `(Profile <name>)`), `Catalog =` die Katalog-Einträge und `Effective =` das wirksame Ergebnis. Dazu `Catalog ACL =` mit dem Ladezustand. Ohne `console=` die eigene Konsole bzw. mit `configure`-Recht alle Konsolen. |
| `set` | Ersetzt die Katalog-Liste der Konsole für jeden angegebenen Typ. |
| `delete` | Entfernt die angegebenen Katalog-Einträge; `<typ>acl` ohne Wert entfernt die Katalog-Liste dieses Typs (er schränkt dann nicht mehr ein). Ein nicht vorhandener Eintrag ist ein Fehler, dann ändert sich nichts. |
| `clear` | Entfernt alle Katalog-Einträge der Konsole. |
| `copy` | Gibt `to` die Katalog-Listen von `from` (Typen, die `from` nicht hat, verlieren bei `to` ihre Einträge). Die Datei-ACLs bleiben, wie sie sind. |
| `console=<name>` | Zielkonsole (bei `set`, `delete`, `clear` Pflicht). `delete`/`clear` gehen auch für eine Console, die nicht mehr konfiguriert ist, aber noch Katalog-Einträge hat. |
| `from=`, `to=` | Quell- und Zielkonsole bei `copy`. |

### `Effective`

- `file ACL only` – der Typ hat keine Katalog-Einträge, die Datei entscheidet allein.
- `file ACL and catalog ACL` – beide Listen müssen erlauben.
- `locked (catalog ACL not readable)` – siehe unten.

Bei Typen mit Ressourcennamen (Job, Client, Storage, Schedule, Pool, FileSet, Catalog) und bei Befehlen folgt, wie viele der konfigurierten Namen bzw. Befehle wirksam erlaubt sind, mit den erlaubten Namen oder (wenn das kürzer ist) `all except:` den verbotenen. `WhereAcl` und `PluginOptionsAcl` haben keine aufzählbaren Namen.

### Wirkungslose Einträge (Entscheidung F-3)

Ein Katalog-Eintrag, der nur erlaubt, was die Datei-ACL verbietet, wird gespeichert und als wirkungslos gemeldet: in der Ausgabe von `set` als `Note: … has no effect`, in `list` als `(no effect: not allowed by the file ACL)`, im JSON als `"ineffective": true`. Gespeichert wird er trotzdem, weil sich die Datei später ändern kann. Als wirkungslos gilt:

- ein Name, den die Datei-ACL verbietet;
- `*all*`, wenn die Datei-ACL für den Typ gar nichts erlaubt;
- ein regulärer Ausdruck, der mindestens einen konfigurierten Namen (bzw. Befehl) trifft und dessen getroffene Namen alle von der Datei-ACL verboten sind;
- `!`-Einträge nie.

### Katalog nicht lesbar (Entscheidung F-4)

Kann der Director die Katalog-ACL nicht lesen, fällt er **nie** still auf die Datei-ACL zurück. Eine Console, die Katalog-Einträge hat oder haben könnte, ist dann gesperrt: sie darf nur noch `status`, `.status`, `version` und `.api` (wie immer auch `quit`, `exit`, `whoami`), nichts aus den übrigen ACL-Typen. `.api` ist dabei, weil es nur das Ausgabeformat der Sitzung umschaltet und nichts preisgibt; API-Clients (bconsole `-api`, WebUI, GUI, MCP) senden es zuerst und könnten sonst nicht einmal die Antworten auf `status`/`version` lesen. `.help` bleibt gesperrt. Als einträgefrei bekannt (und damit weiter mit Datei-ACL) gilt eine Console, wenn das letzte erfolgreiche Laden keine Einträge für sie fand und seitdem keine `.acl`-Änderung an ihr begonnen wurde. Vor dem ersten erfolgreichen Laden ist keine Console bekannt. Die Standardkonsole `*UserAgent*` kann keine Einträge haben und ist nie gesperrt. Der Director versucht alle 30 s neu zu laden. `set`, `delete`, `clear` und `copy` werden abgelehnt, solange die Katalog-ACL nicht lesbar ist. Jede Katalogabfrage der Katalog-ACL (Laden, Schreiben) wartet höchstens 5 s; bei einem Katalogausfall kommt die Ablehnung also nach wenigen Sekunden (`refused, the catalog ACL is not readable (catalog did not answer within 5 s)`), nicht erst nach dem Verbindungs-Timeout des Pools (30 s).

### Änderungen direkt per SQL

Solange die Katalog-ACL lesbar ist, lädt der Director sie alle 60 s neu (Thread `dir:catalog-acl`). Zeilen, die jemand direkt in `onesimus.console_acl` ändert, wirken also spätestens nach einer Minute, ohne `reload`; das Journal meldet `catalog ACL changed in the catalog, reloaded (…)`. Änderungen über `.acl` wirken sofort.

Fehlt die Tabelle (Schema-Erweiterung v2253 nicht eingespielt), gilt das als „keine Einträge“, solange nie Einträge gelesen wurden; Änderungen werden dann abgelehnt.

### Protokoll

Jede Änderung geht je Typ in die Director-Meldungen (wie `config_edit`): `Console [<wer>] catalog acl <aktion> Console "<ziel>" <Typ>: old [...], new [...]`.

## API-Modi

Unter `.api 0` und `.api 1` kommt derselbe Text. Unter `.api 2` liefert `list` die Konsolen unter `acl` als Liste von Objekten: `console`, `profiles`, je ACL-Typ (`jobacl`, …) die Datei-Einträge aus `value` und `source` (`"Console"` oder `"Profile <name>"`) wie bisher, dazu `catalog_state` (`state`: `loaded`, `off`, `table_missing`, `failed` mit `error`), `locked`, `catalog` (je Typ die Einträge mit `value`, `position`, `ineffective`, `changed_by`, `changed_at`) und `effective` (je Typ `source`: `file`, `file+catalog` oder `locked`; `allowed`: die wirksam erlaubten konfigurierten Namen bzw. Befehle oder `null`; `configured`: deren Anzahl oder `null`). Im typisierten gRPC-Ergebnis (`DotAclList`, `ConsoleAclInfo`) stehen `catalog`, `effective`, `catalog_state` und `locked` als JSON-Text in `extra_fields`.

Die ändernden Unterbefehle liefern `acl` mit `action`, `console`, `changes` (je geändertem Typ `type`, `old`, `new`), `ineffective` (wirkungslose neue Einträge mit `type`, `value`) und `effective` (das `list`-Objekt der Konsole danach).

## Beispiele

### Wirksame ACLs einer Konsole mit Profil und Katalog-Einschränkung

```text
*UserAgent*@bareos-dir> .acl set console=op jobacl=!restore,backup-.*
Catalog ACL of Console "op" changed:
  JobAcl: old = (none); new = !restore, backup-.*
Console "op":
  Profile = operator
  Catalog ACL = loaded
  JobAcl = *all* (Profile operator)
    Catalog = !restore, backup-.*
    Effective = file ACL and catalog ACL: 2 of 3 configured, all except: restore
  ClientAcl = fd1
    Catalog = (none)
    Effective = file ACL only: none configured
[…]
```

### Eigene Console darf sich nicht lockern

```text
mcp@bareos-dir> .acl delete console=mcp commandacl=!delete
.acl delete: refused, Console "mcp" may only restrict its own catalog ACL further; the change of CommandAcl could allow more. On the own console only inserting ! entries, removing entries without !, or a first list for a type without catalog entries is allowed.
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.acl: is an invalid command.` | `.acl` ist in der wirksamen `Command ACL` nicht erlaubt. |
| `.acl: usage:` mit Kurzhilfe | Unbekannter Unterbefehl. |
| `.acl <unterbefehl>: permission denied, changing the catalog ACL requires the CommandACL .acl and configure.` | Ändern ohne `configure`-Recht. |
| `.acl: permission denied, listing the ACLs of Console "…" requires the CommandACL configure.` | Fremde Konsole ohne `configure`-Recht angefragt. |
| `.acl <unterbefehl>: refused, Console "…" may only restrict its own catalog ACL further; …` | Änderung an der eigenen Console, die erweitern könnte (Regel oben). |
| `.acl <unterbefehl>: refused, the catalog ACL is not readable (…).` | Katalog nicht erreichbar; der Director versucht es alle 30 s neu. |
| `.acl <unterbefehl>: refused, the catalog has no table onesimus.console_acl …` | Schema-Erweiterung v2253 fehlt (z. B. fehlendes Recht, das Schema `onesimus` anzulegen); Startmeldungen des Directors prüfen (`catalog migration failed`). |
| `.acl <unterbefehl>: no catalog, the catalog ACL is not available.` | Director ohne Katalog. |
| `.acl <unterbefehl>: console=<name> required.` / `.acl copy: from=<console> to=<console> required.` | Pflichtargument fehlt. |
| `Console "…" not found.` | Es gibt keine Console-Ressource dieses Namens. |
| `.acl set: specify at least one <type>acl=<entries>.` | `set` ohne ACL-Argument (ebenso `delete`). |
| `.acl set: invalid ACL entry "…": …` | Leerer Eintrag (auch `!` allein), Steuerzeichen oder länger als 1024 Zeichen. |
| `<Typ> entry "…" not found in the catalog ACL of Console "…".` / `Console "…" has no catalog <Typ> entries.` | `delete` mit einem Eintrag, den die Katalog-ACL nicht hat (Datei-Einträge ändert `.acl` nicht). |

## Siehe auch

- [show](show.md) — `show console=<name>` und `show profile=<name>` (Datei-ACLs)
- [.authorized](dot-authorized.md) — eine einzelne wirksame Berechtigung prüfen
- [.consoles](dot-consoles.md), [.profiles](dot-profiles.md) — Namen der Konsolen und Profile
- [reload](reload.md) — Konfiguration (und Katalog-ACL) neu laden
