//! `oscan`: Katalogeinträge aus Datei-Volumes wiederherstellen (Ersatz für
//! `bscan`).
//!
//! ```text
//! oscan [-s] [-m] [-p <Pool>] [-x] [-c <Konfigurationsverzeichnis>] [-C <Catalog>] [-k] <Volume-Datei>...
//! ```
//!
//! * ohne `-x` wird das SQL-Skript ausgegeben, es ändert nichts
//! * `-x`: das Skript mit `psql` in einer Transaktion ausführen; Verbindung
//!   aus der Catalog-Ressource in `-c` (Voreinstellung `/etc/bareos/bareos-dir.d`)
//! * `-s`: auch die File-Einträge anlegen (mit Prüfsumme)
//! * `-m`: die Größe eines vorhandenen Volumes im Media-Eintrag nachführen
//! * `-p`: Pool, wenn nicht der im Label
//! * `-k`: Blockprüfsummen nicht prüfen

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use bareos_config::{parser, Resource};
use outils::scan::{Options, Scan};
use outils::VolumeReader;

const USAGE: &str = "usage: oscan [-s] [-m] [-p <pool>] [-x] [-c <config dir>] [-C <catalog>] [-k] <volume-file>...";

fn usage(error: &str) -> ExitCode {
    if !error.is_empty() {
        eprintln!("oscan: {error}");
    }
    eprintln!("{USAGE}");
    ExitCode::from(if error.is_empty() { 0 } else { 2 })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut options, mut execute, mut verify) = (Options::default(), false, true);
    let (mut config, mut catalog) = (PathBuf::from("/etc/bareos/bareos-dir.d"), None::<String>);
    let mut volumes = Vec::<PathBuf>::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-s" => options.files = true,
            "-m" => options.update_media = true,
            "-x" => execute = true,
            "-k" => verify = false,
            "-p" => match it.next() {
                Some(p) => options.pool = Some(p.clone()),
                None => return usage("-p needs a pool"),
            },
            "-c" => match it.next() {
                Some(p) => config = PathBuf::from(p),
                None => return usage("-c needs a directory"),
            },
            "-C" => match it.next() {
                Some(c) => catalog = Some(c.clone()),
                None => return usage("-C needs a catalog name"),
            },
            "-h" | "--help" => return usage(""),
            s if s.starts_with('-') && s.len() > 1 => return usage(&format!("unknown option {s}")),
            s => volumes.push(PathBuf::from(s)),
        }
    }
    if volumes.is_empty() {
        return usage("no volume file given");
    }
    let mut scan = Scan::new(options);
    let mut failed = false;
    for path in &volumes {
        let mut reader = match VolumeReader::open(path, verify) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("oscan: {e}");
                failed = true;
                continue;
            }
        };
        scan.begin_volume(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
        loop {
            match reader.next_block() {
                Ok(Some((offset, header, records))) => {
                    scan.block(offset, header.block_len);
                    records.iter().for_each(|r| scan.record(r));
                }
                Ok(None) => break,
                Err(e) => {
                    eprintln!("oscan: {}: {e}", path.display());
                    failed = true;
                    break;
                }
            }
        }
        for (rec, _) in reader.take_pending() {
            scan.record(&rec);
        }
        scan.end_volume(&path.display().to_string());
    }
    for note in &scan.notes {
        eprintln!("oscan: {note}");
    }
    let jobs = scan.jobs_found;
    let sql = scan.finish();
    if failed {
        eprintln!("oscan: a volume could not be read; nothing is applied");
        return ExitCode::from(1);
    }
    if !execute {
        print!("{sql}");
        return ExitCode::SUCCESS;
    }
    let resources = match parser::parse_config_dir(&config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("oscan: configuration {}: {e}", config.display());
            return ExitCode::from(1);
        }
    };
    let catalogs: Vec<_> = resources.into_iter().filter_map(|r| if let Resource::Catalog(c) = r { Some(c) } else { None }).collect();
    let Some(cat) = catalogs.iter().find(|c| catalog.as_ref().is_none_or(|n| c.name.eq_ignore_ascii_case(n))) else {
        eprintln!("oscan: no Catalog resource{} in {}", catalog.map(|n| format!(" \"{n}\"")).unwrap_or_default(), config.display());
        return ExitCode::from(1);
    };
    let mut command = Command::new("psql");
    command.args(["-X", "-q", "-v", "ON_ERROR_STOP=1", "--single-transaction", "-f", "-"])
        .env("PGDATABASE", &cat.db_name).env("PGUSER", &cat.db_user).env("PGPASSWORD", &cat.db_password)
        .stdin(Stdio::piped());
    if !cat.db_address.is_empty() {
        command.env("PGHOST", &cat.db_address);
    }
    if cat.db_port != 0 {
        command.env("PGPORT", cat.db_port.to_string());
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oscan: cannot run psql (package postgresql-client): {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(sql.as_bytes());
    }
    match child.wait() {
        Ok(status) if status.success() => {
            println!("oscan: {jobs} job(s) from {} volume(s) applied to catalog {}", volumes.len(), cat.name);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("oscan: the catalog refused the script; nothing was changed");
            ExitCode::from(1)
        }
    }
}
