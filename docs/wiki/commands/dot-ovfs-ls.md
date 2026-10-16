# .ovfs_ls -- List one directory of a job chain (OVFS)

Onesimus extension, no Bareos equivalent. The direct children of one
directory, for loading a restore tree on demand (`ovfs.ls`).

## Syntax

```
.ovfs_ls jobid=<list> | client=<name> [fileset=<name>] path=<dir> [json]
```

Arguments as for [.ovfs_tree](dot-ovfs-tree.md); `path=` (or `dir=`) is the
directory in catalog form.

## Output

`.api 2` and `json`:

```json
{"path":"ovfs","jobids":[600,615,1,5],"dir":"/usr/sbin/","entries":[
  {"name":"agetty","kind":"file","jobid":1,"fileid":91,"fileindex":91,"mode":33261,"uid":0,"gid":0,
   "size":110152,"mtime":1785512076}]}
```

Each entry has the columns of `ovfs.ls` (name, name_b64, path, path_b64,
kind, jobid, fileid, fileindex, delta_seq, delta_parts, mode, uid, gid, size,
mtime, link_fileindex, entries); NULL columns are left out. Order: files by
name bytes, then directories by name + `/`. Directories carry `path` (the
argument for the next `.ovfs_ls`) and `entries` (live entries below; expand
arrow if > 0). Directories with nothing live below are left out.

Text mode: one line per entry (kind, size, mtime, name, restore identity,
entries).

## Notes

Memory is the children of `path` (both on the extension and the fallback
path); everything below is streamed and only counted. Access, extension or
fallback: see [.ovfs_tree](dot-ovfs-tree.md).
