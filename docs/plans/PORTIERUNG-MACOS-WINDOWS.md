# Portierung: macOS (odir, ofd) und Blocker für Windows

Stand: 21.11.2026. Status: **macOS übersetzt** (x86_64-apple-darwin und
aarch64-apple-darwin, `odir` und `ofd` samt Abhängigkeiten);
**Windows nicht begonnen**, nur Blocker gesammelt.

## Bauen ohne Mac

```sh
# zig + cargo-zigbuild (pip install ziglang cargo-zigbuild)
rustup target add x86_64-apple-darwin aarch64-apple-darwin
cargo zigbuild --release --target aarch64-apple-darwin -p odir -p ofd \
    --features bareos-bnet/vendored-openssl
```

Übersetzen gelingt für beide Ziele ohne Fehler. **Linken** braucht das
macOS-SDK (zig bringt nur `libSystem.tbd` mit): ohne SDK bricht der Linker
an `-lobjc` / `-framework CoreFoundation` ab. Gegenprobe mit leeren
Stub-`.tbd`-Dateien und `-undefined dynamic_lookup`: beide Binaries
entstehen als Mach-O; außer sieben CoreFoundation-Symbolen
(`CFTimeZone*`, `CFString*`, `CFRelease` aus `iana-time-zone`) löst sich
jedes Symbol gegen `libSystem` auf. Auf einem Mac (oder mit osxcross-SDK)
linkt es also normal. oconsole linkt schon ohne SDK (c04d948).

Linux-Verhalten bleibt unverändert: jede Änderung ist ein `cfg`-Zweig,
oder ein Cast, der auf Linux nichts tut (`mode_t`, `dev_t` sind dort `u32`
bzw. `u64`).

## Änderungen für macOS

| Crate / Datei | Änderung | Bareos-Bezug |
| --- | --- | --- |
| `bareos-format/src/stream.rs` | `STREAM_XATTR_DARWIN` (1996) | `include/streams.h` |
| `ofd/src/acl_xattr.rs` | Modul `sys`: `l{list,get,set}xattr` werden auf macOS zu `listxattr/getxattr/setxattr(…, XATTR_NOFOLLOW)`, `f{list,get}xattr` mit den zusätzlichen Argumenten. Stream je Plattform (`XATTR_STREAM`): Linux `STREAM_XATTR_LINUX`, macOS `STREAM_XATTR_DARWIN`; Restore nimmt nur den eigenen an. Skip-Listen je Plattform: macOS `com.apple.system.Security` (bei `aclsupport`), immer `com.apple.system.extendedsecurity` und `com.apple.ResourceFork`. | `findlib/xattr.cc`, Darwin-Zweig (`os_default_xattr_streams`, `xattr_acl_skiplist`, `xattr_skiplist`, die `#define llistxattr …`-Makros) |
| `ofd/src/backup.rs` | `O_NOATIME` nur auf Linux, sonst 0 | `filed/backup.cc` (`#ifdef O_NOATIME`) |
| `ofd/src/stmode.rs` (neu) | `S_IF*`/`S_I*` als `u32` (`mode_t` ist auf macOS `u16`); genutzt in `attribs.rs`, `backup.rs`, `create_file.rs`, `find.rs` | – |
| `ofd/src/find.rs` | `stat_from`: `st_dev`/`st_rdev` (`i32` auf macOS) wie `(int64_t)` erweitert; `st_flags` wird auf macOS gesichert | `lib/attribs.cc` `EncodeStat` (`HAVE_CHFLAGS`) |
| `ofd/src/attribs.rs` | Restore auf macOS: `lchmod` für Symlinks, `chflags` nach den Zeiten | `findlib/attribs.cc` `SetAttributes` (`HAVE_LCHMOD`, `HAVE_CHFLAGS`) |
| `ofd/src/job/backup_cmds.rs` | Fordert das FileSet ACLs an und die Plattform kann keine: eine Warnung pro Job | – (Bareos ohne `HAVE_ACL` überspringt still) |
| `ofd/src/broker.rs` | `spawn`/`spawn_userns` nur auf Linux; sonst Fehler „native plugin brokers are not supported on this platform“ beim Start eines Brokers | – |
| `ofd/src/podman_scan.rs`, `podman_port.rs` | `openat2`, `getrandom`, `setresuid`/`prctl` nur auf Linux; sonst Fehler zur Laufzeit; Typ-Casts für `stat` | – |
| `odir/src/job_messages.rs` | Dezimalzeichen: Linux weiter `nl_langinfo_l(RADIXCHAR)`, macOS `localeconv_l()->decimal_point` (gleiche Quelle: LC_NUMERIC der Umgebung) | – |
| `odir/src/commands/restore_tree.rs` | `mode_t`-Breite (Modul `mode` mit `u32`-Konstanten) | – |
| `odir/src/job_worker.rs` | `proc_name` nur im Linux-Zweig (`PR_SET_NAME`) | – |
| `onesimus-daemon/src/privileges.rs`, `onesimus-podman-core/src/sanitize.rs` | schon in c04d948 | – |

