# Onesimus — Director (onesimus-dird) & Console (oconsole)

Drop-in replacement for the Bareos Director daemon — independent project after Bareos rejected the Rust rewrite.

**Status:** Full, Incremental, and Differential Backup + Restore working end-to-end with C++ FD/SD. All 88 original console commands registered; per-command SHA-256 output parity vs. `bconsole` is the active acceptance gate.

## What works

- **Full / Incremental / Differential Backup**: Dir→SD→FD, CatReq proxy, JobMedia in DB
- **Restore**: bootstrap generation from catalog, round-trip verified
- **TLS-PSK** authentication (TLSv1.2, compatible with C++ bconsole/FD/SD)
- **CRAM-MD5** two-way authentication
- **Console commands** via C++ `bconsole` or `oconsole`: version, status, restore, run, cancel, list, ...
- **Director command parity**: `wait`, `setdebug`, `setbandwidth`, `resolve`, `trace`, `.messages`, `.users`, `setdevice`, scoped `run` — each brought up to `bconsole` parity with regression tests and per-command wiki/JSON status
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

Set `NO_COLOR=1` to disable oconsole styling, including prompts, hints and diagnostics.

## Director command reference

- [Wiki: original Bareos command set and completeness audit](docs/wiki/commands/bareos-reference.md)
- [LLM command reference (JSON)](docs/wiki/commands/bareos-director-commands.json)
- [Command pages](docs/wiki/commands/README.md)

## Parity testing

Parity against the original Bareos `bconsole` is the acceptance gate, and it is
measured, not assumed.

- **Coverage.** A concrete test case (parameters, preconditions, expected side
  effects) for each of the 88 original console commands and the 26 Restore-Shell
  commands. Registering a command name is not parity.
- **Two levels, reported separately.** (1) `bconsole` vs. `oconsole` against the
  *same* Director and *same* reproducible state; (2) the Bareos Director vs. the
  Onesimus Director with identical configuration and catalog snapshot.
- **Byte-exact SHA-256.** The command response is captured as unchanged bytes and
  hashed. For every case `bconsole` and `oconsole` must produce the *same*
  SHA-256. No trimming, sorting, JSON re-serialization, or error removal — line
  endings, whitespace, field order, and encoding count.
- **Deterministic environment.** Identical locale, timezone, API mode, and
  terminal parameters; `NO_COLOR=1` for `oconsole`. Time, IDs, hostnames, and
  asynchronous messages are fixed by the test environment or fixtures; genuinely
  non-deterministic cases stay marked open, not normalized away.
- **Disposable state.** State-changing commands run only in throwaway instances
  with the same initial state restored before each client run.
- **Per-case result JSON.** Command, parameters, context, versions, fixtures,
  return codes, SHA-256 of both responses, byte counts, diff, and status. Errors,
  timeouts, and not-run cases never count as passed; raw files and diffs are
  retained for analysis.

**Tooling.** `tests/compare_directors.py` (wrapper `tests/compare_directors.sh`)
runs identical commands against C++ `bareos-dir` and Rust `onesimus-dird`,
compares outputs, measures response times, and emits JSON + PDF reports.

**Anchoring.** Work is anchored to `director-parity-anchor-20261015` (`fb5242f`),
taken *before* the functional command changes; each main command ships as its own
commit with tests, wiki/JSON status, and a `CHANGELOG.md` entry. Open sub-functions
stay explicitly audited until both the source comparison and the SHA-256 matrix
prove parity (see `docs/BACKLOG.md`).

## oconsole: terminal command rate guard

Terminal/stdin commands and dialog replies share a fixed **three-second**
minimum interval per running console, for both BNET and gRPC. The first input
is allowed immediately. Faster inputs are discarded before sending and are
not queued or added to command history. A `Command Rate Guard violation`
message and remaining wait time are written only to stderr.

The input prompt is **green** when ready and **red** during the cooldown.
It switches back to green automatically, preserving partially edited input.
`NO_COLOR=1` keeps the prompt plain; the guard remains active. Empty top-level
lines do not consume the interval. Multiline pasted commands are rejected.
All three Director dialog types use the same timer and re-prompt after rejection.
Terminal cancellation commands are guarded too; EOF simply closes the connection.

