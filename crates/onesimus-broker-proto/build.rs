// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Generates the Rust types of the three broker protos (messages only, no services).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = "../../proto";
    let protos = ["postgresql_broker.proto", "openldap_broker.proto", "podman_broker.proto"];
    tonic_build::configure()
        .build_server(false)
        .build_client(false)
        .compile_protos(&protos.map(|p| format!("{proto_dir}/{p}")), &[proto_dir])?;
    for p in protos {
        println!("cargo:rerun-if-changed={proto_dir}/{p}");
    }
    Ok(())
}
