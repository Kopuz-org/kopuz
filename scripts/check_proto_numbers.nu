#!/usr/bin/env nu

# A field may be added or retired, but never renumbered: an old peer reads the
# number, not the name, so moving `dont_recommend` from 16 to 17 made a frontend
# read that flag as `browser_playback` and silently drop the button it gates.
# Compare every field's number against the base revision and fail on a move.

def fields [text: string] {
  mut out = []
  # One entry per open block: a message pushes its own name, a `oneof` or enum
  # inside it pushes the enclosing one, so a field after a nested block is still
  # attributed to its message instead of being dropped at the first `}`.
  mut stack = []
  for line in ($text | lines) {
    let trimmed = ($line | str trim)
    if ($trimmed | str starts-with "}") {
      $stack = ($stack | drop 1)
      continue
    }
    if ($trimmed | str ends-with "{") {
      let opened = ($trimmed | parse -r '^message\s+(?<name>\w+)\s*\{$')
      let owner = if ($opened | is-not-empty) {
        $opened | first | get name
      } else if ($stack | is-empty) {
        ""
      } else {
        $stack | last
      }
      $stack = ($stack | append $owner)
      continue
    }
    let message = if ($stack | is-empty) { "" } else { $stack | last }
    if $message == "" { continue }
    let field = ($trimmed | parse -r '^(?:repeated\s+|optional\s+)?[\w\.<>, ]+?\s+(?<name>\w+)\s*=\s*(?<number>\d+)\s*;')
    if ($field | is-not-empty) {
      let one = ($field | first)
      $out = ($out | append {key: $"($message).($one.name)", number: $one.number})
    }
  }
  $out
}

def main [--base: string = "origin/master"] {
  let script_dir = $env.FILE_PWD
  let repo_root = ($script_dir | path dirname)
  let path = "crates/proto/proto/kopuz.proto"
  let file = ($repo_root | path join $path)

  let before = (do -i { ^git -C $repo_root show $"($base):($path)" } | complete)
  if $before.exit_code != 0 {
    print $"No ($base) to compare against; skipping."
    exit 0
  }

  let was = (fields $before.stdout)
  let now = (fields (open $file --raw | decode utf-8))

  let moved = (
    $now
    | each {|field|
        let old = ($was | where key == $field.key)
        if ($old | is-empty) { null } else {
          let old = ($old | first)
          if $old.number == $field.number { null } else {
            {field: $field.key, was: $old.number, now: $field.number}
          }
        }
      }
    | compact
  )

  if ($moved | is-empty) {
    print $"($now | length) fields keep the numbers ($base) gave them."
    exit 0
  }

  print --stderr "These fields were renumbered, which no peer can survive:"
  for one in $moved {
    print --stderr $"  ($one.field): ($one.was) -> ($one.now)"
  }
  print --stderr "Give the new meaning a new number and `reserved` the old one."
  exit 1
}
