# Onesimus Director — Command Test Plan

Aktueller Abgleich: **88 Original-Hauptbefehle und 26 Restore-Dateiauswahlbefehle**
([Quellreferenz](wiki/commands/bareos-reference.md)).

Priorität seit 15.10.2026: Zuerst die offenen Unterbefehle, Optionen und
Ablaufketten der 21 teilweise implementierten Hauptbefehle aus
[BACKLOG.md](BACKLOG.md) vollständig prüfen und vervollständigen.
Die folgenden historischen Kategorien sind keine aktuelle Paritätsbewertung.

Generated: 2026-03-29
Test environment: odir :29101, bareos-fd :9102, bareos-sd :9103, PostgreSQL catalog

## Current State

| Category | Count | Status |
|----------|-------|--------|
| IMPL     | 32    | Real logic with config/DB/FD/SD |
| LITE     | 48    | Accept args, return static/formatted responses |
| STUB     | 4     | Not implemented |
| **Total** | **92** | Registry in `command_registry.rs` |

## Test Infrastructure

```
bconsole:  echo "COMMAND" | bconsole -c config/brconsole.conf
oconsole:  echo "COMMAND" | cargo run --bin oconsole -- --address 127.0.0.1 --port 29101
```

All test commands below use bconsole syntax. Pipe through both consoles.

---

## Priority Tier 1 — Daily Operations (MUST WORK)

These are the commands a backup admin uses every single day.
Every one of them must return real data, not stubs.

### T1-GROUP-A: No FD/SD Required (Config + DB only)

#### 1. `version` [IMPL]
```
*version
```
- **Expected**: `bareos-dir Version: X.Y.Z (Debian GNU/Linux ...)`
- **DB/SP**: none
- **Validate**: Contains version number and OS string

#### 2. `time` [IMPL]
```
*time
```
- **Expected**: `2026-03-29 14:30:00`
- **DB/SP**: none
- **Validate**: Valid ISO datetime

#### 3. `help` [IMPL]
```
*help
*help run
*help restore
```
- **Expected**: Command list / per-command usage with args
- **DB/SP**: none
- **Validate**: Lists all 92 commands, `help run` shows job=/level=/client= args

#### 4. `.help` [IMPL]
```
*.help
*.help all
*.help item=run
```
- **Expected**: Tab-separated machine format (name\tdescription\tusage\targs\taudit)
- **DB/SP**: none
- **Validate**: API mode 1 output matches C++ format, JSON mode works

#### 5. `status dir` [IMPL]
```
*status dir
*status director
```
- **Expected**: Director version, client/job/storage counts, sessions, running jobs
- **DB/SP**: `fn_director_status` (v2252), `vw_running_jobs` (v2252)
- **Validate**: Shows correct resource counts from config
- **UPGRADE**: Currently config-only. Should query `fn_director_status` for job totals and `vw_running_jobs` for running job list

#### 6. `list jobs` [IMPL]
```
*list jobs
*list jobs limit=5
*list jobs job=BackupClient1
*list jobid=1
```
- **Expected**: Table with jobid, name, type, level, status, start, end, files, bytes
- **DB/SP**: `vw_job_summary` (v2251)
- **Validate**: Correct column headers, data from DB, limit works

#### 7. `list volumes` [IMPL]
```
*list volumes
*list media
```
- **Expected**: Table with mediaid, volumename, status, bytes, jobs, files, pool
- **DB/SP**: `vw_volume_status` (v2251)
- **Validate**: Shows real volumes from catalog

#### 8. `list clients` [IMPL]
```
*list clients
```
- **Expected**: clientid, name, uname from DB; falls back to config if no catalog
- **DB/SP**: `SELECT ... FROM client`
- **Validate**: At least `bareos-fd` appears

#### 9. `list pools` [IMPL]
```
*list pools
```
- **Expected**: poolid, name, type, numvols, maxvols
- **DB/SP**: `SELECT ... FROM pool`
- **Validate**: At least `Full` pool appears

#### 10. `list files` [IMPL]
```
*list files jobid=1
```
- **Expected**: fileid, jobid, filepath from `vw_file_list`
- **DB/SP**: `vw_file_list` (v2251)
- **Validate**: File paths include directory component

#### 11. `list jobtotals` [IMPL]
```
*list jobtotals
```
- **Expected**: Total jobs, files, bytes
- **DB/SP**: `SELECT COUNT(*), SUM(jobfiles), SUM(jobbytes) FROM job`
- **Validate**: Non-negative numbers

#### 12. `list storages` [IMPL]
```
*list storages
```
- **Expected**: storageid, name, autochanger
- **DB/SP**: `SELECT ... FROM storage`
- **Validate**: At least `File` storage

#### 13. `list schedules` [IMPL]
```
*list schedules
```
- **Expected**: Schedule names with enabled flag
- **DB/SP**: config-only (schedules not in DB)
- **Validate**: `WeeklyCycle` appears

