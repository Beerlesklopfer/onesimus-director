// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `cargo run -p smartread --example smartread -- /dev/sda`: prints what the drive reports.

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: smartread <device>   (needs CAP_SYS_RAWIO / root)");
        std::process::exit(2);
    };
    // `smartread <device> --scsi` reads through the SCSI log pages only
    let scsi = std::env::args().nth(2).as_deref() == Some("--scsi");
    #[cfg(target_os = "linux")]
    let result = if scsi { smartread::read_scsi(&path) } else { smartread::read(&path) };
    #[cfg(not(target_os = "linux"))]
    let result = {
        let _ = scsi;
        smartread::read(&path)
    };
    match result {
        Ok(r) => {
            println!("{} {} (serial {}, firmware {}), {}", path, r.model, r.serial, r.firmware, r.protocol);
            println!("health: {:?}", r.health);
            println!("temperature: {:?} °C, power-on hours: {:?}", r.temperature, r.power_on_hours);
            if let Some(t) = &r.self_test {
                println!("self test: {} ({})", t.status, t.text());
            }
            for a in &r.attributes {
                println!(
                    "{:>3} {:<26} flags {:#06x} value {:>3} worst {:>3} thresh {:>3} raw {}{}",
                    a.id,
                    a.name().unwrap_or("-"),
                    a.flags,
                    a.value,
                    a.worst,
                    a.threshold.map_or("-".to_string(), |t| t.to_string()),
                    a.raw,
                    if a.failing() { "  FAILING" } else { "" }
                );
            }
            if let Some(n) = &r.nvme {
                println!("{n:#?}");
            }
            if let Some(s) = &r.scsi {
                println!("{s:#?}");
            }
        }
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
}
