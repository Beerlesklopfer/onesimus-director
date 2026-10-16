# .ovfs_tree -- Restore tree of a job chain (OVFS)

Onesimus extension, no Bareos equivalent. Returns the directory tree of a job
or job chain below a directory as JSON, as a restore would bring it back.

## Syntax

```
.ovfs_tree jobid=<list> | client=<name> [fileset=<name>] [root=<dir>] [depth=<n>] [max_entries=<n>] [json]
```

| Argument | Meaning |
|---|---|
| `jobid=1,2,3` (`jobids=`) | the job chain (Full, Differential, Incrementals); order does not matter |
| `client=<name>` | instead of `jobid=`: the chain of the client's latest state, as [.ovfs_latest](dot-ovfs-latest.md) selects it |
| `fileset=<name>` | with `client=`: needed if the client has backups of several FileSets |
| `root=<dir>` (`path=`) | directory in catalog form; a missing trailing `/` is added (default `/`) |
| `depth=<n>` | 0 = root only, 1 = root and its children (default 1) |
| `max_entries=<n>` | most nodes in the result (default 100000; the extension's own default is 1000000) |
| `json` | print the JSON object as one line in every API mode |

## Output

`.api 2` and `json`:

```json
{"path":"ovfs","jobids":[1,5],"root":"/usr","depth":1,
 "tree":{"name":"/usr/","path":"/usr/","type":"directory","children":[
   {"name":"sbin","path":"/usr/sbin/","type":"directory","jobid":5,"fileid":192,"fileindex":12,
    "mode":16877,"uid":0,"gid":0,"size":12382,"mtime":1792085084,"entries":180}]}}
```

`path` names the implementation that answered (`ovfs` = the PostgreSQL
extension, `fallback` = the Director). `tree` is the JSON of `ovfs.tree`
(format in the OVFS wiki, page User-Functions): keys in fixed order,
`children` last; directories at the depth limit carry `entries` instead of
`children`; every node with a catalog row carries its restore identity
(`jobid`, `fileindex`, for delta files `delta_seq`/`delta_parts`, for hard
links `link_fileindex`). `tree` is `null` if nothing live remains below `root`.

Other modes print a readable form, one node per line:

```
* .ovfs_tree jobid=1,5 root=/usr depth=1
Using Catalog "MyCatalog"
jobids: 1,5
/usr/
  sbin/  mtime=2026-10-15 17:24:44 jobid=5 fileindex=12 entries=180
```

Errors are the messages of the extension, e.g. `job 7 does not exist`,
`invalid directory: ""`, `depth must not be negative, got -1`,
`the tree has more than N entries; choose a smaller depth or a deeper root`.

## Extension or fallback

When the Director connects to the catalog it checks whether the PostgreSQL
extension `ovfs` is installed in version 0.1.x with all functions it calls
(signatures, result types, EXECUTE privilege); see
[.ovfs_status](dot-ovfs-status.md). With the extension the command calls
`ovfs.tree`; without it the Director computes the same result with plain SQL
against the Bareos catalog (rows streamed through a server-side cursor in
batches of 10,000, resolved like the extension: newest JobTDate wins per
path/name, FileIndex 0 deletes, hard-link rule of `ua_tree.cc`, BaseFiles and
DeltaSeq). Both paths give byte-identical JSON (equivalence test
`ovfs::tests::equivalence`).

## Access

Command ACL `.ovfs_tree`; Job, Client and FileSet ACL of every job of the
chain and of `client=`/`fileset=`. A denial is audited and answered with
`Unauthorized command from this console.` Unknown JobIds are reported by the
tree call (`job N does not exist`).

## Notes

- Load the top levels with a small `depth` and expand directories with
  [.ovfs_ls](dot-ovfs-ls.md); a full tree of a PC backup is hundreds of MB.
- In `.api 2` the tree is also held as a JSON value; prefer `json` (the text
  is passed on unchanged) for large trees.
- The name is an exact match only; Bareos prefix resolution never reaches it.
- BVFS ([.bvfs_lsdirs](dot-bvfs-lsdirs.md) …) is unchanged and does not use OVFS.

## Related Commands

[.ovfs_status](dot-ovfs-status.md) · [.ovfs_latest](dot-ovfs-latest.md) ·
[.ovfs_ls](dot-ovfs-ls.md) · [.ovfs_versions](dot-ovfs-versions.md)
