#!/usr/bin/env nu

# Build the example community plugins and write a marketplace index for them,
# so installing a plugin from the marketplace can be tried without publishing
# one. Point the daemon at the index it prints:
#
#   nu scripts/plugin_market.nu
#   KOPUZ_PLUGIN_INDEX=file://<printed path> just serve

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  let out = ($root | path join "target" "plugin-market")
  let examples = (glob ($root | path join "examples" "plugins" "*" "kopuz-plugin.toml") | sort)

  let packages = ($examples | each {|manifest| open ($manifest | path dirname | path join "Cargo.toml") | get package.name })
  let args = ($packages | each {|package| ["-p" $package] } | flatten)
  cargo build --locked --target wasm32-wasip2 --profile plugin ...$args

  mkdir $out
  let listings = ($examples | zip $packages | each {|pair|
    let manifest_path = $pair.0
    let package = $pair.1
    let manifest = (open $manifest_path)
    let dir = ($out | path join $manifest.id)
    mkdir $dir
    let wasm = ($root | path join "target" "wasm32-wasip2" "plugin" $"($package | str replace -a '-' '_').wasm")
    cp $manifest_path ($dir | path join "kopuz-plugin.toml")
    cp $wasm ($dir | path join "plugin.wasm")
    let manifest_file = ($dir | path join "kopuz-plugin.toml")
    let component_file = ($dir | path join "plugin.wasm")
    {
      id: $manifest.id
      name: $manifest.name
      version: $manifest.version
      api: $manifest.api
      description: ($manifest.description? | default "")
      author: "Kopuz examples"
      services: ($manifest.service? | default [] | each {|service| $service.id })
      network: ($manifest.permissions?.network? | default [])
      manifest: { url: $"file://($manifest_file)", sha256: (open --raw $manifest_file | hash sha256) }
      component: { url: $"file://($component_file)", sha256: (open --raw $component_file | hash sha256) }
    }
  })

  let index = ($out | path join "index.json")
  { schema: 1, plugins: $listings } | to json | save --force $index
  print $"Marketplace index: ($index)"
  print $"Run kopuz with KOPUZ_PLUGIN_INDEX=file://($index)"
}
