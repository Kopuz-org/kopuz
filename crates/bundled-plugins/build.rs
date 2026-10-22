//! Builds every plugin under `plugins/` for `wasm32-wasip2` and writes the
//! table that embeds them.
//!
//! The nested cargo shares the workspace's lockfile, so a plugin's
//! dependencies are pinned and vendored like everyone else's, but it gets a
//! target directory of its own: the outer build holds a lock on the default
//! one for as long as this script runs.

use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "wasm32-wasip2";
const PROFILE: &str = "plugin";

#[derive(serde::Deserialize)]
struct CargoManifest {
    package: Package,
}

#[derive(serde::Deserialize)]
struct Package {
    name: String,
}

struct PluginCrate {
    package: String,
    manifest: PathBuf,
}

fn main() {
    let crate_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let workspace = crate_dir
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits at crates/bundled-plugins")
        .to_path_buf();
    let plugins_dir = workspace.join("plugins");
    println!("cargo:rerun-if-changed={}", plugins_dir.display());
    println!("cargo:rerun-if-changed={}", workspace.join("wit").display());
    println!("cargo:rerun-if-env-changed=KOPUZ_PLUGIN_WASM_DIR");

    let plugins = plugin_crates(&plugins_dir);
    let wasm_dir = match std::env::var_os("KOPUZ_PLUGIN_WASM_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => build(&workspace, &plugins),
    };

    let mut table = String::from("static BUNDLED: &[plugin_host::Bundled] = &[\n");
    for plugin in &plugins {
        let wasm = wasm_dir.join(format!("{}.wasm", plugin.package.replace('-', "_")));
        assert!(
            wasm.is_file(),
            "{} was not built: {} is missing",
            plugin.package,
            wasm.display()
        );
        table.push_str(&format!(
            "    plugin_host::Bundled {{ manifest: include_str!({:?}), wasm: include_bytes!({:?}) }},\n",
            plugin.manifest.display().to_string(),
            wasm.display().to_string(),
        ));
    }
    table.push_str("];\n");
    let out = PathBuf::from(env("OUT_DIR")).join("bundled.rs");
    std::fs::write(&out, table).expect("write the bundled plugin table");
}

/// Every directory under `plugins/` with a `kopuz-plugin.toml`, sorted so the
/// table does not change with directory order. The SDK has no manifest.
fn plugin_crates(dir: &Path) -> Vec<PluginCrate> {
    let mut plugins: Vec<PluginCrate> = std::fs::read_dir(dir)
        .expect("read plugins/")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("kopuz-plugin.toml").is_file())
        .map(|path| {
            let cargo = std::fs::read_to_string(path.join("Cargo.toml"))
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let cargo: CargoManifest = toml::from_str(&cargo)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            PluginCrate {
                package: cargo.package.name,
                manifest: path.join("kopuz-plugin.toml"),
            }
        })
        .collect();
    plugins.sort_by(|a, b| a.package.cmp(&b.package));
    plugins
}

fn build(workspace: &Path, plugins: &[PluginCrate]) -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"))
        .join("plugins");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .current_dir(workspace)
        .args([
            "build",
            "--locked",
            "--target",
            TARGET,
            "--profile",
            PROFILE,
        ])
        .arg("--target-dir")
        .arg(&target_dir);
    for plugin in plugins {
        command.args(["-p", &plugin.package]);
    }
    // What the outer build was told applies to the host target, not to the
    // plugins: its flags, its target, and a wrapper like clippy-driver.
    for (key, _) in std::env::vars_os() {
        let Some(key) = key.to_str() else { continue };
        if key.starts_with("CARGO_ENCODED_RUSTFLAGS")
            || key.starts_with("CARGO_BUILD_")
            || key.starts_with("CARGO_TARGET_")
            || key == "RUSTFLAGS"
            || key == "RUSTDOCFLAGS"
            || key == "RUSTC_WORKSPACE_WRAPPER"
            || key == "CARGO_MAKEFLAGS"
        {
            command.env_remove(key);
        }
    }
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("could not run cargo to build the plugins: {error}"));
    assert!(
        status.success(),
        "building the plugins for {TARGET} failed; is the target installed? (rustup target add {TARGET})"
    );
    target_dir.join(TARGET).join(PROFILE)
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is set by cargo"))
}
