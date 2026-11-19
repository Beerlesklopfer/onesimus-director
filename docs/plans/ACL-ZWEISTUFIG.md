# Zweistufige ACLs: Dateien als Rahmen, PostgreSQL als innere Einschränkung

Stand: 19.11.2026. Status: **Entwurf zur Abstimmung, noch kein Code.**

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
   Er wird beim Setzen abgelehnt oder zumindest als wirkungslos gemeldet
   (offene Frage F-3).
2. **Ohne Katalog-Einträge gilt die Datei-ACL unverändert.** Ein Director ohne
   Onesimus-Tabelle verhält sich also wie Bareos.
3. **Datei-ACLs nur im Editor.** Kein Befehl und kein RPC schreibt
   ACL-Direktiven in die Dateien.
4. **Eine zentrale Entscheidungsfunktion.** Jede ACL-Prüfung (Befehle,
   typisierte RPCs, CRUD, Push-Ereignisse, `GetConfigCapabilities`) läuft
   durch `crate::acl::allowed` bzw. `any_allowed`. Dort wird die innere Stufe
   eingehängt.
5. **Katalog nicht erreichbar:** konservativ. Hat die Console Katalog-Einträge,
   die gerade nicht lesbar sind, gilt für sie „nichts erlaubt“ außer `status`
   (offene Frage F-4). Nie stillschweigend nur die Datei-ACL, denn das wäre
   eine Rechteausweitung.

## Was sich am Bestand ändert

| Stelle | heute | künftig |
| --- | --- | --- |
| `.acl set/delete/clear/copy` (Onesimus-Erweiterung, `commands/acl.rs`) | **schreibt die Console-Datei um** | schreibt die **Katalog-ACL**; die Datei bleibt unberührt |
| `.acl list` | zeigt Datei-ACLs | zeigt Datei-, Katalog- und effektive ACL |
| `configure add console`/`profile` (Bareos-Befehl) | legt eine Datei mit ACL-Direktiven an | offene Frage F-1 |
| `configure delete console` | seit `233a6f3` abgelehnt | bleibt abgelehnt |
| Konfigurations-CRUD (gRPC, `.config`) | Console/Profile/User nicht editierbar | bleibt so (Regel 3) |
| Push-Ereignisse gRPC | ungefiltert (K-19) | gefiltert über die zentrale Funktion (in Arbeit, nur Datei-Stufe) |

## Speicher der inneren Stufe

Neue Tabelle in der Onesimus-Schemaerweiterung des Katalogs (dieselbe
Erweiterung wie `bareos_schema_version`; die Versionsnummer richtet sich nach
der dort geführten Zählung, sie wird hier nicht festgelegt). Vorschlag:

```sql
-- eine Zeile je Console, ACL-Typ und Eintrag, Reihenfolge wie in Bareos
CREATE TABLE onesimus_console_acl (
    console   text     NOT NULL,   -- Name der Console-Ressource
    acl_type  text     NOT NULL,   -- job, client, storage, schedule, pool,
                                   -- command, fileset, catalog, where, pluginoptions
    position  integer  NOT NULL,   -- Auswertungsreihenfolge
    entry     text     NOT NULL,   -- wie in der Datei: Name, Regex, !Negation, *all*
    changed_by text    NOT NULL,   -- Console, die den Eintrag gesetzt hat
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (console, acl_type, position)
);
```

Semantik eines Typs mit Katalog-Einträgen: Die Liste wird wie eine
Bareos-ACL ausgewertet; erlaubt ist ein Name nur, wenn Datei-Liste und
Katalog-Liste ihn beide erlauben. Ein Typ ohne Katalog-Einträge schränkt
nicht ein.

## Offene Fragen

| # | Frage | Entscheider |
| --- | --- | --- |
| F-1 | `configure add console`/`profile` ist ein Bareos-Befehl und schreibt ACL-Direktiven in eine Datei. Regel 3 schließt das aus. Vorschlag: ablehnen, wenn ACL-Direktiven (`*Acl`, `Profile`) im Aufruf stehen; eine Console ohne ACL-Zeilen hat in Bareos keine Rechte und darf angelegt werden. Abweichung von Bareos, kommt in betrieb-odir.md. | Maintainer |
| F-2 | Wer darf die Katalog-ACL ändern? Vorschlag: Command-ACL `.acl` **und** `configure` (wie heute), und nur für **andere** Consoles; die eigene Console kann sich nur selbst einschränken, nie lockern. | Maintainer |
| F-3 | Ein Katalog-Eintrag, der mehr erlaubt als die Datei: ablehnen oder speichern und als wirkungslos melden? Vorschlag: speichern und melden, weil sich die Datei später ändern kann. | Maintainer |
| F-4 | Katalog nicht erreichbar bei einer Console mit Katalog-Einträgen: nur `status`/`version` (Vorschlag) oder gar nichts? | Maintainer |
| F-5 | Gilt die innere Stufe auch für Profile (Einträge je Profil) oder nur je Console? Vorschlag: nur je Console, Profile bleiben reine Datei-Sache. | Maintainer |
| ~~F-6~~ | ~~Sitzungen des MCP~~ **Entschieden 19.11.2026:** Der MCP-Weg MUSS eine eigene Console in der Konfiguration haben, Suffix `-mcp` (`<console>-mcp`). oconsole meldet sich im MCP-Modus nur so an und startet ohne sie nicht. Die innere Stufe bleibt damit je Console. | Maintainer |

## Umsetzungsschritte (nach Abstimmung)

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
