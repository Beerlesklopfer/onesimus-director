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
| `create [yes confirm=<db>]` | Create all tables in the database (guarded, see below). |
| `drop table=<name> [yes confirm=<db>] [force]` | Drop one schema table with CASCADE (guarded, see below). |
| `reset [yes confirm=<db>] [force]` | Drop all schema tables and recreate them (guarded, see below). |

## Guard for `create`, `drop` and `reset`

These sub-commands are a local Onesimus extension without Bareos equivalent
(compatibility log C-040). `drop` and `reset` run `DROP TABLE ... CASCADE` on
the catalog, so they are guarded in this order:

1. **ACL**: the console's `CommandACL` (or one of its profiles) must name
   `.catalog` literally. `*all*` and regular expressions are **not**
   sufficient, and a `!` entry that matches first still denies. The default
   console (`*UserAgent*`, director password) has no ACL in Bareos and passes
   this step, but not the next ones.
2. **Confirmation**: without both `yes` and `confirm=<db>`, where `<db>` is the
   name of the database the Director is connected to
   (`SELECT current_database()`), the command only reports the database, the
   job record count, the tables it would drop and the number of CREATE
   statements, and changes nothing. This holds in every API mode, including
   `.api 2` (JSON), which never implies a confirmation.
3. **Job records**: if `drop`/`reset` would drop tables of a catalog that
   contains job records, or whose `job` table cannot be read, `force` is also
   required. `create` only uses `CREATE ... IF NOT EXISTS`/`CREATE OR REPLACE`
   and does not need `force`.

`drop` accepts only tables of the loaded schema definition. Every attempt
(denied, preview, refused, executed) and every executed statement is logged
with the prefix `AUDIT .catalog`.

These commands are not available through the oconsole MCP `query` tool.

Note: following Bareos `Do_a_command` prefix matching, a console line starting
with `.catalog` currently resolves to the Bareos command `.catalogs` first, so
the guarded handler is not reachable from a console session; the guard applies
to every path that does reach it.

```
* .catalog reset
Katalog-Datenbank: bareos_scratch
Job-Records: 0
Zu loeschende Tabellen (CASCADE): ..., bareos_schema_version
CREATE-Anweisungen: 42
Nichts geaendert. Ausfuehren mit: .catalog reset yes confirm=bareos_scratch
```

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
- **Destructive operations** (`create`, `drop`, `reset`) are guarded as described above and need a live database connection. `migrate` currently only reads the schema version.
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
