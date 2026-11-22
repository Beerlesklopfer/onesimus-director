//! The plugin trust check on Windows: reads owner and DACL of every
//! component of a path and asks [`crate::trust_acl`].
//!
//! Zweck: Windows-Gegenstück von `plugin::check_trusted` (Unix: root als
//! Eigentümer, nicht schreibbar für Gruppe oder Andere, für den Pfad und
//! jedes Verzeichnis darüber). Die Entscheidung trifft `trust_acl::verdict`
//! aus Besitzer-SID und DACL; dieses Modul besorgt die Daten mit
//! `GetNamedSecurityInfoW` und sorgt dafür, dass keine Stelle des Pfads
//! hinter einem Link verschwindet.
//!
//! Datenfluss: `plugin::check_trusted` → [`check`] → je Bestandteil
//! (`path` und `canonicalize(path)`, jeweils mit allen Verzeichnissen
//! darüber bis zur Laufwerkswurzel oder `top`): kein Reparse-Punkt, Besitzer
//! und DACL lesen, `verdict`.
//!
//! Regeln: Nur ein absoluter Pfad auf einem Laufwerk (`D:\…`); UNC-Pfade
//! sind abgelehnt, weil Rechte eines entfernten Servers hier nicht
//! beurteilt werden. Ein Symlink, eine Junction oder ein anderer
//! Reparse-Punkt an irgendeiner Stelle ist abgelehnt, auch wenn sein Ziel
//! sicher wäre: sonst ließe sich der Pfad nach der Prüfung umbiegen. Jeder
//! Fehler eines Systemaufrufs lehnt ab. Erhöhung: ein Verzeichnis, das ein
//! Administrator ohne Erhöhung anlegt, gehört seinem Benutzer-SID und ist
//! abgelehnt; erst der Besitzer "Administratoren" (erhöhte Sitzung,
//! `icacls /setowner *S-1-5-32-544`, oder ein Installer) wird angenommen.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf, Prefix};

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{GetAce, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};

use crate::trust_acl::{verdict, Ace, AceKind, Descriptor, Role};

/// `FILE_ATTRIBUTE_REPARSE_POINT`: a symlink, a junction or a mount point.
const REPARSE_POINT: u32 = 0x400;
/// `INHERIT_ONLY_ACE` in the flags of an entry.
const INHERIT_ONLY_ACE: u8 = 0x08;
/// `ACCESS_ALLOWED_ACE_TYPE`.
const ACE_ALLOWED: u8 = 0;
/// `ACCESS_DENIED_ACE_TYPE`.
const ACE_DENIED: u8 = 1;

/// The layout of `ACCESS_ALLOWED_ACE` and `ACCESS_DENIED_ACE`: header, mask, then the SID.
#[repr(C)]
struct AceBody {
    header: ACE_HEADER,
    mask: u32,
    sid_start: u32,
}

/// `path` as a wide, NUL-terminated string.
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// A SID as `S-1-5-…`.
///
/// # Safety
/// `sid` points to a valid SID that outlives the call.
unsafe fn sid_string(sid: *mut c_void) -> Result<String, String> {
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is valid (caller); `text` receives a buffer from LocalAlloc.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return Err("cannot convert a SID".into());
    }
    // SAFETY: ConvertSidToStringSidW returned a NUL-terminated wide string.
    let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    let value = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    // SAFETY: the buffer came from LocalAlloc and is not used again.
    unsafe { LocalFree(text.cast()) };
    Ok(value)
}

/// Owner and DACL of `path`; any failure is an error (the caller refuses).
fn descriptor(path: &Path) -> Result<Descriptor, String> {
    let name = wide(path);
    let mut owner: *mut c_void = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut security: *mut c_void = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated; the out pointers are valid; the descriptor is freed below.
    let status = unsafe {
        GetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner, std::ptr::null_mut(), &mut dacl, std::ptr::null_mut(), &mut security)
    };
    if status != ERROR_SUCCESS {
        return Err(format!("cannot read the security descriptor (error {status})"));
    }
    let result = (|| {
        if owner.is_null() {
            return Err("has no owner".to_string());
        }
        // SAFETY: `owner` points into `security`, which is alive until the end of this closure.
        let owner = unsafe { sid_string(owner) }?;
        if dacl.is_null() {
            return Ok(Descriptor { owner, dacl: None });
        }
        // SAFETY: `dacl` points into `security`.
        let count = unsafe { (*dacl).AceCount };
        let mut entries = Vec::with_capacity(usize::from(count));
        for index in 0..u32::from(count) {
            let mut raw: *mut c_void = std::ptr::null_mut();
            // SAFETY: `index` is below AceCount; `raw` receives a pointer into the ACL.
            if unsafe { GetAce(dacl, index, &mut raw) } == 0 || raw.is_null() {
                return Err(format!("cannot read access control entry {index}"));
            }
            // SAFETY: every ACE starts with an ACE_HEADER.
            let header = unsafe { *raw.cast::<ACE_HEADER>() };
            let kind = match header.AceType {
                ACE_ALLOWED => AceKind::Allow,
                ACE_DENIED => AceKind::Deny,
                other => {
                    entries.push(Ace { kind: AceKind::Other(other), sid: String::new(), mask: 0, inherit_only: false });
                    continue;
                }
            };
            let body = raw.cast::<AceBody>();
            // SAFETY: allowed and denied entries have the AceBody layout; the SID starts at sid_start.
            let (mask, sid) = unsafe { ((*body).mask, sid_string(std::ptr::addr_of_mut!((*body).sid_start).cast())?) };
            entries.push(Ace { kind, sid, mask, inherit_only: header.AceFlags & INHERIT_ONLY_ACE != 0 });
        }
        Ok(Descriptor { owner, dacl: Some(entries) })
    })();
    // SAFETY: allocated by GetNamedSecurityInfoW, not used after this point.
    unsafe { LocalFree(security) };
    result
}

