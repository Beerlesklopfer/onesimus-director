# .sql — Execute SQL with Sub-Commands

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L266) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Send an arbitrary SQL command

**Originalsyntax:**

```text
.sql query=<sqlquery>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI.

**Paritätsfälle (30):** `c002-sql-then-list`, `c016-sql-count`, `c016-sql-multicol-null`, `c016-sql-empty`, `c016-sql-syntax-error`, `c016-sql-unknown-table`, `c016-sql-no-query`, `c016-sql-no-value`, `c016-sql-semicolons`, `c016-sql-trailing-semicolon`, `c016-sql-api1`, `c016-sql-api1-empty`, `c016-sql-api1-error`, `c016-sql-api2`, `c016-sql-api2-empty`, `c016-sql-api2-error`, `c016-sql-api2-noquery`, `c016-sql-error-truncated`, `c016-sql-error-end-of-input`, `c016-sql-error-runtime`, `c016-sql-error-hint`, `c016-sql-jobid-bytes`, `c016-sql-api1-semicolons`, `c016-sql-api2-error-hint`, `c020-compact-sql`, `c020-json-sql`, `c027-gui-sql`, `c028-sql`, `c028-sql-bad`, `dot-sql`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/sql.rs:27](../../../crates/onesimus-dird/src/commands/sql.rs).

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
