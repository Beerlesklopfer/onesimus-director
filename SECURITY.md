# Security policy

This repository holds the Onesimus backup stack in Rust: the Director `odir`,
the file daemon `ofd`, the storage daemon `osd`, the console `oconsole` (with
its MCP server for AI assistants), the signed plugin system and the Debian
packages. They speak the Bareos protocols, keep the catalog and run with
access to the data they back up, so we take reports about their security
seriously and handle them as described here. This policy also serves the
vulnerability handling required by the EU Cyber Resilience Act
(Regulation (EU) 2024/2847).

## Supported versions

| Version | Security fixes |
|---|---|
| `development` branch (before v1.0.0) | yes, newest state only |
| 0.x releases and packages | no, update to the newest state |
| from v1.0.0 | **2 years** of security fixes from the release date of each version (the support period is stated with each release) |

## Reporting a vulnerability

Please report a vulnerability **privately**, never in a public issue,
discussion or pull request:

- on GitHub: **Security → Report a vulnerability** in this repository
  (private vulnerability reporting), or
- by e-mail to **contact@onesimus.io** with the subject `SECURITY`.

Please include the affected component and version or commit, the platform,
the configuration that matters (console ACLs, plugins, TLS), the steps to
reproduce and what an attacker gains. A proof of concept helps; please do not
test against systems that are not yours, and never against a catalog or
backup volume that holds data you cannot lose.

## What happens next

1. We confirm receipt within **7 days**.
2. We assess the report and tell you whether we consider it a vulnerability
   and how severe it is.
3. We fix it in the supported versions and agree a publication date with you.
   We aim to publish a fix within 90 days.
4. We publish a GitHub Security Advisory (with a CVE where it applies) and
   credit you, unless you prefer not to be named.

Actively exploited vulnerabilities and severe incidents are reported to the
authorities as Article 14 of the Cyber Resilience Act requires.

## Scope

In scope, for example:

- authentication and encryption of the console, daemon and gRPC connections
  (TLS-PSK, CRAM-MD5, PAM, the gRPC login) and their downgrade or bypass;
- console ACLs, profiles and the limits of assistant consoles (`*-mcp`):
  anything that lets a console do what its ACLs forbid, change ACLs, run SQL
  it may not run, or store a program in the configuration;
- the plugin system: signature and certificate-chain checks, the trust
  policy, the broker and its sandbox, anything that runs unsigned or
  unauthorised code;
- memory safety and parsing of network input, SQL built from input, path
  handling during backup and restore, secrets in logs, messages or the
  catalog;
- the Debian packages: maintainer scripts, systemd units, file permissions.

Out of scope: Bareos itself (report to the Bareos project), third-party
libraries (report upstream; tell us if Onesimus is affected), findings that
need an already compromised machine, root access or Director, and functions
that are documented as not supported yet (`docs/STUBS.md`), unless they fail
in an unsafe way.

The graphical client has its own repository and the same policy; you can
report issues for both there.
