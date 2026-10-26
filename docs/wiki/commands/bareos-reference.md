# Bareos Director: Originalreferenz und Vollständigkeitsabgleich

[LLM-Referenz (JSON)](bareos-director-commands.json) · [JSON-Schema](bareos-director-commands.schema.json) · [Wiki-Übersicht](README.md) · [Audit-Notizen](audit-notes.json)

Stand: 16.10.2026. Originalquelle: [Bareos `WIP/25.1.2-pre-43-g18be77e68`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc), Commit `18be77e6832e33f5acd64094b9cba6066f2503f9`.
Das ist der exakte Quellstand des installierten Referenz-Directors (`bareos-dir --version` → `25.1.2~pre43.18be77e68`): Branch `bareos-25`, 43 Commits nach dem Tag `WIP/25.1.2-pre` (`fc2c79b6`). Einen Release-Tag 25.1.2 gibt es upstream noch nicht; diese Referenz behauptet daher keine Übereinstimmung mit einem finalen Release.

## Ergebnis

**88 Originalbefehle, davon 88 lokal registriert.** Hinzu kommen 26 Befehle der separaten Restore-Dateiauswahl und 12 lokale Zusatznamen.
Registrierung ist kein Nachweis funktionaler Vollständigkeit. Der Director ist nach den belegten Lücken **nicht vollständig kompatibel**.
Der Status stützt sich auf den Paritätsvergleich (`tests/parity`: Original- und Onesimus-Director nacheinander auf identischer Konfiguration, jede Seite auf eigener Katalogkopie, Rohbytes und Signale je Schritt verglichen) und auf das [Kompatibilitätslog](../../COMPATIBILITY-LOG.md). Paritätslauf vom 16.10.2026 auf `development` = `7993ce6`: 1479 von 1481 Fällen byte-identisch (2 Unterschiede, 0 Fehler).
`parity_verified` gilt nur für die in der JSON unter `parity_cases` genannten Varianten; Seiteneffekte auf echte FD/SD (Labeln, Mount, Bandbreite, Debug, Cancel laufender Jobs) werden im Paritätslauf nicht ausgeführt und sind nur durch Einheitentests belegt.

| Status | Anzahl | Bedeutung |
|---|---:|---|
| `parity_verified` | 53 | Alle Paritätsfälle des Befehls (Text, API-Modi, Fehlerpfade laut `parity_cases`) byte-identisch, kein offener Logeintrag |
| `partial` | 31 | Paritätsfälle vorhanden, aber offene Logeinträge, ungedeckte Varianten oder nur einheitengetestete Seiteneffekte |
| `registered_unverified` | 4 | Handler vorhanden; kein Paritätsfall, Funktionsparität nicht nachgewiesen |

## Herkunft und Grenzen

Die Namen, englischen Beschreibungen, Syntaxtexte, RunScript-Freigaben und Audit-Flags stammen direkt aus `ua_cmds.cc::commands[]`; die Flag-Bedeutung aus `ua_cmdstruct.h`.
Die vollständige Tabelle ist maßgeblich, nicht der historisch unvollständige Syntaxtext von `help`.
`list` und `llist` teilen `list_cmd_usage`. `configure` erzeugt seine Syntax aus Ressourcendirektiven in `dird_conf.cc`; die JSON nennt den Generator und seine Ressourcentypen, statt einen Funktionsnamen als Syntax auszugeben.
`NULL`-Hilfetexte bleiben in JSON `null` (insbesondere `.locations`). Ein leerer/fehlender Syntaxtext beweist nicht, dass ein Befehl keine Parameter akzeptiert.

Bareos und Onesimus akzeptieren in der Original-Tabellenreihenfolge Groß-/Kleinschreibung ignorierende Präfixe; lokale Erweiterungen werden zusätzlich über ihre vollständigen Namen und Aliase aufgelöst.
Die JSON bewahrt jeden Originalnamen, auch `exit`, `quit`, `.quit`, `llist` und `umount`.
Konfigurationsdirektiven, FD/SD-Protokollbefehle und lokale bconsole-`@`-Befehle gehören nicht zu dieser Konsolentabelle.
Die Syntaxtexte sind Originalhilfe, keine vollständige formale Grammatik aller internen Optionen und Dialoge.

## MCP-Buildgrenzen

