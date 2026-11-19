//! Logging-Einrichtung (`tracing`).
//!
//! Zweck: Alle Daemons loggen über `tracing` nach stderr. Der Debug-Level
//! `-d` schaltet von `info` auf `debug`; laute Bibliotheken (hyper, h2 …)
//! bleiben eine Stufe darunter. `RUST_LOG` überschreibt den Filter.
//!
//! Datenfluss: `main` → [`init`] → globaler `tracing`-Subscriber → stderr
//! (unter systemd ins Journal).
//!
//! Bareos-Gegenstück: `lib/message.cc` (`InitMsg`, `debug_level`) – Bareos
//! hat eigene Debug-Ausgaben; die Bareos-kompatible `Dmsg`-Ausgabe der
//! Daemons liegt in `bareos_bnet::debug` und bleibt dort.

/// The default filter: `debug` from debug level 1 on, else `info`; the
/// crates in `quiet` one step lower (`info` resp. `warn`).
pub fn default_filter(debug_level: u32, quiet: &[&str]) -> String {
    let (main, lower) = if debug_level > 0 { ("debug", "info") } else { ("info", "warn") };
    let mut filter = main.to_string();
    for name in quiet {
        filter.push_str(&format!(",{name}={lower}"));
    }
    filter
}

/// Installs the global subscriber: `RUST_LOG` if set, else `filter`.
pub fn init(filter: &str) {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ofd_filters_are_unchanged() {
        // ofd/src/main.rs before 19.11.2026
        assert_eq!(default_filter(100, &["hyper", "h2"]), "debug,hyper=info,h2=info");
        assert_eq!(default_filter(0, &["hyper", "h2"]), "info,hyper=warn,h2=warn");
    }
}
