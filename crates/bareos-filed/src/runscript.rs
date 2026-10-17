//! Client run scripts: lib/runscript.cc (RunScripts, RunScript::Run),
//! lib/bpipe_unix.cc (OpenBpipe/BuildArgcArgv: no shell, the command line
//! is split at blanks with '…' / "…" quoting) and lib/util.cc
//! edit_job_codes() with the File Daemon callback (%D, %m).

use std::process::{Command, Stdio};

// lib/runscript.h
pub const SCRIPT_AFTER: i32 = 1 << 0;
pub const SCRIPT_BEFORE: i32 = 1 << 1;
pub const SCRIPT_AFTER_VSS: i32 = 1 << 2;

/// One `Run OnSuccess=%d OnFailure=%d AbortOnError=%d When=%d Command=%s`.
#[derive(Debug, Clone)]
pub struct RunScript {
    pub on_success: bool,
    pub on_failure: bool,
    pub fail_on_error: bool,
    pub when: i32,
    pub command: String,
}

/// RunscriptCmd() parsing (sscanf with `Command=%s`: up to the first blank,
/// then UnbashSpaces).
pub fn parse_runscript(cmd: &str) -> Option<RunScript> {
    let rest = cmd.strip_prefix("Run OnSuccess=")?;
    let (on_success, rest) = rest.split_once(" OnFailure=")?;
    let (on_failure, rest) = rest.split_once(" AbortOnError=")?;
    let (abort, rest) = rest.split_once(" When=")?;
    let (when, rest) = rest.split_once(" Command=")?;
    let command = rest.split_whitespace().next()?.replace('\x01', " ");
    Some(RunScript {
        on_success: on_success.trim().parse::<i32>().ok()? != 0,
        on_failure: on_failure.trim().parse::<i32>().ok()? != 0,
        fail_on_error: abort.trim().parse::<i32>().ok()? != 0,
        when: when.trim().parse().ok()?,
        command,
    })
}

/// BuildArgcArgv()
pub fn build_argv(cmd: &str) -> Vec<String> {
    let b = cmd.as_bytes();
    let mut argv = Vec::new();
    let mut p = 0;
    let skip_blank = |p: &mut usize| {
        while *p < b.len() && (b[*p] == b' ' || b[*p] == b'\t') {
            *p += 1;
        }
    };
    skip_blank(&mut p);
    let mut quote = 0u8;
    if p < b.len() && (b[p] == b'"' || b[p] == b'\'') {
        quote = b[p];
        p += 1;
    }
    while p < b.len() {
        let mut q = p;
        if quote != 0 {
            while q < b.len() && b[q] != quote {
                q += 1;
            }
            quote = 0;
        } else {
            while q < b.len() && b[q] != b' ' {
                q += 1;
            }
        }
        argv.push(String::from_utf8_lossy(&b[p..q]).into_owned());
        p = if q < b.len() { q + 1 } else { q };
        skip_blank(&mut p);
        if p < b.len() && (b[p] == b'"' || b[p] == b'\'') {
            quote = b[p];
            p += 1;
        }
    }
    argv
}

/// OpenBpipe(cmd, 0, "r") + read everything + CloseBpipe(): the output
/// (stdout and stderr) and the exit status.
pub fn run_program(cmd: &str) -> std::io::Result<(String, i32)> {
    let argv = build_argv(cmd);
    let Some((prog, args)) = argv.split_first() else {
        return Err(std::io::Error::from_raw_os_error(libc::ENOENT));
    };
    let (r, w) = std::io::pipe()?;
    let w2 = w.try_clone()?;
    let mut child = Command::new(prog).args(args).stdin(Stdio::null()).stdout(w).stderr(w2).spawn()?;
    let mut out = String::new();
    use std::io::Read;
    let mut r = r;
    // Drop our write ends (held by the Command) before reading.
    let _ = r.read_to_string(&mut out);
    let status = child.wait()?;
    Ok((out, status.code().unwrap_or(1)))
}

/// What edit_job_codes() needs from the job.
pub struct JobCodes<'a> {
    pub job_bytes: u64,
    pub job_files: u32,
    pub client: &'a str,
    pub my_name: &'a str,
    pub status: u8,
    pub job_id: u32,
    pub job: &'a str,
    pub level: &'a str,
    pub job_type: &'a str,
    pub director: &'a str,
    pub since_time: i64,
}

