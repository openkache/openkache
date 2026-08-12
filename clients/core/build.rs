// Pure-Rust dependency gate for `openkache-client-core`.
//
// The shared client implementation must not require a C or C++ build toolchain. This keeps
// supported cross-compilation targets and language adapters independent of native build systems.
// Runtime and target support are still determined by the selected QUIC backend; this gate makes no
// `no_std`, WebAssembly, or universal-target claim.
//
// If a dependency pulls in `libc`, `cxx`, `bindgen`, `cmake`,
// `pkg-config`, `vcpkg`, or any `napi`/`napi-derive` crate, the
// client's "pure Rust" property is violated and the build fails with a clear,
// actionable error message.
//
// ## Detection mechanism
//
// The script parses `Cargo.toml` as plain text (not TOML, to keep the
// dependency footprint itself at zero) and scans each non-comment,
// non-section-header line for known native-dependency names.  Any match
// triggers a hard build failure before Cargo proceeds to dependency resolution.
//
// ## What this does NOT cover
//
// Transitive dependencies are not checked here — Cargo itself resolves
// those.  This gate only catches *direct* dependencies of the client
// crate that are known to introduce native code.  Developers should
// also audit transitive dependency trees with `cargo tree` when adding
// new dependencies.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read the real `Cargo.toml` from the crate root.
    let manifest_directory =
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?;
    let manifest_path = std::path::Path::new(&manifest_directory).join("Cargo.toml");
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;

    // Known crates that either ARE native code or REQUIRE a C/C++
    // toolchain to build.  Keep this list sorted alphabetically.
    let forbidden = [
        "bindgen",     // Generates FFI bindings from C headers.
        "cmake",       // Builds native libraries via CMake.
        "cxx",         // C++/Rust interop — requires a C++ compiler.
        "cxxbridge",   // CXX bridge macros — same requirement as `cxx`.
        "libc",        // Direct FFI to C's libc — links libc.
        "napi",        // Node-API bindings belong in runtime-specific adapters.
        "napi-derive", // Procedural macros for napi — pulls in napi.
        "pkg-config",  // Locates system libraries via pkg-config.
        "vcpkg",       // Microsoft C++ package manager integration.
    ];

    // Scan every line of Cargo.toml.  Lines starting with `#` or `[`
    // are comments/sections and are skipped.
    //
    // A dependency is detected when a line starts with `<name> =`
    // (e.g. `libc = "0.2"`) or `<name>.` (e.g. `libc.features = [...]`).
    // This is a conservative heuristic — it will never produce false
    // negatives and may produce false positives for crates whose names
    // happen to match, but that is acceptable for a safety gate.
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with('[') {
            continue;
        }
        for dep in &forbidden {
            if trimmed.starts_with(&format!("{dep} =")) || trimmed.starts_with(&format!("{dep}.")) {
                return Err(format!(
                    "\n❌ C dependency detected in Cargo.toml: '{dep}'\n\
                     The `openkache-client-core` crate must remain pure Rust.\n\
                     Remove '{dep}' or find a pure-Rust alternative.\n"
                )
                .into());
            }
        }
    }

    let core_directory = std::path::Path::new(&manifest_directory);
    let checkout_clients_directory = core_directory
        .parent()
        .ok_or("client core manifest has no parent directory")?;
    // Source checkouts keep this crate under `clients/core`. Python sdists
    // flatten the shared sources to `core/` and place the generator/model
    // under a sibling `clients/` directory, so resolve both supported layouts
    // without making package builds copy a second model.
    let client_directory = if checkout_clients_directory.join("generate.ts").is_file() {
        checkout_clients_directory.to_path_buf()
    } else {
        let flattened_clients_directory = core_directory
            .parent()
            .map(|root| root.join("clients"))
            .ok_or("client core manifest has no flattened sdist root")?;
        if flattened_clients_directory.join("generate.ts").is_file() {
            flattened_clients_directory
        } else {
            return Err(format!(
                "client generator is missing from {} and {}",
                checkout_clients_directory.display(),
                flattened_clients_directory.display()
            )
            .into());
        }
    };
    let generator = client_directory.join("generate.ts");
    let generator_modules = client_directory.join("generator");
    let api_shape_renderers = client_directory.join("api_shape_renderers.ts");
    let operation_client_projection = client_directory.join("operation_client_projection.ts");
    let protocol_wire_generator = client_directory.join("../protocol/wire.ts");
    let protocol_wire_modules = client_directory.join("../protocol/wire");
    let protocol_wire_descriptor = client_directory.join("../protocol/wire_descriptor.ts");
    let protocol_wire_layout = client_directory.join("../protocol/wire_layout.ts");
    let protocol_wire_types = client_directory.join("../protocol/wire_types.ts");
    let protocol_compatibility_renderer =
        client_directory.join("../protocol/compatibility_v1_renderer.ts");
    let protocol_wire_spec_renderer = client_directory.join("../protocol/wire_spec.ts");
    let client_model = client_directory.join("model");
    let protocol_model = client_directory.join("../protocol/model");
    let output_directory = std::path::PathBuf::from(
        std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?,
    );
    let output = output_directory.join("client_contract.rs");
    let operation_constants_output = output_directory.join("operation_constants.rs");

    println!("cargo:rerun-if-changed={}", generator.display());
    println!("cargo:rerun-if-changed={}", generator_modules.display());
    println!("cargo:rerun-if-changed={}", api_shape_renderers.display());
    println!(
        "cargo:rerun-if-changed={}",
        operation_client_projection.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        protocol_wire_generator.display()
    );
    println!("cargo:rerun-if-changed={}", protocol_wire_modules.display());
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
    println!("cargo:rerun-if-changed={}", client_model.display());
    println!("cargo:rerun-if-changed={}", protocol_model.display());

    let bun = std::env::var_os("OPENKACHE_BUN_EXECUTABLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("bun"));
    let status = std::process::Command::new(bun)
        .arg(&generator)
        .env("OPENKACHE_GENERATION_TARGET", "rust-client")
        .env("OPENKACHE_RUST_CLIENT_OUTPUT", &output)
        .env(
            "OPENKACHE_RUST_OPERATION_CONSTANTS_OUTPUT",
            &operation_constants_output,
        )
        .status()
        .map_err(|error| {
            format!(
                "client contract generation could not start Bun: {error}\n\
                 Install Bun and Smithy CLI, ensure both are on PATH, then rerun Cargo."
            )
        })?;
    if !status.success() {
        return Err(format!(
            "client contract generation failed with {status}\n\
             Run `./generate.ts` from the clients directory for actionable diagnostics."
        )
        .into());
    }
    Ok(())
}