#### 14. `list filesets` [IMPL]
```
*list filesets
```
- **Expected**: Name, Include, Exclude columns
- **DB/SP**: config-only
- **Validate**: Shows include/exclude paths

#### 15. `show jobs` [IMPL]
```
*show jobs
*show clients
*show pools
*show storages
*show jobdefs
*show all
```
- **Expected**: Bareos-style resource dump (`Job { Name = "..." Type = Backup ... }`)
- **DB/SP**: config-only
- **Validate**: Output parseable, all fields present

#### 16. `messages` [IMPL]
```
*messages
```
- **Expected**: Pending messages from bus, or "No pending messages."
- **DB/SP**: none (message bus)
- **Validate**: After a `run`, messages should include job status

#### 17. `.api` [IMPL]
```
*.api 0
*.api 1
*.api 2
```
- **Expected**: `api: mode=0/1/2`
- **DB/SP**: none
- **Validate**: Subsequent commands change format (human/tab/JSON)

#### 18. `reload` [IMPL]
```
*reload
```
- **Expected**: `reloaded` + resource count summary
- **DB/SP**: none
- **Validate**: Counts match config files

#### 19. `whoami` [LITE]
```
*whoami
```
- **Expected**: Console identity
- **DB/SP**: none
- **UPGRADE**: Should return actual console name from TLS-PSK identity

#### 20. `.clients` [IMPL]
```
*.clients
*.clients enabled
*.clients disabled
```
- **Expected**: One client name per line
- **DB/SP**: config-only
- **Validate**: `bareos-fd` in output

#### 21. `.jobs` [IMPL]
```
*.jobs
*.jobs type=Backup
*.jobs enabled
```
- **Expected**: One job name per line
- **DB/SP**: config-only
- **Validate**: `BackupClient1` in output

#### 22. `.pools` [IMPL]
```
*.pools
*.pools type=Backup
```
- **Expected**: One pool name per line
- **DB/SP**: config-only
- **Validate**: `Full` in output

#### 23. `.storage` [IMPL]
```
*.storage
*.storages
*.storage enabled
```
- **Expected**: One storage name per line
- **DB/SP**: config-only
- **Validate**: `File` in output

#### 24. `.filesets` [IMPL]
```
*.filesets
```
- **Expected**: One fileset name per line
- **DB/SP**: config-only

#### 25. `.schedule` [IMPL]
```
*.schedule
*.schedule enabled
```
- **Expected**: One schedule name per line
- **DB/SP**: config-only

#### 26. `.catalogs` [IMPL]
```
*.catalogs
```
- **Expected**: `MyCatalog`
- **DB/SP**: config-only

#### 27. `.consoles` [IMPL]
```
*.consoles
```
- **Expected**: Console resource names
- **DB/SP**: config-only

#### 28. `.jobdefs` [IMPL]
```
*.jobdefs
```
- **Expected**: JobDefs names
- **DB/SP**: config-only

#### 29. `.defaults` [IMPL]
```
*.defaults job=BackupClient1
*.defaults client=bareos-fd
*.defaults storage=File
*.defaults pool=Full
```
- **Expected**: key=value pairs (job=, pool=, client=, level=, etc.)
- **DB/SP**: config-only
- **Validate**: All fields present, values match config

#### 30. `.jobstatus` [IMPL]
```
*.jobstatus
*.jobstatus T
```
- **Expected**: Tab-separated status code table (T=Completed successfully, etc.)
- **DB/SP**: none (static table)
- **Validate**: All 24 status codes present

#### 31. `.levels` [IMPL]
```
*.levels
```
- **Expected**: Full, Incremental, Differential, VirtualFull, ...
- **DB/SP**: none (static)

#### 32. `.types` [IMPL]
```
*.types
```
- **Expected**: Backup, Restore, Verify, Admin, ...
- **DB/SP**: none (static)

#### 33. `.volstatus` [IMPL]
```
*.volstatus
```
- **Expected**: Append, Archive, Disabled, Error, Full, Purged, Recycle, ...
- **DB/SP**: none (static)

#### 34. `.actiononpurge` [IMPL]
```
*.actiononpurge
```
- **Expected**: Truncate, None
- **DB/SP**: none (static)

#### 35. `.authorized` [IMPL]
```
*.authorized job=BackupClient1
*.authorized client=bareos-fd
*.authorized job=NonExistent
```
- **Expected**: `authorized` or `not authorized`
- **DB/SP**: config-only
- **Validate**: Existing resource = authorized, missing = not authorized

#### 36. `enable` / `disable` [IMPL]
```
*disable job=BackupClient1
*enable job=BackupClient1
*disable client=bareos-fd
*enable client=bareos-fd
```
- **Expected**: `Job 'BackupClient1' disabled/enabled`
- **DB/SP**: config-only (in-memory toggle)
- **Validate**: `.jobs enabled` reflects change

