//! Ausgabe von `status` und `.status`.
//!
//! Zweck: Der Director (`status storage`) und Werkzeuge fragen den Zustand
//! des SD ab. osd gibt Kopf (Version, Start, Jobzahlen), laufende Jobs und
//! die Devices aus, im Wortlaut von bareos-sd, aber kürzer (keine Spool-
//! und Volume-Statistik).
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
    ]
}

/// `ListRunningJobs`.
fn running(d: &Daemon) -> Vec<String> {
    let mut out = vec!["\nRunning Jobs:\n".to_string()];
    let jobs = d.jobs.all();
    for job in &jobs {
        if job.authenticated.load(std::sync::atomic::Ordering::Acquire) {
            let device = d.reserved_device(&job.cmd.job).unwrap_or_else(|| "JustInTime Device".into());
            out.push(format!(
                "Writing: {} Backup job {} JobId={} Volume=\"\"\n    pool=\"{}\" device=\"{device}\"\n",
                level_name(job.cmd.level),
                job.cmd.job_name,
                job.cmd.job_id,
                job.storage.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.pool_name.clone()).unwrap_or_default()
            ));
        } else {
            out.push(format!("{} Job {} waiting for Client connection.\n", "Backup", job.cmd.job));
        }
    }
    if jobs.is_empty() {
        out.push("No Jobs running.\n".into());
    }
    out.push("====\n".into());
    out
}

/// `job_level_to_str` for the levels of a backup.
fn level_name(level: char) -> &'static str {
    match level {
        'F' => "Full",
        'I' => "Incremental",
        'D' => "Differential",
        _ => "Unknown Job Level",
    }
}

/// `ListDevices` (File devices only).
fn devices(d: &Daemon) -> Vec<String> {
    let mut out = vec!["\nDevice status:\n".to_string()];
    for conf in &d.config.devices {
        let dev = FileDevice::from_config(conf);
        match d.device_user(&dev.name) {
            Some(job) => out.push(format!("\nDevice {} is in use by job {job}.\n", dev.print_name())),
            None => out.push(format!("\nDevice {} is not open.\n", dev.print_name())),
        }
        out.push(match crate::health::of_device(&dev.archive_device) {
            Ok(h) => format!("    Health: {}\n", h.status_line()),
            Err(e) => format!("    Health: unknown ({e})\n"),
        });
    }
    out.push("====\n".into());
    out
}

/// `OutputStatus` for `status`.
pub fn status_text(d: &Daemon) -> Vec<String> {
    let mut out = header(d);
    out.extend(running(d));
    out.extend(devices(d));
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
        "waitreservation" | "volumes" | "spooling" | "terminated" => Vec::new(),
        _ => return None,
    })
}
