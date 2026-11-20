//! Adds an rpath to the SDK libraries for this crate's tests and examples, and passes
//! the library directory on to dependents as `DEP_ONBOARDAPI_RS_LIB_DIR`.

fn main() {
    let Ok(lib_dir) = std::env::var("DEP_ONBOARDAPI_LIB_DIR") else {
        return;
    };
    println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{lib_dir}");
    println!("cargo:rustc-link-arg-examples=-Wl,-rpath,{lib_dir}");
    println!("cargo:lib_dir={lib_dir}");
}
