# Bareos Director: Originalreferenz und Vollständigkeitsabgleich

[LLM-Referenz (JSON)](bareos-director-commands.json) · [JSON-Schema](bareos-director-commands.schema.json) · [Wiki-Übersicht](README.md) · [Audit-Notizen](audit-notes.json)

Stand: 15.10.2026. Originalquelle: [Bareos `WIP/25.1.2-pre`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc), Commit `fc2c79b6c43eb5752e98ebd86116593dbbbe0c68`.
Der Projektstand nennt 25.1.2; upstream existiert hierfür nur der Tag `WIP/25.1.2-pre`, kein Release-Tag 25.1.2. Diese Referenz behauptet daher keine Übereinstimmung mit einem finalen Release.

## Ergebnis

**88 Originalbefehle, davon 88 lokal registriert.** Hinzu kommen 26 Befehle der separaten Restore-Dateiauswahl und 8 lokale Zusatznamen.
Registrierung ist kein Nachweis funktionaler Vollständigkeit. Der Director ist nach den belegten Lücken **nicht vollständig kompatibel**.
Dies ist ein statischer Quellcodeabgleich; es wurden keine Live-Jobs, Löschungen oder anderen Director-Befehle ausgeführt.

| Status | Anzahl | Bedeutung |
|---|---:|---|
| `partial` | 18 | Konkrete fehlende Optionen oder Teilfunktionen belegt |
| `registered_unverified` | 53 | Handler vorhanden; Funktionsparität nicht nachgewiesen |
| `semantic_mismatch` | 2 | Abweichende Bedeutung gegenüber Bareos |
| `stub` | 15 | Platzhalter statt der beschriebenen Wirkung |

## Herkunft und Grenzen

Die Namen, englischen Beschreibungen, Syntaxtexte, RunScript-Freigaben und Audit-Flags stammen direkt aus `ua_cmds.cc::commands[]`; die Flag-Bedeutung aus `ua_cmdstruct.h`.
Die vollständige Tabelle ist maßgeblich, nicht der historisch unvollständige Syntaxtext von `help`.
`list` und `llist` teilen `list_cmd_usage`. `configure` erzeugt seine Syntax aus Ressourcendirektiven in `dird_conf.cc`; die JSON nennt den Generator und seine Ressourcentypen, statt einen Funktionsnamen als Syntax auszugeben.
`NULL`-Hilfetexte bleiben in JSON `null` (insbesondere `.locations`). Ein leerer/fehlender Syntaxtext beweist nicht, dass ein Befehl keine Parameter akzeptiert.

Bareos akzeptiert in Tabellenreihenfolge Groß-/Kleinschreibung ignorierende Präfixe. Onesimus sucht vollständige Namen und ausdrücklich registrierte Aliase.
Die JSON bewahrt jeden Originalnamen, auch `exit`, `quit`, `.quit`, `llist` und `umount`.
Konfigurationsdirektiven, FD/SD-Protokollbefehle und lokale bconsole-`@`-Befehle gehören nicht zu dieser Konsolentabelle.
Die Syntaxtexte sind Originalhilfe, keine vollständige formale Grammatik aller internen Optionen und Dialoge.

## Übergreifende Befunde

