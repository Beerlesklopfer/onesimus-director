# .ovfs_status -- OVFS extension and restore-tree path

Onesimus extension, no Bareos equivalent.

## Syntax

```
.ovfs_status [refresh] [json]
```

## Behaviour

Shows what the Director found out about the PostgreSQL extension `ovfs` in
its catalog database and which implementation the `.ovfs_*` commands use:

```
* .ovfs_status
OVFS extension: version 0.1.0 available (schema ovfs)
Restore trees: ovfs extension
```

```
OVFS extension: not installed
Restore trees: Director fallback (plain SQL)
```

`.api 2`/`json`: `{"ovfs":{"state":"available","installed":true,"usable":true,
"version":"0.1.0","schema":"ovfs","path":"ovfs"}}`; `state` is one of
`unknown`, `check_failed`, `not_installed`, `unusable` (with `reason`),
`available`.

## Detection

The check runs on every new catalog connection (startup and reconnects, via
the connection pool), after `reload`/SIGHUP, with `refresh`, and when an
extension call fails. The result is logged when it changes, e.g.
`catalog: OVFS extension version 0.1.0 available (schema ovfs) (connect); restore trees use the ovfs path`.

The extension is used only if all of these hold, otherwise the reason is
shown and the Director fallback answers (same results):

- `pg_extension` has `ovfs` with `extversion` 0.1.x (for 0.x a minor bump may
  change the API; the version was not bumped for every API change so far,
  so the next point is the real guard),
- the catalog role has USAGE on the extension schema,
- these functions exist with these result types (`pg_get_function_result`)
  and are executable: `ovfs.tree(integer,text,integer,bigint)` and
  `ovfs.tree(integer[],text,integer,bigint)` → `json`,
  `ovfs.tree_latest(text,text,integer,text,bigint)` → `json`,
  `ovfs.jobids_latest(text,text)` → `integer[]`,
  `ovfs.ls(integer[],text)` → `TABLE(...)` with the 17 columns of `ls`,
  `ovfs.versions(text,text,text,boolean)` → `TABLE(...)` with the 16 columns
  of `versions`.

Access: Command ACL `.ovfs_status`.
