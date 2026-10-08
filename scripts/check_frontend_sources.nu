#!/usr/bin/env nu

# Frontend crates must not know the local-source concept; the daemon owns it.
const FORBIDDEN = '(?i)Source::Local|LocalLibrary|local_source|local_library|music_directory|is_local\b|SourceKind|upsert_local|LocalSourceDraft|i18n::t\("local|i18n::t\("[a-z_]+_local"|local_?sources?\b|\blocal_?librar|(?-i:\bLocal\b)'

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  let crates = [api client hooks pages components kopuz kopuz_route]
  let hits = (
    $crates
    | each {|c| glob ($root | path join "crates" $c "**" "*.rs") }
    | flatten
    | each {|file|
        open $file
        | lines
        | enumerate
        | where {|row| $row.item =~ $FORBIDDEN }
        | each {|row| $"($file | path relative-to $root):($row.index + 1): ($row.item | str trim)" }
      }
    | flatten
  )

  if ($hits | is-empty) {
    print "frontend crates carry no local-source concept"
  } else {
    $hits | each {|h| print --stderr $h }
    error make --unspanned { msg: $"($hits | length) local-source reference\(s\) in frontend crates" }
  }
}
