{ stdenv, fetchurl }:
let
  # `deno_core` pulls in the `v8` crate, whose build script fetches a prebuilt
  # librusty_v8 archive — impossible in the network-less Nix sandbox. Fetch it as
  # a fixed-output derivation and pass it via RUSTY_V8_ARCHIVE (build.rs
  # decompresses the .gz itself). Keep rustyV8Version in sync with Cargo.lock.
  rustyV8Version = "130.0.7";
  rustyV8Target = stdenv.hostPlatform.rust.rustcTarget;
  rustyV8Hashes = {
    "aarch64-apple-darwin" = "1sh0y3dq0llz6hfx8qgx13sc6vjbw1xzzwfrl236wx8f9w7x1nzn";
    "x86_64-apple-darwin" = "1h0j3qw5ad2c83mh36pr18s1vp598rscj3zzxpl7vynzj6q321s3";
    "aarch64-unknown-linux-gnu" = "0nli54vqcrfh9nkz7ma7230k0xmhcrk0jmfbyxcp3rxybarygvxy";
    "x86_64-unknown-linux-gnu" = "0pdp6h7vbjvq5l9lh25qilmp6xrxg7mj8m263h44f0lv9swnqix6";
  };
in
fetchurl {
  url = "https://github.com/denoland/rusty_v8/releases/download/v${rustyV8Version}/librusty_v8_release_${rustyV8Target}.a.gz";
  sha256 =
    rustyV8Hashes.${rustyV8Target}
      or (throw "no prebuilt librusty_v8 hash for target ${rustyV8Target}");
}
