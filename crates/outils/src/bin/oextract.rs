// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `oextract`: Dateien aus Datei-Volumes zurückschreiben, ohne Director,
//! Storage Daemon und Katalog (Ersatz für `bextract`).
//!
//! ```text
//! oextract [-i <JobId>] [-p <Pfad-Präfix>]... [-n] [-O] [-k] <Zielverzeichnis> <Volume-Datei>...
//! ```
//!
//! * `-i`: nur die Dateien dieses Jobs
//! * `-p`: nur Dateien, deren Pfad mit dem Präfix beginnt (mehrfach möglich)
//! * `-n`: nichts schreiben, die Dateien nur zählen
//! * `-O`: Eigentümer und Gruppe setzen (braucht Root)
//! * `-k`: Blockprüfsummen nicht prüfen
//!
//! Die Volumes werden in der angegebenen Reihenfolge gelesen; ein Job, der
//! sich über mehrere Volumes erstreckt, wird mit allen seinen Volumes in
//! Reihenfolge wiederhergestellt.

use std::path::PathBuf;
use std::process::ExitCode;

use outils::extract::{Extractor, Options};
use outils::VolumeReader;

const USAGE: &str = "usage: oextract [-i <JobId>] [-p <path-prefix>]... [-n] [-O] [-k] <destination> <volume-file>...";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut options, mut verify, mut positional) = (Options::default(), true, Vec::<String>::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-i" => match it.next().and_then(|v| v.parse().ok()) {
                Some(j) => options.job = Some(j),
                None => return usage("-i needs a JobId"),
            },
            "-p" => match it.next() {
                Some(p) => options.prefixes.push(p.clone()),
                None => return usage("-p needs a path prefix"),
            },
            "-n" => options.dry_run = true,
            "-O" => options.owner = true,
            "-k" => verify = false,
            "-h" | "--help" => return usage(""),
            s if s.starts_with('-') && s.len() > 1 => return usage(&format!("unknown option {s}")),
            s => positional.push(s.to_string()),
        }
    }
    if positional.len() < 2 {
        return usage("a destination and at least one volume file are needed");
    }
    options.dest = PathBuf::from(positional.remove(0));
    let mut extractor = Extractor::new(options);
    let mut failed = false;
    for volume in &positional {
        let path = PathBuf::from(volume);
        let mut reader = match VolumeReader::open(&path, verify) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("oextract: {e}");
                failed = true;
                continue;
            }
        };
        loop {
            match reader.next_block() {
                Ok(Some((_, _, records))) => records.iter().for_each(|r| extractor.record(r)),
                Ok(None) => break,
                Err(e) => {
                    eprintln!("oextract: {}: {e}", path.display());
                    failed = true;
                    break;
                }
            }
        }
        for (rec, _) in reader.take_pending() {
            extractor.record(&rec);
        }
    }
    extractor.finish();
    for m in &extractor.messages {
        eprintln!("oextract: {m}");
    }
    let c = &extractor.counts;
    println!("{} files, {} directories, {} links, {} bytes{}; {} skipped, {} errors",
        c.files, c.directories, c.links, c.bytes, if extractor.counts.files == 0 && c.directories == 0 { " (nothing found)" } else { "" }, c.skipped, c.errors);
    ExitCode::from(u8::from(failed || c.errors > 0))
}

fn usage(error: &str) -> ExitCode {
    if !error.is_empty() {
        eprintln!("oextract: {error}");
    }
    eprintln!("{USAGE}");
    ExitCode::from(if error.is_empty() { 0 } else { 2 })
}
