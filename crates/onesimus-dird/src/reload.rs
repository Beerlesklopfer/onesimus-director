//! Configuration hot-reload via SIGHUP.
//!
//! When the Director receives SIGHUP, it reloads the configuration file
//! and swaps the SharedConfig atomically via RwLock.

use std::path::PathBuf;

use tracing::{info, error, warn};

use crate::director_config::{DirectorConfig, SharedConfig};

/// Config path + shared config, for the `reload` console command.
static RELOAD_TARGET: std::sync::OnceLock<(PathBuf, SharedConfig)> = std::sync::OnceLock::new();

/// Block SIGHUP in the calling thread. Must run in main() before any other
/// thread is spawned: threads inherit the mask, so only the dir:sighup thread
/// (sigwait) ever receives it. Without this, SIGHUP hits an arbitrary thread
/// and its default action terminates the Director.
pub fn block_sighup() {
    #[cfg(unix)]
    {
        use nix::sys::signal::{SigSet, Signal};
        let mut sigset = SigSet::empty();
        sigset.add(Signal::SIGHUP);
        if let Err(e) = sigset.thread_block() {
            error!("cannot block SIGHUP: {e}");
        }
    }
}

/// Set up SIGHUP handler that triggers config reload.
pub fn setup_sighup_handler(
    config_path: PathBuf,
    shared_config: SharedConfig,
) {
    let _ = RELOAD_TARGET.set((config_path.clone(), shared_config.clone()));
    std::thread::Builder::new()
        .name("dir:sighup".into())
        .spawn(move || {
            sighup_loop(config_path, shared_config);
        })
        .expect("failed to spawn SIGHUP handler");
}

/// Reload the configuration now (console `reload`, SIGHUP).
/// On error the previous configuration stays active.
pub fn reload_now() -> Result<(), String> {
    let (path, shared) = RELOAD_TARGET.get().ok_or("reload not initialized")?;
    reload_config(path, shared)
}

fn reload_config(config_path: &std::path::Path, shared_config: &SharedConfig) -> Result<(), String> {
    info!("reloading configuration from {}", config_path.display());
    let new_config = DirectorConfig::load(config_path).map_err(|e| {
        error!("configuration reload failed: {e}");
        warn!("keeping previous configuration");
        e.to_string()
    })?;
    let mut cfg = shared_config.write().map_err(|e| format!("config lock: {e}"))?;
    *cfg = new_config;
    crate::fd_protocol::reset_handshake_modes();
    info!("configuration reloaded: Director '{}', {} clients, {} jobs",
          cfg.director.name, cfg.clients.len(), cfg.jobs.len());
    Ok(())
}

#[cfg(unix)]
fn sighup_loop(config_path: PathBuf, shared_config: SharedConfig) {
    use nix::sys::signal::{SigSet, Signal};

    let mut sigset = SigSet::empty();
    sigset.add(Signal::SIGHUP);
    sigset.thread_block().ok();

    loop {
        match sigset.wait() {
            Ok(Signal::SIGHUP) => {
                info!("SIGHUP received");
                let _ = reload_config(&config_path, &shared_config);
            }
            Ok(_) => {} // Other signal, ignore
            Err(e) => {
                error!("sigwait error: {e}");
                break;
            }
        }
    }
}

#[cfg(not(unix))]
fn sighup_loop(_config_path: PathBuf, _shared_config: SharedConfig) {
    // No SIGHUP on non-Unix platforms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::RwLock;

    #[test]
    fn config_swap_via_rwlock() {
        let config = DirectorConfig::from_str(
            "Director { Name = old-dir\n Port = 9101\n Password = pass\n }"
        ).unwrap();
        let shared: SharedConfig = Arc::new(RwLock::new(config));

        // Verify initial
        assert_eq!(shared.read().unwrap().director.name, "old-dir");

        // Swap
        let new_config = DirectorConfig::from_str(
            "Director { Name = new-dir\n Port = 9101\n Password = pass\n }"
        ).unwrap();
        *shared.write().unwrap() = new_config;

        // Verify swapped
        assert_eq!(shared.read().unwrap().director.name, "new-dir");
    }

    #[test]
    fn concurrent_reads_during_write() {
        let config = DirectorConfig::from_str(
            "Director { Name = test-dir\n Port = 9101\n Password = p\n }"
        ).unwrap();
        let shared: SharedConfig = Arc::new(RwLock::new(config));

        let handles: Vec<_> = (0..10).map(|_| {
            let c = shared.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    let cfg = c.read().unwrap();
                    assert!(!cfg.director.name.is_empty());
                }
            })
        }).collect();

        // Write while readers are active
        let new = DirectorConfig::from_str(
            "Director { Name = updated\n Port = 9101\n Password = p\n }"
        ).unwrap();
        *shared.write().unwrap() = new;

        for h in handles {
            h.join().unwrap();
        }
    }
}
