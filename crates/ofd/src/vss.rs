//! Volume Shadow Copy (VSS) for the Windows FD (S-53).
//!
//! The Director sends `fileset vss=1` (`EnableVSS = yes`, the default). The
//! FD then creates one shadow copy per drive named in the include list and
//! reads names, metadata and data from the copy: files other programs hold
//! open without sharing can be saved, and the data is consistent as of one
//! point in time. The names in the catalog stay the original ones
//! (`D:/dir/file`); only the calls that read the tree are translated by
//! [`translate`] to `\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopyN\dir\file`.
//!
//! The copy is created and deleted through WMI (`Win32_ShadowCopy`, context
//! `ClientAccessible`, started with the PowerShell of the system directory):
//! it is a file system consistent copy without the VSS writers of
//! applications (no application consistent databases). Several jobs on one
//! drive share one copy; the last one deletes it.

#![cfg_attr(not(windows), allow(dead_code))]

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Mutex;

/// The shadow copies in use, by drive letter (upper case).
static COPIES: Mutex<Option<HashMap<u8, Copy>>> = Mutex::new(None);

/// One shadow copy and the jobs using it.
struct Copy {
    id: String,
    device: String,
    users: usize,
}

/// The drive letter (upper case) of a name like `D:/dir/file`.
pub fn drive_of(path: &[u8]) -> Option<u8> {
    match path {
        [l, b':', rest @ ..]
            if l.is_ascii_alphabetic()
                && (rest.is_empty() || rest[0] == b'/' || rest[0] == b'\\') =>
        {
            Some(l.to_ascii_uppercase())
        }
        _ => None,
    }
}

/// `device` + the part of `path` after the drive, with `\` as separator.
fn join(device: &str, path: &[u8]) -> Vec<u8> {
    let mut out = device.as_bytes().to_vec();
    for &b in &path[2..] {
        out.push(if b == b'/' { b'\\' } else { b });
    }
    if path.len() == 2 {
        out.push(b'\\'); // the root of the copy, not the device
    }
    out
}

/// The path to read `path` from: the shadow copy of its drive when there is
/// one, otherwise `path` itself.
pub fn translate(path: &[u8]) -> Cow<'_, [u8]> {
    if let Some(letter) = drive_of(path) {
        if let Some(copies) = COPIES.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if let Some(c) = copies.get(&letter) {
                return Cow::Owned(join(&c.device, path));
            }
        }
    }
    Cow::Borrowed(path)
}

/// Keeps the shadow copies of a job; the drop releases them.
pub struct Guard {
    drives: Vec<u8>,
}

impl Guard {
    /// The drives with a shadow copy.
    pub fn drives(&self) -> &[u8] {
        &self.drives
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        for &letter in &self.drives {
            release(letter);
        }
    }
}

/// Takes a shadow copy of each of `drives` (or shares the one a running job
/// made). The error names the drive and the reason; the drives that worked
/// stay in the guard.
pub fn acquire(drives: &[u8]) -> (Guard, Vec<String>) {
    let mut have = Vec::new();
    let mut errors = Vec::new();
    for &letter in drives {
        let letter = letter.to_ascii_uppercase();
        if have.contains(&letter) {
            continue;
        }
        // The lock is held while the copy is made: a second job on the same drive waits and shares it.
        let mut guard = COPIES.lock().unwrap_or_else(|e| e.into_inner());
        let copies = guard.get_or_insert_with(HashMap::new);
        if let Some(c) = copies.get_mut(&letter) {
            c.users += 1;
            have.push(letter);
            continue;
        }
        match create(letter) {
            Ok((id, device)) => {
                copies.insert(
                    letter,
                    Copy {
                        id,
                        device,
                        users: 1,
                    },
                );
                have.push(letter);
            }
            Err(e) => errors.push(format!("{}: {e}", letter as char)),
        }
    }
    (Guard { drives: have }, errors)
}

/// One user less; the last one deletes the copy.
fn release(letter: u8) {
    let mut guard = COPIES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(copies) = guard.as_mut() else { return };
    let last = match copies.get_mut(&letter) {
        Some(c) => {
            c.users -= 1;
            c.users == 0
        }
        None => false,
    };
    if last {
        if let Some(c) = copies.remove(&letter) {
            let _ = delete(&c.id);
        }
    }
}

