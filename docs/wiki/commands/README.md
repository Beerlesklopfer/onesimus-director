# Onesimus Director — Command Reference

Command documentation for Onesimus Director and the original Bareos console interface. Registered commands are not necessarily functionally complete.

- [Original Bareos command set and completeness audit](bareos-reference.md)
- [LLM reference: commands, syntax, descriptions and implementation status (JSON)](bareos-director-commands.json)
- [JSON Schema](bareos-director-commands.schema.json)

The upstream reference is pinned to Bareos commit `18be77e6832e33f5acd64094b9cba6066f2503f9` (branch `bareos-25`, `WIP/25.1.2-pre-43-g18be77e68`), the exact source of the installed reference binary `bareos-dir 25.1.2~pre43.18be77e68`. It contains all 88 top-level commands and the 26 commands of the separate restore file-selection shell. The audit distinguishes handler registration from verified behavior; consult each page's source-derived section before relying on older examples.

Each command's status is derived from the parity comparison (`tests/parity`, original and Onesimus Director on identical fixtures) and the [compatibility log](../../COMPATIBILITY-LOG.md): `parity_verified` means every listed parity case replies byte-identically and no open log entry concerns the command; `partial` names open log entries, differing cases, uncovered variants or side effects only covered by unit tests; `registered_unverified` means there is no parity case yet. Status overview and roadmap: [TODO-COMMANDS.md](../../TODO-COMMANDS.md).

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
  - `input> ` bei freier Texteingabe.
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

## Console Commands

| Command | Description |
|---------|-------------|
| [version](version.md) | Show Director version |
| [time](time.md) | Show current date and time |
| [status](status.md) | Show Director/Client/Storage status |
| [help](help.md) | List available commands |
| [quit](quit.md) | Disconnect from Director |

## Job Commands

| Command | Description |
|---------|-------------|
| [run](run.md) | Start a backup/restore/verify job |
| [cancel](cancel.md) | Cancel a running job |
| [restore](restore.md) | Restore files from backup |
| [estimate](estimate.md) | Estimate backup size |
| [enable](enable.md) | Enable a job/client/schedule/storage |
| [disable](disable.md) | Disable a job/client/schedule/storage |
| [rerun](rerun.md) | Re-run a previously failed job |
| [wait](wait.md) | Wait for a job to finish |

## List / Query Commands

| Command | Description |
|---------|-------------|
| [list](list.md) | List catalog records (jobs, volumes, clients, ...) |
| [llist](llist.md) | Long list (same as list, more columns) |
| [sqlquery](sqlquery.md) | Execute raw SQL against catalog |
| [show](show.md) | Show resource configuration |
| [messages](messages.md) | Display pending messages |

## Volume / Storage Commands

| Command | Description |
|---------|-------------|
| [label](label.md) | Label a new volume |
| [relabel](relabel.md) | Re-label an existing volume |
| [mount](mount.md) | Mount a storage device |
| [unmount](unmount.md) | Unmount a storage device |
| [release](release.md) | Release a storage device |
| [update](update.md) | Update volume/pool attributes |
| [delete](delete.md) | Delete a volume or job record |
| [prune](prune.md) | Prune expired records |
| [purge](purge.md) | Purge all records from a volume |
| [truncate](truncate.md) | Truncate a purged volume |
| [import](import.md) | Import volumes from a magazine |
| [export](export.md) | Export volumes to a magazine |
| [move](move.md) | Move a volume between pools |
| [add](add.md) | Add a volume to a pool |
| [create](create.md) | Create a pool from resource |

## Administration

| Command | Description |
|---------|-------------|
| [configure](configure.md) | Add/modify/delete/read resources |
| [reload](reload.md) | Reload configuration |
| [setdebug](setdebug.md) | Set debug level |
| [setbandwidth](setbandwidth.md) | Set bandwidth limit |
| [setip](setip.md) | Set client IP address |
| [setdevice](setdevice.md) | Set device parameters |
| [resolve](resolve.md) | Resolve a hostname |
| [autodisplay](autodisplay.md) | Toggle automatic message display |
| [automount](automount.md) | Toggle automatic mounting |
| [gui](gui.md) | Set GUI mode |
| [trace](trace.md) | Toggle trace output |
| [var](var.md) | Display variable expansion |
| [memory](memory.md) | Show memory usage |
| [whoami](whoami.md) | Show current console identity |
| [use](use.md) | Switch catalog |