## Auf macOS nicht unterstützt (offen)

| Funktion | Verhalten auf macOS | Bareos auf macOS |
| --- | --- | --- |
| **ACLs** (Sicherung und Restore) | Keine ACL-Streams. Bei `aclsupport = yes` eine Job-Warnung „FileSet requests ACL support, but ACLs are not supported on this platform: ACLs are not saved“. ACL-Streams im Restore: „Can't restore ACLs of … - incompatible acl stream encountered - N“ (Warnung, wie Bareos bei fremden Streams). | Erweiterte ACLs über `acl_get_link_np(…, ACL_TYPE_EXTENDED)` / `acl_to_text`, Stream `STREAM_ACL_DARWIN_ACCESS_ACL` (1001), `findlib/acl.cc`. **Offen:** dieselbe Umsetzung (libSystem, kein dlopen nötig). |
| Linux-Xattr-Streams im Restore | Abgewiesen (wie Bareos: nur `os_default_xattr_streams`). | gleich |
| `noatime` | ohne Wirkung | gleich (`O_NOATIME` fehlt) |
| Native Plugin-Broker (PostgreSQL, OpenLDAP, Podman) | Start schlägt mit klarer Meldung fehl; WASM-Plugins ohne Broker laufen. Die Sicherheitsannahmen (`PR_SET_NO_NEW_PRIVS`, `PR_SET_PDEATHSIG`, `fexecve`, Namespaces) gibt es auf macOS nicht. | – |
| Podman-Plugin | Linux-only (`openat2`, User-Namespaces, `/proc`). | – |
| Symlink-Zeiten | wie bisher nicht gesetzt (Bareos setzt sie mit `lutimes` nur, wenn `futimes` fehlt) | – |
| Prozessname der Job-Worker in `odir` | nicht gesetzt (`PR_SET_NAME` ist Linux) | – |

Nicht geprüft (kein Mac): Laufzeit, launchd-Einbindung, Pfade
(`/etc/bareos` → unter macOS üblich `/usr/local/etc/bareos` bzw.
`/opt/bareos/etc`), mDNS über `mdns-sd`.

## Windows: Blocker (x86_64-pc-windows-gnu, gleicher zig-Weg)

Der Bau bricht heute schon in den Abhängigkeiten ab; was in `odir`/`ofd`
dahinter kommt, ist aus dem Quelltext gesammelt (Unix-APIs je Datei).

### Abhängigkeiten / gemeinsame Crates

