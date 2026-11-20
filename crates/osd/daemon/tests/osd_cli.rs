//! `osd -t` and the P0 behaviour of the binary on sample configurations.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("osd-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The shipped `config/bareos-sd.d` with the placeholders the package's
/// postinst fills in, copied to `<dir>/bareos-sd.d`.
fn shipped_config(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/bareos-sd.d");
    let tree = dir.join("bareos-sd.d");
    for sub in std::fs::read_dir(&src).unwrap() {
        let sub = sub.unwrap().path();
        let to = tree.join(sub.file_name().unwrap());
        std::fs::create_dir_all(&to).unwrap();
        for f in std::fs::read_dir(&sub).unwrap() {
            let f = f.unwrap().path();
            let text = std::fs::read_to_string(&f).unwrap()
                .replace("@DIRECTOR_NAME@", "bareos-dir")
                .replace("@STORAGE_NAME@", "test-sd")
                .replace("CHANGEME", "secret");
            std::fs::write(to.join(f.file_name().unwrap()), text).unwrap();
        }
    }
    tree
}

/// Runs `osd` with `args`.
fn osd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_osd")).args(args).env_remove("RUST_LOG").output().unwrap()
}

#[test]
fn t_accepts_the_shipped_config() {
    let dir = scratch("good");
    let tree = shipped_config(&dir);
    for path in [&dir, &tree] {
        let out = osd(&["-t", "-c", path.to_str().unwrap()]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(stdout.trim_end(),
            "osd: configuration OK: Storage \"test-sd\" (port 9103), 1 Director(s), 1 Device(s), 0 Autochanger(s)");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn t_rejects_a_broken_config_with_1() {
    let dir = scratch("bad");
    let tree = shipped_config(&dir);
    std::fs::write(tree.join("device/FileStorage.conf"), "Device {\n Name = FileStorage\n Media Type = File\n}\n").unwrap();
    let out = osd(&["-t", "-c", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Device \"FileStorage\": required directive \"Archive Device\" missing"), "{stderr}");
    // missing configuration
    let out = osd(&["-t", "-c", dir.join("nothing").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn without_t_osd_is_pending_p1() {
    let dir = scratch("run");
    shipped_config(&dir);
    let out = osd(&["-f", "-d", "100", "-v", "-c", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("osd: P1 pending"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unknown_argument_is_41() {
    assert_eq!(osd(&["--no-such-flag"]).status.code(), Some(41));
}
