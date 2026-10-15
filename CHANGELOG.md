onesimus (25.1.2+onesimus.0.2.0-2) UNRELEASED; urgency=medium

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

 -- Joerg Bernau <Joerg.Bernau@onesimus.io>  Thu, 15 Oct 2026 14:00:00 +0200
