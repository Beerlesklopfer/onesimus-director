# oconsole: Vervollständigung aus der Befehlsgrammatik, fish-Tasten, Man-Pages

Stand 20.11.2026. Entwurf und Skelette kamen aus der GUI-Sitzung (onesimus-gui, `DESIGN.md` mit D1–D10 und Q1–Q3, geschrieben gegen `development` 09fc1c0); die Fragen Q1–Q3 hat der odir-Verantwortliche entschieden. Dieses Dokument beschreibt den umgesetzten Stand. Nutzerdoku: [oconsole-vervollstaendigung.md](../wiki/oconsole-vervollstaendigung.md), [.grammar](../wiki/commands/dot-grammar.md).

## Entscheidungen, wie umgesetzt

**D1 – Eine Grammatik in odir.** Die Registry (`crates/odir/src/command_registry.rs`) nennt Befehle, Aliase, Beschreibung und Handler; was ein Befehl annimmt, steht in der Grammatiktabelle `crates/odir/src/grammar_table.rs`, je Registry-Name ein Wurzelknoten (`describe(name)`). Das Modell und alles, was daraus folgt (JSON, Usage-Zeilen, ACL-Filter, Prüfung), liegt in `crates/odir/src/command_grammar.rs`. Die alten `ArgDef`-Argumente der Registry waren außer in ungenutzten Hilfsfunktionen nirgends verwendet und sind entfernt; Wort, Aliase und Punkt-Kennzeichen kommen aus der Registry, die Beschreibung bei Bareos-Befehlen aus der Bareos-Tabelle (`ua_commands_table.rs`), sonst aus der Registry. Ein Befehl ohne Bareos-Gegenstück hat die Herkunft `onesimus`.

Beschrieben sind alle 97 registrierten Befehle (mit `.grammar`), darunter die Erweiterungen: `purge volume all pool=|allpools volstatus=…(Liste) dryrun|yes`, `configure add <typ>` (aus der generierten Bareos-Usage `configure_usage::USAGE`, Listen-Typen wiederholbar; Typ und Werte aus den Itemtabellen: Ressourcenverweise mit der Quelle ihres Typs, `.clients`, `.pools`, `.msgs` …, Enum-Direktiven wie Type und Level mit den Schlüsselwörtern aus `commands::show`), `configure add console` in der engen Form (name, password, profile wiederholbar und als Komma-Liste, description), `configure read|update|delete` (update mit allen Direktiven der Itemtabellen, wiederholbar, typisiert wie bei add), `configure export`, `.acl list|set|delete|clear|copy`, `.config`, `.ovfs_*`, `.sql*`, `list jobs … reverse`, `status …` mit allen Präfixformen, `.grammar` selbst.

**D2 – Modell.** `Node { word, aliases, description, dot, origin, value, long, examples, see_also, free_text, args, subs }`, `GrammarArg { name, kind (Keyword|Value), arg_type (String|Integer|Bool|Enum|Resource), required, repeatable, group, list, positional, value_optional, default, description, values (None|Fixed|Source) }`. Bausteine: `kw`, `val`, `int`, `boolean`, `res`, `enm`, `pos`, `sub`, `vsub` und `.req() .repeat() .list() .group() .optional_value() .default() .values() .source()`, an Knoten `.args() .subs() .alias() .value() .onesimus() .long() .example() .see() .free_text()`.

Regeln: Die Argumente eines Knotens gelten auch in allen Unterbefehlen darunter. Gleiche `group` schließt sich aus. `required` heißt „zuerst anbieten“; die Prüfung verlangt es nicht, weil der Director fehlende Werte meist selbst erfragt.

