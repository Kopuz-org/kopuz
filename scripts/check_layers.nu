#!/usr/bin/env nu

# shared/ is the daemon <-> frontend contract; daemon/ and apps/ meet only through it.

const ALLOWED = {
  shared: [shared]
  daemon: [shared daemon]
  common: [shared common]
}

# Every edge that crosses a layer on purpose. An entry nothing uses fails the check too.
const EXCEPTIONS = [
  # The Dioxus app hosts the daemon core in-process and serves it on the socket.
  [from to kind target];
  [apps/legacy/kopuz daemon/core normal null]
  [apps/legacy/kopuz daemon/db normal null]
  [apps/legacy/kopuz daemon/server normal null]
  [apps/legacy/kopuz daemon/player normal null]
  [apps/legacy/kopuz daemon/kopuzd normal 'cfg(not(target_os = "android"))']
  # On Android the Dioxus frontend drives the platform player directly.
  [apps/legacy/hooks daemon/player normal 'cfg(target_os = "android")']
  [apps/legacy/components daemon/player normal 'cfg(target_os = "android")']
  # The daemon's contract tests talk to it through the real client and CLI.
  [daemon/kopuzd apps/common/client dev null]
  [daemon/kopuzd apps/cli dev null]
]

def layer [dir: string] {
  let parts = ($dir | split row "/")
  match $parts.0 {
    "shared" => "shared"
    "daemon" => "daemon"
    "apps" => (if $parts.1 == "common" { "common" } else { $"app:($parts.1)" })
    _ => "unknown"
  }
}

def allowed [from: string, to: string] {
  if ($from | str starts-with "app:") {
    $to in [shared common $from]
  } else {
    $to in ($ALLOWED | get -o $from | default [])
  }
}

def members [root: path] {
  let ws = (open ($root | path join "Cargo.toml") | get workspace)
  $ws.members
  | each {|pattern| glob ($root | path join $pattern "Cargo.toml") }
  | flatten
  | each {|manifest| $manifest | path dirname | path relative-to $root }
  | uniq
}

def dep-tables [manifest: record] {
  let kinds = [[table kind]; [dependencies normal] [dev-dependencies dev] [build-dependencies build]]
  let plain = ($kinds | each {|k| {target: null, kind: $k.kind, deps: ($manifest | get -o $k.table | default {})} })
  let targeted = (
    $manifest | get -o target | default {} | transpose cfg tables
    | each {|t| $kinds | each {|k| {target: $t.cfg, kind: $k.kind, deps: ($t.tables | get -o $k.table | default {})} } }
    | flatten
  )
  $plain | append $targeted
}

def edges [root: path, dir: string, workspace_deps: record] {
  let manifest = (open ($root | path join $dir "Cargo.toml"))
  dep-tables $manifest
  | each {|table|
      $table.deps | transpose name spec
      | each {|dep|
          let spec = $dep.spec
          let path = if ($spec | describe | str starts-with "record") {
            if ($spec | get -o workspace | default false) {
              let ws = ($workspace_deps | get -o $dep.name)
              if ($ws | describe | str starts-with "record") and ($ws | get -o path) != null {
                $root | path join $ws.path
              }
            } else if ($spec | get -o path) != null {
              $root | path join $dir $spec.path
            }
          }
          if $path != null {
            {from: $dir, to: ($path | path expand | path relative-to ($root | path expand)), kind: $table.kind, target: $table.target}
          }
        }
      | compact
    }
  | flatten
}

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  let workspace_deps = (open ($root | path join "Cargo.toml") | get workspace.dependencies)
  let crates = (members $root)
  let unplaced = ($crates | where {|dir| (layer $dir) == "unknown" })
  let all = ($crates | each {|dir| edges $root $dir $workspace_deps } | flatten)
  let crossing = ($all | where {|e| not (allowed (layer $e.from) (layer $e.to)) })
  let excused = {|e| $EXCEPTIONS | any {|x| $x.from == $e.from and $x.to == $e.to and $x.kind == $e.kind and $x.target == $e.target } }
  let broken = ($crossing | where {|e| not (do $excused $e) })
  let unused = ($EXCEPTIONS | where {|x| not ($crossing | any {|e| $x.from == $e.from and $x.to == $e.to and $x.kind == $e.kind and $x.target == $e.target }) })

  $unplaced | each {|dir| print --stderr $"($dir): not under shared/, daemon/ or apps/" }
  $broken | each {|e|
    let on = if $e.target == null { "" } else { $" on ($e.target)" }
    print --stderr $"($e.from) -> ($e.to) \(($e.kind)($on)\): ((layer $e.from)) may not depend on ((layer $e.to))"
  }
  $unused | each {|x| print --stderr $"stale exception: ($x.from) -> ($x.to) \(($x.kind)\)" }

  let failures = ($unplaced | length) + ($broken | length) + ($unused | length)
  if $failures > 0 {
    error make --unspanned { msg: $"($failures) layer boundary problem\(s\)" }
  }
  print $"($crates | length) crates, ($all | length) workspace edges, ($EXCEPTIONS | length) listed exceptions: layers hold"
}
