"""Shared rules for first-party Kopuz Rust crates."""

load("@crates//:defs.bzl", "aliases", "all_crate_deps", "crate_edition")
load("@rules_rust//rust:defs.bzl", "rust_library", "rust_test")

_VERSION = "0.11.0"

def kopuz_rust_library(
        name,
        compile_data = [],
        data = [],
        extra_deps = [],
        rustc_env = {},
        test = True,
        test_compile_data = [],
        test_data = []):
    """Declares a workspace library and its Cargo-equivalent unit-test target."""
    common_env = {
        "CARGO_PKG_VERSION": _VERSION,
    }
    common_env.update(rustc_env)

    rust_library(
        name = name,
        aliases = aliases(
            normal = True,
            proc_macro = True,
        ),
        compile_data = compile_data,
        crate_name = name,
        data = data,
        deps = all_crate_deps(normal = True) + extra_deps,
        edition = crate_edition(),
        proc_macro_deps = all_crate_deps(proc_macro = True),
        rustc_env = common_env,
        srcs = native.glob(["src/**/*.rs"]),
        version = _VERSION,
    )

    if test:
        rust_test(
            name = name + "_test",
            aliases = aliases(
                normal_dev = True,
                proc_macro_dev = True,
            ),
            compile_data = compile_data + test_compile_data,
            crate = ":" + name,
            data = data + test_data,
            deps = all_crate_deps(normal_dev = True),
            edition = crate_edition(),
            proc_macro_deps = all_crate_deps(proc_macro_dev = True),
            rustc_env = common_env,
        )

def kopuz_rust_integration_test(
        name,
        src,
        compile_data = [],
        data = [],
        extra_deps = [],
        rustc_env = {}):
    """Declares a Cargo-style integration test for a first-party crate."""
    common_env = {
        "CARGO_PKG_VERSION": _VERSION,
    }
    common_env.update(rustc_env)

    rust_test(
        name = name + "_test",
        aliases = aliases(
            normal = True,
            normal_dev = True,
            proc_macro = True,
            proc_macro_dev = True,
        ),
        compile_data = compile_data,
        crate_name = name,
        data = data,
        deps = all_crate_deps(
            normal = True,
            normal_dev = True,
        ) + extra_deps,
        edition = crate_edition(),
        proc_macro_deps = all_crate_deps(
            proc_macro = True,
            proc_macro_dev = True,
        ),
        rustc_env = common_env,
        srcs = [src],
    )
