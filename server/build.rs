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
    let output_directory = PathBuf::from(
        std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?,
    );
    let output = output_directory.join("server_contract.rs");
    let packaged_snapshot = server_directory.join("src/contract_snapshot");

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
    println!("cargo:rerun-if-changed={}", packaged_snapshot.display());

    // A crates.io package contains only the server subtree, so the repository
    // protocol generator and Smithy model are intentionally absent. Keep
    // checkout builds hermetic to the canonical generator while allowing
    // registry and docs.rs builds to consume this immutable contract snapshot.
    if !generator.is_file()
        || !wire_generator.is_file()
        || !wire_spec_renderer.is_file()
        || !model.is_dir()
    {
        let source = packaged_snapshot.join("server_contract.rs");
        if !source.is_file() {
            return Err(format!(
                "generated server fallback is missing: {}",
                source.display()
            )
            .into());
        }
        std::fs::copy(&source, &output).map_err(|error| {
            format!(
                "could not copy packaged server fallback {} to {}: {error}",
                source.display(),
                output.display()
            )
        })?;
        return Ok(());
    }

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
    normalize_protocol_paths(&output)?;
    Ok(())
}

/// Rewrites the generated adapter's historical protocol-crate paths to the
/// package-local wire module. The canonical renderer is shared with clients,
/// so keeping this normalization at the server build boundary avoids a
/// server-only fork of the Smithy renderer.
fn normalize_protocol_paths(output: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(output)?;
    let normalized = source.replace("openkache_protocol::", "crate::openkache_protocol::");
    if normalized != source {
        std::fs::write(output, normalized)?;
    }
    Ok(())
}
