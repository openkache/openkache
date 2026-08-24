use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn generated_output_paths(out_dir: impl AsRef<Path>) -> (PathBuf, PathBuf) {
    let out_dir = out_dir.as_ref();
    (
        out_dir.join("smithy_api.rs"),
        out_dir.join("smithy_operations.rs"),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client_directory = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?,
    );
    let generator = client_directory.join("../generate.ts");
    let generator_sources = client_directory.join("../generator");
    let operation_projection = client_directory.join("../operation_client_projection.ts");
    let protocol_wire_generator = client_directory.join("../../protocol/wire.ts");
    let model = client_directory.join("../model");
    let protocol_model = client_directory.join("../../protocol/model");
    let generated_output =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?);
    let (output, operations_output) = generated_output_paths(&generated_output);
    let packaged_snapshot = client_directory.join("src/contract_snapshot");

    println!("cargo:rerun-if-changed={}", generator.display());
    println!("cargo:rerun-if-changed={}", generator_sources.display());
    println!("cargo:rerun-if-changed={}", operation_projection.display());
    println!(
        "cargo:rerun-if-changed={}",
        protocol_wire_generator.display()
    );
    println!("cargo:rerun-if-changed={}", model.display());
    println!("cargo:rerun-if-changed={}", protocol_model.display());
    println!("cargo:rerun-if-changed={}", packaged_snapshot.display());

    // A crates.io package contains the client source, but not the sibling
    // workspace that owns the Smithy generator and its models. Keep a
    // generated snapshot in the package so docs.rs and downstream Cargo
    // builds do not need Bun or Smithy installed. Source checkouts still
    // regenerate from the canonical inputs above.
    if !generator.is_file()
        || !generator_sources.is_dir()
        || !protocol_wire_generator.is_file()
        || !model.is_dir()
        || !protocol_model.is_dir()
    {
        for (name, destination) in [
            ("smithy_api.rs", &output),
            ("smithy_operations.rs", &operations_output),
        ] {
            let source = packaged_snapshot.join(name);
            if !source.is_file() {
                return Err(
                    format!("generated Smithy fallback is missing: {}", source.display()).into(),
                );
            }
            std::fs::copy(&source, destination).map_err(|error| {
                format!(
                    "could not copy packaged Smithy fallback {} to {}: {error}",
                    source.display(),
                    destination.display()
                )
            })?;
        }
        return Ok(());
    }

    let bun = std::env::var_os("OPENKACHE_BUN_EXECUTABLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bun"));
    let status = Command::new(bun)
        .arg(&generator)
        .env("OPENKACHE_GENERATION_TARGET", "rust-api")
        // Keep every target selected by the Rust generator inside Cargo's
        // build-owned output tree. The per-file overrides below preserve the
        // include! paths while preventing a future rust-api output from
        // falling back to the immutable source checkout.
        .env("OPENKACHE_GENERATION_OUTPUT_ROOT", &generated_output)
        .env("OPENKACHE_RUST_API_OUTPUT", &output)
        .env("OPENKACHE_RUST_OPERATIONS_OUTPUT", &operations_output)
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
