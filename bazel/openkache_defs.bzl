"""Macros shared by the public OpenKache Bazel package."""

load("@crates//:defs.bzl", "aliases", "all_crate_deps", "crate_deps")

def cargo_deps(package_name):
    """Return normal and proc-macro dependencies for a Cargo package."""
    return all_crate_deps(
        normal = True,
        proc_macro = True,
        package_name = package_name,
    )

def public_crate_deps(crate_names, package_name):
    """Return named public crates, including conditional Cargo dependencies."""
    return crate_deps(crate_names, package_name = package_name)

def normal_cargo_deps(package_name):
    """Return only normal dependencies for a Cargo package."""
    return all_crate_deps(
        normal = True,
        package_name = package_name,
    )

def build_deps(package_name):
    """Return build-script and build proc-macro dependencies for a Cargo package."""
    return all_crate_deps(
        build = True,
        build_proc_macro = True,
        package_name = package_name,
    )

def proc_macro_deps(package_name):
    """Return only procedural-macro dependencies for a Cargo package."""
    return all_crate_deps(
        proc_macro = True,
        package_name = package_name,
    )

def crate_aliases(package_name):
    """Return the crate-name aliases for a Cargo package."""
    return aliases(
        normal = True,
        proc_macro = True,
        package_name = package_name,
    )

def crate_aliases_for(package_names):
    """Merge aliases for several public Cargo packages."""
    merged = {}
    for package_name in package_names:
        merged.update(crate_aliases(package_name))
    return merged

def dedupe_labels(labels):
    """Preserve order while removing duplicate dependency labels."""
    seen = {}
    result = []
    for label in labels:
        if label not in seen:
            seen[label] = True
            result.append(label)
    return result

def rust_metadata(name, version = "0.1.0"):
    """Return Cargo package metadata consumed by source-level env! calls."""
    return {
        "CARGO_PKG_NAME": name,
        "CARGO_PKG_VERSION": version,
    }
