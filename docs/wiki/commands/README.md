# Onesimus Director — Befehlsreferenz

Diese Seiten beschreiben die Konsolenbefehle des Onesimus Directors. Er spricht dasselbe Konsolenprotokoll wie der Bareos Director; `oconsole` und `bconsole` funktionieren gleichermaßen. Ein registrierter Befehl ist nicht automatisch vollständig umgesetzt; den Stand nennt der Kasten „Aus den Bareos-Originalquellen“ am Anfang jeder Seite.

- [Bareos-Originalbefehle und Vollständigkeitsabgleich](bareos-reference.md)
- [LLM-Referenz: Befehle, Syntax, Beschreibungen und Umsetzungsstand (JSON)](bareos-director-commands.json)
- [JSON-Schema](bareos-director-commands.schema.json)

Die Originalreferenz ist auf den Bareos-Commit `18be77e6832e33f5acd64094b9cba6066f2503f9` festgelegt (Branch `bareos-25`, `WIP/25.1.2-pre-43-g18be77e68`), die genaue Quelle des installierten Vergleichsprogramms `bareos-dir 25.1.2~pre43.18be77e68`. Sie umfasst alle 88 Befehle der Konsole und die 26 Befehle der Dateiauswahl beim Restore.

Der Status jedes Befehls stammt aus dem Paritätsvergleich (`tests/parity`: Original und Onesimus mit denselben Daten) und dem [Kompatibilitätslog](../../COMPATIBILITY-LOG.md): `parity_verified` heißt, alle genannten Paritätsfälle antworten byteweise gleich und kein offener Logeintrag betrifft den Befehl; `partial` nennt offene Einträge, abweichende Fälle, ungeprüfte Varianten oder Nebenwirkungen, die nur Unit-Tests abdecken; `registered_unverified` heißt, es gibt noch keinen Paritätsfall. Übersicht und Planung: [TODO-COMMANDS.md](../../TODO-COMMANDS.md).

Wer zum ersten Mal hier ist: [help](help.md) zeigt die erlaubten Befehle, [status](status.md) den Zustand des Directors, [list](list.md) den Inhalt des Katalogs. Ein einzelner Punkt (`.`) bricht an jeder Nachfrage den laufenden Befehl ab.

## Aufbau der Befehlsseiten

Jede Seite `<befehl>.md` (Punktbefehle: `dot-<name>.md`) ist Endanwenderdokumentation auf Deutsch. Befehlsnamen, Schlüsselwörter, Ressourcennamen und Ausgaben des Directors bleiben unverändert (meist Englisch). Die Seiten folgen alle demselben Aufbau.

### Reihenfolge der Abschnitte

1. **Titelzeile** `# <befehl> — <deutscher Kurztitel>`, zum Beispiel `# version — Version des Directors anzeigen`. Der Generator lässt die erste Zeile stehen.
2. **Generierter Block** zwischen `<!-- bareos-reference:start -->` und `<!-- bareos-reference:end -->`. Er wird nicht von Hand geändert; `python3 scripts/generate_director_reference.py --check <bareos-quelle>` muss danach weiter bestehen.
3. **`## Zweck`**: zwei bis fünf Sätze. Was der Befehl tut, wann man ihn braucht, ob er etwas verändert (Katalog, Konfiguration, Daemon) oder nur liest.
4. **`## Syntax`**: alle Aufrufformen in einem ```` ```text ````-Block, eine Form pro Zeile. Schreibweise: `<wert>` steht für einen einzusetzenden Wert, `[ … ]` ist optional, `a | b` heißt „genau eins davon“, `…` heißt „wiederholbar“. Schlüsselwörter stehen wörtlich da.
5. **`## Argumente`**: Tabelle mit allen Schlüsselwörtern und Argumenten, auch selten genutzten und solchen, die nur der Quelltext kennt:

   | Argument | Werte | Bedeutung |
   |---|---|---|
   | `jobid=<n>` | Ganzzahl | … |

   Ohne Argumente: der Satz „Der Befehl hat keine Argumente.“ und gegebenenfalls, was mit überzähligen Wörtern geschieht.
