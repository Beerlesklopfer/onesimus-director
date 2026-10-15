# .sqlcount — Count Records in Job Table

Machine-interface command that returns the number of records in the job table. Provides a quick overview of catalog activity.

## Syntax

```
.sqlcount
```

## Arguments

None.

## Examples

### Count job records
```
* .sqlcount
Jobs: 42
```

### Empty catalog
```
* .sqlcount
Jobs: 0
```

### No catalog configured
```
* .sqlcount
No catalog
```

## Notes

- Returns a single line: `Jobs: <count>`.
- Output is terminated by the EOD signal.
- Internally runs `SELECT COUNT(*) FROM job` against the catalog database.
- Requires a catalog database connection.

## Related Commands

- [.sql](dot-sql.md) -- Execute arbitrary SQL
- [.sqltables](dot-sqltables.md) -- List catalog table names
- [list](list.md) -- List jobs from catalog
