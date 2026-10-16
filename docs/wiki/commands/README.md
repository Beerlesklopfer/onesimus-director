# Onesimus Director — Command Reference

Command documentation for Onesimus Director and the original Bareos console interface. Registered commands are not necessarily functionally complete.

- [Original Bareos command set and completeness audit](bareos-reference.md)
- [LLM reference: commands, syntax, descriptions and implementation status (JSON)](bareos-director-commands.json)
- [JSON Schema](bareos-director-commands.schema.json)

The upstream reference is pinned to Bareos commit `18be77e6832e33f5acd64094b9cba6066f2503f9` (branch `bareos-25`, `WIP/25.1.2-pre-43-g18be77e68`), the exact source of the installed reference binary `bareos-dir 25.1.2~pre43.18be77e68`. It contains all 88 top-level commands and the 26 commands of the separate restore file-selection shell. The audit distinguishes handler registration from verified behavior; consult each page's source-derived section before relying on older examples.

Each command's status is derived from the parity comparison (`tests/parity`, original and Onesimus Director on identical fixtures) and the [compatibility log](../../COMPATIBILITY-LOG.md): `parity_verified` means every listed parity case replies byte-identically and no open log entry concerns the command; `partial` names open log entries, differing cases, uncovered variants or side effects only covered by unit tests; `registered_unverified` means there is no parity case yet. Status overview and roadmap: [TODO-COMMANDS.md](../../TODO-COMMANDS.md).

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
