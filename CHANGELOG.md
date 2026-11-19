onesimus (25.1.2+onesimus.0.2.0-2) UNRELEASED; urgency=medium

  * odir: the TLS-PSK connection to a File or Storage daemon no longer
    checks a certificate the daemon presents (system CAs, host name
    "bareos"); the C++ Director does not without "TLS Verify Peer = yes".
    A File daemon with its own TLS certificate failed every job with
    "certificate verify failed".
  * odir: a RunScript the kernel cannot execute (ENOEXEC, e.g. "#!" not
    on line 1) runs via /bin/sh as with glibc execvp() in bareos-dir.

  * odir: jobs left created or running by an earlier run end as fatal at
    start, as CheckCatalog(UPDATE_AND_FIX) does (J-3): C -> f with
    Start/EndTime = SchedTime, R -> f with EndTime = StartTime. Only at
    start, not on reload. Before, every restart (also the package
    upgrade) left such jobs "running" for good.

  * odir: catalog at start and while running (J-5, T-13). The start checks
    every catalog like CheckCatalog() and ends with exit 1 when one does
    not open (before: it ran on without a catalog or hung forever). The
    catalog worker and the start migration connect with the same
    parameters as -t (DbPassword, DbPort and socket were dropped before).
    The worker's pool connects lazily: an outage fails requests with an
    error and the next request after it reconnects.
  * odir unit as bareos-director (T-7): RestartSec=30, at most 10 restarts
    a day, no restart on exit 41/42; a configuration error at start now
    ends with 42 like bareos-dir. Reload runs "odir -t" first.

  * odir: the start migration finds the installed schema (T-11). The
    package installs the schema JSON to /usr/lib/bareos/scripts/ddl/, the
    Director looked in /usr/lib/bareos/ddl/schema, so the Onesimus
    extension (v2251 views and functions, v2252) never reached an
    installed catalog. After the upgrade the first start applies it; the
    Bareos Version table stays at 2250.

  * odir: time and size directives are read with every word of the value
    and the Bareos units (K-1). Max Run Time, Full/Incremental/Differential
    Max Run Time, Max Wait Time, Max Start Delay, Duplicate Job Proximity
    and Reschedule Interval took only the first word ("2 hours" = 2 s);
    Spool Size likewise ("10 GB" = 10 bytes); Migration High/Low Bytes
    used k = 1000 instead of Bareos' k = 1024, kb = 1000.

  * odir: listen addresses as Bareos 25. DirAddresses (block form) binds
    every entry instead of stopping the Director at start; -t reports an
    unresolvable address. Address/Addresses are aliases of the Director
    resource. Mixing DirAddresses with DirAddress or DirPort is an error,
    an entry without port takes 9101. IPv6 sockets are IPv6-only.
    As bareos-dir: "ip" entries are IPv4, addr/port must be unquoted
    (addr = "::1" is refused), one addr and one port per entry, and an
    address that cannot be bound is retried three times, 5 s apart, then
    the start fails (no listening on a subset of the configured addresses).
    BEHAVIOUR CHANGE: without any address the Director now listens on
    0.0.0.0 AND :: (BNET on DirPort, gRPC on DirPort+1000), as Bareos does.
    Hosts with a routed IPv6 address need IPv6 firewall rules for 9101 and
    10101, or a DirAddress/DirAddresses entry, before the upgrade.

  * Set director-parity-anchor-20261015 before per-command parity changes.
  * Define per-command SHA-256 response parity for bconsole and oconsole,
    including deterministic fixtures and explicit reporting of untested cases.
  * oconsole: honor NO_COLOR for output, readline decoration and diagnostics.

  * .users: list actual User resources in source order instead of Console
    resources, with matching text and JSON output; add regression coverage.

  * messages/.messages: drain queued message text verbatim and once only;
    preserve pending API messages until explicitly fetched.

  * setdevice: validate required arguments and booleans, send the real SD
    command and return the daemon response; reject invalid requests before I/O.

  * command dispatch: resolve case-insensitive prefixes in original Bareos
    table order, preserving aliases and session-changing abbreviated commands.

  * MCP query: allow explicitly known read-only dot-command names only;
    reject mutating extensions, BVFS cache writes and abbreviated commands.

  * help/.help: remove terminal styling from Director protocol replies so
    console decoration cannot alter command-response hashes.

  * run: keep incremental/differential backups with a valid reference chain,
    scoped to client/job/FileSet; upgrade missing Full bases before pool and
    catalog selection, propagate lookup failures, and honor FileSet/pool overrides.

  * MCP: compile mutually exclusive production (default, read-only) and
    development policies. Compile the execute handler and write CLI option
    only for development; retain runtime opt-in and force Debian production builds.

  * Add the full AGPL-3.0-only license to Director and proto repositories,
    make Cargo/SPDX and Debian copyright metadata explicit, and update the
    maintainer contact to Joerg.Bernau@onesimus.io.

  * trace: implement synchronized append-only trace files for daemon and
    protocol diagnostics; trace off closes the sink, failures preserve the
    previous destination, and files are private and reject symlinks.

  * setdebug: implement Director/FD/SD/all targets, trace and timestamp flags,
    daemon endpoint deduplication, FD protocol-version handling and live log
    filter reload; control replies no longer wait for an absent EOD.

  * setbandwidth: send actual client/storage limits using Bareos byte-rate
    control packets, handle unlimited mode, reject overflow and unsupported
    job selectors before I/O, and require a successful daemon acknowledgement.

  * resolve: resolve each hostname locally or on the explicitly selected FD/SD;
    preserve Bareos IPv4/IPv6 output and multi-frame daemon replies, reuse
    authenticated connections, and report unknown targets or resolution failures.

  * Serialize diagnostic-filter reloads with worker forks so children cannot
    inherit an exclusive reload lock from another Director thread.

  * wait: implement job/jobname/ujobid selection, silent wait-all, mount-state
    waits and mount timeouts; report actual catalog outcomes in text and JSON,
    including warning/error/cancel exit statuses and visible catalog failures.
    Forward numeric SD mount/media states, preserve buffered pipe messages and
    FD termination codes, and process final catalog updates before completion.
    Add deterministic coverage for selection, polling and failure paths.
  * Prioritize the open subcommands and execution paths of the 21 partially
    implemented commands before other command groups.

  * MCP: mirror stdio and HTTP/SSE requests and responses to stderr immediately,
    including handshake, notifications, tool discovery, rejected calls and
    errors. Label accesses with timestamps and IDs; escape terminal controls,
    preserve protocol bytes and keep HTTP Authorization headers out of output.
    Add transport-level regression coverage for both compiled MCP policies.

  * oconsole: enforce a shared three-second terminal/stdin command rate guard,
    including yes/no, selection, text replies and terminal cancellation.
    Discard fast or multiline input before dispatch and report violations only
    on stderr; rejected input is neither queued nor added to command history.
  * Show a green/red admission state in the input prompt, restore green after
    cooldown without losing edited text, and preserve NO_COLOR behavior.
    Pin rustyline to the upstream buffered-input fix needed for timed redraws.
    Add deterministic boundary/spam tests and real-binary BNET/stdin/PTY tests.

  * wait: keep canceled jobs active until the worker exits, preserve final
    counters and Bareos status priorities, and expose failed catalog completion
    to selected-job and wait-all callers. Close the completion-snapshot race,
    reject failed cancellation signals and retain outcomes from actual finish.
    Verify with real held child processes and injected catalog failures.

  * help/.help: follow the original command table, selection rules, byte wrapping,
    text/API/JSON layout and all/full/item variants. Update oconsole completion
    to consume original .help metadata instead of treating .help all as a table.
    Cross-review syntax and NULL-description handling; retain ACL and dynamic
    configure usage as explicit parity gaps.

  * .defaults: port Client retentions/autoprune, Pool fields, device lists,
    Pool/Job storage precedence and Restore defaults from effective JobDefs.
    Preserve source field order/types and purge_oldest_volume= spelling.
    Fix exponent/underflow, integer bounds and selector-order counterexamples
    found by independent review; add ten focused regression tests.

  * Document repeated independent command reviews and the resulting roadmap
    in docs/TOD-COMMANDS.md. Separate known gaps, unverified registrations and
    demonstrated parity, and record shared ACL/fixture dependencies and gates.

 -- Joerg Bernau <Joerg.Bernau@onesimus.io>  Thu, 15 Oct 2026 14:00:00 +0200
