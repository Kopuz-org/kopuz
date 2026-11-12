#!/usr/bin/env bash
# Prepares a Kopuz Legacy patch release on its maintenance branch.
#
#   scripts/legacy-release.sh <version> "<release summary>" [commit...]
#
# For 0.67.1 it switches to release-0.67, creating it from the newest v0.67.x
# tag when it does not exist yet, cherry-picks the commits in the order given,
# bumps every version file, adds the AppStream and F-Droid entries, runs fmt
# and clippy, and commits the bump. It never pushes: it prints the commands
# that push the branch and the tag, and the tag push starts the release.
#
# A cherry-pick that conflicts stops the script. Resolve it, run
# `git cherry-pick --continue`, then run the script again with the commits
# that were not picked yet. Set SKIP_CHECKS=1 to skip fmt and clippy.

set -euo pipefail

if [[ -z ${LEGACY_RELEASE_SELF:-} ]]; then
    self="$(mktemp -t legacy-release.XXXXXX)"
    cp "$0" "$self"
    LEGACY_RELEASE_SELF="$self" exec bash "$self" "$@"
fi
trap 'rm -f "$LEGACY_RELEASE_SELF"' EXIT

die() {
    printf 'legacy-release: %s\n' "$*" >&2
    exit 1
}

[[ $# -ge 2 ]] || die 'usage: scripts/legacy-release.sh <version> "<release summary>" [commit...]'
version="$1"
summary="$2"
shift 2

[[ $version =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || die "version must look like 0.67.1, got $version"
major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"
(( patch > 0 )) || die "$version is not a patch release; cut x.y.0 from master"
series="$major.$minor"
branch="release-$series"
tag="v$version"

cd "$(git rev-parse --show-toplevel)"
[[ -z $(git status --porcelain) ]] || die 'the working tree is not clean'
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "$tag already exists"

for commit in "$@"; do
    git rev-parse -q --verify "$commit^{commit}" >/dev/null || die "unknown commit $commit"
done

git fetch --quiet origin --tags
if git rev-parse -q --verify "refs/heads/$branch" >/dev/null; then
    git switch --quiet "$branch"
elif git rev-parse -q --verify "refs/remotes/origin/$branch" >/dev/null; then
    git switch --quiet --track "origin/$branch"
else
    base="$(git tag --list "v$series.*" | grep -E "^v$series\.[0-9]+$" | sort -V | tail -1)"
    [[ -n $base ]] || die "no v$series.x tag to branch from"
    git switch --quiet -c "$branch" "$base"
    printf 'Created %s from %s\n' "$branch" "$base"
fi

for commit in "$@"; do
    printf 'Picking %s\n' "$(git log --format='%h %s' -1 "$commit")"
    git cherry-pick "$commit" || die "$commit conflicts; resolve it, run git cherry-pick --continue, then rerun with the commits after it"
done

previous="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
[[ -n $previous ]] || die 'could not read the workspace version from Cargo.toml'
[[ $previous != "$version" ]] || die "Cargo.toml is already at $version"
[[ $(printf '%s\n%s\n' "$previous" "$version" | sort -V | tail -1) == "$version" ]] \
    || die "$version is older than the branch's $previous"

printf 'Bumping %s to %s\n' "$previous" "$version"
perl -pi -e "s/version = \"\Q$previous\E\"/version = \"$version\"/g" Cargo.toml packaging/nix/*.nix
SQLX_OFFLINE=true cargo update --workspace --offline --quiet

VERSION="$version" PREVIOUS="$previous" SUMMARY="$summary" \
    CODE="$(( major * 10000 + minor * 100 + patch ))" \
    python3 - <<'PY'
import datetime
import html
import os
import re
import sys

version = os.environ["VERSION"]
previous = os.environ["PREVIOUS"]
code = os.environ["CODE"]

metainfo_path = "data/moe.kopuz.kopuz.metainfo.xml"
metainfo = open(metainfo_path, encoding="utf-8").read()
first_release = metainfo.find("    <release ")
if first_release < 0:
    sys.exit(f"no <release> entry in {metainfo_path}")
entry = (
    f'    <release version="{version}" date="{datetime.date.today().isoformat()}">\n'
    "      <description>\n"
    "        <p>\n"
    f"          {html.escape(os.environ['SUMMARY'], quote=False)}\n"
    "        </p>\n"
    "      </description>\n"
    "    </release>\n"
)
open(metainfo_path, "w", encoding="utf-8").write(metainfo[:first_release] + entry + metainfo[first_release:])

fdroid_path = "packaging/fdroid/moe.kopuz.kopuz.yml"
fdroid = open(fdroid_path, encoding="utf-8").read()
blocks = list(re.finditer(rf"^  - versionName: {re.escape(previous)}\n(?:    .*\n|      .*\n|        .*\n)+", fdroid, re.M))
if not blocks:
    sys.exit(f"no F-Droid build for {previous} in {fdroid_path}")
block = blocks[-1]
new_block = (
    block.group(0)
    .replace(f"versionName: {previous}", f"versionName: {version}")
    .replace(f"commit: v{previous}", f"commit: v{version}")
)
new_block = re.sub(r"versionCode: \d+", f"versionCode: {code}", new_block)
fdroid = fdroid[: block.end()] + new_block + fdroid[block.end():]
fdroid = re.sub(r"^CurrentVersion: .*$", f"CurrentVersion: {version}", fdroid, flags=re.M)
fdroid = re.sub(r"^CurrentVersionCode: .*$", f"CurrentVersionCode: {code}", fdroid, flags=re.M)
open(fdroid_path, "w", encoding="utf-8").write(fdroid)
PY

if [[ -z ${SKIP_CHECKS:-} ]]; then
    printf 'Running cargo fmt --check and clippy (debug and release)\n'
    cargo fmt --all -- --check
    SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings
    SQLX_OFFLINE=true cargo clippy --workspace --all-targets --release -- -D warnings
fi

git add Cargo.toml Cargo.lock packaging/nix data/moe.kopuz.kopuz.metainfo.xml packaging/fdroid/moe.kopuz.kopuz.yml
git commit --quiet -m "chore: bump version to $version"

cat <<EOF

$branch is ready at $(git rev-parse --short HEAD). Run the tests that cover the picked
commits, then publish it; the tag push starts the release workflow:

    git push -u origin $branch
    git -c tag.gpgSign=false tag $tag
    git push origin $tag
EOF