#### 37. `configure` [IMPL]
```
*configure add client name=TestFD address=10.0.0.1 password=test123 port=9102
*configure read client=TestFD
*configure update client=TestFD address=10.0.0.2
*configure delete client=TestFD
*configure export
```
- **Expected**: CRUD confirmation messages
- **DB/SP**: config-only (in-memory)
- **Validate**: Full lifecycle works, export shows all resources

#### 38. `sqlquery` / `.sql` [IMPL]
```
*sqlquery SELECT COUNT(*) FROM job
*.sql query="SELECT version FROM version"
*.sql tables
*.sql schema
```
- **Expected**: Query results, table list, schema dump
- **DB/SP**: raw SQL passthrough
- **Validate**: Returns actual data rows

#### 39. `.sqltables` [IMPL]
```
*.sqltables
```
- **Expected**: Table names (job, client, media, pool, path, file, ...)
- **DB/SP**: `pg_catalog.pg_tables`

#### 40. `.sqlschema` [IMPL]
```
*.sqlschema
```
- **Expected**: table_name, column_name, data_type for all tables
- **DB/SP**: `information_schema.columns`

#### 41. `.sqlcount` [IMPL]
```
*.sqlcount
```
- **Expected**: `Jobs: N`
- **DB/SP**: `SELECT COUNT(*) FROM job`

#### 42. `quit` / `exit` [IMPL]
```
*quit
*exit
```
- **Expected**: `bye` + connection close
- **DB/SP**: none

#### 43. `.quit` [IMPL]
```
*.quit
```
- **Expected**: Connection close
- **DB/SP**: none

#### 44. `.` (noop) [IMPL]
```
*.
```
- **Expected**: Empty response (no output)
- **DB/SP**: none

#### 45. `.grpc` [IMPL]
```
*.grpc on
*.grpc off format=json
```
- **Expected**: `gRPC mode: on/off`
- **DB/SP**: none

### T1-GROUP-B: Needs FD

#### 46. `status client` [IMPL]
```
*status client=bareos-fd
```
- **Expected**: Connects to FD, shows version, running jobs, files examined
- **DB/SP**: none (live TLS-PSK connection)
- **Validate**: FD version string, "Running Jobs: ..." section

#### 47. `run` (Full backup) [IMPL]
```
*run job=BackupClient1 level=Full yes
```
- **Expected**: `Job queued. JobId=N` + job summary
- **DB/SP**: `sp_create_job_record` (v2251), `fn_find_appendable_volume` (v2251)
- **Validate**: JobId > 0, job appears in `list jobs`, backup completes

#### 48. `cancel` [IMPL]
```
*cancel jobid=1
```
- **Expected**: `Job 1 marked for cancellation`
- **DB/SP**: none (signals worker)
- **UPGRADE**: Should send SIGTERM to worker, update DB status to 'A'

#### 49. `restore` [IMPL]
```
*restore client=bareos-fd where=/tmp/bareos-restore jobid=1 all done yes
```
- **Expected**: `Restore job queued. JobId=N` + restore summary
- **DB/SP**: `fn_find_latest_backup` (v2251), bootstrap from jobmedia
- **Validate**: Files actually restored to /tmp/bareos-restore

#### 50. `estimate` [LITE]
```
*estimate job=BackupClient1
*estimate job=BackupClient1 listing
```
- **Expected**: Currently stub. Should connect to FD and list files.
- **DB/SP**: none
- **UPGRADE**: Connect to FD, send estimate command, return file count + bytes

### T1-GROUP-C: Needs SD

#### 51. `status storage` [IMPL]
```
*status storage=File
```
- **Expected**: Connects to SD, shows version, device status, running jobs
- **DB/SP**: none (live TLS-PSK connection)
- **Validate**: SD version, device name, volume name if mounted

#### 52. `label` [LITE]
```
*label storage=File volume=TestVol-0001 pool=Full
```
- **Expected**: Currently returns static text. Should send label to SD + create media record.
- **DB/SP**: `INSERT INTO media` (new SP needed: `sp_label_volume`)
- **UPGRADE**: Full SD label protocol + DB media record creation

#### 53. `mount` [LITE]
```
*mount storage=File
```
- **Expected**: Currently static. Should send mount to SD.
- **DB/SP**: none
- **UPGRADE**: TLS-PSK to SD, send "mount" command

#### 54. `unmount` [LITE]
```
*unmount storage=File
*umount storage=File
```
- **Expected**: Currently static. Should send unmount to SD.
- **DB/SP**: none
- **UPGRADE**: TLS-PSK to SD, send "unmount" command

#### 55. `release` [LITE]
```
*release storage=File
```
- **Expected**: Currently static. Should send release to SD.
- **DB/SP**: none
- **UPGRADE**: TLS-PSK to SD, send "release" command

### T1-GROUP-D: Needs FD + SD

