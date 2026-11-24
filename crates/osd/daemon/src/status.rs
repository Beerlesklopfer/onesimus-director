// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Ausgabe von `status` und `.status`.
//!
//! Zweck: Der Director (`status storage`) und Werkzeuge fragen den Zustand
//! des SD ab. osd gibt den Text von bareos-sd 25.1.3 aus (Kopf mit `Sizes`,
//! `Job inventory`, wartende und beendete Jobs, Devices mit Fähigkeiten und
//! Zustand, benutzte Volumes). Nicht belegt, weil osd die Zähler nicht hat:
//! `FDReadSeqNo`, `in_msg`, `out_msg`, `fd` eines laufenden Jobs, die
//! Block- und Positionszeilen eines geöffneten Devices, `LastBytes/sec`
//! (hier wie `AveBytes/sec`), Spooling.
//!
//! Bareos-Gegenstück: `core/src/stored/status.cc` (`OutputStatus`,
//! `ListStatusHeader`, `ListRunningJobs`, `ListDevices`, `DotstatusCmd`).

use crate::daemon::Daemon;
use crate::device::FileDevice;

/// `bstrftime_nc` (`%d-%b-%y %H:%M`) of `secs` since 1970, local time.
fn time_nc(secs: u64) -> String {
    // SAFETY: localtime_r/strftime with valid buffers.
    unsafe {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 64];
        let n = libc::strftime(buf.as_mut_ptr().cast(), buf.len(), c"%d-%b-%y %H:%M".as_ptr(), &tm);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

/// `ListStatusHeader`.
fn header(d: &Daemon) -> Vec<String> {
    vec![
        format!(
            "{} Version: {} ({}) {} \n",
            d.name(),
            daemon_crate::version::VERSION,
            daemon_crate::version::BUILD_DATE,
            daemon_crate::version::os_info()
        ),
        format!(
            "Daemon started {}. Jobs: run={}, running={}, Onesimus binary\n",
            time_nc(d.start_time.into()),
            d.jobs_run.load(std::sync::atomic::Ordering::Acquire),
            d.jobs.all().len()
        ),
        " Sizes: boffset_t=8 size_t=8 int32_t=4 int64_t=8 bwlimit=0kB/s\n".to_string(),
    ]
}

/// `edit_uint64_with_commas`: `12058733` → `12,058,733`.
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `edit_uint64_with_suffix`: `13200000` → `13.20 M`; below 1000 the plain number.
fn with_suffix(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let mut value = n as f64;
    let mut unit = ' ';
    for u in ['K', 'M', 'G', 'T', 'P', 'E'] {
        value /= 1000.0;
        unit = u;
        if value < 1000.0 {
            break;
        }
    }
    format!("{value:.2} {unit}")
}

/// `ListRunningJobs`: the section `Job inventory`.
fn running(d: &Daemon) -> Vec<String> {
    let mut out = vec!["\nJob inventory:\n\n".to_string()];
    let jobs = d.jobs.all();
    for job in &jobs {
        let kind = if job.cmd.job_type == 'B' { "Backup" } else if job.cmd.job_type == 'R' { "Restore" } else { "Job" };
        let status = if job.authenticated.load(std::sync::atomic::Ordering::Acquire) { "Running" } else { "Waiting for Client connection" };
        out.push(format!(
            "JobId={} Level={} Type={kind} Name={} Status={status}\n",
            job.cmd.job_id,
            level_name(job.cmd.level),
            job.cmd.job_name
        ));
        if job.authenticated.load(std::sync::atomic::Ordering::Acquire) {
            let device = d.reserved_device(&job.cmd.job);
            let path = device.as_deref().and_then(|n| d.config.find_device(n)).map(|c| FileDevice::from_config(c).archive_device.display().to_string()).unwrap_or_default();
            let bytes = *job.bytes.lock().unwrap_or_else(|e| e.into_inner());
            let secs = job.created.elapsed().as_secs().max(1);
            let rate = bytes / secs;
            out.push(format!(
                "{}: Volume=\"{}\"\n    pool=\"{}\" device=\"{}\" ({path})\n    spooling=0 despooling=0 despool_wait=0\n    Files={} Bytes={} AveBytes/sec={} LastBytes/sec={}\n\n",
                if job.cmd.job_type == 'B' { "Writing" } else { "Reading" },
                job.volume.lock().unwrap_or_else(|e| e.into_inner()),
                job.storage.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.pool_name.clone()).unwrap_or_default(),
                device.as_deref().unwrap_or("JustInTime Device"),
                commas(job.files.load(std::sync::atomic::Ordering::Acquire).into()),
                commas(bytes),
                commas(rate),
                commas(rate),
            ));
        }
    }
    if jobs.is_empty() {
        out.push("No Jobs running.\n".into());
    }
    out.push("====\n".into());
    out
}