**D3 – `.grammar`.** API 2: `{"grammar": {"schema": 1, "commands": […]}}`; sonst je Befehl `wort -- beschreibung` und eingerückte Usage-Zeilen; `command=<befehl>` beschränkt auf einen Befehl. ACL-gefiltert mit derselben Prüfung wie die Ausführung (`permitted_roots`, je Wort der Name, den der Dispatcher prüft; `quit`, `exit`, `whoami` immer). Bareos antwortet „is an invalid command“. Kein RPC: `GetGrammar` braucht eine Änderung in onesimus-proto (mit der GUI geteilt) und folgt später; `crates/odir/src/grpc/coverage.rs` führt `.grammar` deshalb in `NO_RPC_YET`, die Abdeckungstabelle in grpc-api.md zeigt es als „noch kein RPC“, der MCP-Abdeckungstest überspringt solche Zeilen.

**D4 – Lader in oconsole.** `.api 2; .grammar; .api 0` beim Verbinden und nach `reload`/`configure` (gRPC: `ExecuteCommand` mit Format JSON). Ohne Grammatik das alte Parsen von `.help` (`.api 1`), übersetzt in dasselbe Modell (flach). Danach einmal die Werte jeder Quelle der Grammatik.

**D5 – Vervollständigung** (`grammar::complete`, rein): Tokenisieren mit Bareos-Anführungszeichen, Pfad über die Unterbefehlswörter; angeboten werden Unterbefehle (mit eigenem Pflichtwert als `wort=`), Schlüsselwörter, `schlüssel=` und Positionswerte; benutzte Schlüssel fehlen außer wiederholbaren, Gruppengeschwister fehlen; Pflichtargumente zuerst, sonst alphabetisch; Werte nach `schlüssel=` (fest, aus dem Cache, `@commands`), nach Komma bei Listen, mit Leerzeichen in Anführungszeichen; Anzeige `wort  -- beschreibung`.

**D6 – Vorschläge** (`grammar::suggest`, rein): 1. neueste passende History-Zeile, abgelehnte Zeilen nicht (Menge im Speicher, gefüllt bei „is an invalid command“ oder CMD_FAILED); 2. Rest des einzigen Kandidaten; 3. Usage (Unterbefehle als `<a|b|…>`, sonst offene Argumente). Usage-Hinweise werden nur gezeigt: `Hint::completion()` ist `None`, die Übernahmetasten tun dort das Normale.

**D7 – Tasten** (`crates/oconsole/src/fishkeys.rs`): Up/Down `HistorySearchBackward/Forward` (Präfix), Right/Ctrl-F/End `CompleteHint` mit Vorschlag am Zeilenende, Alt-Right/Alt-F nächstes Wort des Vorschlags (`Cmd::Insert`), sonst Wortsprung, Esc blendet den Vorschlag aus, bis sich die Zeile ändert (`HiddenHint`, geteilt mit dem Hinter, danach `Cmd::Repaint`). Tab, Ctrl-R, Ctrl-C (Zeile leeren, Sitzung bleibt), Ctrl-D unverändert. Die Entscheidungen sind Funktionen von (Zeile, Cursor, Vorschlag).

**D8 – Tests.** odir: `grammar_covers_every_registered_command`, `grammar_json_is_stable` (Snapshot `crates/odir/tests/fixtures/grammar_snapshot.json`: purge, .acl, list, .grammar), `grammar_accepts_the_parity_cases`, `grammar_respects_command_acl`, `groups_and_repeatable_keys`, `console_grammar_doc_is_current` (`docs/wiki/commands/console-grammar.json`), `configure_directives_have_value_sources` (jeder Ressourcenverweis hat eine Quelle, jede Enum-Direktive Werte), `value_sources_are_registered_commands`, Handler-Tests in `commands/grammar.rs`. oconsole: Lesen von Schema 1, Rückfall auf `.help`, Unterbefehle, benutzte und wiederholbare Schlüssel, Gruppen, Werte aus Liste und Cache mit Anführungszeichen, Listen nach Komma, Beschreibungen, Vorschläge (History zuerst, abgelehnte Zeilen, einziger Kandidat, Usage), `help`-Text gleich den odir-Fixtures, Lader mit gefälschtem Director, Tastenentscheidungen.

