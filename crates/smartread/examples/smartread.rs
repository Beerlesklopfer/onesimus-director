//! `cargo run -p smartread --example smartread -- /dev/sda`: prints what the drive reports.

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: smartread <device>   (needs CAP_SYS_RAWIO / root)");
        std::process::exit(2);
    };
    match smartread::read(&path) {
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
        }
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
}
