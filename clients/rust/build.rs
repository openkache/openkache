use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client_directory = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?,
    );
    let generator = client_directory.join("../generate.ts");
    let generator_modules = client_directory.join("../generator");
    let api_shape_renderers = client_directory.join("../api_shape_renderers.ts");
    let operation_client_projection = client_directory.join("../operation_client_projection.ts");
    let compatibility_response_framing =
        client_directory.join("../compatibility_response_framing.ts");
    let protocol_wire_generator = client_directory.join("../../protocol/wire.ts");
    let protocol_wire_modules = client_directory.join("../../protocol/wire");
    let protocol_wire_descriptor = client_directory.join("../../protocol/wire_descriptor.ts");
    let protocol_wire_layout = client_directory.join("../../protocol/wire_layout.ts");
    let protocol_wire_types = client_directory.join("../../protocol/wire_types.ts");
    let protocol_compatibility_renderer =
        client_directory.join("../../protocol/compatibility_v1_renderer.ts");
    let protocol_wire_spec_renderer = client_directory.join("../../protocol/wire_spec.ts");
    let model = client_directory.join("../model");
    let protocol_model = client_directory.join("../../protocol/model");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?)
        .join("smithy_api.rs");
    let operations_output =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?)
            .join("smithy_operations.rs");

    println!("cargo:rerun-if-changed={}", generator.display());
    println!("cargo:rerun-if-changed={}", generator_modules.display());
    println!("cargo:rerun-if-changed={}", api_shape_renderers.display());
    println!(
        "cargo:rerun-if-changed={}",
        operation_client_projection.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        compatibility_response_framing.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        protocol_wire_generator.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        protocol_wire_modules.display()
    );
    for dependency in [
        &protocol_wire_descriptor,
        &protocol_wire_layout,
        &protocol_wire_types,
    ] {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        protocol_compatibility_renderer.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        protocol_wire_spec_renderer.display()
    );
    println!("cargo:rerun-if-changed={}", model.display());
    println!("cargo:rerun-if-changed={}", protocol_model.display());

    let bun = std::env::var_os("OPENKACHE_BUN_EXECUTABLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bun"));
    let status = Command::new(bun)
        .arg(&generator)
        .env("OPENKACHE_GENERATION_TARGET", "rust-api")
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
