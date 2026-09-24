#!/usr/bin/env nu

# An old peer reads the number, not the name: a moved or reused number silently changes what it decodes.

def reserved_ranges [owner: string, list: string] {
  $list | split row "," | each {|item| $item | str trim } | where {|item| not ($item | str starts-with '"') } | each {|item|
    let range = ($item | parse -r '^(?<lo>\d+)\s+to\s+(?<hi>\d+|max)$')
    if ($range | is-not-empty) {
      let hi = if ($range.0.hi == "max") { 536870911 } else { $range.0.hi | into int }
      {owner: $owner, lo: ($range.0.lo | into int), hi: $hi}
    } else {
      {owner: $owner, lo: ($item | into int), hi: ($item | into int)}
    }
  }
}

# Every numbered field and enum value, keyed by the path of the message or enum that declares it.
def schema [text: string] {
  mut numbers = []
  mut reserved = []
  mut owners = []
  mut stack = []
  let statements = ($text | lines | each {|line|
    $line | split row "//" | first | str replace -a "{" "{\n" | str replace -a "}" "\n}\n" | str replace -a ";" ";\n" | lines
  } | flatten | each {|s| $s | str trim } | where {|s| $s != "" })
  for trimmed in $statements {
    let parent = if ($stack | is-empty) { "" } else { $stack | last | get path }
    let declared = ($trimmed | parse -r '^(?<kind>message|enum)\s+(?<name>\w+)\s*\{$')
    if ($declared | is-not-empty) {
      let one = ($declared | first)
      let path = if $parent == "" { $one.name } else { $"($parent).($one.name)" }
      $owners = ($owners | append $path)
      $stack = ($stack | append {kind: $one.kind, path: $path})
      continue
    }
    if ($trimmed | str starts-with "}") {
      $stack = ($stack | drop 1)
      continue
    }
    if ($trimmed | str ends-with "{") {
      let kind = if ($trimmed | str starts-with "oneof ") { "message" } else { "other" }
      let path = if $kind == "message" { $parent } else { "" }
      $stack = ($stack | append {kind: $kind, path: $path})
      continue
    }
    if ($stack | is-empty) { continue }
    let block = ($stack | last)
    if $block.path == "" { continue }
    let reserve = ($trimmed | parse -r '^reserved\s+(?<list>[^;]+);')
    if ($reserve | is-not-empty) {
      $reserved = ($reserved | append (reserved_ranges $block.path $reserve.0.list))
      continue
    }
    let pattern = if $block.kind == "enum" {
      '^(?<name>\w+)\s*=\s*(?<number>-?\d+)\s*(?:\[[^\]]*\])?\s*;'
    } else {
      '^(?:repeated\s+|optional\s+)?[\w\.<>, ]+?\s+(?<name>\w+)\s*=\s*(?<number>\d+)\s*(?:\[[^\]]*\])?\s*;'
    }
    let found = ($trimmed | parse -r $pattern)
    if ($found | is-not-empty) {
      $numbers = ($numbers | append {owner: $block.path, name: $found.0.name, number: ($found.0.number | into int)})
    }
  }
  {numbers: $numbers, reserved: $reserved, owners: $owners}
}

def violations [before: string, after: string] {
  let was = (schema $before)
  let now = (schema $after)
  $was.numbers | each {|old|
    let same = ($now.numbers | where owner == $old.owner and name == $old.name)
    if ($same | is-not-empty) {
      if $same.0.number == $old.number { null } else {
        {field: $"($old.owner).($old.name)", problem: $"renumbered ($old.number) -> ($same.0.number)"}
      }
    } else if not ($old.owner in $now.owners) {
      null
    } else if ($now.reserved | any {|r| $r.owner == $old.owner and $r.lo <= $old.number and $old.number <= $r.hi }) {
      null
    } else {
      {field: $"($old.owner).($old.name)", problem: $"removed without `reserved ($old.number)`"}
    }
  } | compact
}

