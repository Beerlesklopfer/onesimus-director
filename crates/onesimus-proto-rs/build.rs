fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = "../../proto";
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                &format!("{proto_dir}/director_service.proto"),
                &format!("{proto_dir}/director_commands.proto"),
                &format!("{proto_dir}/plugin_ccc.proto"),
            ],
            &[proto_dir],
        )?;

    // Typed API in a separate pass: only here RPCs without an implementation
    // answer UNIMPLEMENTED (write RPCs until S-35); the Director service
    // keeps its required methods and stream types.
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .generate_default_stubs(true)
        // Descriptors (with imports) for the typed API codec (typed.rs)
        .file_descriptor_set_path(out_dir.join("onesimus_descriptor.bin"))
        .compile_protos(&[format!("{proto_dir}/director_api.proto")], &[proto_dir])?;

    // Rebuild if protos change
    println!("cargo:rerun-if-changed={proto_dir}/director_service.proto");
    println!("cargo:rerun-if-changed={proto_dir}/director_commands.proto");
    println!("cargo:rerun-if-changed={proto_dir}/director_api.proto");
    println!("cargo:rerun-if-changed={proto_dir}/onesimus_options.proto");
    println!("cargo:rerun-if-changed={proto_dir}/plugin_ccc.proto");

    Ok(())
}