6. **`## Dialoge`** (nur wenn der Befehl nachfragt): welche Frage wann kommt, welche Antworten gelten (Auswahlnummer, Name, `yes`/`mod`/`no`, `.` zum Abbrechen), was bei falscher Eingabe passiert. Jede Frage mit einem echten Beispiel.
7. **`## API-Modi`**: wie die Antwort unter `.api 0` (Standard, Text), `.api 1` und `.api 2` (JSON, auch `.api json compact=yes`) aussieht. Gleiche Ausgabe in mehreren Modi wird einmal gezeigt und gesagt, dass sie gleich ist. Für reine Textbefehle reicht ein Satz.
8. **`## Beispiele`**: je Variante ein `### <deutsche Überschrift>` mit einem Satz Einleitung und einem Terminalblock (siehe unten). Jede Form aus „Syntax“ und jeder Dialog bekommt ein Beispiel.
9. **`## Fehlermeldungen`**: Tabelle `| Meldung | Ursache und Abhilfe |` mit dem genauen Text des Directors, so wie ihn der Benutzer sieht.
10. **`## Siehe auch`**: Liste verwandter Befehle als relative Links, `- [status](status.md) — Status von Director, Client und Storage`.

Abschnitte ohne Inhalt fallen weg (etwa „Dialoge“ bei `time`). Der alte englische Fließtext und die alten erfundenen Beispiele unter dem Block werden ersetzt.

### Echte Ausgaben

Antworten in Beispielen stammen nur aus echten Läufen: aus den Paritätsfällen (`tests/parity/cases/*.jsonl` für die gesendeten Befehle, `target/parity/out/<fall>.<schritt>.rust.raw` für die Antwort, `target/parity/report.json` für die Signale je Schritt) oder aus einem neuen Paritätslauf. Ausgaben werden Byte für Byte übernommen, nur Passwörter und Schlüssel fallen weg. Lässt sich für eine Variante keine echte Ausgabe erzeugen (sie würde etwa einen Job starten oder einen Daemon verändern), steht dort genau eine Zeile:

```text
Beispielausgabe nicht verfügbar: <Grund>.
```

Die Seiten beschreiben Onesimus: Beispiele zeigen die Antwort des Onesimus Directors (`*.rust.raw`). Antwortet das Original anders, folgt unter dem Terminalblock genau eine Textzeile (kein Codeblock) mit dem Text des Originals und der Nummer im [Kompatibilitätslog](../../COMPATIBILITY-LOG.md), zum Beispiel:

**Abweichung vom Original:** Das Original fragt den Storage Daemon nach den Slots und antwortet `No slots in changer.` (C-071)

### So sieht oconsole aus

Die Beispiele zeigen eine `oconsole`-Sitzung, so wie sie im Terminal erscheint, mit Prompt und ohne Farbe. Die Farben werden hier einmal beschrieben und in den Seiten nicht nachgebildet:

- **Prompt:** `<console>@bareos-dir[<oconsole-version>]> `. In allen Beispielen wird die Standardkonsole verwendet, die auch die Paritätsläufe nutzen: `*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> `. Mit einer benannten Konsole steht deren Name vorn, zum Beispiel `parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> `. Der Prompt ist grün, wenn der nächste Befehl gesendet werden darf, und rot während der Sperrzeit des Command Rate Guards (3 Sekunden zwischen zwei gesendeten Zeilen).
- **Eingabe:** Beim Tippen ist ein bekannter Befehl grün, ein Punktbefehl gelb und ein unbekannter rot. Bei `schlüssel=wert` ist der Schlüssel cyan und der Wert magenta. Vervollständigungshinweise sind abgeblendet.
- **Antwort:** Jede Zeile der Antwort wird einzeln ausgegeben. Zeilen mit `Error`, `FEHLER` oder `failed` sind rot, mit `Warning` oder `WARNUNG` gelb, mit `OK` oder `success` grün, alle anderen ohne Farbe. Endet die Antwort ohne Zeilenumbruch (JSON-Antworten, Fragen des Directors), beginnt die nächste Zeile trotzdem neu.
- **Dialoge:** Stellt der Director eine Frage, erscheint ihr Text als eigene Zeile und darunter ein Eingabeprompt, je nach Signal:
  - `> ` bei einer Unterfrage (Auswahlliste, „Turn on or off?“ und ähnliche). Ein einzelner `.` bricht den Dialog ab.
  - `(yes/no): ` bei einer Ja/Nein-Frage.
  - `input> ` bei freier Texteingabe; unter `.api 1` und `.api 2` kommen Fragen so. Der Fragetext folgt dort erst nach der Antwort.

  Eine leere Eingabe an einer dieser Fragen wird gesendet (am Hauptprompt nicht). Auswahllisten für Programme (Signal für Auswahleingabe unter `.api 1`, etwa bei `query`) zeigt oconsole nicht als Prompt an; dort hilft nur CTRL-C.