#### 56. `run` (Incremental) [IMPL*]
```
*run job=BackupClient1 level=Incremental yes
```
- **Expected**: Currently force-upgrades to Full. Should use `fn_since_utime`.
- **DB/SP**: `fn_since_utime` (v2251)
- **UPGRADE**: Remove forced Full upgrade, use since_utime from catalog

#### 57. `run` (Differential) [IMPL*]
```
*run job=BackupClient1 level=Differential yes
```
- **Expected**: Same issue as Incremental.
- **DB/SP**: `fn_since_utime` (v2251)
- **UPGRADE**: Remove forced Full upgrade

---

## Priority Tier 2 — Weekly Admin Operations

Commands used for maintenance, troubleshooting, and catalog management.

### T2-GROUP-A: No FD/SD Required

#### 58. `prune files` [LITE]
```
*prune files client=bareos-fd
```
- **Expected**: Currently static. Should call `sp_prune_expired_jobs`.
- **DB/SP**: `sp_prune_expired_jobs` (v2252)
- **UPGRADE**: Call SP, return actual count of pruned records

#### 59. `prune jobs` [LITE]
```
*prune jobs client=bareos-fd
```
- **Expected**: Same as above.
- **DB/SP**: `sp_prune_expired_jobs` (v2252)
- **UPGRADE**: Call SP with client filter

#### 60. `prune volumes` [LITE]
```
*prune volumes pool=Full
```
- **Expected**: Currently static. Should call `sp_prune_expired_volumes`.
- **DB/SP**: `sp_prune_expired_volumes` (v2252)
- **UPGRADE**: Call SP, return count

#### 61. `purge volume` [LITE]
```
*purge volume=Full-0001
```
- **Expected**: Currently static. Should call `sp_purge_volume`.
- **DB/SP**: `sp_purge_volume` (v2252)
- **UPGRADE**: Call SP, return purged jobmedia count

#### 62. `purge files` [LITE]
```
*purge files jobid=1
```
- **Expected**: Purge file records for a job.
- **DB/SP**: `DELETE FROM file WHERE jobid = $1` (new SP needed)
- **UPGRADE**: Real catalog operation

#### 63. `purge jobs` [LITE]
```
*purge jobs client=bareos-fd
```
- **Expected**: Purge all job records for a client.
- **DB/SP**: new SP needed: `sp_purge_client_jobs`
- **UPGRADE**: Real catalog operation

#### 64. `update volume` [LITE]
```
*update volume=Full-0001
```
- **Expected**: Currently static. Should update media attributes in catalog.
- **DB/SP**: `UPDATE media SET ... WHERE volumename = $1` (new SP needed)
- **UPGRADE**: Accept volstatus=, maxvolbytes=, volretention=, etc.

#### 65. `update pool` [LITE]
```
*update pool=Full
```
- **Expected**: Sync pool config to DB.
- **DB/SP**: `UPDATE pool SET ... WHERE name = $1` (new SP needed)
- **UPGRADE**: Sync from config to DB

#### 66. `update slots` [LITE]
```
*update slots storage=File
```
- **Expected**: SD autochanger slot update.
- **DB/SP**: none (SD only)
- **UPGRADE**: Autochanger support

#### 67. `delete volume` [LITE]
```
*delete volume=Full-0001
```
- **Expected**: Currently static. Should delete from catalog.
- **DB/SP**: `DELETE FROM media WHERE volumename = $1` (new SP needed)
- **UPGRADE**: Check no active jobs use volume, cascade jobmedia

#### 68. `delete jobid` [LITE]
```
*delete jobid=1
```
- **Expected**: Currently static. Should delete job + jobmedia + file records.
- **DB/SP**: `sp_delete_job` (new SP needed)
- **UPGRADE**: Cascade delete (jobmedia, file, log, basefiles)

#### 69. `rerun` [LITE]
```
*rerun jobid=1
```
- **Expected**: Currently static. Should look up job params and re-submit.
- **DB/SP**: `SELECT * FROM job WHERE jobid = $1`
- **UPGRADE**: Query original job, create new run with same params

#### 70. `.media` [STUB]
```
*.media
```
- **Expected**: Volume names from catalog, one per line.
- **DB/SP**: `vw_media_list` (v2252)
- **UPGRADE**: Query `SELECT volumename FROM vw_media_list`

#### 71. `.profiles` [IMPL]
```
*.profiles
```
- **Expected**: Profile resource names
- **DB/SP**: config-only

#### 72. `.locations` [LITE]
```
*.locations
```
- **Expected**: Empty (locations rarely used)
- **DB/SP**: config-only

#### 73. `.users` [LITE]
```
*.users
```
- **Expected**: Empty or user names
- **DB/SP**: config-only

#### 74. `.msgs` [IMPL]
```
*.msgs
```
- **Expected**: Message resource names
- **DB/SP**: config-only

#### 75. `.messages` [LITE]
```
*.messages
```
- **Expected**: Pending dot-messages (machine format)
- **DB/SP**: message bus

#### 76. `.mediatypes` [IMPL]
```
*.mediatypes
```
- **Expected**: Unique media types from config (e.g., `File`)
- **DB/SP**: config-only