Der Standard- und Debian-Build verwendet `mcp-production`: ausschließlich das lesende MCP-Tool `query`, kein aktivierbarer Schreibhandler und kein `--mcp-allow-write`-Schalter.
Nur `--no-default-features --features mcp-development` kompiliert `execute` ein; zum Start ist zusätzlich `--mcp-allow-write` erforderlich. Beide Features zusammen oder keines werden vom Compiler abgewiesen. Diese Grenze gilt für stdio und HTTP. BNET/gRPC zum Director sowie normale interaktive Konsolenzugriffe bleiben davon unabhängig.

## Übergreifende Befunde

- **Scheduler startet keine Worker** dispatch_ready_jobs entfernt fällige Jobs und zählt sie, enthält aber weiterhin den TODO zum Fork eines Workers. Geplante Backups sind dadurch nicht funktionsgleich zum Original; dies ist im Backlog dokumentiert und vom Paritätslauf nicht erfasst. [crates/odir/src/scheduler.rs:165](../../../crates/odir/src/scheduler.rs) · [docs/BACKLOG.md:41](../../../docs/BACKLOG.md)
- **Daemon-Seiteneffekte nicht im Paritätslauf** FD und SD sind im Paritätslauf echt und gemeinsam. Zustandsändernde Daemonbefehle (Labeln, Unmount, Bandbreiten- und Debuglimits, Cancel laufender Jobs, echte Backup-/Restore-Jobs) werden deshalb nicht live verglichen; ihre Wirkung ist nur durch Einheitentests belegt. Betroffene Befehle stehen höchstens auf partial. [docs/COMPATIBILITY-LOG.md:110](../../../docs/COMPATIBILITY-LOG.md)
- **Katalogwirkung wird nicht verglichen** Der Paritätsvergleich prüft Antwortbytes, Signale und Endgrund je Schritt sowie das Audit-Log, nicht den Katalogzustand nach einem Befehl. Katalogändernde Befehle sind nur so weit belegt, wie Folgeschritte im selben Fall (z. B. list/show) die Wirkung sichtbar machen. [tests/parity/parity.py:162](../../../tests/parity/parity.py)

## Director-Konsole: vollständige Originaltabelle