- **Abweichende Befehlsauflösung** Bareos sucht das erste passende Präfix in Tabellenreihenfolge; Onesimus erlaubt vollständige Namen und explizite Aliase. Abkürzungen dürfen daher nicht als portiert gelten. [core/src/dird/ua_cmds.cc:540](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L540) · [crates/onesimus-dird/src/command_registry.rs:315](../../../crates/onesimus-dird/src/command_registry.rs)
- **ACL-Parität fehlt** Das Original prüft Command_ACL vor dem Dispatch. Im lokalen Dispatch ist keine entsprechende Prüfung vorhanden; .authorized prüft Existenz und akzeptiert cmd pauschal. Authentifizierung mit Passwort/Token ist kein Ersatz für diese Autorisierung. [core/src/dird/ua_cmds.cc:540](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L540) · [crates/onesimus-dird/src/command_registry.rs:323](../../../crates/onesimus-dird/src/command_registry.rs) · [crates/onesimus-dird/src/commands/dot.rs:237](../../../crates/onesimus-dird/src/commands/dot.rs)
- **MCP-query klassifiziert Punktbefehle zu großzügig** Die aktuelle MCP-Read-only-Prüfung lässt Punktbefehle bis auf eine Sperrliste zu. Dadurch sind unter anderem .bvfs_update und lokale Erweiterungen wie .catalog erreichbar, obwohl ein führender Punkt keine Read-only-Garantie ist. Die LLM-Referenz vergibt deshalb keine pauschale Read-only-Freigabe. [crates/oconsole/src/mcp.rs:301](../../../crates/oconsole/src/mcp.rs) · [core/src/dird/ua_cmds.cc:290](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L290) · [crates/onesimus-dird/src/commands/catalog.rs:12](../../../crates/onesimus-dird/src/commands/catalog.rs)
- **Scheduler startet keine Worker** dispatch_ready_jobs entfernt fällige Jobs und zählt sie, enthält aber weiterhin den TODO zum Fork eines Workers. Geplante Backups sind dadurch nicht funktionsgleich zum Original; dies ist bereits im Backlog dokumentiert. [crates/onesimus-dird/src/scheduler.rs:148](../../../crates/onesimus-dird/src/scheduler.rs) · [docs/BACKLOG.md:3](../../../docs/BACKLOG.md)
- **Hilfetext ist kein Implementierungsnachweis** Die vorhandene UA_COMMANDS-Tabelle beschreibt Originalsyntax, während die tatsächliche Ausführung über command_registry erfolgt. configure enthält in der lokalen UA-Tabelle einen Generator-Funktionsnamen statt expandierter Syntax, .locations den Text NULL statt eines fehlenden Hilfetextes. [crates/onesimus-dird/src/ua_commands_table.rs:4](../../../crates/onesimus-dird/src/ua_commands_table.rs) · [crates/onesimus-dird/src/command_executor.rs:53](../../../crates/onesimus-dird/src/command_executor.rs)

## Director-Konsole: vollständige Originaltabelle

