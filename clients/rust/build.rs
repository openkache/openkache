fn main() {
    let content = std::fs::read_to_string(
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("Cargo.toml"),
    )
    .expect("Cargo.toml not found");

    let forbidden = [
        "libc", "napi", "napi-derive", "cxx", "cxxbridge", "bindgen", "cmake", "pkg-config",
        "vcpkg",
    ];

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with('[') {
            continue;
        }
        for dep in &forbidden {
            if trimmed.starts_with(&format!("{dep} =")) || trimmed.starts_with(&format!("{dep}."))
            {
                panic!(
                    "\n❌ C dependency detected in Cargo.toml: '{dep}'\n\
                     The `openkache-client` crate must remain pure Rust.\n\
                     Remove '{dep}' or find a pure-Rust alternative.\n"
                );
            }
        }
    }
}