/// `job_level_to_str` for the levels of a job.
fn level_name(level: char) -> &'static str {
    match level {
        'F' => "Full",
        'I' => "Incremental",
        'D' => "Differential",
        'B' => "Base",
        'V' => "Verify",
        _ => "Unknown Job Level",
    }
}

/// The section `Jobs waiting to reserve a drive` (osd has no such queue: a job waits in the Director).
fn waiting() -> Vec<String> {
    vec!["\nJobs waiting to reserve a drive:\n====\n".to_string()]
}

/// The section `Terminated Jobs`.
fn terminated(d: &Daemon) -> Vec<String> {
    let mut out = vec!["\nTerminated Jobs:\n".to_string()];
    let last = d.terminated_jobs();
    if !last.is_empty() {
        out.push(" JobId  Level    Files      Bytes   Status   Finished        Name \n".into());
        out.push("===================================================================\n".into());
        for job in last {
            let status = match job.status {
                'T' | 'W' => "OK",
                'I' => "Incomplete",
                'E' | 'e' => "Error",
                'f' => "Fatal Error",
                'A' => "Canceled",
                'D' => "Diffs",
                _ => "Unknown term code",
            };
            out.push(format!(
                "{:6}  {:<6} {:>8} {:>10}  {:<7}  {} {}\n",
                job.job_id,
                level_name(job.level),
                commas(job.files.into()),
                with_suffix(job.bytes),
                status,
                time_nc(job.end_time),
                job.name
            ));
        }
    }
    out.push("====\n".into());
    out
}