/// `path` without the `\\?\` prefix `canonicalize` adds (a drive path only).
fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

/// Lower-case text of a path, for comparisons (Windows names ignore case).
fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Whether `path` is below (or is) `top`.
fn within(path: &Path, top: &Path) -> bool {
    let (p, t) = (key(path), key(top));
    let t = t.trim_end_matches('\\');
    p == t || p.starts_with(&format!("{t}\\"))
}

/// `path` and every directory above it (as written and after resolving it)
/// are owned by SYSTEM, Administrators or TrustedInstaller, hold no
/// reparse point, and cannot be changed by anybody else. `top` limits the
/// walk (tests); `None` walks to the drive root. `Err` is `(component, reason)`.
pub fn check(path: &Path, directory: bool, top: Option<&Path>) -> Result<(), (PathBuf, String)> {
    let refuse = |p: &Path, why: String| (p.to_path_buf(), why);
    let drive = |p: &Path| matches!(p.components().next(), Some(Component::Prefix(c)) if matches!(c.kind(), Prefix::Disk(_)));
    if !path.is_absolute() || !drive(path) {
        return Err(refuse(path, "path is not an absolute path on a drive".into()));
    }
    let real = plain(std::fs::canonicalize(path).map_err(|e| refuse(path, e.to_string()))?);
    if !drive(&real) {
        return Err(refuse(&real, "resolves to a path that is not on a drive".into()));
    }
    let meta = std::fs::metadata(&real).map_err(|e| refuse(&real, e.to_string()))?;
    if directory != meta.is_dir() || (!directory && !meta.is_file()) {
        let want = if directory { "a directory" } else { "a regular file" };
        return Err(refuse(&real, format!("is not {want}")));
    }
    let top = top.map(|t| plain(std::fs::canonicalize(t).unwrap_or_else(|_| t.to_path_buf())));
    let mut seen: Vec<String> = Vec::new();
    for chain in [path, real.as_path()] {
        for (depth, component) in chain.ancestors().enumerate() {
            if top.as_deref().is_some_and(|t| !within(component, t)) || seen.contains(&key(component)) {
                continue;
            }
            seen.push(key(component));
            let link = std::fs::symlink_metadata(component).map_err(|e| refuse(component, e.to_string()))?;
            if link.file_attributes() & REPARSE_POINT != 0 {
                return Err(refuse(component, "is a symbolic link, junction or mount point".into()));
            }
            let is_dir = link.is_dir();
            let role = if depth == 0 { Role::Leaf } else { Role::Ancestor };
            let described = descriptor(component).map_err(|e| refuse(component, e))?;
            verdict(&described, is_dir, role).map_err(|e| refuse(component, e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows system files are owned by TrustedInstaller and fine (a real
    /// directory tree, no setup needed).
    #[test]
    fn system_files_pass() {
        let hosts = Path::new(r"C:\Windows\System32\drivers\etc\hosts");
        assert_eq!(check(hosts, false, None), Ok(()));
        assert_eq!(check(Path::new(r"C:\Windows\System32"), true, None), Ok(()));
    }

    /// The cases below need directories prepared with `icacls`
    /// (scripts/windows-vm/trust-prep.ps1); `ONESIMUS_WIN_TRUST` is their base.
    #[test]
    fn prepared_directories() {
        let Some(base) = std::env::var_os("ONESIMUS_WIN_TRUST").map(PathBuf::from) else {
            eprintln!("ONESIMUS_WIN_TRUST not set: skipped");
            return;
        };
        let top = Some(base.as_path());
        let why = |p: PathBuf, directory: bool| check(&p, directory, top).expect_err(&p.display().to_string()).1;
        eprintln!("good dir:        {:?}", check(&base.join("good"), true, top));
        eprintln!("good file:       {:?}", check(&base.join("good").join("f.bin"), false, top));
        assert_eq!(check(&base.join("good"), true, top), Ok(()));
        assert_eq!(check(&base.join("good").join("f.bin"), false, top), Ok(()));
        for (name, directory, expect) in [
            ("user_writable", true, "may"),
            ("user_owned", true, "owned by"),
            ("everyone_file.bin", false, "S-1-1-0 may"),
            ("inherit_dir\\child.bin", false, "may"),
            ("deny_other_group", true, "may"),
            ("with_delete_child_above\\sub", true, "delete child"),
        ] {
            let reason = why(base.join(name), directory);
            eprintln!("{name:24} refused: {reason}");
            assert!(reason.contains(expect), "{name}: {reason}");
        }
        for (name, directory) in [("inherit_dir", true), ("deny_cancels", true), ("adds_above\\sub", true)] {
            eprintln!("{name:24} {:?}", check(&base.join(name), directory, top));
            assert_eq!(check(&base.join(name), directory, top), Ok(()), "{name}");
        }
        let junction = why(base.join("junction").join("f.bin"), false);
        eprintln!("junction                 refused: {junction}");
        assert!(junction.contains("junction"), "{junction}");
        assert!(check(&base.join("missing"), true, top).is_err());
        assert!(check(Path::new(r"\\server\share\x"), true, None).is_err());
        assert!(check(Path::new(r"relative\x"), true, None).is_err());
        // without `top` the walk reaches the drive root; the result depends on who
        // owns the directories above (a user-owned test directory is refused there,
        // an Administrators-owned one is fine), so it is only reported
        eprintln!("good dir to the root     {:?}", check(&base.join("good"), true, None));
    }
}