- **Abmelden:** `quit`, `exit` und `.quit` erkennt oconsole und beendet sich. Bei einer Abkürzung wie `q` schließt der Director die Verbindung, und oconsole meldet `Connection closed`.
- **CTRL-C** während einer Antwort schickt einen Abbruch an den Director und zeigt `<CTRL-C> — sending cancel to Director` (gelb).
- **Meldungen von oconsole selbst** (nicht vom Director) gehen auf stderr, zum Beispiel `Connecting to Director <host>:<port>`, `Director terminated connection` oder `oconsole: Command Rate Guard violation: multiline input discarded.`
- `NO_COLOR=1` schaltet alle Farben ab; die Antwort bleibt Byte für Byte gleich.

Regeln für die Terminalblöcke:

- Immer ```` ```text ````.
- Jeder Befehl steht hinter dem vollständigen Prompt, die Antwort ohne Einrückung darunter.
- Wird ein API-Modus vorher umgeschaltet, steht der `.api`-Befehl mit seiner echten Antwort im selben Block.
- Der Block endet mit einem leeren Prompt; so sieht man, dass die Antwort vollständig ist.
- Keine Kommentare im Block; Erklärungen stehen davor oder danach im Text.
- Lange Ausgaben dürfen mit einer eigenen Zeile `[…]` gekürzt werden. Kopf, Fuß und die Zeilen, um die es geht, bleiben stehen.

Ein Dialog sieht dann so aus (Paritätsfall `c005-automount-prompt-then-command`):

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> automount
Turn on or off?
> off
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Vollständiges Beispiel

So sieht der Teil unterhalb des generierten Blocks für `version` aus (Paritätsfälle `c009-version`, `c009-version-api1`, `c009-version-json`, `c020-compact-version`):

````markdown
## Zweck

`version` zeigt Name, Version und Build-Datum des Directors sowie das Betriebssystem, auf dem er läuft. Der Befehl liest nur und verändert nichts. Man braucht ihn zum Beispiel für Fehlerberichte oder um nach einem Update zu prüfen, ob der neue Director läuft.

## Syntax

```text
version
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `version` werden ignoriert.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Textzeile. Unter `.api 2` steht die Information als JSON-Objekt `result.version` mit den Feldern `name`, `type`, `version`, `bdate`, `operatingsystem`, `distname`, `distversion` und `customversionid`. Mit `.api json compact=yes` kommt dasselbe JSON in einer Zeile.

## Beispiele

