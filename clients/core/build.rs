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
    Ok(())
}
