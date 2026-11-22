# Portierung: macOS (odir, ofd) und Blocker für Windows

Stand: 21.11.2026. Status: **macOS übersetzt** (x86_64-apple-darwin und
aarch64-apple-darwin, `odir` und `ofd` samt Abhängigkeiten);
**Windows: ofd sichert und stellt Dateien wieder her** (gemessen auf Windows 11, Abschnitt „Windows: Stand von ofd“); odir und der Rest bleiben Blocker.

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
| `daemon-crate/src/privileges.rs`, `onesimus-podman-core/src/sanitize.rs` | schon in c04d948 | – |

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
| `daemonize` (extern, über `daemon-crate` und `odir`) | durchgehend POSIX (`fork`, `setsid`, `umask`, `getpwnam`, `flock`, `chroot`); auf Windows durch Dienst-Start (Service Control Manager) ersetzen, Abhängigkeit per `cfg(unix)` |
| `bareos-format/src/breg.rs`, `bsr.rs` | POSIX-Regex über `libc::regex_t`/`regcomp`/`regexec`; Windows braucht eine eigene Regex-Engine mit gleicher Semantik (Bareos nutzt auf Windows die mitgelieferte GNU-regex) |
| `onesimus-podman-core/src/journal.rs`, `sanitize.rs`, `idmap.rs`, `policy.rs` | `openat`/`mkdirat`/`renameat`/`unlinkat`, `flock`, `fcntl`, `S_IF*`, `std::os::unix`; Podman ist Linux-only → Crate auf Windows gar nicht bauen (optional machen) |
| `daemon-crate/src/privileges.rs` | uid/gid, `initgroups`/`setuid` (Windows: Dienstkonto, kein Rechteabwurf) |
| `daemon-crate/src/signals.rs` | SIGHUP/SIGTERM/SIGUSR; Windows: Console-Control-Handler / Dienst-Stopp |
| `daemon-crate` (PID-Datei, `-u/-g`) | POSIX-Semantik |
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
| `config.rs`, `fileset.rs`, `version.rs`, `compression.rs`, `main.rs` | Unix-Pfade, Signale, `daemonize` über `daemon-crate` |

Reihenfolge, wenn Windows angegangen wird: `daemonize` und Signale hinter
`cfg(unix)` (Dienst-Rahmen für Windows), Podman-Crates optional machen,
Regex in `breg.rs`, danach `odir` (fork → Prozessstart) und zuletzt der
Win32-Dateizugriff in `ofd`.

## Windows: Anforderungen aus dem MSI-Installer der GUI (21.11.2026)

Die GUI-Sitzung hat einen MSI-Installer (WiX 6, GitHub Actions im GUI-Repo, Branch `ci`) gebaut. Für ofd unter Windows folgt daraus:

