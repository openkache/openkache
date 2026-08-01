use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client_directory = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?,
    );
    let generator = client_directory.join("../generate.ts");
    let protocol_wire_generator = client_directory.join("../../protocol/wire.ts");
    let model = client_directory.join("../model");
    let protocol_model = client_directory.join("../../protocol/model");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?)
        .join("smithy_api.rs");

    println!("cargo:rerun-if-changed={}", generator.display());
    println!(
        "cargo:rerun-if-changed={}",
        protocol_wire_generator.display()
    );
    println!("cargo:rerun-if-changed={}", model.display());
    println!("cargo:rerun-if-changed={}", protocol_model.display());

    let status = Command::new("bun")
        .arg(&generator)
        .env("OPENKACHE_GENERATION_TARGET", "rust-api")
        .env("OPENKACHE_RUST_API_OUTPUT", &output)
        .status()
        .map_err(|error| {
            format!(
                "Smithy API generation could not start Bun: {error}\n\
                 Install Bun and Smithy CLI, ensure both are on PATH, then rerun Cargo."
            )
        })?;
    if !status.success() {
        return Err(format!(
            "Smithy API generation failed with {status}\n\
             Run `./generate.ts` from the clients directory for actionable diagnostics."
        )
        .into());
    }
    Ok(())
}