def main [--base: string = "origin/master"] {
  let repo_root = ($env.FILE_PWD | path dirname)
  let path = "crates/proto/proto/kopuz.proto"

  let before = (do -i { ^git -C $repo_root show $"($base):($path)" } | complete)
  if $before.exit_code != 0 {
    print $"No ($base) to compare against; skipping."
    exit 0
  }

  let after = (open ($repo_root | path join $path) --raw | decode utf-8)
  let broken = (violations $before.stdout $after)
  if ($broken | is-empty) {
    print $"(schema $after | get numbers | length) numbers keep the meaning ($base) gave them."
    exit 0
  }

  print --stderr "These numbers changed meaning, which no peer can survive:"
  for one in $broken {
    print --stderr $"  ($one.field): ($one.problem)"
  }
  print --stderr "Give the new meaning a new number and `reserved` the old one."
  exit 1
}

def "main test" [] {
  let base = '
message Ping {}
message Skip { string key = 1; bool undo = 2; }
message Track {
  string title = 1;
  oneof art {
    string url = 2;
    bytes data = 3;
  }
  bool hq = 4;
  repeated string tags = 5 [deprecated = true];
  map<string, string> extra = 6;
  message Credit {
    string name = 1;
  }
  reserved 9, 11 to 13;
}
message Album {
  message Credit {
    string name = 1;
  }
}
enum State {
  STATE_IDLE = 0;
  STATE_PLAYING = 1; // trailing comment
}
service Api {
  rpc Get(Ping) returns (Track);
}
'
  let cases = [
    [name, after, expect];
    ["unchanged", $base, []]
    ["field added", ($base | str replace "bool hq = 4;" "bool hq = 4;\n  bool lossless = 7;"), []]
    ["message removed", ($base | str replace "message Ping {}" "" | str replace "rpc Get(Ping)" "rpc Get(Track)"), []]
    ["one-line message renumbered", ($base | str replace "undo = 2" "undo = 3"), ["Skip.undo"]]
    ["field after a oneof renumbered", ($base | str replace "hq = 4" "hq = 7"), ["Track.hq"]]
    ["field inside a oneof renumbered", ($base | str replace "data = 3" "data = 7"), ["Track.data"]]
    ["field with options renumbered", ($base | str replace "tags = 5" "tags = 7"), ["Track.tags"]]
    ["map field renumbered", ($base | str replace "extra = 6" "extra = 7"), ["Track.extra"]]
    ["nested message keyed by its parent", ($base | str replace "message Album {\n  message Credit {\n    string name = 1;" "message Album {\n  message Credit {\n    string name = 2;"), ["Album.Credit.name"]]
    ["removed and reserved", ($base | str replace "bool hq = 4;" "reserved 4;"), []]
    ["removed into a reserved range", ($base | str replace "  bool hq = 4;\n" "" | str replace "reserved 9, 11 to 13;" "reserved 4, 9, 11 to 13;"), []]
    ["removed without reserving", ($base | str replace "  bool hq = 4;\n" ""), ["Track.hq"]]
    ["number reused under a new name", ($base | str replace "bool hq = 4;" "bool lossless = 4;"), ["Track.hq"]]
    ["enum value renumbered", ($base | str replace "STATE_PLAYING = 1" "STATE_PLAYING = 2"), ["State.STATE_PLAYING"]]
    ["enum value reused", ($base | str replace "STATE_PLAYING = 1" "STATE_PAUSED = 1"), ["State.STATE_PLAYING"]]
    ["enum value removed and reserved", ($base | str replace "STATE_PLAYING = 1; // trailing comment" "reserved 1;"), []]
  ]
  let failed = ($cases | each {|case|
    let got = (violations $base $case.after | each {|v| $v.field })
    if $got == $case.expect { null } else { $"($case.name): expected ($case.expect | to nuon), got ($got | to nuon)" }
  } | compact)
  if ($failed | is-empty) {
    print $"($cases | length) cases pass."
    exit 0
  }
  for one in $failed { print --stderr $one }
  exit 1
}
