#!/usr/bin/env python3
"""Vendor web fonts into the repo so the app works fully offline.

Downloads Font Awesome and JetBrains Mono and writes:
  - the raw ``.woff2`` files into ``crates/kopuz/assets/fonts/``
  - the matching CSS (``fontawesome.css`` / ``jetbrains-mono.css``) with
    ``@font-face`` ``src`` rewritten to local ``url(fonts/NAME.woff2)`` refs.

At build time ``crates/kopuz/build.rs`` inlines each referenced woff2 as a
base64 ``data:`` URI, so the fonts end up compiled into the binary. That means
styling works with a bare ``cargo run`` on any OS (no CDN, no asset collection,
no path resolution) — the fonts live in the repo as normal binary files rather
than as giant base64 blobs in source.

Re-run after a version bump:

    python3 packaging/scripts/vendor-fonts.py

Licenses:
  - Font Awesome Free 6.5.1 — SIL OFL 1.1 (fonts), CC BY 4.0 (icons), MIT (code)
  - JetBrains Mono — SIL OFL 1.1
"""

import re
import sys
import urllib.request
from pathlib import Path

FA_VERSION = "6.5.1"
FA_BASE = f"https://cdnjs.cloudflare.com/ajax/libs/font-awesome/{FA_VERSION}"
FA_FONTS = ["fa-solid-900", "fa-regular-400", "fa-brands-400", "fa-v4compatibility"]

JBM_CSS_URL = "https://fonts.bunny.net/css?family=jetbrains-mono:400,500,700,800&display=swap"

# Toki Pona (sitelen pona) glyphs for the tok / tok-SP locales. Referenced by an
# @font-face in assets/main.css, which must point at url(fonts/<this file>).
NASIN_NANPA_URL = "https://github.com/etbcor/nasin-nanpa/releases/download/n4.0.2/nasin-nanpa-4.0.2-UCSUR.otf"

# bunny.net only serves woff2 to browser-like clients.
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36"

ASSETS = Path(__file__).resolve().parents[2] / "crates" / "kopuz" / "assets"
FONTS_DIR = ASSETS / "fonts"


def fetch(url: str, binary: bool = False):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req) as resp:
        data = resp.read()
    return data if binary else data.decode("utf-8")


def save_woff2(name: str, data: bytes) -> None:
    (FONTS_DIR / f"{name}.woff2").write_bytes(data)


def vendor_font_awesome() -> None:
    css = fetch(f"{FA_BASE}/css/all.min.css")
    for name in FA_FONTS:
        save_woff2(name, fetch(f"{FA_BASE}/webfonts/{name}.woff2", binary=True))
        # Point at the vendored woff2 and drop the ttf fallback (woff2 is universal).
        css = css.replace(f"url(../webfonts/{name}.woff2)", f"url(fonts/{name}.woff2)")
        css = re.sub(
            rf',\s*url\(\.\./webfonts/{re.escape(name)}\.ttf\)\s*format\("truetype"\)',
            "",
            css,
        )
    leftover = re.findall(r"url\(\.\./webfonts/[^)]+\)", css)
    if leftover:
        sys.exit(f"font-awesome: unresolved font refs remain: {sorted(set(leftover))}")
    out = ASSETS / "fontawesome.css"
    out.write_text(f"/* Font Awesome Free {FA_VERSION} — vendored by packaging/scripts/vendor-fonts.py */\n{css}")
    print(f"wrote {out.relative_to(ASSETS.parents[2])}")


def vendor_jetbrains_mono() -> None:
    css = fetch(JBM_CSS_URL)
    # Drop the legacy .woff fallback entries; every webview supports woff2.
    css = re.sub(r",\s*url\(https://[^)]+\.woff\)\s*format\(['\"]woff['\"]\)", "", css)
    for url in sorted(set(re.findall(r"https://[^)]+\.woff2", css))):
        name = Path(url).stem
        save_woff2(name, fetch(url, binary=True))
        css = css.replace(f"url({url})", f"url(fonts/{name}.woff2)")
    leftover = re.findall(r"url\(https://[^)]+\)", css)
    if leftover:
        sys.exit(f"jetbrains-mono: unresolved font refs remain: {sorted(set(leftover))}")
    out = ASSETS / "jetbrains-mono.css"
    out.write_text(f"/* JetBrains Mono — vendored by packaging/scripts/vendor-fonts.py */\n{css}")
    print(f"wrote {out.relative_to(ASSETS.parents[2])}")


def vendor_nasin_nanpa() -> None:
    name = Path(NASIN_NANPA_URL).name
    (FONTS_DIR / name).write_bytes(fetch(NASIN_NANPA_URL, binary=True))
    print(f"wrote fonts/{name} (referenced by assets/main.css)")


if __name__ == "__main__":
    FONTS_DIR.mkdir(parents=True, exist_ok=True)
    vendor_font_awesome()
    vendor_jetbrains_mono()
    vendor_nasin_nanpa()
    n = len(list(FONTS_DIR.glob("*.woff2"))) + len(list(FONTS_DIR.glob("*.otf")))
    print(f"font files in {FONTS_DIR.relative_to(ASSETS.parents[2])}: {n}")
