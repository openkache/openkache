use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protocol_directory = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?,
    );
    let generator = protocol_directory.join("generate.ts");
    let wire_generator = protocol_directory.join("wire.ts");
    let model = protocol_directory.join("model");
    let output_directory =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?);
    let output = output_directory.join("wire_values.rs");

    println!("cargo:rerun-if-changed={}", generator.display());
    println!("cargo:rerun-if-changed={}", wire_generator.display());
    println!(
        "cargo:rerun-if-changed={}",
        protocol_directory.join("wire").display()
    );
    for dependency in ["wire_types.ts", "wire_spec.ts"] {
        println!(
            "cargo:rerun-if-changed={}",
            protocol_directory.join(dependency).display()
        );
    }
    println!("cargo:rerun-if-changed={}", model.display());

    let bun = std::env::var_os("OPENKACHE_BUN_EXECUTABLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bun"));
    let status = Command::new(bun)
        .arg(&generator)
        .env("OPENKACHE_GENERATION_TARGET", "rust-wire")
        .env("OPENKACHE_RUST_WIRE_OUTPUT", &output)
        .status()
        .map_err(|error| {
            format!(
                "protocol generation could not start Bun: {error}\n\
                 Install Bun and Smithy CLI, ensure both are on PATH, then rerun Cargo."
            )
        })?;
    if !status.success() {
        return Err(format!(
            "protocol generation failed with {status}\n\
             Run `./generate.ts` from the protocol directory for actionable diagnostics."
        )
        .into());
    }
    Ok(())
}