/// The PowerShell of the system directory (never one found through PATH).
#[cfg(windows)]
fn powershell() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    std::path::Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// Runs `script` in PowerShell; its output is the result.
#[cfg(windows)]
fn run(script: &str) -> Result<String, String> {
    let out = std::process::Command::new(powershell())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot start PowerShell: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err
            .trim()
            .lines()
            .next()
            .unwrap_or("PowerShell failed")
            .to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Creates the shadow copy of drive `letter`: its ID and device path.
#[cfg(windows)]
fn create(letter: u8) -> Result<(String, String), String> {
    let l = letter as char;
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $r = Invoke-CimMethod -ClassName Win32_ShadowCopy -MethodName Create -Arguments @{{Volume='{l}:\\';Context='ClientAccessible'}}; \
         if ($r.ReturnValue -ne 0) {{ throw \"Win32_ShadowCopy.Create returned $($r.ReturnValue)\" }}; \
         $s = Get-CimInstance Win32_ShadowCopy | Where-Object {{ $_.ID -eq $r.ShadowID }}; \
         \"$($s.ID)|$($s.DeviceObject)\""
    );
    parse(&run(&script)?)
}

/// The ID and device path from the line `create` printed.
fn parse(out: &str) -> Result<(String, String), String> {
    let line = out.lines().last().unwrap_or("");
    match line.split_once('|') {
        Some((id, dev))
            if id.starts_with('{')
                && dev.starts_with(r"\\?\GLOBALROOT\Device\")
                && !dev.ends_with('\\') =>
        {
            Ok((id.to_string(), dev.to_string()))
        }
        _ => Err(format!(
            "unexpected answer of the shadow copy service: {line:?}"
        )),
    }
}

/// Deletes the shadow copy `id`.
#[cfg(windows)]
fn delete(id: &str) -> Result<(), String> {
    if !id.starts_with('{')
        || !id.ends_with('}')
        || !id
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == '{' || c == '}')
    {
        return Err("bad shadow copy ID".into());
    }
    run(&format!(
        "$ErrorActionPreference='Stop'; Get-CimInstance Win32_ShadowCopy | Where-Object {{ $_.ID -eq '{id}' }} | Remove-CimInstance"
    ))
    .map(|_| ())
}

#[cfg(not(windows))]
fn create(_letter: u8) -> Result<(String, String), String> {
    Err("VSS exists on Windows only".into())
}

#[cfg(not(windows))]
fn delete(_id: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_letters() {
        assert_eq!(drive_of(b"d:/x"), Some(b'D'));
        assert_eq!(drive_of(b"C:"), Some(b'C'));
        assert_eq!(drive_of(b"C:\\x"), Some(b'C'));
        assert_eq!(drive_of(b"//server/share"), None);
        assert_eq!(drive_of(b"/home"), None);
        assert_eq!(drive_of(b"CD:/x"), None);
    }

    #[test]
    fn translation_follows_the_copy() {
        let dev = r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy7";
        COPIES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(
                b'Q',
                Copy {
                    id: "{x}".into(),
                    device: dev.into(),
                    users: 1,
                },
            );
        assert_eq!(
            translate(b"q:/a/b.txt").as_ref(),
            format!(r"{dev}\a\b.txt").as_bytes()
        );
        assert_eq!(translate(b"Q:").as_ref(), format!(r"{dev}\").as_bytes());
        assert_eq!(translate(b"Q:/").as_ref(), format!(r"{dev}\").as_bytes());
        assert_eq!(translate(b"R:/a").as_ref(), b"R:/a");
        COPIES.lock().unwrap().as_mut().unwrap().remove(&b'Q');
        assert_eq!(translate(b"Q:/a").as_ref(), b"Q:/a");
    }

    #[test]
    fn answer_of_the_service() {
        let ok = r"{1D2A3B4C-0000-1111-2222-333344445555}|\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3";
        assert_eq!(
            parse(ok).unwrap().1,
            r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3"
        );
        assert!(parse("").is_err());
        assert!(parse("{x}|C:\\").is_err());
    }
}
