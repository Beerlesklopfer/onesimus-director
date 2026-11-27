// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
// Included by the build scripts that stamp the build date (odir, daemon-crate).

/// Lets cargo run the build script again after every commit or checkout
/// (`.git/logs/HEAD`, `.git/HEAD`) and after a change under the crate's
/// `src`, so that the stamped build date follows the build. Without this the
/// script ran only when `SOURCE_DATE_EPOCH` changed, and odir reported the
/// date of its first build ("23 November 2026" in a build of 27.11.2026).
/// A worktree's `.git` file is followed to its git directory.
fn rerun_on_commit() {
    println!("cargo:rerun-if-changed=src");
    let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") else { return };
    let mut dir = std::path::PathBuf::from(manifest);
    loop {
        let dot_git = dir.join(".git");
        let git_dir = if dot_git.is_dir() {
            Some(dot_git)
        } else if dot_git.is_file() {
            std::fs::read_to_string(&dot_git).ok()
                .and_then(|t| t.trim().strip_prefix("gitdir:").map(|p| dir.join(p.trim())))
        } else {
            None
        };
        if let Some(git_dir) = git_dir {
            for file in ["logs/HEAD", "HEAD"] {
                let path = git_dir.join(file);
                if path.exists() {
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
            return;
        }
        if !dir.pop() {
            return;
        }
    }
}
