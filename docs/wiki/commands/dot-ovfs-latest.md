# .ovfs_latest -- Job chain of a client's latest state (OVFS)

Onesimus extension, no Bareos equivalent (`ovfs.jobids_latest`).

## Syntax

```
.ovfs_latest client=<name> [fileset=<name>] [json]
```

## Behaviour

Selects the chain as Bareos `restore` option 5 ("most recent backup for a
client"): the last Full of the FileSet (by name), the last Differential after
it and all Incrementals after Full or Differential; only backups (`Type B`)
with status `T`/`W` started before now; Differentials and Incrementals only
if they are on an enabled volume. If the Full is on no enabled volume the
chain lacks it (as in Bareos) and the command adds the warning
`Full backup job N of client "…" is on no enabled volume; the chain [...] lacks it and a restore would be incomplete`.

Without `fileset=` the client must have backups of exactly one FileSet,
otherwise: `client "…" has backups of several FileSets [...]; choose one`.

## Output

Text: the JobIds, comma separated (ordered by JobTDate). `.api 2`/`json`:
`{"path":"ovfs","client":"bareos-fd","jobids":[600,615,616,620,622,624,626,1,5]}`.

Fix the chain once per restore session and pass it as `jobid=` to
[.ovfs_tree](dot-ovfs-tree.md) and [.ovfs_ls](dot-ovfs-ls.md), so a backup
finishing in between does not change the tree while browsing.

Access: Command ACL, Client/FileSet ACL of the arguments, Job/Client/FileSet
ACL of every job of the chain (audited).