- **Dienststeuerung:** ofd muss sich beim Service Control Manager anmelden (`StartServiceCtrlDispatcher`/`SetServiceStatus`, z. B. Crate `windows-service`) und auf Stop/Shutdown reagieren; sonst bricht der Dienststart mit Fehler 1053 ab. Bis dahin registriert der Installer den Dienst `ofd` nur mit Startart „manuell“ und startet ihn nicht.
- **Konfigurationsvorlage:** Ersetzung der Platzhalter (`@CLIENT_NAME@`, Passwort) beim ersten Start oder durch den Installer; der Installer legt nur `C:\ProgramData\Onesimus\bareos-fd.d\` an (Rechte: SYSTEM/Administratoren).
- **Release-Artefakt:** der Installer holt `onesimus-dird-<version>-windows-x86_64.zip` (`bin/oconsole.exe`, `bin/ofd.exe`) per URL und SHA-256; das liefert erst die Windows-CI dieses Repos.
- **Offen beim Maintainer:** Dienstname (`ofd`), Ports (9102/10102) und Pfade (`C:\ProgramData\Onesimus\…`) sind Vorschlag; sie stehen im GUI-Repo in `packaging/windows/config.wxi`.

## Windows: Stand von ofd (21.11.2026)

Gemessen auf `joerg.bernau@vm` (Windows 11, `VM-WIN11-DEV`), gebaut mit MSVC 14.44 in `D:\odir-test` (`scripts/windows-vm/ship.sh`), **kein Wine**. Director und Storage Daemon laufen auf der Workstation, der Client ist passiv (`Passive = yes`, der SD verbindet sich zum FD auf Port 19102). Der Test ist `crates/onesimus-rig/tests/windows_fd.rs` (`ONESIMUS_WINDOWS_FD=<Adresse> cargo test -p onesimus-rig --test windows_fd -- --ignored --nocapture`); er richtet den Baum ein (`prep.ps1`), sichert, stellt wieder her und lässt `cmp.ps1` auf der VM alles vergleichen (`bad=0`).

### Was funktioniert (jeweils im Test geprüft)

| Bereich | Ergebnis |
|---|---|
| Netz | `ofd` lauscht auf `[::]` für IPv4 und IPv6 (`bareos_bnet::listen::bind`; vorher nahm Windows kein IPv4 an), TLS-PSK, CRAM-MD5, passiver SD |
| Dateien | leer, Unicode-Name, 20 MB, verschachtelt: SHA-256 gleich |
| Zeiten | Änderungs- und Erstellungszeit auf 100 ns gleich (`attribsEx`), Verzeichniszeiten gleich |
| Attribute | schreibgeschützt, versteckt bleiben erhalten |
| Links | Datei- und Verzeichnis-Symlink, Junction (wieder als Junction, `FSCTL_SET_REPARSE_POINT`), Hardlink (Zahl bleibt 2, FileSet braucht `HardLinks = yes`) |
| Lange Pfade | 347 Zeichen (`\\?\`-Präfix, wo ein Win32-Aufruf es braucht) |
| Sicherheit | Besitzer, DACL (auch ein Deny-Eintrag, geschützte Vererbung) und Alternate Data Streams, auch einer leeren Datei; ebenso die ACL eines Verzeichnisses (`BackupRead`/`BackupWrite`, Strom 31) |
| Gesperrte Datei | `Cannot open "…": ERR=Permission denied.`, Job „OK -- with warnings“, die übrigen Dateien sind gesichert |
| Wiederherstellung unter `where=` | `D:/x/y` landet unter `<where>/D/x/y` (Laufwerksbuchstabe ohne Doppelpunkt) |

### Formate (nicht gegen einen Bareos-Windows-FD geprüft)

- `attribsEx`: zehn Zahlen in Bareos-Base64 (Attribute, Erstellungs-, Zugriffs-, Schreibzeit je niedrig/hoch, Größe hoch/niedrig, Reparse-Tag), `winattr.rs`; aus dem Gedächtnis des Bareos-Quelltextes, ein Bareos-FD könnte ein anderes Format erwarten. Das Lesen akzeptiert auch neun Zahlen.
- Dateimodus `0777` (schreibgeschützt `0555`, Verzeichnisse `0777`), Besitzer und Gruppe 0, `st_ctime` = `ChangeTime`; Bareos setzt dort nach meiner Erinnerung ähnliche Werte, geprüft ist es nicht.
- Der Datenstrom ist das Ergebnis von `BackupRead`; Bareos sendet dasselbe als `STREAM_WIN32_DATA`. Ein Unix-FD kann ihn wiederherstellen (`Win32Decomp` in `restore.rs`, nur die Dateidaten).

### Was fehlt oder abweicht

- **VSS:** umgesetzt und gemessen (22.11.2026, `crates/ofd/src/vss.rs`). Bei `fileset vss=1` (`EnableVSS`, Vorgabe yes; odir sendet es jetzt auch ohne ausdrückliche Angabe) legt ofd je Laufwerk der Include-Liste eine Schattenkopie (`Win32_ShadowCopy`, `ClientAccessible`, per PowerShell des Systemverzeichnisses) an. Namen, Metadaten und Daten werden aus `\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopyN\…` gelesen, der Katalog behält `D:/…`; Jobs auf demselben Laufwerk teilen eine Kopie. Gemessen: eine ohne Freigabe gesperrte Datei wird gesichert und wiederhergestellt. Grenzen: nur dateisystemkonsistent (keine VSS-Writer, keine anwendungskonsistenten Datenbanken), keine Aufräumung verwaister Kopien nach einem ofd-Absturz, Fehler beim Anlegen sind nur eine Warnung. Ein nativer Weg über `vssapi` (IVssBackupComponents) wäre die Alternative mit Writern.
- **OneDrive:** eigenes Thema, voraussichtlich als Plugin (Platzhalter/Cloud-Dateien, Hydrierung); noch nicht entworfen.
- **Plugin-Vertrauensprüfung:** unverändert fail-closed (S-52), Plugins laden auf Windows nicht.
- **Dienstbetrieb:** `winservice.rs` ist nicht ohne Dienst-Installation getestet und wurde nicht installiert.
- **Gemessen am 22.11.2026:** Estimate (voll und inkrementell, vor und nach dem Lauf: 4 Einträge, danach `files=0`), inkrementelle Sicherung (der Job enthält nur `a.txt`, `hard1.txt`, `new.txt`) und die Wiederherstellung der Kette Voll + Inkrementell mit `bad=0` (`windows_fd::incremental_and_estimate_through_the_windows_fd`).
- **Datei über 4 GiB gemessen (22.11.2026):** `huge.bin` mit 4 924 125 241 Byte (Zufallsblöcke bei 4 GiB − 64 KiB, 4 GiB, 4 GiB + 12 345 und am Ende) wird gesichert (FD liest 4,92 GB, gespeichert 5,4 MB mit GZIP, 1:47 min) und wiederhergestellt, Länge und SHA-256 gleich (`windows_fd::a_file_over_four_gigabytes_through_the_windows_fd`).
- **UNC-Pfade (gemessen 22.11.2026):** `File = //localhost/odirunc` (Freigabe auf dem Testhost, per SMB gelesen): Sicherung ohne Fehler, Katalognamen `//localhost/odirunc/…` wie gesendet (Umlaute in UTF-8), kein VSS (kein Laufwerksbuchstabe, daher auch keine Schattenkopie für Netzfreigaben – gesperrte Dateien dort bleiben gesperrt). Restore mit `where=D:/…` legt `…\localhost\odirunc\…` an, Restore mit `where=//localhost/odirunc/back` auf die Freigabe selbst; beide bit-gleich (Test `a_unc_path_through_the_windows_fd`). Nicht gemessen: eine Freigabe eines anderen Rechners mit Anmeldung, zugeordnete Laufwerksbuchstaben (`net use Z:` gilt je Anmeldesitzung; ein FD als Dienst sieht sie nicht, daher UNC-Namen verwenden), Rechte des Dienstkontos auf der Freigabe.
- **Nicht gemessen:** ganze Laufwerke (`C:/`), `fstype`-Filter, `Portable = yes`, Verify auf Windows (der Prüfsummenpfad benutzt den Win32-Strom, damit er mit dem Backup übereinstimmt; Verify gegen einen passiven Client hängt, STUBS S-59), EFS-verschlüsselte und komprimierte NTFS-Dateien, OneDrive-Platzhalter.
- **Rechte:** Der Test lief mit einer erhöhten Administrator-Sitzung (`IsInRole(Administrator)` war wahr); Backup- und Restore-Privilegien werden eingeschaltet (`enable_backup_privileges`). Ohne Administratorrechte nicht gemessen.