#### 77. `.acl` [LITE]
```
*.acl list
*.acl list console=admin
*.acl set console=admin jobacl=BackupClient1
*.acl delete console=admin jobacl=BackupClient1
*.acl clear console=admin
*.acl copy from=admin to=readonly
```
- **Expected**: ACL management responses
- **DB/SP**: config-only
- **UPGRADE**: Persist ACLs in ConsoleResource, enforce during command dispatch

#### 78. `.catalog` [ORIGINAL]
```
*.catalog
```
- **Expected**: same as `.catalogs` (first prefix match in the Bareos command table, as the original); the former local schema subcommands were unreachable and are removed (S-17)

#### 79. `setdebug` [LITE]
```
*setdebug level=100
*setdebug level=0
```
- **Expected**: `Debug level set to 100`
- **DB/SP**: none
- **UPGRADE**: Actually change tracing filter level

#### 80. `use` [LITE]
```
*use catalog=MyCatalog
```
- **Expected**: `Using default catalog`
- **DB/SP**: none
- **UPGRADE**: Switch CatalogHandle if multiple catalogs configured

#### 81. `autodisplay` [LITE]
```
*autodisplay on
*autodisplay off
```
- **Expected**: `autodisplay on/off`
- **DB/SP**: none
- **UPGRADE**: Toggle auto-message display per session

#### 82. `automount` [LITE]
```
*automount on
*automount off
```
- **Expected**: `automount on/off`
- **DB/SP**: none

#### 83. `gui` [LITE]
```
*gui on
*gui off
```
- **Expected**: `gui mode on/off`
- **DB/SP**: none

#### 84. `trace` [LITE]
```
*trace on
*trace off
```
- **Expected**: `trace on/off`
- **DB/SP**: none

#### 85. `var` [LITE]
```
*var name
```
- **Expected**: Variable expansion
- **DB/SP**: none
- **UPGRADE**: Resolve job/config variables like %c, %n, %j

#### 86. `wait` [LITE]
```
*wait
*wait jobid=1
```
- **Expected**: Block until job completes or "No jobs running."
- **DB/SP**: none
- **UPGRADE**: Actually poll JobManager until job finishes

#### 87. `memory` [LITE]
```
*memory
```
- **Expected**: Memory usage stats
- **DB/SP**: none
- **UPGRADE**: Read /proc/self/status for VmRSS, VmSize

### T2-GROUP-B: Needs SD

#### 88. `truncate` [LITE]
```
*truncate volume=Full-0001 storage=File
```
- **Expected**: Currently static. Should send truncate to SD + update media record.
- **DB/SP**: Update media VolBytes=0
- **UPGRADE**: SD truncate protocol + DB update

#### 89. `relabel` [LITE]
```
*relabel storage=File volume=OldVol
```
- **Expected**: Currently static. Should relabel volume on SD.
- **DB/SP**: Update media record
- **UPGRADE**: SD relabel protocol

#### 90. `import` [LITE]
```
*import storage=File
```
- **Expected**: Import volumes from autochanger magazine.
- **DB/SP**: none
- **UPGRADE**: Autochanger protocol

#### 91. `export` [LITE]
```
*export storage=File
```
- **Expected**: Export volumes to magazine.
- **DB/SP**: none
- **UPGRADE**: Autochanger protocol

---

## Priority Tier 3 — Advanced / Rarely Used

### T3-GROUP-A: No FD/SD Required

#### 92. `status all` [IMPL]
```
*status all
```
- **Expected**: Combined director + all clients + all storages status
- **DB/SP**: FD/SD connections
- **Validate**: All sections present

#### 93. `.status` [IMPL]
```
*.status dir
```
- **Expected**: Same as status but machine format
- **DB/SP**: same as status

#### 94. `create` [LITE]
```
*create pool=Full
```
- **Expected**: Currently static. Should create pool record in catalog from config.
- **DB/SP**: `INSERT INTO pool`
- **UPGRADE**: Sync pool resource to catalog

#### 95. `add` [LITE]
```
*add pool=Full
```
- **Expected**: Add volume to pool.
- **DB/SP**: `INSERT INTO media`
- **UPGRADE**: Create media record with label_format

#### 96. `move` [LITE]
```
*move srcslot=1 dstslot=2 storage=File
```
- **Expected**: Autochanger slot move.
- **DB/SP**: none
- **UPGRADE**: Autochanger protocol

#### 97. `setbandwidth` [LITE]
```
*setbandwidth client=bareos-fd limit=1024
```
- **Expected**: `Bandwidth limit set to 1024`
- **DB/SP**: none
- **UPGRADE**: Send to FD via status protocol

#### 98. `setip` [LITE]
```
*setip client=bareos-fd
```
- **Expected**: Update client address from connecting IP.
- **DB/SP**: none
- **UPGRADE**: Update client resource address