## Dot Commands (Machine Interface)

Used by oconsole/bconsole for tab-completion and automation.

| Command | Description |
|---------|-------------|
| [.help](dot-help.md) | List commands with arguments (machine format) |
| [.status](dot-status.md) | Status in machine format |
| [.clients](dot-clients.md) | List client names |
| [.jobs](dot-jobs.md) | List job names |
| [.pools](dot-pools.md) | List pool names |
| [.storage](dot-storage.md) | List storage names |
| [.filesets](dot-filesets.md) | List fileset names |
| [.schedule](dot-schedule.md) | List schedule names |
| [.catalogs](dot-catalogs.md) | List catalog names |
| [.consoles](dot-consoles.md) | List console names |
| [.jobdefs](dot-jobdefs.md) | List jobdefs names |
| [.profiles](dot-profiles.md) | List profile names |
| [.locations](dot-locations.md) | List location names |
| [.media](dot-media.md) | List volume names |
| [.mediatypes](dot-mediatypes.md) | List media types |
| [.messages](dot-messages.md) | List message resource names |
| [.msgs](dot-msgs.md) | List pending message types |
| [.users](dot-users.md) | List user resource names |
| [.defaults](dot-defaults.md) | Show default values for a resource |
| [.jobstatus](dot-jobstatus.md) | List job status codes |
| [.authorized](dot-authorized.md) | Check command authorization |
| [.levels](dot-levels.md) | List backup levels |
| [.types](dot-types.md) | List job types |
| [.volstatus](dot-volstatus.md) | List volume status values |
| [.actiononpurge](dot-actiononpurge.md) | List action-on-purge options |
| [.api](dot-api.md) | Set API output mode (0/1/2) |
| [.grpc](dot-grpc.md) | Toggle gRPC mode |
| [.sql](dot-sql.md) | Execute SQL query |
| [.sqltables](dot-sqltables.md) | List catalog tables |
| [.sqlschema](dot-sqlschema.md) | Show catalog schema |
| [.sqlcount](dot-sqlcount.md) | Count catalog records |
| [.acl](dot-acl.md) | Manage console ACLs |
| [.catalog](dot-catalog.md) | Catalog management (migrate, validate, ...) |

## BVFS Commands (Virtual File System)

| Command | Description |
|---------|-------------|
| [.bvfs_update](dot-bvfs-update.md) | Update BVFS cache |
| [.bvfs_cleanup](dot-bvfs-cleanup.md) | Cleanup BVFS temp tables |
| [.bvfs_clear_cache](dot-bvfs-clear-cache.md) | Clear BVFS cache |
| [.bvfs_get_jobids](dot-bvfs-get-jobids.md) | Get job IDs for BVFS restore |
| [.bvfs_lsdirs](dot-bvfs-lsdirs.md) | List directories (BVFS) |
| [.bvfs_lsfiles](dot-bvfs-lsfiles.md) | List files (BVFS) |
| [.bvfs_versions](dot-bvfs-versions.md) | List file versions (BVFS) |
| [.bvfs_restore](dot-bvfs-restore.md) | Restore via BVFS selection |

## OVFS Commands (Onesimus restore trees)

Onesimus extensions without Bareos equivalent. They use the PostgreSQL
extension `ovfs` when it is installed in the catalog and the Director fallback
(plain SQL, same results) otherwise. Also available as read-only MCP tools in
`oconsole` (`ovfs_status`, `ovfs_latest`, `ovfs_tree`, `ovfs_ls`, `ovfs_versions`).

| Command | Description |
|---------|-------------|
| [.ovfs_status](dot-ovfs-status.md) | Extension version and which path restore trees use |
| [.ovfs_latest](dot-ovfs-latest.md) | Job chain of a client's latest state (restore option 5) |
| [.ovfs_tree](dot-ovfs-tree.md) | Restore tree of a job chain as JSON, with depth |
| [.ovfs_ls](dot-ovfs-ls.md) | Direct children of one directory |
| [.ovfs_versions](dot-ovfs-versions.md) | All versions of one file |