/// `ListDevices` (File devices only), then the Distributors with their routes (Onesimus).
fn devices(d: &Daemon) -> Vec<String> {
    let mut out = vec!["\nDevice status:\n".to_string()];
    for conf in &d.config.devices {
        let dev = FileDevice::from_config(conf);
        let user = d.device_user(&dev.name);
        let job = user.as_deref().and_then(|name| d.jobs.find(name));
        let writing = job.as_ref().is_some_and(|j| j.cmd.job_type == 'B');
        let flag = |on: bool, name: &str| if on { name.to_string() } else { format!("!{name}") };
        match &job {
            Some(job) => out.push(format!(
                "\nDevice {} is mounted with:\n    Volume:      {}\n    Pool:        {}\n    Media type:  {}\n",
                dev.print_name(),
                job.volume.lock().unwrap_or_else(|e| e.into_inner()),
                job.storage.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.pool_name.clone()).unwrap_or_default(),
                conf.media_type
            )),
            None => match &user {
                Some(name) => out.push(format!("\nDevice {} is in use by job {name}.\n", dev.print_name())),
                None => out.push(format!("\nDevice {} is not open.\n", dev.print_name())),
            },
        }
        out.push(format!(
            "Configured device capabilities:\n  EOF BSR BSF FSR FSF EOM {} {} {} {} !ANONVOLS {}\n",
            flag(conf.removable_media, "REM"),
            flag(conf.random_access, "RACCESS"),
            flag(conf.automatic_mount, "AUTOMOUNT"),
            flag(conf.label_media, "LABEL"),
            flag(conf.always_open, "ALWAYSOPEN")
        ));
        let open = job.is_some();
        out.push(format!(
            "Device state:\n  {} {} !MALLOC {} {} {} !WEOT !EOF !NEXTVOL !SHORT {}\n  num_writers={} reserves=0 block=0\n",
            flag(open, "OPENED"),
            flag(open, "LABEL"),
            flag(open && writing, "APPEND"),
            flag(open && !writing, "READ"),
            flag(open && writing, "EOT"),
            flag(open, "MOUNTED"),
            u32::from(open && writing)
        ));
        out.push(format!("Attached Jobs: {}\n", job.as_ref().map(|j| j.cmd.job_id.to_string()).unwrap_or_default()));
        out.push(format!(
            "Device parameters:\n  Archive name: {}\nDevice name: {}\nDevice Type: file\n  File=0 block=0\n  Min block=0 Max block={}\n  Auto Select=yes\n",
            dev.archive_device.display(), dev.name, conf.maximum_block_size
        ));
        out.push("==\n".into());
        out.push(match crate::health::of_device(&dev.archive_device) {
            Ok(h) => format!("    Health: {}\n", h.status_line()),
            Err(e) => format!("    Health: unknown ({e})\n"),
        });
    }
    for dist in &d.config.distributors {
        out.push(format!("\nDistributor \"{}\" (Media Type \"{}\") routes jobs to its devices:\n", dist.name, dist.media_type));
        for (i, route) in dist.routes.iter().enumerate() {
            let state = match d.device_user(&route.device) {
                Some(job) => format!("in use by job {job}"),
                None => "free".to_string(),
            };
            out.push(format!("    Route {}: {} -> Device \"{}\" ({state})\n", i + 1, route.conditions(), route.device));
        }
    }
    out.push("====\n".into());
    out
}

/// The section `Used Volume status`, then the end of the spooling section.
fn volumes(d: &Daemon) -> Vec<String> {
    let mut out = vec!["\nUsed Volume status:\n".to_string()];
    for conf in &d.config.devices {
        let dev = FileDevice::from_config(conf);
        let Some(job) = d.device_user(&dev.name).and_then(|name| d.jobs.find(&name)) else { continue };
        let volume = job.volume.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let writing = job.cmd.job_type == 'B';
        out.push(format!(
            "{volume} on device \"{}\" ({})\n    Reader={} writers={} reserves=0 volinuse=1\n",
            dev.name,
            dev.archive_device.display(),
            u32::from(!writing),
            u32::from(writing)
        ));
    }
    out.push("====\n".into());
    out.push("\n====\n\n".into());
    out
}

/// `OutputStatus` for `status`.
pub fn status_text(d: &Daemon) -> Vec<String> {
    let mut out = header(d);
    out.extend(running(d));
    out.extend(waiting());
    out.extend(terminated(d));
    out.extend(devices(d));
    out.extend(volumes(d));
    out
}

/// `.status <what>`; `None` for an unknown argument.
pub fn dot_status(d: &Daemon, what: &str) -> Option<Vec<String>> {
    Some(match what.to_ascii_lowercase().as_str() {
        "current" => {
            let mut out = vec!["3000 OK .status\n".to_string()];
            for job in d.jobs.all() {
                out.push(format!("JobId={} JobStatus={} JobErrors=0\n", job.cmd.job_id, job.status()));
            }
            out
        }
        "last" => vec!["3000 OK .status\n".to_string()],
        "header" => header(d),
        "running" => running(d),
        "devices" => devices(d),
        "health" => {
            let mut out = vec!["3000 OK .status\n".to_string()];
            for conf in &d.config.devices {
                let dev = FileDevice::from_config(conf);
                out.push(match crate::health::of_device(&dev.archive_device) {
                    Ok(h) => h.dot_line(&dev.name),
                    Err(e) => format!("Device={} Level=unknown Reasons=\"{e}\"\n", dev.name),
                });
            }
            out
        }
        "terminated" => terminated(d),
        "waitreservation" => waiting(),
        "volumes" => volumes(d),
        "spooling" => Vec::new(),
        _ => return None,
    })
}
