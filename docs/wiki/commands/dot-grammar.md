# .grammar — Grammatik der Konsolenbefehle (Onesimus)

## Zweck

`.grammar` liefert die Grammatik aller Befehle, die die Konsole ausführen darf: Unterbefehle, Schlüsselwörter, `schlüssel=wert`-Argumente, Wertequellen, wiederholbare Schlüssel und sich ausschließende Gruppen. `oconsole` lädt sie beim Verbinden und nach `reload`/`configure` für Vervollständigung und Hinweise ([oconsole: Vervollständigung und Tasten](../oconsole-vervollstaendigung.md)); die Man-Pages `odir-commands(7)` und `odir-<befehl>(7)` entstehen aus derselben Grammatik.

Quelle ist die Befehlsregistrierung des Directors (`crates/odir/src/command_registry.rs`) mit der Grammatiktabelle (`crates/odir/src/grammar_table.rs`). Ein Test prüft jede Konsolenzeile der Paritätsfälle gegen die Grammatik, damit sie bei Erweiterungen eines Befehls vollständig bleibt.

**Onesimus-Erweiterung, nicht im Original.** Das Bareos-Original antwortet `.grammar: is an invalid command.` (kein Bareos-Befehl beginnt mit `.grammar`). Daran erkennen Clients einen Director ohne Grammatik und fallen auf `.help` zurück.

## Rechte

Command-ACL `.grammar` wie jeder Befehl. Die Antwort enthält nur Befehle, die die Konsole ausführen darf (dieselbe Prüfung wie beim Ausführen, Datei- und Katalog-ACL); ein Alias, den die ACL verbietet (etwa `llist` bei erlaubtem `list`), fehlt. `quit`, `exit` und `whoami` sind wie beim Ausführen immer dabei.

## Syntax

```text
.grammar [command=<befehl>]
```

`command=` beschränkt die Antwort auf einen Befehl (Name, Alias oder Abkürzung wie beim Ausführen).

## API-Modi

`.api 2` liefert als `result` das Objekt `grammar` (Schema 1):

```json
{"grammar": {"schema": 1, "commands": [
  {"word": "purge", "aliases": [], "dot": false, "origin": "bareos",
   "description": "Purge records from catalog", "value": null,
   "long": ["…"], "examples": [{"line": "purge volume all allpools yes", "explanation": "…"}],
   "see_also": ["prune", "delete", "truncate"], "free_text": false,
   "args": [{"name": "catalog", "kind": "value", "type": "resource", "source": ".catalogs", …}],
   "subs": [
     {"word": "volume", "value": {"name": "volume", "kind": "value", "type": "resource",
                                  "source": ".media", "value_optional": true, …},
      "args": [
        {"name": "all", "kind": "keyword", …},
        {"name": "pool", "kind": "value", "type": "resource", "source": ".pools", "group": "scope", …},
        {"name": "allpools", "kind": "keyword", "group": "scope", …},
        {"name": "volstatus", "kind": "value", "type": "enum", "list": true,
         "values": ["Full", "Used", "Error", "Append", "Archive", "Disabled", "Read-Only"], …},
        {"name": "dryrun", "kind": "keyword", "group": "mode", …},
        {"name": "yes", "kind": "keyword", "group": "mode", …}], "subs": []}]}]}}
```

Felder eines Knotens (Befehl oder Unterbefehl): `word`, `aliases`, `dot`, `origin` (`bareos` oder `onesimus`), `description`, `value` (der eigene Wert des Worts wie `jobid=<jobid>`, sonst `null`), `long` (Absätze der Beschreibung), `examples`, `see_also`, `free_text` (der Rest der Zeile ist freier Text, `var`), `args`, `subs`.

Felder eines Arguments: `name`, `kind` (`keyword` oder `value`), `type` (`string`, `integer`, `bool`, `enum`, `resource`), `required`, `repeatable`, `group` (`null` oder Gruppenname: Argumente derselben Gruppe schließen sich aus), `list` (mehrere Werte mit Komma in einem Wert), `positional` (Wert ohne Schlüssel, `name` ist der Platzhalter), `value_optional` (der Schlüssel darf auch ohne `=wert` stehen), `description`, dazu `values` bei festen Werten, `source` bei Werten aus einem Punktbefehl (`@commands`: die Befehlswörter der Grammatik selbst) und `default`, wenn der Director einen Vorgabewert hat.

Regeln für Clients: Die Argumente eines Knotens gelten auch in allen Unterbefehlen darunter. `required` heißt „zuerst anbieten“; fehlende Werte fragt der Director meist selbst ab. Unbekannte Felder werden ignoriert; `schema` steigt nur bei inkompatiblen Änderungen.

`.api 0`/`.api 1` zeigen je Befehl eine Zeile `wort -- beschreibung` und darunter eingerückt eine Usage-Zeile je Befehl und Unterbefehl.

## Beispiele

```text
*.grammar command=purge
purge -- Purge records from catalog
    purge [catalog=<catalog>]
    purge files [job=<job>] [jobid=<jobid>[,...]] [client=<client>] [volume=<volume>] [yes]
    purge jobs [client=<client>] [volume=<volume>] [pool=<pool>] [jobstatus=<jobstatus>[,...]] [yes]
    purge volume[=<volume>] [all] [pool=<pool> | allpools] [volstatus=<volstatus>[,...]] [volume=<volume> ...] [storage=<storage>] [devicetype=<devicetype>] [drive=<drive>] [action=<truncate|all>] [dryrun | yes]
    purge quota [client=<client>] [yes]
```

Die vollständige Grammatik ohne ACL-Filter liegt als [console-grammar.json](console-grammar.json) im Repository (vom Test `console_grammar_doc_is_current` geschrieben).

## Siehe auch

[.help](dot-help.md), [help](help.md), Planung: [OCONSOLE-COMPLETION.md](../../plans/OCONSOLE-COMPLETION.md).