#### 99. `setdevice` [LITE]
```
*setdevice storage=File device=FileChgr1-Dev1
```
- **Expected**: Device selection.
- **DB/SP**: none

#### 100. `resolve` [LITE]
```
*resolve client=bareos-fd
```
- **Expected**: Currently stub. Should DNS resolve the client address.
- **DB/SP**: none
- **UPGRADE**: `std::net::ToSocketAddrs` on client address

### T3-GROUP-B: BVFS (Needs DB, some need FD+SD for restore)

#### 101. `.bvfs_update` [LITE]
```
*.bvfs_update jobid=1
```
- **Expected**: Currently stub. Should populate BVFS path hierarchy cache.
- **DB/SP**: new SP needed: `sp_bvfs_update`
- **UPGRADE**: Build path_hierarchy + path_visibility tables

#### 102. `.bvfs_cleanup` [LITE]
```
*.bvfs_cleanup path=b21234
```
- **Expected**: Remove temporary BVFS tables.
- **DB/SP**: `DROP TABLE IF EXISTS b21234`
- **UPGRADE**: Drop temp table

#### 103. `.bvfs_clear_cache` [LITE]
```
*.bvfs_clear_cache yes
```
- **Expected**: Clear all BVFS cache tables.
- **DB/SP**: Truncate path_hierarchy, path_visibility
- **UPGRADE**: Real truncate

#### 104. `.bvfs_get_jobids` [LITE]
```
*.bvfs_get_jobids jobid=1
```
- **Expected**: Comma-separated list of jobids needed for full restore.
- **DB/SP**: new SP needed: `fn_bvfs_get_jobids`
- **UPGRADE**: Walk dependency chain (Full + last Inc/Diff)

#### 105. `.bvfs_lsdirs` [LITE]
```
*.bvfs_lsdirs jobid=1 path=/
*.bvfs_lsdirs jobid=1 pathid=1
```
- **Expected**: Directory listing in BVFS format.
- **DB/SP**: new SP needed: `fn_bvfs_lsdirs`
- **UPGRADE**: Query path_hierarchy + file table

#### 106. `.bvfs_lsfiles` [LITE]
```
*.bvfs_lsfiles jobid=1 path=/etc/
*.bvfs_lsfiles jobid=1 pathid=1
```
- **Expected**: File listing in BVFS format.
- **DB/SP**: new SP needed: `fn_bvfs_lsfiles`
- **UPGRADE**: Query file + path tables

#### 107. `.bvfs_versions` [LITE]
```
*.bvfs_versions client=bareos-fd pathid=1 filename=passwd
```
- **Expected**: All versions of a file across backups.
- **DB/SP**: new SP needed: `fn_bvfs_versions`
- **UPGRADE**: Query file table across multiple jobs

#### 108. `.bvfs_restore` [LITE]
```
*.bvfs_restore path=b21234 jobid=1
```
- **Expected**: Generate restore table from BVFS selection.
- **DB/SP**: Create temp table with selected files
- **UPGRADE**: Build bootstrap from selection, feed to restore command

---

## Suggested New Stored Procedures and Views (v2253)

These are needed to upgrade LITE commands to IMPL.

### Tier 1 Upgrades

| SP/Function | For Command | Purpose |
|-------------|------------|---------|
| `sp_label_volume(pool, name, mediatype, storage)` | `label` | Create media record + set initial attributes |
| `sp_delete_job(jobid)` | `delete jobid=` | Cascade delete (job, jobmedia, file, log, basefiles) |
| `sp_delete_volume(volname)` | `delete volume=` | Delete media + cascade jobmedia |
| `sp_update_volume_attrs(volname, status, maxbytes, retention...)` | `update volume=` | Bulk attribute update |
| `sp_sync_pool_from_config(name, type, recycle, maxvols, ...)` | `update pool=` / `create` | Sync config to catalog |
| `sp_rerun_job(jobid)` | `rerun` | Copy job params, create new job record |

### Tier 2 Upgrades (BVFS)

| SP/Function | For Command | Purpose |
|-------------|------------|---------|
| `sp_bvfs_update(jobid)` | `.bvfs_update` | Populate path_hierarchy + path_visibility |
| `fn_bvfs_get_jobids(jobid)` | `.bvfs_get_jobids` | Walk Full+Inc/Diff chain |
| `fn_bvfs_lsdirs(jobid, pathid, limit, offset)` | `.bvfs_lsdirs` | Directory listing |
| `fn_bvfs_lsfiles(jobid, pathid, limit, offset)` | `.bvfs_lsfiles` | File listing |
| `fn_bvfs_versions(client, pathid, filename)` | `.bvfs_versions` | File version history |

### Tier 3 Upgrades (Nice to have)

| SP/Function | For Command | Purpose |
|-------------|------------|---------|
| `sp_purge_files(jobid)` | `purge files` | Delete file records for a job |
| `sp_purge_client_jobs(client)` | `purge jobs` | Delete all jobs for a client |
| `fn_memory_stats()` | `memory` | Return process memory via pg_stat_activity |

