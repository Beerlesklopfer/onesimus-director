// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
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
                    crate::config_check::Diag::Parser(_) | crate::config_check::Diag::Config(_) => {
                        error!("configuration error: {}", diag.text());
                    }
                }
                if reason.is_empty() {
                    reason = diag.text().trim_end().to_string();
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
    for note in crate::config_test::deprecated_notes(&new_config.raw) {
        warn!("{note}");
    }
    let mut cfg = install_config(shared_config, new_config)?;
    // DoReloadConfig(): CheckCatalog(UPDATE_CATALOG) for the new resources
    if let Some(catalog) = RELOAD_CATALOG.get() {
        crate::check_catalog::ensure_basic_records(&cfg, catalog);
    }
    info!("configuration reloaded: Director '{}', {} clients, {} jobs",
          cfg.director.name, cfg.clients.len(), cfg.jobs.len());
    let catalog_acl = cfg.catalog_acl.clone();
    drop(cfg);
    // Two-tier ACLs: reload the catalog ACL cache with the configuration
    // (outside the configuration lock; a failure locks the consoles that
    // have or may have catalog entries, catalog_acl F-4).
    catalog_acl.refresh();
    crate::ovfs::recheck_after_reload();
    Ok(())
}

/// Makes a checked configuration the active one. The scheduler's queue is
/// cleared with it (reload.cc:212 `Scheduler::GetMainScheduler().ClearQueue()`):
/// the generation is bumped while the write lock is held, so the scheduler
/// never runs an item planned from the old configuration with the Run
/// directives of the new one. The catalog ACL store (inner ACL tier) of the
/// old configuration goes on to the new one, so its cache and catalog
/// attachment survive the reload.
fn install_config(shared_config: &SharedConfig, mut new_config: DirectorConfig)
    -> Result<std::sync::RwLockWriteGuard<'_, DirectorConfig>, String> {
    let mut cfg = shared_config.write().map_err(|e| format!("config lock: {e}"))?;
    new_config.catalog_acl = cfg.catalog_acl.clone();
    crate::fd_grpc::configure(&new_config);
    crate::sd_grpc::configure(&new_config);
    *cfg = new_config;
    crate::scheduler::config_replaced();
    crate::fd_protocol::reset_handshake_modes();
    Ok(cfg)
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
    fn installing_a_reloaded_configuration_clears_the_scheduler_queue() {
        let shared: SharedConfig = Arc::new(RwLock::new(DirectorConfig::from_str(
            "Director { Name = old-dir\n Password = p\n }").unwrap()));
        let before = crate::scheduler::CONFIG_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
        let guard = install_config(&shared, DirectorConfig::from_str(
            "Director { Name = new-dir\n Password = p\n }").unwrap()).unwrap();
        // bumped while the write lock is still held
        assert!(crate::scheduler::CONFIG_GENERATION.load(std::sync::atomic::Ordering::SeqCst) > before);
        assert_eq!(guard.director.name, "new-dir");
        drop(guard);
        assert_eq!(shared.read().unwrap().director.name, "new-dir");
    }

    #[test]
    fn failed_reload_keeps_the_previous_configuration() {
        let dir = std::env::temp_dir().join(format!("odir-reload-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bareos-dir.conf");
        std::fs::write(&path, "Director { Name = broken\n Password = p\n ").unwrap();
        let shared: SharedConfig = Arc::new(RwLock::new(DirectorConfig::from_str(
            "Director { Name = old-dir\n Password = p\n }").unwrap()));
        assert!(reload_config(&path, &shared).is_err());
        assert_eq!(shared.read().unwrap().director.name, "old-dir");
        let _ = std::fs::remove_dir_all(&dir);
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
