onesimus (25.1.2+onesimus.0.2.0-2) UNRELEASED; urgency=medium

  * Set director-parity-anchor-20261015 before per-command parity changes.
  * Define per-command SHA-256 response parity for bconsole and oconsole,
    including deterministic fixtures and explicit reporting of untested cases.
  * oconsole: honor NO_COLOR for output, readline decoration and diagnostics.

  * .users: list actual User resources in source order instead of Console
    resources, with matching text and JSON output; add regression coverage.

  * messages/.messages: drain queued message text verbatim and once only;
    preserve pending API messages until explicitly fetched.

 -- Joerg Bernau <joerg@bernau.family>  Thu, 15 Oct 2026 14:00:00 +0200