| Befehl | Originalbeschreibung | Lokaler Status |
|---|---|---|
| [.](dot-noop.md) | no op | `registered_unverified` |
| [.actiononpurge](dot-actiononpurge.md) | List possible actions on purge | `registered_unverified` |
| [.api](dot-api.md) | Switch between different api modes | `registered_unverified` |
| [.authorized](dot-authorized.md) | Check for authorization | `partial` |
| [.catalogs](dot-catalogs.md) | List all catalog resources | `registered_unverified` |
| [.clients](dot-clients.md) | List all client resources | `registered_unverified` |
| [.consoles](dot-consoles.md) | List all console resources | `registered_unverified` |
| [.users](dot-users.md) | List all user resources | `registered_unverified` |
| [.defaults](dot-defaults.md) | Get default settings | `partial` |
| [.filesets](dot-filesets.md) | List all filesets | `registered_unverified` |
| [.help](dot-help.md) | Print parsable information about a command | `registered_unverified` |
| [.jobdefs](dot-jobdefs.md) | List all job defaults resources | `registered_unverified` |
| [.jobs](dot-jobs.md) | List all job resources | `registered_unverified` |
| [.jobstatus](dot-jobstatus.md) | List jobstatus information | `registered_unverified` |
| [.levels](dot-levels.md) | List all backup levels | `registered_unverified` |
| [.locations](dot-locations.md) | Keine Beschreibung in der Originaltabelle (NULL) | `registered_unverified` |
| [.messages](dot-messages.md) | Display pending messages | `partial` |
| [.media](dot-media.md) | List all medias | `registered_unverified` |
| [.mediatypes](dot-mediatypes.md) | List all media types | `registered_unverified` |
| [.msgs](dot-msgs.md) | List all message resources | `registered_unverified` |
| [.pools](dot-pools.md) | List all pool resources | `registered_unverified` |
| [.profiles](dot-profiles.md) | List all profile resources | `registered_unverified` |
| [.quit](dot-quit.md) | Close connection | `registered_unverified` |
| [.sql](dot-sql.md) | Send an arbitrary SQL command | `registered_unverified` |
| [.schedule](dot-schedule.md) | List all schedule resources | `registered_unverified` |
| [.status](dot-status.md) | Report status | `partial` |
| [.storages](dot-storages.md) | List all storage resources | `registered_unverified` |
| [.types](dot-types.md) | List all job types | `registered_unverified` |
| [.volstatus](dot-volstatus.md) | List all volume status | `registered_unverified` |
| [.bvfs_lsdirs](dot-bvfs-lsdirs.md) | List directories using BVFS | `stub` |
| [.bvfs_lsfiles](dot-bvfs-lsfiles.md) | List files using BVFS | `stub` |
| [.bvfs_update](dot-bvfs-update.md) | Update BVFS cache | `stub` |
| [.bvfs_get_jobids](dot-bvfs-get-jobids.md) | Get jobids required for a restore | `stub` |
| [.bvfs_versions](dot-bvfs-versions.md) | List versions of a file | `stub` |
| [.bvfs_restore](dot-bvfs-restore.md) | Mark BVFS files/directories for restore. Stored in handle. | `stub` |
| [.bvfs_cleanup](dot-bvfs-cleanup.md) | Cleanup BVFS cache for a certain handle | `stub` |
| [.bvfs_clear_cache](dot-bvfs-clear-cache.md) | Clear BVFS cache | `stub` |
| [add](add.md) | Add media to a pool | `partial` |
| [autodisplay](autodisplay.md) | Autodisplay console messages | `registered_unverified` |
| [automount](automount.md) | Automount after label | `stub` |
| [cancel](cancel.md) | Cancel a job | `registered_unverified` |
| [configure](configure.md) | Configure director resources | `partial` |
| [create](create.md) | Create DB Pool from resource | `registered_unverified` |
| [delete](delete.md) | Delete volume, pool or job | `registered_unverified` |
| [disable](disable.md) | Disable a job/client/schedule | `registered_unverified` |
| [enable](enable.md) | Enable a job/client/schedule | `registered_unverified` |
| [estimate](estimate.md) | Performs FileSet estimate, listing gives full listing | `stub` |
| [exit](exit.md) | Terminate Bconsole session | `registered_unverified` |
| [export](export.md) | Export volumes from normal slots to import/export slots | `partial` |
| [gui](gui.md) | Switch between interactive (gui off) and non-interactive (gui on) mode | `registered_unverified` |
| [help](help.md) | Print help on specific command | `registered_unverified` |
| [import](import.md) | Import volumes from import/export slots to normal slots | `partial` |
| [label](label.md) | Label a tape | `registered_unverified` |
| [list](list.md) | List objects from catalog | `partial` |
| [llist](llist.md) | Full or long list like list command | `partial` |
| [messages](messages.md) | Display pending messages | `registered_unverified` |
| [memory](memory.md) | Print current memory usage | `registered_unverified` |
| [mount](mount.md) | Mount storage | `registered_unverified` |
| [move](move.md) | Move slots in an autochanger | `semantic_mismatch` |
| [prune](prune.md) | Prune records from catalog | `registered_unverified` |
| [purge](purge.md) | Purge records from catalog | `registered_unverified` |
| [quit](quit.md) | Terminate Bconsole session | `registered_unverified` |
| [query](query.md) | Query catalog | `semantic_mismatch` |
| [restore](restore.md) | Restore files | `partial` |
| [relabel](relabel.md) | Relabel a tape | `registered_unverified` |
| [release](release.md) | Release storage | `registered_unverified` |
| [reload](reload.md) | Reload conf file | `registered_unverified` |
| [rerun](rerun.md) | Rerun a job | `partial` |
| [resolve](resolve.md) | Resolve a hostname | `partial` |
| [run](run.md) | Run a job | `partial` |
| [status](status.md) | Report status | `partial` |
| [setbandwidth](setbandwidth.md) | Sets bandwidth | `stub` |
| [setdebug](setdebug.md) | Sets debug level | `partial` |
| [setdevice](setdevice.md) | Sets device parameter | `partial` |
| [setip](setip.md) | Sets new client address -- if authorized | `stub` |
| [show](show.md) | Show resource records | `registered_unverified` |
| [sqlquery](sqlquery.md) | Use SQL to query catalog | `registered_unverified` |
| [time](time.md) | Print current time | `registered_unverified` |
| [trace](trace.md) | Turn on/off trace to file | `stub` |
| [truncate](truncate.md) | Truncate purged volumes | `registered_unverified` |
| [unmount](unmount.md) | Unmount storage | `registered_unverified` |
| [umount](umount.md) | Umount - for old-time Unix guys, see unmount | `registered_unverified` |
| [update](update.md) | Update volume, pool, slots, job or statistics | `registered_unverified` |
| [use](use.md) | Use specific catalog | `stub` |
| [var](var.md) | Does variable expansion | `stub` |
| [version](version.md) | Print Director version | `registered_unverified` |
| [wait](wait.md) | Wait until no jobs are running | `partial` |
| [whoami](whoami.md) | Print the user name associated with this console | `registered_unverified` |

