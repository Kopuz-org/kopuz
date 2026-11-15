#!/usr/bin/env bash
# Usage: release-version.sh, from the repo root in a release job. Checks the pushed tag against the workspace version
# and writes what the job builds to $GITHUB_OUTPUT.
#
#   v<ver>                     the release
#   v<ver>-beta.N, v<ver>-rc.N a pre-release of it: both apps, stable flavour, published as a GitHub pre-release
#   v<ver>-nightly.YYYYMMDD[.N] Kopuz Nightly, the GPUI app alone under its own name
#
# Cargo.toml keeps the version being worked towards, so a beta or nightly carries its tag's version in this build
# only; nothing is committed. Shared by the bash, macOS and Windows (Git Bash) jobs, so only portable sed, and no
# CR from a Windows jq.
set -euo pipefail

: "${GITHUB_OUTPUT:=/dev/stdout}"

base="$(
  cargo metadata --no-deps --format-version 1 |
    jq -r '.packages[] | select(.name == "kopuz-legacy") | .version' | tr -d '\r'
)"
base_re="${base//./\\.}"

version="$base"
channel=release
if [[ "${GITHUB_REF_TYPE:-}" == tag ]]; then
  tag="${GITHUB_REF_NAME#v}"
  if [[ "$tag" =~ ^${base_re}-nightly\.[0-9]{8}(\.[0-9]+)?$ ]]; then
    channel=nightly
  elif [[ "$tag" =~ ^${base_re}-(beta|rc)\.[0-9]+$ ]]; then
    channel=beta
  elif [[ "$tag" != "$base" ]]; then
    echo "::error::Tag ${GITHUB_REF_NAME} is none of v${base}, v${base}-beta.N, v${base}-rc.N or v${base}-nightly.YYYYMMDD"
    exit 1
  fi
  version="$tag"
fi

if [[ "$channel" != release ]]; then
  # Only the workspace version and the workspace's own crates, never a dependency that happens to share the number.
  sed -e "s/^version = \"${base_re}\"/version = \"${version}\"/" \
      -e "/package = \"kopuz-/s/version = \"${base_re}\"/version = \"${version}\"/" \
      Cargo.toml > Cargo.toml.next
  mv Cargo.toml.next Cargo.toml
  cargo update -w
  stamped="$(
    cargo metadata --no-deps --format-version 1 |
      jq -r '.packages[] | select(.name == "kopuz-legacy") | .version' | tr -d '\r'
  )"
  if [[ "$stamped" != "$version" ]]; then
    echo "::error::Stamping ${version} into Cargo.toml left kopuz-legacy at ${stamped}"
    exit 1
  fi
fi

nightly=false
prerelease=false
[[ "$channel" == nightly ]] && nightly=true
[[ "$channel" != release ]] && prerelease=true

{
  echo "version=${version}"
  echo "channel=${channel}"
  echo "nightly=${nightly}"
  echo "prerelease=${prerelease}"
  if [[ "$nightly" == true ]]; then
    echo "flavor=nightly"
    echo "package=kopuz-nightly"
    echo "suffix=-nightly"
    echo "asset=Kopuz-Nightly"
    echo "product=Kopuz Nightly"
  else
    echo "flavor=stable"
    echo "package=kopuz"
    echo "suffix="
    echo "asset=Kopuz"
    echo "product=Kopuz"
  fi
} >> "$GITHUB_OUTPUT"