/// edit_uint64_with_suffix(): "1.500 K" style.
fn with_suffix(v: u64) -> String {
    const SUF: [&str; 10] = ["", "K", "M", "G", "T", "P", "E", "Z", "Y", "FIX ME"];
    let digits = v.to_string();
    let mut groups = Vec::new();
    let mut end = digits.len();
    while end > 3 {
        groups.push(&digits[end - 3..end]);
        end -= 3;
    }
    groups.push(&digits[..end]);
    groups.reverse();
    let commas = groups.len() - 1;
    let mut m = groups[0].to_string();
    if commas > 0 {
        m.push('.');
        m.push_str(groups[1]);
        m.truncate(5);
    }
    format!("{m} {}", SUF[commas.min(SUF.len() - 1)])
}

/// job_status_to_str()
fn job_status_to_str(s: u8) -> &'static str {
    match s {
        b'T' => "OK",
        b'W' => "OK -- with warnings",
        b'E' | b'f' => "Error",
        b'A' => "Canceled",
        b'D' => "Differences",
        _ => "Unknown term code",
    }
}

/// edit_job_codes() with job_code_callback_filed().
pub fn edit_job_codes(cmd: &str, j: &JobCodes) -> String {
    let mut out = String::new();
    let mut it = cmd.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(code) = it.next() else { break };
        match code {
            '%' => out.push('%'),
            'B' => out.push_str(&format!("{}B", with_suffix(j.job_bytes))),
            'F' => out.push_str(&j.job_files.to_string()),
            'P' => out.push_str(&std::process::id().to_string()),
            'b' => out.push_str(&j.job_bytes.to_string()),
            'c' => out.push_str(j.client),
            'd' => out.push_str(j.my_name),
            'e' => out.push_str(job_status_to_str(j.status)),
            'i' => out.push_str(&j.job_id.to_string()),
            'j' => out.push_str(j.job),
            'l' => out.push_str(j.level),
            'n' => {
                let mut name = j.job.to_string();
                for _ in 0..3 {
                    if let Some(p) = name.rfind('.') {
                        name.truncate(p);
                    }
                }
                out.push_str(&name);
            }
            'r' => {}
            's' => out.push_str("*None*"),
            't' => out.push_str(j.job_type),
            'v' => out.push_str("*None*"),
            'D' => out.push_str(j.director),
            'm' => out.push_str(&j.since_time.to_string()),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

/// Whether RunScripts() runs `s` for `label` at job status `status`.
pub fn should_run(s: &RunScript, when: i32, status: u8, canceled: bool) -> bool {
    let terminated_ok = matches!(status, b'T' | b'W');
    ((s.when & SCRIPT_BEFORE) != 0 && (when & SCRIPT_BEFORE) != 0
        && ((s.on_success && matches!(status, b'R' | b'C')) || (s.on_failure && (canceled || status == b'D'))))
        || ((s.when & SCRIPT_AFTER_VSS) != 0 && (when & SCRIPT_AFTER_VSS) != 0
            && ((s.on_success && status == b'B') || (s.on_failure && canceled)))
        || ((s.when & SCRIPT_AFTER) != 0 && (when & SCRIPT_AFTER) != 0
            && ((s.on_success && terminated_ok) || (s.on_failure && (canceled || status == b'D'))))
}

/// ScriptDirAllowed()
pub fn script_dir_allowed(command: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let dir = match command.rfind('/') {
        Some(p) => &command[..p],
        None => command,
    };
    if dir.contains("..") {
        return false;
    }
    allowed.iter().any(|a| a.eq_ignore_ascii_case(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_and_parse() {
        assert_eq!(build_argv("  /bin/echo 'a b' c"), vec!["/bin/echo", "a b", "c"]);
        assert_eq!(build_argv("\"/x y\" z"), vec!["/x y", "z"]);
        let s = parse_runscript("Run OnSuccess=1 OnFailure=0 AbortOnError=1 When=2 Command=/bin/echo\x01hi\x01%j").unwrap();
        assert!(s.on_success && !s.on_failure && s.fail_on_error);
        assert_eq!((s.when, s.command.as_str()), (SCRIPT_BEFORE, "/bin/echo hi %j"));
        let (out, st) = run_program("/bin/echo hello").unwrap();
        assert_eq!((out.as_str(), st), ("hello\n", 0));
        assert_eq!(run_program("/bin/false").unwrap().1, 1);
    }

    #[test]
    fn job_codes() {
        let j = JobCodes { job_bytes: 1500, job_files: 3, client: "c-fd", my_name: "c-fd", status: b'T', job_id: 7,
            job: "Back.2026-10-17_10.00.00_01", level: "Full", job_type: "Backup", director: "bareos-dir", since_time: 5 };
        assert_eq!(edit_job_codes("%i %j %n %c %D %m %e %% %x", &j), "7 Back.2026-10-17_10.00.00_01 Back c-fd bareos-dir 5 OK % %x");
        assert!(script_dir_allowed("/etc/x.sh", &["/etc".into()]));
        assert!(!script_dir_allowed("/etc/../x.sh", &["/etc/..".into()]));
    }
}