**D9 – Reihenfolge**: siehe unten.

**D10 – Man-Pages** (`crates/odir/src/manpages.rs`): `odir-commands(7)` und je Befehl `odir-<befehl>(7)` (Punktbefehle `odir-dot-<name>.7`, `.` als `odir-dot-noop.7`) mit NAME, SYNOPSIS, DESCRIPTION, SUBCOMMANDS, ARGUMENTS, EXAMPLES, ORIGIN, SEE ALSO. `oconsole(1)`, `odir(8)`, `ofd(8)` sind Vorlagen in `packaging/man/src/*.in` mit eingefügter Befehlsliste (`.\" @COMMANDS@`) und Version. Verstecktes `odir --write-man-pages <dir>`; die Seiten liegen in `packaging/man` (Test `committed_pages_are_up_to_date`, neu schreiben mit `UPDATE_MAN_PAGES=1`), Lint mit `mandoc -T lint`, ohne mandoc mit `groff -man -ww -z`. Deb-Assets: odir (`man8/odir.8`, `man7/odir-*.7`), oconsole (`man1/oconsole.1`), ofd (`man8/ofd.8`). `help <befehl>` zeigt in oconsole denselben Inhalt als Klartext. Die Beschreibungsabsätze und Beispiele stehen in `crates/odir/src/grammar_docs.rs` (Pfad wie `list jobs`), kurze Texte auch direkt in `grammar_table.rs`, nie beides für denselben Knoten; Absätze von Unterbefehlen erscheinen unter SUBCOMMANDS, ihre Beispiele nach denen des Befehls unter EXAMPLES. Tests: `every_command_has_long_and_an_example`, `examples_are_grammatical` (jede Beispielzeile besteht `validate`), `docs_name_existing_nodes`.

## Offene Fragen, entschieden

- **Q1 – ja.** Komma-Listen in einem Wert tragen `"list": true` (`volstatus=`, `jobid=`, `.acl … jobacl=`, `profile=`); oconsole vervollständigt nach dem Komma und lässt schon genannte Werte weg. `.acl` trennt nur an Kommas außerhalb von `{}`, `[]`, `()` (`acl.rs` `split_entries`); die ACL-Typ-Schlüssel sind `repeatable` und `list`.
- **Q2:** Werte ohne Schlüssel tragen `"positional": true`, `name` ist der Platzhalter (`.api <mode>`, `help <command>`, `mount <storage-name>`, `use <catalog-name>`, `resolve <hostname>`).
- **Q3 – nein.** `.grammar` lässt Befehle weg, die die Konsole nicht ausführen darf.

## Abweichungen von den Skeletten, mit Grund

- **Strings statt `&'static str`** im odir-Modell: `configure add` wird zur Laufzeit aus der generierten Usage und `configure update` aus den Itemtabellen gebaut.
- **`Node.value`** (neu): Unterbefehlswörter mit eigenem Wert (`list jobid=5`, `status client=fd`, `purge volume[=v]`, `configure update schedule=X`). Ohne das ließen sich `key=value`-Unterbefehle nicht von Argumenten trennen.
- **`value_optional`** (neu): Schlüssel, die auch allein stehen (`.acl delete jobacl`, `show client`); **`free_text`** (neu): `var` und `.` nehmen jede Eingabe.
- **Argumente gelten im Pfad abwärts**: `list … limit=` und `catalog=` hängen am Wurzelknoten statt an jedem Unterbefehl.
- **Tabelle getrennt vom Modell** (`grammar_table.rs`) statt `CommandDef` um einen Knoten zu erweitern; die Registry-Zeilen bleiben kurz, der Test prüft, dass jeder Registry-Eintrag einen Tabelleneintrag hat.
- **Handler in `commands/grammar.rs`**: `scripts/generate_director_reference.py` erwartet `commands::<modul>::<fn>`. Das Skript liest `declared_arguments` jetzt aus `docs/wiki/commands/console-grammar.json` (die Registry hat keine Argumente mehr) und führt die Grammatikdateien in `local_source_files`.
- **Prüfung lockerer als im Skelett**: Unterbefehlswörter dürfen wie in Bareos (FindArgKeyword) an beliebiger Stelle stehen; Werte und Pflichtargumente prüft sie nicht.
- **JSON**: jedes Argument trägt alle Felder (auch `false`/`null`), Knoten zusätzlich `origin`, `value`, `long`, `examples`, `see_also`, `free_text`; Quelle `@commands` für Befehlsnamen.
- **`.grammar command=X` im Textmodus** zeigt die Usage-Zeilen, nicht den Hilfetext; den Hilfetext rendert oconsole aus dem JSON (Fixture-Test gegen `manpages::help_text`).
- **Man-Pages ohne Datum** im `.TH`, damit die erzeugten Seiten reproduzierbar sind; nur die Version.
- **Commits**: die Schritte 3–5 sind ein Commit, weil sie ein Modul teilen und Completer und Hinter in einem Zug ersetzen.

