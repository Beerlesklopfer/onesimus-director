//! Reload the daemon's tracing filter when the console changes debug level.
use std::sync::OnceLock;
use tracing_subscriber::{EnvFilter, Registry, reload};

static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

pub fn install(handle: reload::Handle<EnvFilter, Registry>) {
    let _ = FILTER.set(handle);
}

pub fn set_level(level: u32) -> Result<(), String> {
    if let Some(handle) = FILTER.get() {
        apply_filter(handle, level)?;
    }
    bareos_bnet::debug::set_debug_level(level);
    Ok(())
}

fn apply_filter(handle: &reload::Handle<EnvFilter, Registry>, level: u32) -> Result<(), String> {
    let filter = if level > 0 { "debug,hyper=info,h2=info,tower=info" }
        else { "info,hyper=warn,h2=warn,tower=warn" };
    handle.reload(EnvFilter::new(filter)).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().extend_from_slice(bytes); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    #[test]
    fn reload_changes_which_diagnostics_reach_the_sink() {
        let writer = Writer(Arc::new(Mutex::new(vec![])));
        let output = writer.0.clone();
        let (layer, handle) = reload::Layer::new(EnvFilter::new("info"));
        let subscriber = tracing_subscriber::registry().with(layer)
            .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(move || writer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!("before-reload");
            apply_filter(&handle, 100).unwrap();
            tracing::debug!("enabled-debug");
            apply_filter(&handle, 0).unwrap();
            tracing::debug!("after-disable");
        });
        let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(text.contains("enabled-debug"));
        assert!(!text.contains("before-reload"));
        assert!(!text.contains("after-disable"));
    }
}
