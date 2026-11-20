# oconsole: Vervollständigung, Vorschläge und Tasten

Die interaktive Konsole `oconsole` vervollständigt Befehle aus der Grammatik des Directors, zeigt Vorschläge wie die fish-Shell und hat die Tastenbelegung von fish. Alles hier gilt für den interaktiven Modus (nicht für `-e` und `--mcp`). Planung und Entscheidungen: [OCONSOLE-COMPLETION.md](../plans/OCONSOLE-COMPLETION.md); Man-Page: `man oconsole`.

## Woher die Konsole die Befehle kennt

Beim Verbinden und nach `reload` oder `configure` sendet `oconsole` `.api 2`, [`.grammar`](commands/dot-grammar.md) und `.api 0` (über gRPC dasselbe im Antwortformat JSON). Die Grammatik enthält nur die Befehle, die die Konsole ausführen darf, mit Unterbefehlen, Schlüsselwörtern, `schlüssel=wert`-Argumenten, Wertequellen, wiederholbaren Schlüsseln und sich ausschließenden Gruppen.

Ein Director ohne `.grammar` – das Bareos-Original oder ein älterer odir – antwortet „is an invalid command“. Dann baut `oconsole` dasselbe Modell aus den Usage-Zeilen von `.help` (`.api 1`); es ist flach (keine Unterbefehle, keine Gruppen), die Vervollständigung funktioniert gleich.

Danach liest `oconsole` einmal die Werte jeder Quelle, die die Grammatik nennt (`.clients`, `.jobs`, `.pools`, `.storages`, `.media`, `.filesets`, `.consoles`, `.profiles`, …). Die Meldung beim Start nennt die Zahl der Befehle, der Unterbefehle und Argumente und der gelesenen Werte; „(from .help)“ zeigt den Rückfall an.

## Tab: Vervollständigen

- Am Zeilenanfang: Befehlsnamen mit Beschreibung (`purge                  -- Purge records from catalog`).
- Danach: die Unterbefehle des erreichten Knotens (`purge vo<Tab>` → `volume`), Schlüsselwörter und `schlüssel=` der Argumente; ein Unterbefehl mit eigenem Pflichtwert erscheint als `jobid=`.
- Die Argumente eines Knotens gelten auch in seinen Unterbefehlen (`list jobs … limit=`).
- Schon gegebene Argumente fehlen, außer wiederholbare (`.acl set console=x jobacl=a <Tab>` bietet `jobacl=` wieder an, ebenso `profile=` bei `configure add console`).
- Aus einer Gruppe bleibt nach dem ersten Mitglied keins mehr übrig: nach `allpools` kein `pool=`, nach `dryrun` kein `yes`.
- Pflichtargumente stehen vorn (`.acl copy <Tab>` → `from=`, `to=`), der Rest alphabetisch.
- Nach `schlüssel=`: feste Werte oder die gelesenen Namen, Werte mit Leerzeichen in Anführungszeichen (`fileset=F<Tab>` → `"Full Set"`); bei Listenwerten auch nach einem Komma (`volstatus=Full,<Tab>` → `Used`, `Error`, … ohne `Full`).
- Werte ohne Schlüssel: `.api <Tab>` → `0 1 2 off on json`, `help pu<Tab>` → `purge`.

## Vorschläge beim Tippen

Steht der Cursor am Zeilenende, folgt ihm ein abgeblendeter Vorschlag:

1. die neueste Zeile der History, die mit der Eingabe beginnt (wie fish);
2. sonst der Rest der einzigen möglichen Vervollständigung (`purge volume all allp` → `ools`);
3. sonst, nach einem Leerzeichen, was folgen kann: die Unterbefehle (`purge ` → `<files|jobs|volume[=<volume>]|quota>`) oder die offenen Argumente (`.acl copy ` → `from=<from> to=<to>`). Diese Usage wird nur gezeigt; die Übernahmetasten fügen sie nicht ein.

Zeilen, die der Director in dieser Sitzung abgelehnt hat („is an invalid command“ oder ein CMD_FAILED-Signal), werden nicht mehr vorgeschlagen; sie bleiben in der History und mit Pfeil hoch erreichbar.

## help \<befehl\>

Mit der Grammatik eines odir beantwortet `oconsole` `help <befehl>` selbst: Usage, Beschreibung, Unterbefehle (die wichtigen mit eigenen Absätzen), Argumente, Beispiele (die des Befehls, dann die der Unterbefehle), Herkunft (Bareos oder Onesimus) – der Text der Man-Page `odir-<befehl>(7)` ohne Formatierung. `help` allein und alles beim Rückfall auf `.help` geht wie bisher an den Director.

## Tasten

| Taste | Mit Vorschlag am Zeilenende | Sonst |
|---|---|---|
| Pfeil hoch / runter | History rückwärts / vorwärts nach Zeilen, die mit der Eingabe beginnen; leere Zeile: vorige / nächste Zeile | gleich |
| Pfeil rechts, Ctrl-F, Ende | ganzen Vorschlag übernehmen | ein Zeichen nach rechts / ans Ende |
| Alt-Pfeil rechts, Alt-F | nächstes Wort des Vorschlags übernehmen | ein Wort weiter |
| Tab | vervollständigen, mehrere Kandidaten als Liste | gleich |
| Ctrl-R | inkrementelle Suche in der History | gleich |
| Esc | Vorschlag ausblenden, bis sich die Zeile ändert | – |
| Ctrl-C | Zeile leeren, Verbindung bleibt; während ein Befehl läuft: abbrechen | gleich |
| Ctrl-D | auf leerer Zeile: beenden | – |

Die History liegt in `~/.oconsole_history`.

## Für Entwickler

- Modell, Vervollständigung, Vorschläge und `help`: `crates/oconsole/src/grammar.rs` (reine Funktionen `complete`, `suggest`, `help_text`, ohne Terminal getestet).
- Tasten: `crates/oconsole/src/fishkeys.rs` (Entscheidungen als Funktionen von Zeile, Cursor und Vorschlag).
- Die Tests lesen die vollständige Grammatik aus [commands/console-grammar.json](commands/console-grammar.json) und vergleichen `help_text` mit den Fixtures von odir (`crates/odir/tests/fixtures/help_*.txt`).