## Paritätsfälle

`grammar_accepts_the_parity_cases` liest jede Zeichenkette aus `setup` und `steps` aller `tests/parity/cases/*.jsonl`. Als vom Director angenommene Konsolenzeile gilt jede, deren erstes Wort der Director zu einem registrierten Befehl auflöst (sonst „is an invalid command“); Antworten in Dialogen (`yes`, `mod`, Zahlen, Baumbefehle wie `mark *`) lösen nicht auf. Ergebnis: 3372 Zeilen (rund 1200 verschiedene) passen zur Grammatik; 120 verschiedene Zeilen stehen in der kommentierten Liste `PARITY_REFUSED` mit Grund – Fehlerfälle, die prüfen, wie der Director Eingaben außerhalb seiner Grammatik behandelt (unbekannte Wörter und Schlüssel, Schlüssel ohne Wert, Schlüsselwort mit Wert, Widersprüche wie `enabled disabled`, doppelte Schlüssel, Präfixe wie `.catalog` → `.catalogs`, `status ConfigurationX`, `show POO`), vier Antworten der Restore-Dateiauswahl, die zufällig auf einen Befehl auflösen (`a b`, `add sbin/zic`, `delete sbin/`, `m /usr`), `sqlquery select 1;` und `.levels` mit mehr als einem Wort. Der Test schlägt auch fehl, wenn ein Listeneintrag inzwischen passt oder in keinem Fall mehr vorkommt.

## Reihenfolge (D9) und Commits

1. Grammatikmodell, Tabelle, `.grammar`, odir-Tests, Wiki, Abdeckung, Bareos-Referenz – `cfeb844`
2. Man-Pages aus der Grammatik, `--write-man-pages`, Vorlagen, Deb-Assets – `7648475`
3. –5. oconsole: Client-Modell, Lader mit Rückfall, Vervollständigung, Vorschläge, `help` – `5620a33`
6. fish-Tasten – `1e973d0`
7. Nicht gemacht: Teilstring-Suche in der History (optional), typisiertes `GetGrammar` und MCP-Ressource (brauchen Proto-Arbeit, später).

## Pflege

- Befehl erweitert: Eintrag in `grammar_table.rs` anpassen, dann `UPDATE_GRAMMAR_SNAPSHOT=1 cargo test -p odir grammar` (Snapshot, console-grammar.json, Hilfe-Fixtures) und `UPDATE_MAN_PAGES=1 cargo test -p odir manpages` (oder `odir --write-man-pages packaging/man`), danach `scripts/generate_director_reference.py` wie in [bareos-reference.md](../wiki/commands/bareos-reference.md).
- Neuer Paritätsfall, den die Grammatik ablehnt: Grammatik ergänzen, wenn der Director die Zeile bestimmungsgemäß annimmt; nur echte Fehlerfälle in `PARITY_REFUSED`, mit Grund.
