# Onesimus — Director (onesimus-dird) & Console (oconsole)

Drop-in replacement for the Bareos Director daemon — independent project after Bareos rejected the Rust rewrite.

**Status:** Full, Incremental, and Differential Backup + Restore working end-to-end with C++ FD/SD.

## What works

- **Full / Incremental / Differential Backup**: Dir→SD→FD, CatReq proxy, JobMedia in DB
- **Restore**: bootstrap generation from catalog, round-trip verified
- **TLS-PSK** authentication (TLSv1.2, compatible with C++ bconsole/FD/SD)
- **CRAM-MD5** two-way authentication
- **Console commands** via C++ `bconsole` or `oconsole`: version, status, restore, run, cancel, list, ...
- **Job Worker FSM**: 17 states, fork() per job, pipe-based IPC to Director
- **SD CatReq proxy**: FindMedia, GetVolInfo, UpdateMedia, CreateJobMedia
- **Config parser**: Bareos `bareos-dir.d/*/*.conf` directory format
- **PostgreSQL catalog**: schema migration, stored procedures, views
- **MPSC Catalog Worker**: single DB pool (flume channel), 8k sessions → 8 connections
- **Scheduler**, **Watchdog**, **Message Bus**
- **DB Job Lifecycle**: CreateJob (DB-generated JobId) → UpdateJobEnd (status, files, bytes)

## Architecture

- **Named threads** (`dir:accept`, `dir:catalog`, `dir:scheduler`, `dir:watchdog`, `dir:grpc`, `dir:monitor`)
- **fork() per job** for process isolation
- **flume MPSC** for catalog queries, crossbeam channels for command queue
- **BNET + gRPC** dual protocol (native Rust, no FFI)
- **TLS-PSK** via `openssl` crate (TLSv1.2 — see `tls_psk.rs` for TLSv1.3 limitation docs)
- **Stored Procedures**: `sp_create_job_record`, `sp_update_job_end`, `sp_create_jobmedia`
- **Views**: `vw_job_summary`, `vw_volume_status`, `vw_file_list`, `vw_last_backup_per_client`

## Crates

| Crate | Type | Description |
|-------|------|-------------|
| `onesimus-dird` | Product | Director daemon binary |
| `oconsole` | Product | `oconsole` — interactive console client |
| `bareos-bnet` | Protocol | BNET wire protocol, TLS-PSK, CRAM-MD5 |
| `bareos-config` | Protocol | Bareos configuration parser |
| `bareos-catalog` | Protocol | PostgreSQL catalog queries (sqlx) |
| `bareos-orm` | Protocol | ORM + schema migration + codegen |
| `bareos-stored` | Protocol | SD protocol library (connect, job, catrec, status) |
| `bareos-proto-rs` | Protocol | gRPC codegen from proto definitions |
| `bareos-filed` | Protocol | File Daemon skeleton |
| `bareos-plugins` | Protocol | Python plugin system (PyO3) |

## Build & Run

```bash
cargo build
cargo test

# Run with test config (requires PostgreSQL + bareos-fd + bareos-sd)
target/debug/onesimus-dird -c tests/ -f --port 29101 --name bareos-dir

# Connect with C++ bconsole
echo "version" | bconsole -c tests/bconsole.conf

# Or with oconsole
target/debug/oconsole -c tests/bconsole.conf
```

## gRPC & Auto-Discovery

Besides BNET (port 9101, compatible with C++ `bconsole`/WebUI) the Director
offers gRPC (`proto/director_service.proto`) for the Onesimus GUI:

- **TLS only.** Without `Grpc Tls Certificate`/`Grpc Tls Key` a self-signed
  certificate is generated once in the working directory
  (`onesimus-grpc.crt`/`.key`); clients pin its SHA-256 fingerprint on first use.
- **Login** per challenge/response (`GetChallenge` → `Login`) with Console
  name + password (`*UserAgent*` = Director password); the password is never
  sent. Every further call carries the session token in `x-onesimus-session`.
- **mDNS/DNS-SD:** announced as `_onesimus-dir._tcp` (instance = Director name,
  port = BNET port, TXT `grpc`, `tls`, `fp`, `version`, `product`).
  Check with `avahi-browse -rt _onesimus-dir._tcp`.

```
Director {
  ...
  Enable Grpc = yes                 # gRPC endpoint (TLS)
  Grpc Port = 10101                 # default: Port + 1000
  # Grpc Tls Certificate = /etc/bareos/tls/dir.crt   # default: self-signed
  # Grpc Tls Key = /etc/bareos/tls/dir.key
  Mdns Advertise = yes              # default: yes
  Working Directory = /var/lib/bareos
}
```

## Drop-in Replacement

Both packages can be installed side-by-side, only one runs at a time:

| | Binary | Package |
|--|--------|---------|
| C++ | `/usr/sbin/bareos-dir` | `bareos-director` |
| Onesimus | `/usr/sbin/onesimus-dird` | `onesimus-dird` |

## Build Debian packages

```bash
dpkg-buildpackage -us -uc -b
```

## License

AGPL-3.0
