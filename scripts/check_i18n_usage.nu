#!/usr/bin/env nu

def extract-locale-keys [path: path] {
  open $path
  | lines
  | parse -r '^(?<key>[A-Za-z0-9_-]+)\s*='
  | get key
  | uniq
  | sort
}

def extract-rust-i18n-usages [repo_root: path] {
  glob ($repo_root | path join "**" "*.rs")
  | each {|file|
      open $file
      | lines
      | enumerate
      | each {|row|
          let line = ($row.index + 1)
          let fragments = ($row.item | split row 'i18n::t("' | skip 1)

          $fragments
          | each {|fragment|
              {
                path: ($file | into string)
                line: $line
                key: ($fragment | split row '"' | first)
              }
            }
        }
    }
  | flatten
  | flatten
}

def main [] {
  let script_dir = $env.FILE_PWD
  let repo_root = ($script_dir | path dirname)
  let baseline = ($repo_root | path join "locales" "en.ftl")

  if not ($baseline | path exists) {
    print --stderr "Missing baseline locale: locales/en.ftl"
    exit 1
  }

  let locale_keys = (extract-locale-keys $baseline)
  let usages = (extract-rust-i18n-usages $repo_root)
  let missing = (
    $usages
    | where {|usage| $usage.key not-in $locale_keys }
    | sort-by key path line
  )

  if not ($missing | is-empty) {
    print --stderr "Missing en.ftl keys for Rust i18n::t(...) usages:"
    $missing
    | each {|usage|
        print --stderr $"  ($usage.key) -> ($usage.path):($usage.line)"
      }
    exit 1
  }

  print "All Rust i18n::t(...) keys exist in locales/en.ftl."
}
