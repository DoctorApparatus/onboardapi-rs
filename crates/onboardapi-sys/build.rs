//! Compiles the C++ shim against the onboardapi SDK and links the SDK libraries.
//!
//! SDK location, first match wins:
//! 1. `ONBOARDAPI_SDK_DIR` (directory containing `include/` and `lib/`)
//! 2. `<workspace>/.sdk/<SDK_VERSION>/<arch>-linux` (what `just fetch-sdk` downloads)
//!
//! On docs.rs (`DOCS_RS` set) nothing is compiled or linked: the SDK is proprietary and
//! not available there, and rustdoc does not link.

use std::{
    env,
    path::{Path, PathBuf},
};

const LIBS: &[&str] = &["onboardapi-cyclone", "ddkit-cyclone", "ddscxx", "ddsc"];

/// onboardapi SDK release the shim was generated for. Independent of the crate version,
/// so wrapper-only releases (9.10.1, ...) still find the 9.10.0 SDK.
const SDK_VERSION: &str = "9.10.0";

fn main() {
    println!("cargo:rerun-if-env-changed=ONBOARDAPI_SDK_DIR");
    println!("cargo:rerun-if-env-changed=DOCS_RS");
    println!("cargo:rerun-if-changed=shim");
    if std::env::var_os("DOCS_RS").is_some() {
        return;
    }

    let sdk = sdk_dir();
    let include = sdk.join("include");
    let lib = sdk.join("lib");
    if !include.join("onboardapi").is_dir() || !lib.is_dir() {
        panic!(
            "onboardapi SDK not found at {} — run `just fetch-sdk` or set ONBOARDAPI_SDK_DIR",
            sdk.display()
        );
    }

    let shim = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo")).join("shim");
    let mut sources = vec![shim.join("obapi_runtime.cpp")];
    let mut generated: Vec<_> = std::fs::read_dir(shim.join("generated"))
        .expect("shim/generated exists — run `just codegen`")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "cpp"))
        .collect();
    generated.sort();
    sources.extend(generated);

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .files(&sources)
        .include(&include)
        .include(include.join("ddscxx"))
        .define("DDS_VENDOR_CYCLONE", None)
        // The SDK headers are not warning-clean; keep build output readable.
        .warnings(false)
        .flag_if_supported("-Wno-deprecated-declarations")
        .compile("onboardapi_shim");

    println!("cargo:rustc-link-search=native={}", lib.display());
    for l in LIBS {
        println!("cargo:rustc-link-lib=dylib={l}");
    }
    // Exposed to dependents as DEP_ONBOARDAPI_LIB_DIR / DEP_ONBOARDAPI_INCLUDE_DIR.
    println!("cargo:lib_dir={}", lib.display());
    println!("cargo:include_dir={}", include.display());
}

fn sdk_dir() -> PathBuf {
    if let Ok(dir) = env::var("ONBOARDAPI_SDK_DIR") {
        return PathBuf::from(dir);
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let arch = match env::var("CARGO_CFG_TARGET_ARCH")
        .unwrap_or_default()
        .as_str()
    {
        "aarch64" => "armv8".to_owned(),
        "arm" => "armv7".to_owned(),
        "" => "x86_64".to_owned(),
        other => other.to_owned(),
    };
    workspace_root(&manifest)
        .join(".sdk")
        .join(SDK_VERSION)
        .join(format!("{arch}-linux"))
}

fn workspace_root(manifest: &Path) -> PathBuf {
    manifest
        .ancestors()
        .find(|p| p.join(".sdk").is_dir() || p.join("datamodel").is_dir())
        .unwrap_or(manifest)
        .to_path_buf()
}
