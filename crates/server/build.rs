fn main() {
    build_widevine_shim();
}

/// Compile the C++ host for the system Widevine CDM (see `shim/widevine_shim.cc`).
///
/// Android is excluded: it has no loadable `libwidevinecdm`, exposing Widevine
/// only through the `MediaDrm` Java API, so that target takes a separate path
/// and never links this shim.
fn build_widevine_shim() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "android" {
        return;
    }

    println!("cargo:rerun-if-changed=shim/widevine_shim.cc");
    println!("cargo:rerun-if-changed=vendor/content_decryption_module.h");

    cc::Build::new()
        .cpp(true)
        .file("shim/widevine_shim.cc")
        .include("vendor")
        .flag_if_supported("-std=c++14")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("widevine_shim");

    match target_os.as_str() {
        "macos" | "ios" => println!("cargo:rustc-link-lib=dylib=c++"),
        "windows" => {}
        _ => {
            println!("cargo:rustc-link-lib=dylib=stdc++");
            println!("cargo:rustc-link-lib=dylib=dl");
        }
    }
}
