#!/usr/bin/env bash
# Update the kopuz-bin AUR package to the current release and push it.
#
# Must run as a NON-root user (makepkg refuses root) with passwordless sudo and,
# unless DRY_RUN=1, an AUR-authorized SSH key already usable by `git`.
#
# Env:
#   VERSION           release version, e.g. 0.9.0            (required)
#   TARBALL           path to the freshly built local tarball (required; for sha256)
#   PKGBUILD_TEMPLATE path to repo packaging/aur/PKGBUILD    (required)
#   AUR_TEST_BUILD    run `makepkg` before pushing (default 1)
#   DRY_RUN           do everything except `git push`        (default 0)
set -euo pipefail

: "${VERSION:?}"; : "${TARBALL:?}"; : "${PKGBUILD_TEMPLATE:?}"
AUR_TEST_BUILD="${AUR_TEST_BUILD:-1}"
DRY_RUN="${DRY_RUN:-0}"
[[ "$(id -u)" != 0 ]] || { echo "::error::publish-aur.sh must not run as root (makepkg)"; exit 1; }
[[ -f "$TARBALL" ]] || { echo "::error::tarball not found: $TARBALL"; exit 1; }

sha="$(sha256sum "$TARBALL" | cut -d' ' -f1)"
echo "==> Publishing kopuz-bin $VERSION (sha256=$sha, dry_run=$DRY_RUN)"

work="$(mktemp -d)"; cd "$work"
GIT_SSH_COMMAND="${GIT_SSH_COMMAND:-ssh -o StrictHostKeyChecking=accept-new}" \
  git clone ssh://aur@aur.archlinux.org/kopuz-bin.git
cd kopuz-bin

# pkgrel: bump within the same pkgver, else reset to 1.
cur_ver="$(grep -m1 -oP '^pkgver=\K.*' PKGBUILD || echo '')"
cur_rel="$(grep -m1 -oP '^pkgrel=\K[0-9]+' PKGBUILD || echo 0)"
if [[ "$cur_ver" == "$VERSION" ]]; then rel=$((cur_rel + 1)); else rel=1; fi
echo "==> current AUR: ${cur_ver:-none}-${cur_rel}; new: ${VERSION}-${rel}"

# Render the repo template -> the exact PKGBUILD we publish.
sed -e "s/^pkgver=.*/pkgver=${VERSION}/" \
    -e "s/^pkgrel=.*/pkgrel=${rel}/" \
    -e "s/^sha256sums=.*/sha256sums=('${sha}')/" \
    "$PKGBUILD_TEMPLATE" > PKGBUILD
makepkg --printsrcinfo > .SRCINFO

echo "==> Rendered PKGBUILD:"; sed -n '1,6p;/^source=/,/^sha256sums=/p' PKGBUILD

if [[ "$AUR_TEST_BUILD" == 1 ]]; then
  echo "==> Test build (makepkg; downloads + sha-verifies the published asset)"
  makepkg -f --noconfirm --nodeps
fi

if git diff --quiet; then echo "==> No changes; nothing to publish."; exit 0; fi

git add PKGBUILD .SRCINFO
git -c user.name="kopuz release bot" -c user.email="noreply@kopuz.org" \
  commit -m "upgpkg: kopuz-bin ${VERSION}-${rel}"

if [[ "$DRY_RUN" == 1 ]]; then
  echo "==> DRY_RUN: skipping push. Would push:"; git show --stat HEAD; exit 0
fi
git push origin master
echo "==> Published kopuz-bin ${VERSION}-${rel} to AUR."
