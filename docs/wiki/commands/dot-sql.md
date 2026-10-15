# .sql — Execute SQL with Sub-Commands

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L266) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Send an arbitrary SQL command

**Originalsyntax:**

```text
.sql query=<sqlquery>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/sql.rs:9](../../../crates/onesimus-dird/src/commands/sql.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command to execute raw SQL queries against the catalog database or retrieve schema information.

## Syntax

```
.sql query="<SQL statement>"
.sql tables
.sql schema
```

## Arguments

| Argument | Description |
|----------|-------------|
| `query="<SQL>"` | Execute an arbitrary SQL statement |
| `tables` | List all public table names (shortcut for `.sqltables`) |
| `schema` | Show full schema: table_name, column_name, data_type (shortcut for `.sqlschema`) |

## Examples

### Execute a query
```
* .sql query="SELECT jobid, name, jobstatus FROM job ORDER BY jobid DESC LIMIT 3"
10	backup-bareos-fd	T
9	backup-bareos-fd	T
8	RestoreFiles	T
```

### List tables
```
* .sql tables
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

### Show schema
```
* .sql schema
basefiles	baseid	bigint
basefiles	jobid	integer
basefiles	fileid	bigint
client	clientid	integer
client	name	text
client	uname	text
...
```

## Notes

- Query results are returned as tab-separated values, one row per line.
- Output is terminated by the EOD signal.
- Requires a catalog database connection. Returns an error if no catalog is configured.
- The SQL is executed directly against PostgreSQL; use with caution.
- For read-only exploration, prefer `.sql tables` and `.sql schema` over raw queries.

## Related Commands

- [.sqltables](dot-sqltables.md) -- List catalog table names
- [.sqlschema](dot-sqlschema.md) -- Show full catalog schema
- [.sqlcount](dot-sqlcount.md) -- Count records in job table
- [sqlquery](sqlquery.md) -- Interactive SQL query command
- [.catalogs](dot-catalogs.md) -- List catalog names
