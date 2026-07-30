use std::path::PathBuf;
use std::process::Command;

fn main() {
    let client_directory = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo always sets CARGO_MANIFEST_DIR"),
    );
    let generator = client_directory.join("../../protocol/generate.ts");
    let model = client_directory.join("../../protocol/model");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo always sets OUT_DIR"))
        .join("smithy_api.rs");

    println!("cargo:rerun-if-changed={}", generator.display());
    println!("cargo:rerun-if-changed={}", model.display());

    let status = Command::new("bun")
        .arg(&generator)
        .env("OPENKACHE_GENERATION_TARGET", "rust-api")
        .env("OPENKACHE_RUST_API_OUTPUT", &output)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "Smithy API generation could not start Bun: {error}\n\
                 Install Bun and Smithy CLI, ensure both are on PATH, then rerun Cargo."
            )
        });
    assert!(
        status.success(),
        "Smithy API generation failed with {status}\n\
         Run `./generate.ts` from the protocol directory for actionable diagnostics."
    );
}
