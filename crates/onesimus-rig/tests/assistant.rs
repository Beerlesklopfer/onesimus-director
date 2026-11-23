// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Claims of the website page "AI assistants (MCP)" checked against a
//! running Director (docs/wiki/mcp.md): which commands an `*-mcp` console
//! may run, whatever its ACLs say, over the classic console protocol with a
//! client that is not oconsole (a bconsole-style session of the rig).
//!
//! Start: `cargo test -p onesimus-rig --test assistant -- --ignored --nocapture`

#![deny(missing_docs)]

use std::net::SocketAddr;
use std::time::Duration;

use onesimus_rig::console::Console;
use onesimus_rig::{Rig, RigOptions};

/// The assistant profile as shipped.
const PROFILE: &str = include_str!("../../../config/bareos-dir.d/profile/assistant.conf");

/// Runs `command` as console `name` and returns the reply (or the error).
fn ask(rig: &Rig, name: &str, command: &str) -> String {
    let address = SocketAddr::from(([127, 0, 0, 1], rig.ports.dir));
    match Console::connect_as(address, name, "pw") {
        Ok(mut console) => console.command(command, Duration::from_secs(20)).unwrap_or_else(|e| format!("ERROR {e}")),
        Err(e) => format!("LOGIN FAILED {e}"),
    }
}

/// All the claims, one line per command and console.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored --nocapture"]
fn assistant_claims() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39591);
    let acls = "JobACL=*all*; ClientACL=*all*; StorageACL=*all*; PoolACL=*all*; FileSetACL=*all*; CatalogACL=*all*; ScheduleACL=*all*; WhereACL=*all*";
    options.extra_director = vec![
        ("profile/assistant.conf".into(), PROFILE.into()),
        ("console/admin-mcp.conf".into(), "Console { Name = admin-mcp; Password = pw; Profile = assistant }\n".into()),
        ("console/Wide-MCP.conf".into(), format!("Console {{ Name = Wide-MCP; Password = pw; CommandACL = *all*; {acls} }}\n")),
        ("console/plain.conf".into(), format!("Console {{ Name = plain; Password = pw; CommandACL = *all*; {acls} }}\n")),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let mut table = String::new();
    let commands = [
        ".acl list", ".profile list", ".profile create name=x", "sqlquery", ".sql", "query", "configure add console name=z password=p",
        "status dir", "list jobs", "show jobs", "version", "whoami", ".jobs", ".bvfs_lsdirs jobid=1 path=",
        "run job=VBackup yes", "restore", "purge jobid=999", "delete jobid=999", "cancel jobid=999", "label", "reload", "messages",
    ];
    let more = [
        "configure add console name=z password=p profile=assistant",
        "show consoles",
        "show director",
        "show clients",
        "show filesets",
        "configure add job name=evil type=Backup messages=Standard pool=VPool client=vfd fileset=VSet storage=vsd runbeforejob=\"touch /tmp/assistant-was-here\"",
        "configure update job=VBackup runbeforejob=\"touch /tmp/assistant-was-here\"",
        "configure add schedule name=evilsched run=\"Full daily at 23:00\"",
        ".profile",
        ".profiles",
    ];
    for name in ["admin-mcp", "Wide-MCP"] {
        for command in more {
            let reply = ask(&rig, name, command);
            let first = reply.lines().filter(|l| !l.trim().is_empty()).take(3).collect::<Vec<_>>().join(" / ").chars().take(260).collect::<String>();
            table += &format!("{name:10} | {command:60.60} | {first}\n");
        }
    }
    let shown = ask(&rig, "admin-mcp", "show director");
    table += &format!("show director contains the password 'pw': {}\n", shown.contains("pw"));
    let evil = rig.layout.dir_conf.join("job").join("evil.conf").exists();
    table += &format!("job evil on disk: {evil}\n");
    let changed = std::fs::read_to_string(rig.layout.dir_conf.join("job").join("VBackup.conf")).unwrap_or_default();
    table += &format!("VBackup got a Run Before Job: {}\n", changed.to_ascii_lowercase().contains("runbeforejob"));
    assert!(!evil, "an assistant console stored a Job with Run Before Job\n{table}");
    assert!(!changed.to_ascii_lowercase().contains("runbeforejob"), "an assistant console changed a Job to run a program\n{table}");
    let commands: [&str; 0] = [];
    for name in ["admin-mcp", "Wide-MCP", "plain"] {
        for command in commands {
            let reply = ask(&rig, name, command);
            let first = reply.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(110).collect::<String>();
            table += &format!("{name:10} | {command:42} | {first}\n");
        }
    }
    eprintln!("\n{table}");
    std::fs::write(std::env::temp_dir().join("assistant-claims.txt"), &table).ok();
}
