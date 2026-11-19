# openldap: signed ofd plugin for OpenLDAP (slapd)

Consistent backups of a local slapd 2.5/2.6 without any password: the
configuration (slapd.d via `slapcat -n 0`, or slapd.conf with all include
files and `/etc/default/slapd`) and one `slapcat -b <suffix>` LDIF per `mdb`
database. The broker `onesimus-ldap-broker` runs as the user slapd runs as
(never root). Plan: `docs/plans/OPENLDAP-PLUGIN.md`.

```
FileSet {
  Include {
    Plugin = "openldap:instance=main"
    File = /etc/ldap/tls          # TLS keys: a normal File =, the plugin warns if missing
  }
}
```

| Option | Default | |
|---|---|---|
| `instance` | `main` | name in `/@openldap/<instance>/` |
| `databases`, `exclude` | `*`, none | suffixes, `;`-separated, `*` globs, case ignored |
| `config` | `yes` | back up the configuration |
| `config_path` | – | selects the slapd when several run (`-F`/`-f` path) |

A line with a password option is refused. Restore goes to files (`Where`,
no plugin code); `Plugin Options = "openldap:restore_mode=online"` restores
with slapadd into a new target (`target=`, default
`/var/lib/onesimus-ldap-restore/<jobid>`), never into the running server.

Build (no_std, no imports but the broker interfaces):

```sh
CARGO_TARGET_DIR=../../target/plugins cargo +1.99.1 build --release --target wasm32-wasip2
cargo build -p onesimus-ldap-broker
cd ../.. && cargo test -p ofd -- --ignored ldap_e2e::podman_both_configurations --nocapture
```