### Version anzeigen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
bareos-dir Version: 25.1.2+onesimus.0.2.0 (15 October 2026) Debian GNU/Linux 13 (trixie) debian Debian GNU/Linux 13 (trixie)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Version als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> version
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "version": {
      "name": "bareos-dir",
      "type": "bareos-director",
      "version": "25.1.2+onesimus.0.2.0",
      "bdate": "15 October 2026",
      "operatingsystem": "Debian GNU/Linux 13 (trixie)",
      "distname": "debian",
      "distversion": "Debian GNU/Linux 13 (trixie)",
      "customversionid": ""
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

`version` gibt keine eigenen Fehlermeldungen aus.

## Siehe auch

- [status](status.md) — Status des Directors mit Startzeit und laufenden Jobs
- [time](time.md) — aktuelle Uhrzeit des Directors
````

## Befehle nach Themen

### Sitzung und Konsole

| Befehl | Beschreibung |
|---|---|
| [help](help.md) | Erlaubte Befehle und ihre Argumente anzeigen |
| [version](version.md) | Version des Directors anzeigen |
| [time](time.md) | Uhrzeit des Directors anzeigen |
| [whoami](whoami.md) | Name der eigenen Konsole anzeigen |
| [quit](quit.md) | Sitzung beenden |
| [exit](exit.md) | Sitzung beenden (wie `quit`) |
| [use](use.md) | Katalog für die Sitzung wählen |
| [gui](gui.md) | Modus für Oberflächen und Skripte ein- oder ausschalten |
| [autodisplay](autodisplay.md) | Nachrichten automatisch anzeigen |
| [messages](messages.md) | Wartende Nachrichten abholen |
| [var](var.md) | Variablen auflösen, z.B. für `LabelFormat` |
| [wait](wait.md) | Auf das Ende von Jobs warten |

### Jobs

| Befehl | Beschreibung |
|---|---|
| [run](run.md) | Job starten (Backup, Restore, Verify, Copy, Migrate, Admin) |
| [rerun](rerun.md) | Job erneut starten |
| [cancel](cancel.md) | Laufenden Job abbrechen |
| [restore](restore.md) | Dateien wiederherstellen |
| [estimate](estimate.md) | Umfang einer Sicherung abschätzen |
| [enable](enable.md) | Job, Client oder Zeitplan aktivieren |
| [disable](disable.md) | Job, Client oder Zeitplan deaktivieren |

### Status und Katalog

| Befehl | Beschreibung |
|---|---|
| [status](status.md) | Status von Director, Client, Storage, Zeitplänen und Slots |
| [memory](memory.md) | Kopfzeilen des Director-Status |
| [list](list.md) | Katalogeinträge auflisten (Jobs, Volumes, Pools, …) |
| [llist](llist.md) | Katalogeinträge ausführlich auflisten |
| [show](show.md) | Konfiguration von Ressourcen anzeigen |
| [query](query.md) | Vordefinierte Katalogabfragen |
| [sqlquery](sqlquery.md) | SQL direkt im Katalog ausführen |

### Volumes, Pools und Storage

| Befehl | Beschreibung |
|---|---|
| [label](label.md) | Neues Volume beschriften |
| [relabel](relabel.md) | Volume neu beschriften |
| [add](add.md) | Vorhandenes Volume in einen Pool aufnehmen |
| [create](create.md) | Pool aus der Konfiguration im Katalog anlegen |
| [update](update.md) | Volume-, Pool-, Slot-, Job- oder Statistikdaten aktualisieren |
| [delete](delete.md) | Volume, Pool, Job oder Storage aus dem Katalog löschen |
| [prune](prune.md) | Abgelaufene Einträge aus dem Katalog entfernen |
| [purge](purge.md) | Einträge ohne Rücksicht auf Aufbewahrungszeiten entfernen |
| [truncate](truncate.md) | Bereinigte Volumes leeren |
| [mount](mount.md) | Volume in ein Laufwerk laden |
| [unmount](unmount.md) | Volume aus einem Laufwerk nehmen |
| [umount](umount.md) | Wie `unmount` |
| [release](release.md) | Laufwerk freigeben |
| [automount](automount.md) | Automatisches Laden nach `label` ein- oder ausschalten |
| [import](import.md) | Volumes aus den Mailslots eines Autochangers holen |
| [export](export.md) | Volumes in die Mailslots eines Autochangers legen |
| [move](move.md) | Volumes zwischen Slots eines Autochangers bewegen |

### Verwaltung und Diagnose

| Befehl | Beschreibung |
|---|---|
| [configure](configure.md) | Ressourcen hinzufügen, exportieren oder ändern |
| [reload](reload.md) | Konfiguration neu laden |
| [setdebug](setdebug.md) | Debug-Level von Director, Client oder Storage setzen |
| [trace](trace.md) | Debug-Ausgaben in eine Datei schreiben |
| [setbandwidth](setbandwidth.md) | Bandbreite begrenzen |
| [setip](setip.md) | Adresse eines Clients setzen |
| [setdevice](setdevice.md) | Geräteparameter im Storage Daemon setzen |
| [resolve](resolve.md) | Hostnamen auf Director, Client oder Storage auflösen |

### Punktbefehle: Ressourcen und Werte auflisten

Punktbefehle sind für Programme gedacht (Tab-Vervollständigung in oconsole, GUIs, Skripte) und geben knappe Listen aus.

| Befehl | Beschreibung |
|---|---|
| [.clients](dot-clients.md) | Namen der Clients |
| [.jobs](dot-jobs.md) | Namen der Jobs |
| [.jobdefs](dot-jobdefs.md) | Namen der JobDefs |
| [.pools](dot-pools.md) | Namen der Pools |
| [.storages](dot-storages.md) | Namen der Storages |
| [.storage](dot-storage.md) | Storage-Namen (Seite zu `.storage` und `.storages`) |
| [.filesets](dot-filesets.md) | Namen der FileSets |
| [.schedule](dot-schedule.md) | Namen der Zeitpläne |
| [.catalogs](dot-catalogs.md) | Namen der Kataloge |
| [.consoles](dot-consoles.md) | Namen der Konsolen |
| [.users](dot-users.md) | Namen der Benutzer |
| [.profiles](dot-profiles.md) | Namen der Profile |
| [.messages](dot-messages.md) | Wartende Nachrichten im Maschinenformat |
| [.msgs](dot-msgs.md) | Namen der Messages-Ressourcen |
| [.media](dot-media.md) | Volume-Namen aus dem Katalog |
| [.mediatypes](dot-mediatypes.md) | Medientypen |
| [.locations](dot-locations.md) | Standorte |
| [.levels](dot-levels.md) | Sicherungslevel |
| [.types](dot-types.md) | Jobtypen |
| [.jobstatus](dot-jobstatus.md) | Jobstatus-Codes und ihre Bedeutung |
| [.volstatus](dot-volstatus.md) | Mögliche Volume-Status |
| [.actiononpurge](dot-actiononpurge.md) | Optionen für `ActionOnPurge` |
| [.defaults](dot-defaults.md) | Vorgabewerte eines Jobs, Clients, Storage oder Pools |

### Punktbefehle: Steuerung und Abfragen

| Befehl | Beschreibung |
|---|---|
| [.help](dot-help.md) | Befehlsliste im Maschinenformat |
| [.api](dot-api.md) | Ausgabeformat wählen: Text, API 1 oder JSON |
| [.authorized](dot-authorized.md) | Berechtigung prüfen |
| [.status](dot-status.md) | Status im Maschinenformat |
| [.sql](dot-sql.md) | SQL-Abfrage in einer Zeile |
| [.quit](dot-quit.md) | Sitzung beenden |
| [.](dot-noop.md) | Leerer Befehl, z.B. zum Abbrechen |

### BVFS: Dateibaum für grafische Restores

| Befehl | Beschreibung |
|---|---|
| [.bvfs_update](dot-bvfs-update.md) | BVFS-Cache für Jobs aufbauen |
| [.bvfs_get_jobids](dot-bvfs-get-jobids.md) | Jobs für einen Restore-Stand ermitteln |
| [.bvfs_lsdirs](dot-bvfs-lsdirs.md) | Verzeichnisse auflisten |
| [.bvfs_lsfiles](dot-bvfs-lsfiles.md) | Dateien auflisten |
| [.bvfs_versions](dot-bvfs-versions.md) | Versionen einer Datei |
| [.bvfs_restore](dot-bvfs-restore.md) | Auswahl für einen Restore vorbereiten |
| [.bvfs_cleanup](dot-bvfs-cleanup.md) | Temporäre Auswahltabelle löschen |
| [.bvfs_clear_cache](dot-bvfs-clear-cache.md) | BVFS-Cache leeren |

### Onesimus-Erweiterungen

Diese Befehle gibt es nur im Onesimus Director.

| Befehl | Beschreibung |
|---|---|
| [.grpc](dot-grpc.md) | gRPC-Modus der Sitzung |
| [.sqltables](dot-sqltables.md) | Tabellen des Katalogs |
| [.sqlschema](dot-sqlschema.md) | Spalten der Katalogtabellen |
| [.sqlcount](dot-sqlcount.md) | Zeilen von Katalogtabellen zählen |
| [.acl](dot-acl.md) | ACLs von Konsolen anzeigen und ändern |
| [.catalog](dot-catalog.md) | Katalogschema verwalten (Prüfung, Migration, …) |

### OVFS: Restore-Bäume (Onesimus)

Die OVFS-Befehle nutzen die PostgreSQL-Erweiterung `ovfs`, wenn sie im Katalog installiert ist, sonst einen Director-eigenen Weg mit einfachem SQL (gleiche Ergebnisse). In `oconsole` gibt es sie auch als MCP-Werkzeuge nur zum Lesen (`ovfs_status`, `ovfs_latest`, `ovfs_tree`, `ovfs_ls`, `ovfs_versions`).

| Befehl | Beschreibung |
|---|---|
| [.ovfs_status](dot-ovfs-status.md) | Version der Erweiterung und genutzter Weg |
| [.ovfs_latest](dot-ovfs-latest.md) | Jobkette des neuesten Stands eines Clients (wie Restore-Option 5) |
| [.ovfs_tree](dot-ovfs-tree.md) | Restore-Baum einer Jobkette als JSON, mit Tiefe |
| [.ovfs_ls](dot-ovfs-ls.md) | Direkte Einträge eines Verzeichnisses |
| [.ovfs_versions](dot-ovfs-versions.md) | Alle Versionen einer Datei |
