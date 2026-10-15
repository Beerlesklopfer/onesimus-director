# .catalog -- Catalog Schema Management

Manages the catalog database schema: inspect tables and columns, generate DDL and Rust code, run migrations, validate the schema against the live database, and perform destructive admin operations (create, drop, reset).

## Syntax

```
.catalog <sub-command> [arguments]
```

## Sub-Commands

| Sub-command | Description |
|-------------|-------------|
| `schema` | Display the full schema definition as JSON. |
| `version` | Show all schema versions and the current version number. |
| `tables` | List all tables with column/index counts and descriptions. |
| `describe table=<name>` | Show columns, types, indices, and foreign keys for a table. |
| `ddl` | Generate the complete CREATE TABLE SQL for the current schema. |
| `validate` | Validate the schema definition against the live database. |
| `migrate` | Run pending schema migrations on the catalog database. |
| `diff` | Show changes between the two most recent schema versions. |
| `codegen` | Generate Rust structs from the schema definition. |
| `create` | Create all tables in the database (requires CatalogAdmin ACL). |
| `drop table=<name>` | Drop a specific table with CASCADE (requires CatalogAdmin ACL). |
| `reset` | Drop all tables and recreate them (requires CatalogAdmin ACL). |

## Examples

### Inspect the schema
```
* .catalog tables
Tabellen in Schema v001:

Tabelle              Spalten    Indizes    Beschreibung
----------------------------------------------------------------------
job                  12         3          Backup/Restore jobs
client               3          1          Client definitions
media                15         4          Volume/media information
...
```

### Describe a specific table
```
* .catalog describe table=job
Tabelle: job (PK: jobid)
Backup/Restore jobs

Spalte               Typ             NOT NULL   Default         Beschreibung
--------------------------------------------------------------------------------
jobid                SERIAL          YES        -               Primary key
name                 TEXT            YES        -               Job name
...
```

### Generate DDL
```
* .catalog ddl
CREATE TABLE job (
  jobid SERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  ...
);
CREATE TABLE client (
  ...
);
```

## Notes

- The schema definitions are loaded from `catalog/schema/` (YAML files with versioned schemas).
- Sub-commands without arguments (called as just `.catalog`) display the help text with all available sub-commands.
- **Destructive operations** (`create`, `drop`, `reset`, `migrate`) require CatalogAdmin ACL and a live database connection.
- `validate` compares the schema definition against the actual database state and reports drift.
- `codegen` produces Rust struct definitions suitable for the `bareos-orm` crate.
- `diff` requires at least two schema versions to compare.
- `drop` uses CASCADE, which removes dependent foreign key constraints.
- `reset` is equivalent to dropping all tables and then running `create`.

## Related Commands

- [.catalogs](dot-catalogs.md) -- List configured catalog names
- [.sql](dot-sql.md) -- Execute raw SQL queries
- [.sqltables](dot-sqltables.md) -- List catalog table names via SQL
- [.sqlschema](dot-sqlschema.md) -- Show catalog schema via SQL
- [use](use.md) -- Switch the active catalog