| Befehl | Originalbeschreibung | Lokaler Status |
|---|---|---|
| [.](dot-noop.md) | no op | `parity_verified` |
| [.actiononpurge](dot-actiononpurge.md) | List possible actions on purge | `parity_verified` |
| [.api](dot-api.md) | Switch between different api modes | `parity_verified` |
| [.authorized](dot-authorized.md) | Check for authorization | `parity_verified` |
| [.catalogs](dot-catalogs.md) | List all catalog resources | `parity_verified` |
| [.clients](dot-clients.md) | List all client resources | `parity_verified` |
| [.consoles](dot-consoles.md) | List all console resources | `parity_verified` |
| [.users](dot-users.md) | List all user resources | `parity_verified` |
| [.defaults](dot-defaults.md) | Get default settings | `parity_verified` |
| [.filesets](dot-filesets.md) | List all filesets | `parity_verified` |
| [.help](dot-help.md) | Print parsable information about a command | `parity_verified` |
| [.jobdefs](dot-jobdefs.md) | List all job defaults resources | `parity_verified` |
| [.jobs](dot-jobs.md) | List all job resources | `parity_verified` |
| [.jobstatus](dot-jobstatus.md) | List jobstatus information | `parity_verified` |
| [.levels](dot-levels.md) | List all backup levels | `parity_verified` |
| [.locations](dot-locations.md) | Keine Beschreibung in der Originaltabelle (NULL) | `parity_verified` |
| [.messages](dot-messages.md) | Display pending messages | `parity_verified` |
| [.media](dot-media.md) | List all medias | `parity_verified` |
| [.mediatypes](dot-mediatypes.md) | List all media types | `parity_verified` |
| [.msgs](dot-msgs.md) | List all message resources | `parity_verified` |
| [.pools](dot-pools.md) | List all pool resources | `partial` |
| [.profiles](dot-profiles.md) | List all profile resources | `parity_verified` |
| [.quit](dot-quit.md) | Close connection | `parity_verified` |
| [.sql](dot-sql.md) | Send an arbitrary SQL command | `parity_verified` |
| [.schedule](dot-schedule.md) | List all schedule resources | `parity_verified` |
| [.status](dot-status.md) | Report status | `partial` |
| [.storages](dot-storages.md) | List all storage resources | `parity_verified` |
| [.types](dot-types.md) | List all job types | `parity_verified` |
| [.volstatus](dot-volstatus.md) | List all volume status | `parity_verified` |
| [.bvfs_lsdirs](dot-bvfs-lsdirs.md) | List directories using BVFS | `parity_verified` |
| [.bvfs_lsfiles](dot-bvfs-lsfiles.md) | List files using BVFS | `parity_verified` |
| [.bvfs_update](dot-bvfs-update.md) | Update BVFS cache | `parity_verified` |
| [.bvfs_get_jobids](dot-bvfs-get-jobids.md) | Get jobids required for a restore | `parity_verified` |
| [.bvfs_versions](dot-bvfs-versions.md) | List versions of a file | `parity_verified` |
| [.bvfs_restore](dot-bvfs-restore.md) | Mark BVFS files/directories for restore. Stored in handle. | `registered_unverified` |
| [.bvfs_cleanup](dot-bvfs-cleanup.md) | Cleanup BVFS cache for a certain handle | `partial` |
| [.bvfs_clear_cache](dot-bvfs-clear-cache.md) | Clear BVFS cache | `partial` |
| [add](add.md) | Add media to a pool | `partial` |
| [autodisplay](autodisplay.md) | Autodisplay console messages | `parity_verified` |
| [automount](automount.md) | Automount after label | `parity_verified` |
| [cancel](cancel.md) | Cancel a job | `partial` |
| [configure](configure.md) | Configure director resources | `parity_verified` |
| [create](create.md) | Create DB Pool from resource | `partial` |
| [delete](delete.md) | Delete volume, pool or job | `partial` |
| [disable](disable.md) | Disable a job/client/schedule | `parity_verified` |
| [enable](enable.md) | Enable a job/client/schedule | `parity_verified` |
| [estimate](estimate.md) | Performs FileSet estimate, listing gives full listing | `parity_verified` |
| [exit](exit.md) | Terminate Bconsole session | `parity_verified` |
| [export](export.md) | Export volumes from normal slots to import/export slots | `partial` |
| [gui](gui.md) | Switch between interactive (gui off) and non-interactive (gui on) mode | `parity_verified` |
| [help](help.md) | Print help on specific command | `parity_verified` |
| [import](import.md) | Import volumes from import/export slots to normal slots | `partial` |
| [label](label.md) | Label a tape | `partial` |
| [list](list.md) | List objects from catalog | `partial` |
| [llist](llist.md) | Full or long list like list command | `partial` |
| [messages](messages.md) | Display pending messages | `parity_verified` |
| [memory](memory.md) | Print current memory usage | `parity_verified` |
| [mount](mount.md) | Mount storage | `partial` |
| [move](move.md) | Move slots in an autochanger | `partial` |
| [prune](prune.md) | Prune records from catalog | `partial` |
| [purge](purge.md) | Purge records from catalog | `partial` |
| [quit](quit.md) | Terminate Bconsole session | `parity_verified` |
| [query](query.md) | Query catalog | `parity_verified` |
| [restore](restore.md) | Restore files | `partial` |
| [relabel](relabel.md) | Relabel a tape | `partial` |
| [release](release.md) | Release storage | `partial` |
| [reload](reload.md) | Reload conf file | `registered_unverified` |
| [rerun](rerun.md) | Rerun a job | `partial` |
| [resolve](resolve.md) | Resolve a hostname | `parity_verified` |
| [run](run.md) | Run a job | `partial` |
| [status](status.md) | Report status | `partial` |
| [setbandwidth](setbandwidth.md) | Sets bandwidth | `partial` |
| [setdebug](setdebug.md) | Sets debug level | `partial` |
| [setdevice](setdevice.md) | Sets device parameter | `partial` |
| [setip](setip.md) | Sets new client address -- if authorized | `partial` |
| [show](show.md) | Show resource records | `parity_verified` |
| [sqlquery](sqlquery.md) | Use SQL to query catalog | `parity_verified` |
| [time](time.md) | Print current time | `parity_verified` |
| [trace](trace.md) | Turn on/off trace to file | `partial` |
| [truncate](truncate.md) | Truncate purged volumes | `partial` |
| [unmount](unmount.md) | Unmount storage | `registered_unverified` |
| [umount](umount.md) | Umount - for old-time Unix guys, see unmount | `registered_unverified` |
| [update](update.md) | Update volume, pool, slots, job or statistics | `partial` |
| [use](use.md) | Use specific catalog | `parity_verified` |
| [var](var.md) | Does variable expansion | `parity_verified` |
| [version](version.md) | Print Director version | `parity_verified` |
| [wait](wait.md) | Wait until no jobs are running | `partial` |
| [whoami](whoami.md) | Print the user name associated with this console | `parity_verified` |

