use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server_directory = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?,
    );
    let protocol_directory = server_directory.join("../protocol");
    let generator = protocol_directory.join("generate.ts");
    let model = protocol_directory.join("model");
    let wire_generator = protocol_directory.join("wire.ts");
    let wire_spec_renderer = protocol_directory.join("wire_spec.ts");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?)
        .join("server_contract.rs");

    println!("cargo:rerun-if-changed={}", generator.display());
    println!("cargo:rerun-if-changed={}", wire_generator.display());
    println!(
        "cargo:rerun-if-changed={}",
        protocol_directory.join("wire").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        protocol_directory
            .join("compatibility_v1_renderer.ts")
            .display()
    );
    println!("cargo:rerun-if-changed={}", wire_spec_renderer.display());
    println!("cargo:rerun-if-changed={}", model.display());

    let bun = std::env::var_os("OPENKACHE_BUN_EXECUTABLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bun"));
    let status = Command::new(bun)
        .arg(&generator)
        .env("OPENKACHE_GENERATION_TARGET", "rust-server")
        .env("OPENKACHE_RUST_SERVER_OUTPUT", &output)
        .env(
            "OPENKACHE_SMITHY_EXECUTABLE",
            std::env::var_os("OPENKACHE_SMITHY_EXECUTABLE").unwrap_or_else(|| "smithy".into()),
        )
        .env(
            "OPENKACHE_SMITHY_USE_SHELL",
            std::env::var_os("OPENKACHE_SMITHY_USE_SHELL").unwrap_or_default(),
        )
        .status()
        .map_err(|error| {
            format!(
                "server operation-contract generation could not start Bun: {error}\n\
                 Install Bun and Smithy CLI, ensure both are on PATH, then rerun the server build."
            )
        })?;
    if !status.success() {
        return Err(format!("server operation-contract generation failed with {status}").into());
    }
    Ok(())
}
