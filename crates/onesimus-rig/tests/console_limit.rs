// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `Maximum Console Connections` with real console connections (S-44).
//!
//! Start: `cargo test -p onesimus-rig --test console_limit -- --ignored`.

#![deny(missing_docs)]

use std::net::SocketAddr;
use std::time::Duration;

use onesimus_rig::console::Console;
use onesimus_rig::{Rig, RigOptions};

/// With `Maximum Console Connections = 2` and two consoles open, a third default console gets through
/// (with an info message in the log) and a named console is refused; after the others closed, the named
/// console works again (`dird/authenticate.cc`, `console_connection_lease.h`).
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn named_console_is_refused_over_the_limit_and_the_default_console_is_not() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39981);
    options.extra_director = vec![(
        "console/limited.conf".into(),
        "Console {\n  Name = limited\n  Password = \"limited-pw\"\n  Command ACL = version\n}\n".into(),
    )];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let file = rig.layout.dir_conf.join("director").join("bareos-dir.conf");
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&file, text.replacen("\n}\n", "\n  Maximum Console Connections = 2\n}\n", 1)).unwrap_or_else(|e| panic!("{e}"));
    let reply = rig.command("reload").unwrap_or_else(|e| panic!("{e}"));
    assert!(!reply.contains("failed"), "reload: {reply}");

    let address = SocketAddr::from(([127, 0, 0, 1], rig.ports.dir));
    let named = || Console::connect_as(address, "limited", "limited-pw");
    assert!(named().is_ok(), "below the limit the named console works");

    let mut first = rig.console().unwrap_or_else(|e| panic!("{e}"));
    let mut second = rig.console().unwrap_or_else(|e| panic!("{e}"));
    // both are really open: they answer
    assert!(first.command("version", Duration::from_secs(10)).is_ok());
    assert!(second.command("version", Duration::from_secs(10)).is_ok());
    let mut third = rig.console().unwrap_or_else(|e| panic!("the default console is let through: {e}"));
    assert!(third.command("version", Duration::from_secs(10)).is_ok());
    assert!(named().is_err(), "a named console over the limit is refused\n{}", rig.daemon_log("odir"));
    assert!(rig.daemon_log("odir").contains("Number of console connections exceeded Maximum :2"), "{}", rig.daemon_log("odir"));

    drop((first, second, third));
    std::thread::sleep(Duration::from_millis(500));
    assert!(named().is_ok(), "after the others closed the named console works again");
}