| Crate / Datei | Blocker |
| --- | --- |
| `daemonize` (extern, über `onesimus-daemon` und `odir`) | durchgehend POSIX (`fork`, `setsid`, `umask`, `getpwnam`, `flock`, `chroot`); auf Windows durch Dienst-Start (Service Control Manager) ersetzen, Abhängigkeit per `cfg(unix)` |
| `bareos-format/src/breg.rs`, `bsr.rs` | POSIX-Regex über `libc::regex_t`/`regcomp`/`regexec`; Windows braucht eine eigene Regex-Engine mit gleicher Semantik (Bareos nutzt auf Windows die mitgelieferte GNU-regex) |
| `onesimus-podman-core/src/journal.rs`, `sanitize.rs`, `idmap.rs`, `policy.rs` | `openat`/`mkdirat`/`renameat`/`unlinkat`, `flock`, `fcntl`, `S_IF*`, `std::os::unix`; Podman ist Linux-only → Crate auf Windows gar nicht bauen (optional machen) |
| `onesimus-daemon/src/privileges.rs` | uid/gid, `initgroups`/`setuid` (Windows: Dienstkonto, kein Rechteabwurf) |
| `onesimus-daemon/src/signals.rs` | SIGHUP/SIGTERM/SIGUSR; Windows: Console-Control-Handler / Dienst-Stopp |
| `onesimus-daemon` (PID-Datei, `-u/-g`) | POSIX-Semantik |
| `onesimus-bnet-grpc/src/tls.rs` | `OpenOptionsExt::mode`, `PermissionsExt` (Schlüsseldatei 0600), `nix::unistd::gethostname` |
| `onesimus-plugins/src/discovery.rs` | `MetadataExt` (Eigentümer-Prüfung uid 0), Quadlet-Pfade |
| `bareos-bnet/src/debug.rs` | Unix-spezifische Teile (prüfen) |

### odir

| Datei | Blocker |
| --- | --- |
| `job_worker.rs` | **`fork()`** pro Job-Worker (+ `guard_reload_for_fork` in `logging.rs`); Windows braucht Worker-Prozesse per `CreateProcess` mit eigener Parameterübergabe oder Threads |
| `main.rs`, `reload.rs`, `accept.rs`, `kill.rs`, `commands/cancel.rs`, `job_manager.rs`, `watchdog.rs`, `runscript.rs` | Signale (SIGHUP-Reload, SIGTERM, SIGCHLD, `kill`), `tokio::signal::unix` |
| `localtime.rs`, `btime.rs`, `run_time.rs`, `job_messages.rs` | `localtime_r`/`strftime`/`newlocale`/`nl_langinfo_l` über libc |
| `runscript.rs`, `mail.rs` | `/bin/sh -c`, Mailer per Shell |
| `config_check.rs`, `director_config.rs`, `config_edit.rs`, `secrets.rs`, `commands/configure.rs` | Unix-Pfade, Dateirechte (`PermissionsExt`), Eigentümer |
| `mdns.rs` | Interface-Abfrage (prüfen, `mdns-sd` selbst ist portabel) |
| `grpc/tls.rs` | Dateirechte der Schlüssel |

### ofd

| Datei | Blocker |
| --- | --- |
| `find.rs`, `attribs.rs`, `create_file.rs`, `backup.rs`, `restore.rs` | Dateibaum über `libc::stat/lstat`, `chown`, `mknod`, `mkfifo`, `S_IF*`; Windows braucht den Bareos-Win32-Weg (`BackupRead`/`BackupWrite`, `STREAM_WIN32_DATA`, `FT_REPARSE`/`FT_JUNCTION`, VSS) – das ist eine eigene Umsetzung, kein Port |
| `acl_xattr.rs` | POSIX-ACL/xattr; Windows: Sicherheitsdeskriptoren stecken in den BackupRead-Daten |
| `broker.rs`, `pg_*`, `ldap_*`, `podman_*` | `fork`/`fexecve`/`prctl`, Unix-Sockets (`UnixStream`); Broker auf Windows nicht vorgesehen |
| `plugin.rs` | Unix-Sockets zum Broker, `MetadataExt` |
| `runscript.rs` | `/bin/sh -c` |
| `config.rs`, `fileset.rs`, `version.rs`, `compression.rs`, `main.rs` | Unix-Pfade, Signale, `daemonize` über `onesimus-daemon` |

Reihenfolge, wenn Windows angegangen wird: `daemonize` und Signale hinter
`cfg(unix)` (Dienst-Rahmen für Windows), Podman-Crates optional machen,
Regex in `breg.rs`, danach `odir` (fork → Prozessstart) und zuletzt der
Win32-Dateizugriff in `ofd`.
