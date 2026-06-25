#!/usr/bin/env nu

def extract-keys [path: path] {
  open $path
  | lines
  | parse -r '^(?<key>[A-Za-z0-9_-]+)\s*='
  | get key
  | uniq
  | sort
}

def main [] {
  let ignored_extra_keys = [
    arabic
    english
    japanese
    russian
    toki_pona
    turkish
  ]

  let script_dir = $env.FILE_PWD
  let repo_root = ($script_dir | path dirname)
  let locales_dir = ($repo_root | path join "crates" "i18n" "locales")
  let baseline = ($locales_dir | path join "en.ftl")

  if not ($baseline | path exists) {
    print --stderr "Missing baseline locale: crates/i18n/locales/en.ftl"
    exit 1
  }

  let baseline_keys = (extract-keys $baseline)
  mut failures = []

  for locale_path in (glob ($locales_dir | path join "*.ftl") | sort) {
    if $locale_path == $baseline {
      continue
    }

    let locale_name = ($locale_path | path basename)
    let locale_keys = (extract-keys $locale_path)
    let missing = ($baseline_keys | where {|key| $key not-in $locale_keys })
    let extra = (
      $locale_keys
      | where {|key| ($key not-in $baseline_keys) and ($key not-in $ignored_extra_keys) }
    )

    if (not ($missing | is-empty)) or (not ($extra | is-empty)) {
      $failures = ($failures | append $"Locale ($locale_name) is out of sync with en.ftl:")

      if not ($missing | is-empty) {
        $failures = ($failures | append $"  Missing keys: (($missing | str join ', '))")
      }

      if not ($extra | is-empty) {
        $failures = ($failures | append $"  Extra keys: (($extra | str join ', '))")
      }
    }
  }

  if not ($failures | is-empty) {
    $failures | each {|line| print --stderr $line }
    exit 1
  }

  print "All locale files match en.ftl."
}