The guard applies to terminal/stdin command entry. MCP tool calls, single-command
`-e` mode and internal completion queries retain their existing behavior.
For timed prompt updates, rustyline is pinned to the
[upstream input-buffer fix](https://github.com/kkawakam/rustyline/commit/f2bbcc5cdf7b99dfe7d97af03976b111bc6fd256).

## oconsole: gRPC and MCP

The MCP server uses the official Rust SDK `rmcp` (requires Rust 1.88 or newer).

```bash
# gRPC instead of BNET (asks once to trust the Director certificate,
# stored in ~/.config/oconsole/known_directors)
oconsole --grpc -a dir.example.org -n admin -w secret

# MCP server on stdio for AI assistants (read-only: tool "query")
oconsole --mcp
# Development build only: also allow state-changing commands (tool "execute")
# Build first with --no-default-features --features mcp-development
oconsole --mcp --mcp-allow-write
# MCP over gRPC needs a known certificate (connect once interactively or pass it)
oconsole --mcp --grpc --grpc-fingerprint <sha256>
```

Every MCP request and response is mirrored to **stderr** automatically, independent
of `RUST_LOG`: initialize, notifications, tool discovery, tool calls and errors.
An admin running oconsole in a terminal sees access immediately. Each `MCP` line
contains a timestamp, transport, access ID, direction and the complete content.
HTTP/SSE data is mirrored as it streams; frames from concurrent accesses are
labelled separately. HTTP authentication failures are also visible, while
Authorization headers and the server's bearer token are omitted.
Control characters are escaped for terminal display; payloads are not truncated.
When running as a service, follow stderr in the service journal. In stdio mode,
keep stderr separate from stdout, which carries the MCP protocol.

MCP permissions are selected at compile time:

```bash
# Production: read-only MCP, no execute tool or --mcp-allow-write option
cargo build --release -p oconsole
# Development: write-capable MCP; runtime --mcp-allow-write is still required
cargo build -p oconsole --no-default-features --features mcp-development
```

`mcp-production` is the default feature. The two policies are mutually exclusive;
building both or neither fails. Debian packages explicitly select production.
This policy applies to both MCP transports; interactive and single-command
Director access keep their existing behavior. Use separate target directories
when retaining development and production binaries side by side.

For a separately running MCP server that clients can register by URL:

```bash
oconsole --mcp --mcp-http 127.0.0.1:9104 -c /etc/bareos/oconsole.conf
# Read the bearer token locally to enter it in the client's authentication settings:
cat ~/.config/oconsole/mcp.token
```

The endpoint is `http://127.0.0.1:9104/mcp` (Streamable HTTP). Every request
requires `Authorization: Bearer <token>`. The token is generated from 32 OS-random
bytes on first start and reused after restarts. It is stored with permissions
`0600` in `$XDG_CONFIG_HOME/oconsole/mcp.token` (default
`~/.config/oconsole/mcp.token`). Override with `--mcp-token-file /path/to/token`.
The file belongs to the OS user running oconsole. To rotate the token, stop
oconsole, remove the file, restart, and update clients. The token is never logged.
All clients using this token share the configured Director console and its ACLs.

For remote clients, expose `/mcp` through an HTTPS reverse proxy and add its public
hostname with `--mcp-host mcp.example.org`. The proxy must forward the
`Authorization` header. Use the public HTTPS URL in the client. stdio mode does
not require this token. The Director connection still uses BNET or `--grpc`.

Register with Claude Code, for example:

```bash
claude mcp add onesimus -- oconsole --mcp -c /etc/bareos/oconsole.conf
```

The read-only `query` tool accepts `status`, `list`, `llist`, `show`,
`version`, `help`, `time`, `messages`, `estimate` and dot-commands (except
`.sql` and BVFS restore/cleanup). Multi-line input and session commands
(`quit`, `.api`, `@…`) are rejected.

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

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`); see [LICENSE](LICENSE).

Maintainer: Joerg Bernau <Joerg.Bernau@onesimus.io>.
