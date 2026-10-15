# .sqlschema — Show Full Catalog Schema

Machine-interface command that shows the complete schema of the catalog database: table name, column name, and data type for every column. Shortcut for `.sql schema`.

## Syntax

```
.sqlschema
```

## Arguments

None.

## Examples

### Show schema
```
* .sqlschema
basefiles	baseid	bigint
basefiles	jobid	integer
basefiles	fileid	bigint
client	clientid	integer
client	name	text
client	uname	text
file	fileid	bigint
file	fileindex	integer
file	jobid	integer
file	pathid	integer
file	markid	integer
file	lstat	text
file	md5	text
...
```

### No catalog connected
```
* .sqlschema
No catalog configured. Query: SELECT table_name, column_name, data_type FROM information_schema.columns WHERE table_schema = 'public' ORDER BY table_name, ordinal_position
```

## Notes

- Output is tab-separated: `table_name\tcolumn_name\tdata_type`, one column per line.
- Output is terminated by the EOD signal.
- Internally delegates to `.sql schema`, which queries `information_schema.columns`.
- Results are ordered by table name, then ordinal position (column order in CREATE TABLE).
- Requires a catalog database connection.

## Related Commands

- [.sql](dot-sql.md) -- Execute SQL with sub-commands
- [.sqltables](dot-sqltables.md) -- List catalog table names
- [.sqlcount](dot-sqlcount.md) -- Count records in job table
- [.catalog](dot-catalog.md) -- Catalog schema management (describe, ddl, validate)