---

## Test Execution Matrix

### Phase 1: Smoke Test (Config-only, no DB)

Run odir without catalog. Every command should either return
real data from config or a graceful "no catalog" message.

```bash
for cmd in version time "help" ".help" "status dir" \
    "list jobs" "list clients" "show jobs" ".clients" \
    ".jobs" ".pools" ".storage" ".filesets" ".schedule" \
    ".defaults job=BackupClient1" ".jobstatus" ".levels" \
    ".types" ".volstatus" reload whoami ".api 1" \
    "enable job=BackupClient1" "disable job=BackupClient1" \
    "configure export" quit; do
  echo "=== $cmd ==="
  echo "$cmd" | bconsole -c config/brconsole.conf 2>/dev/null | tail -n +3
done
```

### Phase 2: Catalog Integration (DB, no FD/SD)

Start with catalog connected. Verify all list/query commands return real data.

```bash
for cmd in "list jobs" "list volumes" "list clients" "list pools" \
    "list files" "list jobtotals" ".sql tables" ".sqltables" \
    ".sqlschema" ".sqlcount" \
    "sqlquery SELECT COUNT(*) FROM job" \
    ".catalog version" ".catalog tables"; do
  echo "=== $cmd ==="
  echo "$cmd" | bconsole -c config/brconsole.conf 2>/dev/null | tail -n +3
done
```

### Phase 3: FD Integration

Test backup, restore, status with real File Daemon.

```bash
# Full backup cycle
echo "run job=BackupClient1 level=Full yes" | bconsole -c config/brconsole.conf
sleep 10
echo "list jobs limit=1" | bconsole -c config/brconsole.conf
echo "messages" | bconsole -c config/brconsole.conf
echo "status client=bareos-fd" | bconsole -c config/brconsole.conf

# Restore
echo "restore client=bareos-fd where=/tmp/bareos-restore all done yes" | bconsole -c config/brconsole.conf
sleep 10
echo "messages" | bconsole -c config/brconsole.conf
ls -la /tmp/bareos-restore/
```

### Phase 4: SD Integration

Test storage operations with real Storage Daemon.

```bash
echo "status storage=File" | bconsole -c config/brconsole.conf
echo "label storage=File volume=TestVol-0001 pool=Full" | bconsole -c config/brconsole.conf
echo "mount storage=File" | bconsole -c config/brconsole.conf
echo "unmount storage=File" | bconsole -c config/brconsole.conf
```

### Phase 5: Full E2E

Complete backup/restore cycle with all components.

```bash
# 1. Label a volume
echo "label storage=File volume=E2E-Vol-001 pool=Full" | bconsole -c config/brconsole.conf

# 2. Full backup
echo "run job=BackupClient1 level=Full yes" | bconsole -c config/brconsole.conf
sleep 15

# 3. Verify in catalog
echo "list jobs limit=1" | bconsole -c config/brconsole.conf
echo "list volumes" | bconsole -c config/brconsole.conf
echo "list files jobid=1 limit=5" | bconsole -c config/brconsole.conf

# 4. Incremental backup (after changing a file)
echo "test" >> /tmp/bareos-test-file
echo "run job=BackupClient1 level=Incremental yes" | bconsole -c config/brconsole.conf
sleep 15

# 5. Restore
echo "restore client=bareos-fd where=/tmp/bareos-restore all done yes" | bconsole -c config/brconsole.conf
sleep 15

# 6. Verify restore
diff /etc/hosts /tmp/bareos-restore/etc/hosts

# 7. Cleanup
echo "prune jobs client=bareos-fd" | bconsole -c config/brconsole.conf
echo "purge volume=E2E-Vol-001" | bconsole -c config/brconsole.conf
echo "truncate volume=E2E-Vol-001 storage=File" | bconsole -c config/brconsole.conf
```

---

## Implementation Priority Roadmap

### Sprint 1: Make LITE commands real (DB queries)

Highest value, lowest effort. Just wire up the catalog:

1. `.media` -- query `vw_media_list` (v2252) -- 30 min
2. `prune files/jobs/volumes` -- call SPs from v2252 -- 2h
3. `purge volume` -- call `sp_purge_volume` from v2252 -- 1h
4. `delete jobid/volume` -- new SPs or raw SQL -- 2h
5. `update volume/pool` -- catalog UPDATE -- 2h
6. `rerun` -- query original job, re-submit -- 2h
7. `wait` -- poll JobManager -- 1h
8. `memory` -- read /proc/self/status -- 30 min
9. `whoami` -- return TLS identity -- 30 min
10. `resolve` -- DNS lookup -- 30 min
11. `setdebug` -- change tracing filter -- 1h

### Sprint 2: SD protocol commands

Wire up the existing `sd_protocol` module:

