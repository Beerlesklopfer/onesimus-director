//! Configuration hot-reload via SIGHUP.
//!
//! When the Director receives SIGHUP, it reloads the configuration file
//! and swaps the SharedConfig atomically via RwLock.

use std::path::PathBuf;

use tracing::{info, error, warn};

use crate::director_config::{DirectorConfig, SharedConfig};

/// Config path + shared config, for the `reload` console command.
static RELOAD_TARGET: std::sync::OnceLock<(PathBuf, SharedConfig)> = std::sync::OnceLock::new();

/// Catalog for CheckCatalog(UPDATE_CATALOG) after a reload.
static RELOAD_CATALOG: std::sync::OnceLock<crate::catalog_worker::CatalogHandle> = std::sync::OnceLock::new();

/// Register the catalog DoReloadConfig() updates (Client/Storage records).
pub fn register_catalog(catalog: crate::catalog_worker::CatalogHandle) {
    let _ = RELOAD_CATALOG.set(catalog);
}

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

/// Configuration path the Director was started with (C++
/// `my_config->get_base_config_path()`); None before startup and in tests.
pub fn config_path() -> Option<PathBuf> {
    RELOAD_TARGET.get().map(|target| target.0.clone())
}

/// Reload the configuration now (console `reload`, SIGHUP).
/// On error the previous configuration stays active.
pub fn reload_now() -> Result<(), String> {
    let (path, shared) = RELOAD_TARGET.get().ok_or("reload not initialized")?;
    reload_config(path, shared)
}

fn reload_config(config_path: &std::path::Path, shared_config: &SharedConfig) -> Result<(), String> {
    info!("reloading configuration from {}", config_path.display());
    let base = crate::config_check::base_config_path(config_path);
    let loaded = DirectorConfig::load(config_path)
        .map_err(|e| vec![crate::config_check::Diag::Parser(e.to_string())])
        .and_then(|config| crate::config_check::check(&config.raw, &base).map(|()| config))
        // CheckCatalog(UPDATE_CATALOG): every Catalog must open (C-075)
        .and_then(|config| {
            crate::check_catalog::check_connections(&config, |partial| {
                if let Some(catalog) = RELOAD_CATALOG.get() {
                    crate::check_catalog::ensure_basic_records(partial, catalog);
                }
            })
            .map_err(|text| vec![crate::config_check::Diag::Job(crate::message_bus::MsgType::Fatal, text)])
            .map(|()| config)
        });
    let mut new_config = match loaded {
        Ok(config) => config,
        Err(diags) => {
            // DoReloadConfig(): the parser/CheckResources() messages, then
            // "Please correct ..." and "Resetting ..." through the daemon
            // messages of the configuration that stays active.
            let current = shared_config.read().map_err(|e| format!("config lock: {e}"))?;
            let mut reason = String::new();
            for diag in &diags {
                match diag {
                    crate::config_check::Diag::Job(msg_type, text) => job_msg(&current, *msg_type, text),
                    crate::config_check::Diag::Daemon(text) => {
                        crate::audit::daemon_message(&current, crate::message_bus::MsgType::Error, text)
                    }
                    crate::config_check::Diag::Parser(text) => {
                        error!("configuration error: {text}");
                    }
                }
                if reason.is_empty() {
                    reason = match diag {
                        crate::config_check::Diag::Job(_, t) | crate::config_check::Diag::Daemon(t)
                        | crate::config_check::Diag::Parser(t) => t.trim_end().to_string(),
                    };
                }
            }
            job_msg(&current, crate::message_bus::MsgType::Error, &format!("Please correct the configuration in {base}\n"));
            job_msg(&current, crate::message_bus::MsgType::Error, "Resetting to previous configuration.\n");
            error!("configuration reload failed: {reason}");
            warn!("keeping previous configuration");
            return Err(reason);
        }
    };
    // C++ CheckResources() set omit_defaults_ at startup: every later parse
    // leaves unset items without the inherit bit (show prints them).
    new_config.reloaded = true;
    let mut cfg = shared_config.write().map_err(|e| format!("config lock: {e}"))?;
    *cfg = new_config;
    crate::fd_protocol::reset_handshake_modes();
    // DoReloadConfig(): CheckCatalog(UPDATE_CATALOG) for the new resources
    if let Some(catalog) = RELOAD_CATALOG.get() {
        crate::check_catalog::ensure_basic_records(&cfg, catalog);
    }
    info!("configuration reloaded: Director '{}', {} clients, {} jobs",
          cfg.director.name, cfg.clients.len(), cfg.jobs.len());
    drop(cfg);
    crate::ovfs::recheck_after_reload();
    Ok(())
}

/// Jmsg(nullptr, type, 0, text): JobId 0 through the daemon messages.
fn job_msg(config: &DirectorConfig, msg_type: crate::message_bus::MsgType, text: &str) {
    let msg = crate::console_msg::job_message_text(&config.director.name, 0, msg_type, text);
    if let Some(res) = crate::audit::daemon_messages(config) {
        crate::audit::dispatch(res, crate::audit::code(msg_type), 0, &msg, None);
    }
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