## Restore-Dateiauswahl (eigener Kontext)

Diese Befehle stehen in `ua_tree.cc::commands[]`, nicht in der Hauptkonsole. Onesimus besitzt derzeit keine entsprechende interaktive Dateiauswahl. Gleichnamige Konsolenbefehle zählen nicht als Implementierung dieses Kontexts.

| Befehl | Originalbeschreibung | Originalquelle |
|---|---|---|
| `abort` | abort and do not do restore | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1080) |
| `add` | add dir/file to be restored recursively, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L548) |
| `cd` | change current directory | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L958) |
| `count` | count marked files in and below the cd | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L604) |
| `delete` | delete dir/file to be restored recursively in dir | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1022) |
| `dir` | long list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L890) |
| `.dir` | long list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L885) |
| `done` | leave file selection mode | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1078) |
| `estimate` | estimate restore size | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L895) |
| `exit` | same as done command | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1078) |
| `find` | find files, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L623) |
| `help` | print help | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L938) |
| `ls` | list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L693) |
| `.ls` | list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L678) |
| `.lsdir` | list subdir in current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L654) |
| `lsmark` | list the marked files in and below the cd | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L765) |
| `.lsmark` | list the marked files in | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L716) |
| `mark` | mark dir/file to be restored recursively, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L548) |
| `markdir` | mark directory name to be restored (no files) | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L571) |
| `pwd` | print current working directory | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L992) |
| `.pwd` | print current working directory | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1009) |
| `unmark` | unmark dir/file to be restored recursively in dir | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1022) |
| `unmarkdir` | unmark directory name only no recursion | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1045) |
| `quit` | quit and do not do restore | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L1080) |
| `.help` | print help | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L669) |
| `?` | print help | [Handler](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_tree.cc#L938) |

## Onesimus-Erweiterungen

Zusatznamen sind keine Belege für upstream vorhandene Funktionen. Die JSON enthält ihre lokalen Handler und Aliase.

`.acl`, `.catalog`, `.grpc`, `.profile`, `.sqlcount`, `.sqlschema`, `.sqltables`, `.storage`.

## Reproduzieren und aktualisieren

```bash
git clone --depth 1 --branch WIP/25.1.2-pre --filter=blob:none --sparse https://github.com/bareos/bareos.git /tmp/bareos-reference
git -C /tmp/bareos-reference sparse-checkout set core/src/dird
python3 scripts/generate_director_reference.py /tmp/bareos-reference
python3 scripts/generate_director_reference.py /tmp/bareos-reference --check
```

Der Generator prüft Commit und unveränderte Originalquellen, extrahiert beide Tabellen samt Handler-Quellen, prüft eindeutige Namen und gleicht die lokale Registry einschließlich Aliase ab. Die SHA-256-Prüfsummen der gelesenen upstream Director-Dateien stehen in der JSON.
Belegte Verhaltenslücken werden in `audit-notes.json` gepflegt. Bei Quellcodeänderungen müssen diese Befunde fachlich neu bewertet werden; eine Registrierung darf nicht automatisch als vollständige Implementierung gelten.
Die JSON hält außerdem die SHA-256-Prüfsummen der betrachteten lokalen Implementierungen fest; `--check` erkennt dadurch auch Handler-Änderungen ohne neue Befehlsnamen.
Original-Hilfetexte und Syntax: Bareos GmbH & Co. KG und Mitwirkende, AGPL-3.0.