## Restore-Dateiauswahl (eigener Kontext)

Diese Befehle stehen in `ua_tree.cc::commands[]`, nicht in der Hauptkonsole. Gleichnamige Konsolenbefehle zählen nicht als Implementierung dieses Kontexts. Status: `missing_context` 26.

| Befehl | Originalbeschreibung | Originalquelle | Lokaler Status |
|---|---|---|---|
| `abort` | abort and do not do restore | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1080) | `missing_context` |
| `add` | add dir/file to be restored recursively, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L548) | `missing_context` |
| `cd` | change current directory | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L958) | `missing_context` |
| `count` | count marked files in and below the cd | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L604) | `missing_context` |
| `delete` | delete dir/file to be restored recursively in dir | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1022) | `missing_context` |
| `dir` | long list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L890) | `missing_context` |
| `.dir` | long list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L885) | `missing_context` |
| `done` | leave file selection mode | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1078) | `missing_context` |
| `estimate` | estimate restore size | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L895) | `missing_context` |
| `exit` | same as done command | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1078) | `missing_context` |
| `find` | find files, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L623) | `missing_context` |
| `help` | print help | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L938) | `missing_context` |
| `ls` | list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L693) | `missing_context` |
| `.ls` | list current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L678) | `missing_context` |
| `.lsdir` | list subdir in current directory, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L654) | `missing_context` |
| `lsmark` | list the marked files in and below the cd | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L765) | `missing_context` |
| `.lsmark` | list the marked files in | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L716) | `missing_context` |
| `mark` | mark dir/file to be restored recursively, wildcards allowed | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L548) | `missing_context` |
| `markdir` | mark directory name to be restored (no files) | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L571) | `missing_context` |
| `pwd` | print current working directory | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L992) | `missing_context` |
| `.pwd` | print current working directory | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1009) | `missing_context` |
| `unmark` | unmark dir/file to be restored recursively in dir | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1022) | `missing_context` |
| `unmarkdir` | unmark directory name only no recursion | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1045) | `missing_context` |
| `quit` | quit and do not do restore | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L1080) | `missing_context` |
| `.help` | print help | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L669) | `missing_context` |
| `?` | print help | [Handler](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_tree.cc#L938) | `missing_context` |

## Onesimus-Erweiterungen

Zusatznamen sind keine Belege für upstream vorhandene Funktionen. Die JSON enthält ihre lokalen Handler und Aliase.

`.acl`, `.grpc`, `.ovfs_latest`, `.ovfs_ls`, `.ovfs_status`, `.ovfs_tree`, `.ovfs_versions`, `.profile`, `.sqlcount`, `.sqlschema`, `.sqltables`, `.storage`.

## Reproduzieren und aktualisieren

```bash
git clone --filter=blob:none --no-checkout --sparse https://github.com/bareos/bareos.git /tmp/bareos-director-reference-18be77e6
git -C /tmp/bareos-director-reference-18be77e6 sparse-checkout set --no-cone core/src/dird core/src/lib core/src/cats core/src/console core/src/stored core/src/filed core/src/include core/src/findlib
git -C /tmp/bareos-director-reference-18be77e6 checkout --detach 18be77e6832e33f5acd64094b9cba6066f2503f9
python3 scripts/generate_director_reference.py /tmp/bareos-director-reference-18be77e6
python3 scripts/generate_director_reference.py /tmp/bareos-director-reference-18be77e6 --check
python3 scripts/generate_configure_usage.py /tmp/bareos-director-reference-18be77e6 --check
```

Der Generator prüft Commit und unveränderte Originalquellen, extrahiert beide Tabellen samt Handler-Quellen, prüft eindeutige Namen und gleicht die lokale Registry einschließlich Aliase ab. Die SHA-256-Prüfsummen der gelesenen upstream Director-Dateien stehen in der JSON.
Belegte Verhaltenslücken werden in `audit-notes.json` gepflegt. Bei Quellcodeänderungen müssen diese Befunde fachlich neu bewertet werden; eine Registrierung darf nicht automatisch als vollständige Implementierung gelten.
Die JSON hält außerdem die SHA-256-Prüfsummen der betrachteten lokalen Implementierungen fest; `--check` erkennt dadurch auch Handler-Änderungen ohne neue Befehlsnamen.
Original-Hilfetexte und Syntax: Bareos GmbH & Co. KG und Mitwirkende, AGPL-3.0.
