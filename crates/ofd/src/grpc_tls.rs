//! TLS identity of the FD's gRPC listener: a self-signed certificate generated
//! once in the working directory, so its fingerprint stays stable for pinning
//! (daemon-crate `grpc_tls` since 20.11.2026, onesimus-bnet-grpc `tls`
//! since 21.11.2026).
use std::path::PathBuf;

/// File base name of the FD's certificate and key in the working directory.
const BASE: &str = "onesimus-fd-grpc";

pub fn load_or_generate(working_directory: &str, fd_name: &str) -> Result<(PathBuf, PathBuf), String> {
    onesimus_bnet_grpc::tls::load_or_generate(working_directory, BASE, fd_name)
}
