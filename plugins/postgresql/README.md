# postgresql: signed ofd plugin for PostgreSQL

Logical backups (`pg_dump -Fc` per database, `pg_dumpall --globals-only`) of a
local PostgreSQL instance, without any password: the broker
`onesimus-pg-broker` runs as the instance's own user and connects over the
local socket (peer). Plan: `docs/plans/POSTGRESQL-PLUGIN.md`, `POSTGRESQL-PHASE2.md`.

```
FileSet { Include { Plugin = "postgresql:instance=main:exclude=test_*" } }
```

| Option | Default | |
|---|---|---|
| `instance` | `main` | name in `/@postgresql/<instance>/` |
| `socket`, `port` | `/var/run/postgresql`, `5432` | the instance |
| `databases`, `exclude` | `*`, none | comma-separated, `*` globs |
| `globals` | `yes` | roles and tablespaces |

ofd decides what the guest may see: never templates, databases without
connections, or the Bareos catalog `bareos`. Restore goes to files
(`Where`); restore the dumps with `pg_restore`.

Build (no_std, no imports but the broker interface):

```sh
CARGO_TARGET_DIR=../../target/plugins cargo build --release --target wasm32-wasip2
cargo build -p onesimus-pg-broker
cd ../.. && cargo test -p ofd -- --ignored postgresql_backs_up
```
