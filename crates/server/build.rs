fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(&["proto/wv.proto"], &["proto/"])?;
    build_widevine_shim();
    Ok(())
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
        // The vendored Chromium header declares far more of the interface than
        // this shim uses; unused parameters are inherent to implementing it.
        .flag_if_supported("-Wno-unused-parameter")
        .compile("widevine_shim");

    // The shim needs the C++ runtime, and dlopen on POSIX. MSVC links its own
    // runtime and resolves LoadLibrary from kernel32, so it needs neither.
    match target_os.as_str() {
        "macos" | "ios" => println!("cargo:rustc-link-lib=dylib=c++"),
        "windows" => {}
        _ => {
            println!("cargo:rustc-link-lib=dylib=stdc++");
            println!("cargo:rustc-link-lib=dylib=dl");
        }
    }
}
