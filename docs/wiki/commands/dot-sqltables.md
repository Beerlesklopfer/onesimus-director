# .sqltables — List Catalog Table Names

Machine-interface command that lists all table names in the catalog database's public schema. Shortcut for `.sql tables`.

## Syntax

```
.sqltables
```

## Arguments

None.

## Examples

### List all tables
```
* .sqltables
basefiles
client
file
fileset
job
jobmedia
log
media
path
pool
storage
version
```

### Empty catalog (no tables)
```
* .sqltables
```

## Notes

- Output is one table name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Internally delegates to `.sql tables`, which runs: `SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = 'public' ORDER BY tablename`.
- Requires a catalog database connection.

## Related Commands

- [.sql](dot-sql.md) -- Execute SQL with sub-commands
- [.sqlschema](dot-sqlschema.md) -- Show full catalog schema
- [.sqlcount](dot-sqlcount.md) -- Count records in job table
- [.catalogs](dot-catalogs.md) -- List catalog names
