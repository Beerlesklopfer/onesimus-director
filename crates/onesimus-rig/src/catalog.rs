//! Wegwerf-Katalog: ein eigener PostgreSQL-Cluster je Gerüst.
//!
//! Zweck: Der Director braucht einen Bareos-Katalog. Statt den Systemdienst
//! anzufassen, legt das Gerüst mit `initdb` einen eigenen Cluster im
//! Gerüstverzeichnis an. Er hört nur per TCP auf `127.0.0.2` und einem hohen
//! Port; Unix-Sockets sind abgeschaltet, weil Socket-Pfade unter `/tmp/…`
//! die Grenze von 107 Zeichen leicht überschreiten. odir verbindet sich bei
//! `127.0.0.1` oder `localhost` über den Socket in `/var/run/postgresql`
//! (`check_catalog::uses_socket`), deshalb `127.0.0.2`.
//!
//! Datenfluss: [`Cluster::create`] → `initdb` (Vertrauen für den eigenen
//! Benutzer, `SQL_ASCII`, Locale C) → `pg_ctl start` → `CREATE DATABASE` →
//! Schema aus `postgresql.sql`. [`Cluster::query`] liest per `psql`.
//! `Drop` stoppt den Cluster (`pg_ctl stop -m immediate`).
//!
//! Bareos-Gegenstück: `create_bareos_database` und `make_bareos_tables`
//! (`core/src/cats/`), die dasselbe Schema in eine bestehende Instanz laden.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Address the throwaway cluster listens on (not 127.0.0.1, see module doc).
pub const LISTEN: &str = "127.0.0.2";

/// Name of the catalog database inside the cluster.
pub const DATABASE: &str = "bareos";

/// A running PostgreSQL cluster owned by the rig; stopped on drop.
#[derive(Debug)]
pub struct Cluster {
    /// Data directory of the cluster.
    data: PathBuf,
    /// TCP port on [`LISTEN`].
    pub port: u16,
    /// Database user (the current login user, the cluster superuser).
    pub user: String,
    /// `pg_ctl` used to stop the cluster.
    pg_ctl: PathBuf,
    /// `psql` used for queries.
    psql: PathBuf,
    /// Whether the cluster runs and must be stopped.
    running: bool,
}

impl Cluster {
    /// Creates and starts a cluster in `dir`, then loads the schema `ddl`.
    pub fn create(dir: &Path, port: u16, programs: &crate::prereq::Programs) -> Result<Cluster, String> {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
        let data = dir.join("pg");
        let log = dir.join("pg.log");
        let mut cluster = Cluster {
            data: data.clone(),
            port,
            user: user.clone(),
            pg_ctl: programs.pg_ctl.clone(),
            psql: programs.psql.clone(),
            running: false,
        };
        run(Command::new(&programs.initdb)
            .arg("-D")
            .arg(&data)
            .args(["-A", "trust", "-U", &user, "-E", "SQL_ASCII", "--locale=C", "--no-sync"]))?;
        let options = format!(
            "-c listen_addresses={LISTEN} -p {port} -c unix_socket_directories='' -c fsync=off \
             -c synchronous_commit=off -c full_page_writes=off"
        );
        run(Command::new(&programs.pg_ctl)
            .arg("-D")
            .arg(&data)
            .arg("-l")
            .arg(&log)
            .args(["-o", &options, "-w", "-t", "60", "start"]))
        .map_err(|e| format!("{e}\n{}", std::fs::read_to_string(&log).unwrap_or_default()))?;
        cluster.running = true;
        cluster.sql("postgres", &format!("CREATE DATABASE {DATABASE} TEMPLATE template0"))?;
        run(cluster.psql_command(DATABASE).arg("-q").arg("-f").arg(&programs.ddl))?;
        Ok(cluster)
    }

    /// `psql` against `database` of this cluster, stopping at the first error.
    fn psql_command(&self, database: &str) -> Command {
        let mut command = Command::new(&self.psql);
        command
            .args(["-h", LISTEN, "-p", &self.port.to_string(), "-U", &self.user, "-d", database])
            .args(["-v", "ON_ERROR_STOP=1", "-X"])
            .env("PGCONNECT_TIMEOUT", "10");
        command
    }

    /// Runs `sql` in `database`; the output in unaligned form (`psql -At`).
    pub fn sql(&self, database: &str, sql: &str) -> Result<String, String> {
        run(self.psql_command(database).args(["-At", "-F", "|", "-c", sql]))
    }

    /// Runs `sql` in the catalog database ([`DATABASE`]).
    pub fn query(&self, sql: &str) -> Result<String, String> {
        self.sql(DATABASE, sql)
    }

    /// Stops the cluster immediately (no checkpoint: it is thrown away).
    pub fn stop(&mut self) {
        if self.running {
            self.running = false;
            let _ = Command::new(&self.pg_ctl)
                .arg("-D")
                .arg(&self.data)
                .args(["-m", "immediate", "-w", "stop"])
                .output();
        }
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Runs `command`; `Ok(stdout)` on exit 0, else `Err` with stderr.
fn run(command: &mut Command) -> Result<String, String> {
    let output = command.output().map_err(|e| format!("{command:?}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{command:?} endet mit {}:\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
