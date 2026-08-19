#!/usr/bin/env nu

# Vendor web fonts into the repo so the app works fully offline.
#
# Downloads Font Awesome, JetBrains Mono and nasin-nanpa, writing the raw
# woff2/otf into crates/kopuz/assets/fonts/ and the matching CSS with @font-face
# `src` rewritten to local `url(fonts/NAME)` refs. At build time
# crates/kopuz/build.rs inlines each referenced font as a base64 `data:` URI, so
# the fonts end up compiled into the binary — styling works with a bare
# `cargo run` on any OS (no CDN, no asset collection, no path resolution).
#
# Re-run after a version bump:  nu scripts/vendor-fonts.nu
#
# Licenses: Font Awesome Free 6.5.1 — OFL 1.1 (fonts) / CC BY 4.0 (icons) / MIT
# (code); JetBrains Mono — OFL 1.1; nasin-nanpa — OFL 1.1.

const FA_VERSION = "6.5.1"
const FA_FONTS = ["fa-solid-900" "fa-regular-400" "fa-brands-400" "fa-v4compatibility"]
const JBM_CSS_URL = "https://fonts.bunny.net/css?family=jetbrains-mono:400,500,700,800&display=swap"
# Toki Pona (sitelen pona) glyphs for tok / tok-SP. Referenced by an @font-face
# in assets/main.css, which must point at url(fonts/<this file>).
const NASIN_URL = "https://github.com/etbcor/nasin-nanpa/releases/download/n4.0.2/nasin-nanpa-4.0.2-UCSUR.otf"
# bunny.net only serves woff2 to browser-like clients.
const UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36"

def main [] {
  let assets = ($env.FILE_PWD | path dirname | path join crates kopuz assets)
  let fonts = ($assets | path join fonts)
  mkdir $fonts

  vendor-font-awesome $assets $fonts
  vendor-jetbrains-mono $assets $fonts
  vendor-nasin-nanpa $fonts

  print $"font files in crates/kopuz/assets/fonts: (ls $fonts | length)"
}

def vendor-font-awesome [assets: path, fonts: path] {
  let base = $"https://cdnjs.cloudflare.com/ajax/libs/font-awesome/($FA_VERSION)"
  mut css = (http get $"($base)/css/all.min.css")
  for name in $FA_FONTS {
    http get $"($base)/webfonts/($name).woff2" | save -f ($fonts | path join $"($name).woff2")
    # Point at the vendored woff2 and drop the ttf fallback (woff2 is universal).
    $css = ($css | str replace --all ('url(../webfonts/' + $name + '.woff2)') ('url(fonts/' + $name + '.woff2)'))
    $css = ($css | str replace --all --regex (',\s*url\(\.\./webfonts/' + $name + '\.ttf\) format\("truetype"\)') "")
  }
  if ($css | str contains "../webfonts/") {
    error make {msg: "font-awesome: unresolved ../webfonts/ refs remain"}
  }
  $"/* Font Awesome Free ($FA_VERSION) — vendored by scripts/vendor-fonts.nu */\n($css)" | save -f ($assets | path join fontawesome.css)
  print "wrote crates/kopuz/assets/fontawesome.css"
}

def vendor-jetbrains-mono [assets: path, fonts: path] {
  mut css = (http get $JBM_CSS_URL --headers {User-Agent: $UA})
  # Drop the legacy .woff fallback entries; every webview supports woff2.
  $css = ($css | str replace --all --regex ',\s*url\(https://[^)]+\.woff\) format\([\x27"]woff[\x27"]\)' "")
  let urls = ($css | parse --regex 'url\((?<u>https://[^)]+\.woff2)\)' | get u | uniq)
  for url in $urls {
    let name = ($url | path basename | str replace ".woff2" "")
    http get $url | save -f ($fonts | path join $"($name).woff2")
    $css = ($css | str replace --all ('url(' + $url + ')') ('url(fonts/' + $name + '.woff2)'))
  }
  if ($css | str contains "https://") {
    error make {msg: "jetbrains-mono: unresolved remote refs remain"}
  }
  $"/* JetBrains Mono — vendored by scripts/vendor-fonts.nu */\n($css)" | save -f ($assets | path join jetbrains-mono.css)
  print "wrote crates/kopuz/assets/jetbrains-mono.css"
}

def vendor-nasin-nanpa [fonts: path] {
  let name = ($NASIN_URL | path basename)
  http get $NASIN_URL | save -f ($fonts | path join $name)
  print $"wrote fonts/($name) — referenced by assets/main.css"
}
