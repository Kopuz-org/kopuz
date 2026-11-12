#!/usr/bin/env bash
# Render packaging/aur/$PKG/PKGBUILD for $VERSION, checksummed against $SOURCE_FILE, and push it to the AUR.
set -euo pipefail

: "${PKG:?}" "${VERSION:?}" "${SOURCE_FILE:?}"
# pkgver cannot hold "-", so a nightly 0.68.0-nightly.20261028 is 0.68.0_nightly.20261028 to pacman.
PKGVER="${VERSION//-/_}"
VERIFY="${VERIFY:-build}"
DRY_RUN="${DRY_RUN:-0}"
[[ "$(id -u)" != 0 ]] || { echo "::error::publish-aur.sh must not run as root (makepkg)"; exit 1; }
[[ -f "$SOURCE_FILE" ]] || { echo "::error::source file not found: $SOURCE_FILE"; exit 1; }

template="$(cd "$(dirname "$0")/.." && pwd)/packaging/aur/$PKG/PKGBUILD"
[[ -f "$template" ]] || { echo "::error::no PKGBUILD template at $template"; exit 1; }

sha="$(sha256sum "$SOURCE_FILE" | cut -d' ' -f1)"
echo "==> Publishing $PKG $VERSION (sha256=$sha, verify=$VERIFY, dry_run=$DRY_RUN)"

work="$(mktemp -d)"
cd "$work"
GIT_SSH_COMMAND="${GIT_SSH_COMMAND:-ssh -o StrictHostKeyChecking=accept-new}" \
  git clone "${AUR_REMOTE:-ssh://aur@aur.archlinux.org/$PKG.git}" "$PKG"
cd "$PKG"

cur_ver="$(grep -m1 -oP '^pkgver=\K.*' PKGBUILD 2>/dev/null || true)"
cur_rel="$(grep -m1 -oP '^pkgrel=\K[0-9]+' PKGBUILD 2>/dev/null || echo 0)"
# A late tag from an older branch, such as a release-0.67 fix after 0.68.0, never takes the AUR backwards.
if [[ -n "$cur_ver" ]] && (($(vercmp "$PKGVER" "$cur_ver") < 0)); then
  echo "==> AUR already has the newer ${cur_ver}; not going back to ${PKGVER}."
  exit 0
fi

render() {
  sed -e "s/^pkgver=.*/pkgver=${PKGVER}/" \
      -e "s/^_version=.*/_version=${VERSION}/" \
      -e "s/^pkgrel=.*/pkgrel=$1/" \
      -e "s/^sha256sums=.*/sha256sums=('${sha}')/" \
      "$template" > PKGBUILD
}

# Rerunning a release the AUR has changes nothing; a changed PKGBUILD for it bumps pkgrel; a new version starts at 1.
if [[ "$cur_ver" == "$PKGVER" ]]; then
  render "$cur_rel"
  if git diff --quiet -- PKGBUILD; then
    echo "==> AUR already has ${PKGVER}-${cur_rel}; nothing to publish."
    exit 0
  fi
  rel=$((cur_rel + 1))
else
  rel=1
fi
echo "==> AUR has ${cur_ver:-nothing}-${cur_rel}; publishing ${PKGVER}-${rel}"
render "$rel"
makepkg --printsrcinfo > .SRCINFO

case "$VERIFY" in
  build) makepkg -f --noconfirm --nodeps ;;
  source) makepkg --verifysource --noconfirm ;;
  *) echo "::error::VERIFY must be build or source, not $VERIFY"; exit 1 ;;
esac

if git diff --quiet && git ls-files --error-unmatch PKGBUILD >/dev/null 2>&1; then
  echo "==> No changes; nothing to publish."
  exit 0
fi

git add PKGBUILD .SRCINFO
git -c user.name="kopuz release bot" -c user.email="noreply@kopuz.org" \
  commit -m "upgpkg: $PKG ${PKGVER}-${rel}"

if [[ "$DRY_RUN" == 1 ]]; then
  echo "==> DRY_RUN: not pushing"
  git show --stat HEAD
  exit 0
fi
git push origin HEAD:master
echo "==> Published $PKG ${PKGVER}-${rel} to the AUR."
