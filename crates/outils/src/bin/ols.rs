// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `ols`: zeigt, was auf einem Datei-Volume steht (Ersatz für `bls`).
//!
//! ```text
//! ols [-j] [-l] [-k] [-i <JobId>] <Volume-Datei>...
//! ```
//!
//! * ohne Option: Volume-Label und die Jobs (SOS/EOS-Labels) des Volumes
//! * `-j`: nur Label und Jobs, keine Dateien
//! * `-l`: jede Datei mit Typ, Größe und Zeit (Attribute des Katalogs)
//! * `-i <JobId>`: nur die Dateien dieses Jobs
//! * `-k`: Blockprüfsummen nicht prüfen

use std::path::PathBuf;
use std::process::ExitCode;

use bareos_format::label::{SessionLabel, VolumeLabel, EOS_LABEL, PRE_LABEL, SOS_LABEL, VOL_LABEL};
use bareos_format::lstat::decode_stat;
use bareos_format::stream::{STREAMMASK_TYPE, STREAM_UNIX_ATTRIBUTES, STREAM_UNIX_ATTRIBUTES_EX};
use outils::{parse_attributes, VolumeReader};

/// Aufrufoptionen.
struct Options {
    /// Nur Label und Jobs.
    jobs_only: bool,
    /// Lange Zeile je Datei.
    long: bool,
    /// Prüfsummen überspringen.
    skip_checksums: bool,
    /// Nur diese JobId.
    job: Option<u32>,
    /// Die Volume-Dateien.
    volumes: Vec<PathBuf>,
}

const USAGE: &str = "usage: ols [-j] [-l] [-k] [-i <JobId>] <volume-file>...\n\
  -j  only the volume label and the jobs\n  -l  long listing of the files\n  -k  do not check block checksums\n  -i  only the files of this JobId";

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options { jobs_only: false, long: false, skip_checksums: false, job: None, volumes: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-j" => o.jobs_only = true,
            "-l" => o.long = true,
            "-k" => o.skip_checksums = true,
            "-i" => o.job = Some(it.next().and_then(|v| v.parse().ok()).ok_or("-i needs a JobId")?),
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option {s}")),
            s => o.volumes.push(PathBuf::from(s)),
        }
    }
    if o.volumes.is_empty() {
        return Err("no volume file given".into());
    }
    Ok(o)
}

/// Mikrosekunden seit 1970 als `YYYY-MM-DD HH:MM:SS` (UTC).
fn time_text(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Eine Zeile je Job.
fn print_session(start: &SessionLabel, end: Option<&SessionLabel>) {
    let status = end.and_then(|e| e.job_status).and_then(char::from_u32).unwrap_or('?');
    println!(
        "Job: JobId={} Job={} Client={} FileSet={} Level={} Pool={} Written={}",
        start.job_id, start.job, start.client_name, start.fileset_name,
        char::from_u32(start.job_level).unwrap_or('?'), start.pool_name, time_text(start.write_btime / 1_000_000)
    );
    match end {
        Some(e) => println!("  End: Files={} Bytes={} Blocks={}-{} Errors={} Status={status}",
            e.job_files.unwrap_or(0), e.job_bytes.unwrap_or(0), e.start_block.unwrap_or(0), e.end_block.unwrap_or(0), e.job_errors.unwrap_or(0)),
        None => println!("  End: no EOS label (the job did not finish on this volume)"),
    }
}

/// Typbuchstabe und Größe aus den LStat-Feldern.
fn kind_of(mode: u32) -> char {
    match mode & 0o170_000 {
        0o040_000 => 'd',
        0o120_000 => 'l',
        0o020_000 => 'c',
        0o060_000 => 'b',
        0o010_000 => 'p',
        0o140_000 => 's',
        _ => '-',
    }
}

/// Listet ein Volume; Fehler als Text.
fn list(path: &PathBuf, o: &Options) -> Result<(), String> {
    let mut reader = VolumeReader::open(path, !o.skip_checksums)?;
    println!("Volume file: {}", path.display());
    let mut sessions: Vec<(u32, u32, SessionLabel, Option<SessionLabel>)> = Vec::new();
    let (mut blocks, mut records, mut files) = (0u64, 0u64, 0u64);
    let mut in_job: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    let mut printed_files: Vec<String> = Vec::new();
    let mut handle = |rec: &bareos_format::volume::VolRecord, sessions: &mut Vec<(u32, u32, SessionLabel, Option<SessionLabel>)>| -> Result<(), String> {
        records += 1;
        match rec.file_index {
            VOL_LABEL | PRE_LABEL => {
                let l = VolumeLabel::parse(&rec.data)?;
                println!("Volume Label: Id={} VerNum={} VolName={} Pool={} MediaType={} Host={} Prog={} {}\n  Labeled={} Written={}",
                    l.id.trim_end(), l.ver_num, l.volume_name, l.pool_name, l.media_type, l.host_name, l.label_prog, l.prog_version,
                    time_text(l.label_btime / 1_000_000), time_text(l.write_btime / 1_000_000));
            }
            SOS_LABEL => {
                let s = SessionLabel::parse(&rec.data, false)?;
                in_job.insert((rec.vol_session_id, rec.vol_session_time), s.job_id);
                sessions.push((rec.vol_session_id, rec.vol_session_time, s, None));
            }
            EOS_LABEL => {
                let e = SessionLabel::parse(&rec.data, true)?;
                if let Some(s) = sessions.iter_mut().rev().find(|s| s.0 == rec.vol_session_id && s.1 == rec.vol_session_time) {
                    s.3 = Some(e);
                }
            }
            fi if fi > 0 && !o.jobs_only => {
                let masked = rec.stream & STREAMMASK_TYPE;
                if masked == STREAM_UNIX_ATTRIBUTES || masked == STREAM_UNIX_ATTRIBUTES_EX {
                    let job = in_job.get(&(rec.vol_session_id, rec.vol_session_time)).copied();
                    if o.job.is_some() && job != o.job {
                        return Ok(());
                    }
                    if let Some((name, lstat, link)) = parse_attributes(&rec.data) {
                        files += 1;
                        if o.long {
                            let (st, _, _) = decode_stat(&lstat);
                            let arrow = if link.is_empty() { String::new() } else { format!(" -> {link}") };
                            printed_files.push(format!("  {}{:04o} {:>12} {} {name}{arrow}", kind_of(st.mode), st.mode & 0o7777, st.size, time_text(st.mtime)));
                        } else {
                            printed_files.push(format!("  {name}"));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    };
    while let Some((_, _, recs)) = reader.next_block()? {
        blocks += 1;
        for rec in &recs {
            handle(rec, &mut sessions)?;
        }
    }
    for (rec, missing) in reader.take_pending() {
        handle(&rec, &mut sessions)?;
        println!("The last record continues on the next volume ({missing} bytes missing).");
    }
    for (_, _, start, end) in &sessions {
        if o.job.is_none_or(|j| j == start.job_id) {
            print_session(start, end.as_ref());
        }
    }
    for line in &printed_files {
        println!("{line}");
    }
    println!("{blocks} blocks, {records} records, {files} files listed");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("ols: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(if e.is_empty() { 0 } else { 2 });
        }
    };
    let mut failed = false;
    for volume in &options.volumes {
        if let Err(e) = list(volume, &options) {
            eprintln!("ols: {e}");
            failed = true;
        }
    }
    ExitCode::from(u8::from(failed))
}