1. `label` -- SD label + create media record -- 4h
2. `mount` / `unmount` / `release` -- SD commands -- 2h each
3. `truncate` -- SD truncate + DB update -- 2h
4. `relabel` -- SD relabel + DB update -- 2h

### Sprint 3: Incremental/Differential

Remove the forced Full upgrade:

1. Remove level override in `cmd_run` -- 30 min
2. Wire `fn_since_utime` result to FD `since=` parameter -- 1h
3. Test Inc+Diff restore chain -- 4h

### Sprint 4: BVFS (WebUI restore browsing)

Full BVFS implementation for the web frontend:

1. Create BVFS SPs (v2253) -- 4h
2. `.bvfs_update` -- populate cache -- 4h
3. `.bvfs_lsdirs` / `.bvfs_lsfiles` -- browsing -- 4h
4. `.bvfs_get_jobids` -- dependency chain -- 2h
5. `.bvfs_versions` -- file history -- 2h
6. `.bvfs_restore` -- selection to bootstrap -- 4h

### Sprint 5: estimate (FD connection)

1. `estimate` -- FD estimate protocol -- 4h

---

## Summary

| Category | Commands | Current | Target |
|----------|----------|---------|--------|
| Config-only | 35 | All IMPL | All IMPL |
| DB query | 20 | 12 IMPL, 8 LITE | All IMPL |
| FD protocol | 5 | 3 IMPL, 2 LITE | All IMPL |
| SD protocol | 8 | 2 IMPL, 6 LITE | All IMPL |
| FD+SD | 3 | 1 IMPL, 2 IMPL* | All IMPL |
| BVFS | 8 | 0 IMPL, 8 LITE | All IMPL |
| Session flags | 13 | 1 IMPL, 12 LITE | All IMPL |

**Total: 92 commands, currently 32 IMPL, target 92 IMPL.**

Every command can be implemented. There are no blockers, only priorities.
The catalog SPs in v2252 already cover the hardest part (prune/purge).
Sprint 1 alone turns 11 LITE commands into IMPL in under 2 days.

## MCP-Terminalspiegelung (15.10.2026)

- Produktions- und Entwicklungsbuild: stdio-Handshake, initialized-Notification,
  tools/list, tools/call, Schreibablehnung und Fehler müssen auf stderr sichtbar
  sein; stdout bleibt unverändertes MCP. Spiegelung gilt unabhängig von RUST_LOG.
- HTTP: initialisieren, Session fortsetzen, Tools abfragen/aufrufen, abgewiesene
  Aufrufe und Director-Fehler. Request-/Response-Body, HTTP-Status, Zeit und
  Zugriffskennung müssen sichtbar sein. Authorization-Header dürfen nicht erscheinen.
- SSE: Datenframes vor EOF sichtbar; mehrere Zugriffe mit verschiedenen Kennungen.
  Große Antworten vollständig; Steuerzeichen als Text, keine Terminal-Escape-Wirkung.
- Automatische Transporttests verwenden echte rmcp-Verarbeitung und erfassen den
  Spiegelausgabekanal getrennt von den unveränderten Protokollantworten.

## Command Rate Guard und GN/RD-Prompt (15.10.2026)

- Terminal/stdin mit BNET und gRPC: erster Befehl sofort, Folgebefehle erst ab
  exakt drei Sekunden seit der letzten zugelassenen Eingabe. Ablehnung darf
  keine Director-Nachricht senden und den Timer nicht verlängern.
- YesNo/SubPrompt/TextInput teilen den Timer mit dem Hauptprompt. Zu schnelle
  Antworten verwerfen und denselben Dialog erneut anzeigen; keine Warte-Queue.
- Verstoßmeldung ausschließlich stderr, niemals stdout/Director; abgewiesene
  Befehle nicht in die History übernehmen. Multiline-Paste darf nicht umgehen.
- Prompt GN (grün) bereit, RD (rot) gesperrt. Ohne Tastendruck wieder GN;
  angefangene Eingabe/Cursor erhalten. NO_COLOR=1 ohne ANSI, Guard weiter aktiv.
- EOF beendet ohne zusätzlichen ungebremsten quit-Befehl; manuelle Abbruch-
  Befehle nutzen denselben Guard. MCP und interne Completion-Abfragen unverändert.

Automatische Prüfungen:

```bash
cargo +1.96.0 test -p oconsole
cargo +1.96.0 test -p oconsole --no-default-features --features mcp-development
cargo +1.96.0 build -p oconsole
python3 scripts/test_oconsole_rate_guard.py --binary target/debug/oconsole -v
```

Der Python-Test startet nur einen lokalen BNET-Test-Director und das echte
oconsole-Binary, verwendet temporäre Konfiguration/History und prüft stdin,
alle drei Dialogtypen, Multiline-Paste sowie den Farbwechsel in einem PTY.
Die transportunabhängige gRPC-Eingabe nutzt denselben geprüften Haupt-/Dialogloop;
für einen Live-gRPC-Abgleich dieselben Fälle an der Testinstanz wiederholen.
