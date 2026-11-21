//! State file of the Director (`WriteStateFile` / `ReadStateFile`,
//! dird/dir_plugins and lib/bstat of the original): the JobIds of the last
//! jobs that ended, so that `status dir` still lists them after a restart.
//!
//! The original writes a binary file `<name>.<port>.state`. Onesimus keeps
//! its own format (JSON, `<working directory>/odir.<port>.state`): nothing
//! else reads it, and a file of the original is never overwritten.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Version of the file layout; a file of another version is ignored.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct State {
    version: u32,
    /// JobIds in the order the jobs ended, oldest first.
    recent_jobs: Vec<u32>,
}

/// Path of the state file of the Director on `port`.
pub fn path(working_directory: &str, port: u16) -> PathBuf {
    Path::new(working_directory).join(format!("odir.{port}.state"))
}

/// The JobIds stored in `file`; empty when it is missing, unreadable or of
/// another version (the Director then starts without history, as before).
pub fn read(file: &Path) -> Vec<u32> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    match serde_json::from_str::<State>(&text) {
        Ok(state) if state.version == VERSION => state.recent_jobs,
        _ => Vec::new(),
    }
}

/// Writes `recent` to `file` through a temporary file, so a crash never
/// leaves a half-written state.
pub fn write(file: &Path, recent: &[u32]) -> std::io::Result<()> {
    let state = State { version: VERSION, recent_jobs: recent.to_vec() };
    let text = serde_json::to_string(&state).map_err(std::io::Error::other)?;
    let mut temporary = file.to_path_buf().into_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_written_is_read_back() {
        let dir = std::env::temp_dir().join(format!("odir-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = path(dir.to_str().unwrap(), 9101);
        assert!(read(&file).is_empty(), "a missing file is an empty history");
        write(&file, &[3, 5, 8]).unwrap();
        assert_eq!(read(&file), vec![3, 5, 8]);
        std::fs::write(&file, r#"{"version":99,"recent_jobs":[1]}"#).unwrap();
        assert!(read(&file).is_empty(), "another version is ignored");
        std::fs::write(&file, "not json").unwrap();
        assert!(read(&file).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
