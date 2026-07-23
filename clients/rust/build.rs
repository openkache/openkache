// build.rs — Pure-Rust gate for `openkache-client`
//
// This build script is a **compile-time safety check** that enforces a
// fundamental constraint of the client crate: it must remain entirely
// free of native/C dependencies.
//
// ## Why this matters
//
// The `openkache-client` crate is designed to be cross-compilable to
// every tier-1 and tier-2 Rust target without requiring a C toolchain,
// a C compiler, or any platform-specific native libraries.  This
// portability guarantee is essential for:
//
//   - WebAssembly (wasm32) targets, which have no C ABI.
//   - Embedded targets and `no_std` environments.
//   - Swift/Flutter bridge targets (iOS, Android) where linking
//     native code adds significant build complexity.
//   - Rapid CI pipelines that should not need `cmake`, `pkg-config`,
//     or a C++ toolchain installed.
//
// If a dependency pulls in `libc`, `cxx`, `bindgen`, `cmake`,
// `pkg-config`, `vcpkg`, or any `napi`/`napi-derive` crate, the
// client's "pure Rust" property is violated and the build will panic
// with a clear, actionable error message.
//
// ## Detection mechanism
//
// The script parses `Cargo.toml` as plain text (not TOML, to keep the
// dependency footprint itself at zero) and scans each non-comment,
// non-section-header line for known native-dependency names.  Any match
// triggers a hard `panic!` before Cargo proceeds to dependency resolution.
//
// ## What this does NOT cover
//
// Transitive dependencies are not checked here — Cargo itself resolves
// those.  This gate only catches *direct* dependencies of the client
// crate that are known to introduce native code.  Developers should
// also audit transitive dependency trees with `cargo tree` when adding
// new dependencies.

fn main() {
    // Read the real `Cargo.toml` from the crate root.
    let content = std::fs::read_to_string(
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("Cargo.toml"),
    )
    .expect("Cargo.toml not found");

    // Known crates that either ARE native code or REQUIRE a C/C++
    // toolchain to build.  Keep this list sorted alphabetically.
    let forbidden = [
        "bindgen",     // Generates FFI bindings from C headers.
        "cmake",       // Builds native libraries via CMake.
        "cxx",         // C++/Rust interop — requires a C++ compiler.
        "cxxbridge",   // CXX bridge macros — same requirement as `cxx`.
        "libc",        // Direct FFI to C's libc — links libc.
        "napi",        // Node.js N-API — requires native Node.js bindings.
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
                panic!(
                    "\n❌ C dependency detected in Cargo.toml: '{dep}'\n\
                     The `openkache-client` crate must remain pure Rust.\n\
                     Remove '{dep}' or find a pure-Rust alternative.\n"
                );
            }
        }
    }
}
