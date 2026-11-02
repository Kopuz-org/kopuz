//! Compiles the contract with `protox` (a pure-Rust protobuf compiler), so
//! building Kopuz never requires a system `protoc`: CI, the nix sandbox,
//! and contributor machines all work with cargo alone.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use prost::Message;
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptors = protox::compile(["proto/kopuz.proto"], ["proto"])?;
    std::fs::write(out_dir.join("kopuz.bin"), descriptors.encode_to_vec())?;
    tonic_build::configure().compile_fds(descriptors)?;
    println!("cargo:rerun-if-changed=proto/kopuz.proto");
    Ok(())
}
