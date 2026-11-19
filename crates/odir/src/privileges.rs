//! `-u <user>` / `-g <group>`: drop root privileges (like C++ lib/bsys drop()).
//!
//! The implementation moved unchanged to `onesimus_daemon::privileges`
//! (19.11.2026), shared with ofd and the coming osd. Must run while the
//! process is still single-threaded and before any database connection.

pub use onesimus_daemon::privileges::drop_privileges;
