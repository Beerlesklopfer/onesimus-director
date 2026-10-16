# prune -- Prune Expired Records from Catalog

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L383) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Prune records from catalog

**Originalsyntax:**

```text
prune files [client=<client>] [pool=<pool>] [yes] | jobs [client=<client>] [pool=<pool>] [yes] | volume [all] [=volume] [pool=<pool>] [yes] | stats [yes] | directory [=directory] [client=<client>] [recursive] [yes]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 35 von 35 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-c030-client, parity-c030-job, parity-c030-pool, parity-c030-storage.
- Verglichen sind Dialoge, ACL-Prüfungen (C-030) und Bestätigungstexte auf der Katalogkopie; der Katalogzustand nach dem Prunen wird nicht verglichen.

**Paritätsfälle (35):** `c011-prune-cancel`, `c011-prune-api1-cancel`, `c011-prune-files-cancel`, `c011-prune-volume-cancel`, `c011-prune-jobs-cancel`, `c011-prune-jobs-mod-no`, `c011-prune-stats-cancel`, `c011-prune-volume-select-cancel`, `c011-prune-volume-unknown`, `c011-prune-directory-cancel`, `c012-prune-jobs-confirm`, `c030-prune-job`, `c030-prune-storage`, `c030-prune-client`, `c030-prune-pool`, `c030-prune-volume-client`, `c030-prune-stats-client`, `c030-prune-stats-pool`, `c030-prune-directory-pool`, `c030-prune-directory-client-cancel`, `c030-prune-files-pool-cancel`, `c030-prune-jobs-pool-cancel`, `c030-prune-volume-pool-cancel`, `c030-prune-job-api1`, `c030-prune-storage-api2`, `c030-prune-volume-pool-report`, `c030-prune-volume-all-no`, `c030-prune-volume-all-pool-no`, `c030-prune-volume-wrong-pool-no`, `c030-prune-volume-invalid-name`, `c030-prune-files-no`, `c030-prune-files-pool-no`, `c030-prune-directory-client-no-path`, `c030-prune-volume-pool-api1`, `prune-dialog`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:739](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Remove expired file, job, or volume records from the catalog based on configured retention periods.

## Syntax

```
prune [files|jobs|volumes] [client=<name>] [pool=<name>] [volume=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `files` | Prune expired file records |
| `jobs` | Prune expired job records |
| `volumes` | Prune expired volume records |
| `client=<name>` | Restrict pruning to this client |
| `pool=<name>` | Restrict pruning to this pool |
| `volume=<name>` | Prune records for this specific volume |

## Examples

### Prune expired file records for a client
```
* prune files client=bareos-fd
Pruning files for client bareos-fd...
Pruned 1,234 files, 5 jobs from catalog.
```

### Prune expired job records
```
* prune jobs client=bareos-fd
Pruning jobs for client bareos-fd...
Pruned 3 jobs from catalog.
```

### Prune expired volume records
```
* prune volumes pool=Full
Pruning volumes in pool Full...
Pruned 2 volumes from catalog.
```

## Behavior

- Prune respects the retention periods configured in the Client, Pool, and Volume resources:
  - `File Retention`: how long to keep file records
  - `Job Retention`: how long to keep job records
  - `Volume Retention`: how long before a volume can be recycled
- Records older than the retention period are removed from the catalog.
- Pruning does not delete physical data from volumes -- it only cleans up the catalog.
- Volumes whose records have all been pruned are set to `Purged` status, making them eligible for reuse.
- Without arguments, prints available sub-commands.
- This is an audited command.

## Related Commands

- [purge](purge.md) -- Forcefully purge all records (ignores retention)
- [delete](delete.md) -- Delete specific records by name/id
- [list](list.md) -- List records before pruning
