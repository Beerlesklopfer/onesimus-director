# .ovfs_versions -- All versions of one file (OVFS)

Onesimus extension, no Bareos equivalent (`ovfs.versions`; the counterpart of
[.bvfs_versions](dot-bvfs-versions.md) without path ids).

## Syntax

```
.ovfs_versions client=<name> path=<dir> name=<file> [copies] [json]
```

`name=` empty or missing lists the versions of the directory itself;
`fname=`/`filename=` are accepted as well. `copies` includes copy jobs (`C`).

## Output

Newest first (JobTDate, then FileId). Only versions on a volume (FileIndex in
a JobMedia range), of jobs of type `B`, `A`, `a` (and `C` with `copies`).

Text: `jobid level type starttime size mtime fileindex volumes inchanger md5`
separated by tabs. `.api 2`/`json`: `{"path":…,"client":…,"dir":…,"name":…,
"versions":[{"jobid","level","job_type","jobtdate","starttime","fileid",
"fileindex","delta_seq","md5","mode","uid","gid","size","mtime","volumes",
"inchanger"}]}`.

Access: Command ACL and Client ACL (audited, as `.bvfs_versions`); versions
of jobs outside the console's Job ACL are left out.
