#!/usr/bin/env bash
# Release version gate.
#
# Always checks that the workspace version is consistent across every place it
# is duplicated (internal crate pins, the Nix package, the AppStream metainfo).
# With --require-bump it additionally asserts the version is strictly greater
# than the latest v* git tag -- used on pull requests into the release branch so
# a merge that forgets to bump (or half-bumps) fails red.
#
# Usage: scripts/check_release_version.sh [--require-bump]
set -euo pipefail

require_bump=0
[[ "${1:-}" == "--require-bump" ]] && require_bump=1

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail() { echo "::error::$*" >&2; echo "FAIL: $*" >&2; exit 1; }
ok()   { echo "  ok: $*"; }

# --- workspace version (source of truth) ------------------------------------
# First `version = "x"` at column 0 is [workspace.package].
ver="$(grep -m1 -oP '^version = "\K[^"]+' Cargo.toml)" || true
[[ -n "$ver" ]] || fail "could not read [workspace.package] version from Cargo.toml"
echo "workspace version: $ver"

# --- consistency: internal crate path-dep pins ------------------------------
# Every internal crate ({ ..., path = "./crates/..." }) must pin == $ver.
bad_pins="$(grep -nP 'path = "\./crates/' Cargo.toml \
  | grep -oP 'version = "\K[^"]+' | grep -vFx "$ver" || true)"
[[ -z "$bad_pins" ]] || fail "internal crate pins in Cargo.toml disagree with $ver (found: $(echo "$bad_pins" | sort -u | tr '\n' ' '))"
ok "internal crate pins all == $ver"

# --- consistency: Nix package version ---------------------------------------
crane_ver="$(grep -m1 -oP 'version = "\K[^"]+' packaging/nix/crane.nix)" || true
[[ "$crane_ver" == "$ver" ]] || fail "packaging/nix/crane.nix version ($crane_ver) != $ver"
ok "crane.nix version == $ver"

# --- consistency: AppStream metainfo has a release entry --------------------
grep -qP "<release version=\"$(printf '%s' "$ver" | sed 's/\./\\./g')\"" \
  data/com.temidaradev.kopuz.metainfo.xml \
  || fail "data/com.temidaradev.kopuz.metainfo.xml has no <release version=\"$ver\"> entry"
ok "metainfo has <release version=\"$ver\">"

# --- bump check (PR mode) ---------------------------------------------------
if [[ "$require_bump" == 1 ]]; then
  latest_tag="$(git tag -l 'v*' | sed 's/^v//' \
    | grep -E '^[0-9]+\.[0-9]+\.[0-9]+' | sort -V | tail -1 || true)"
  if [[ -z "$latest_tag" ]]; then
    ok "no prior v* tag; any version accepted"
  else
    echo "latest released tag: v$latest_tag"
    if [[ "$ver" == "$latest_tag" ]]; then
      fail "version not bumped: Cargo.toml is still $ver (== latest tag v$latest_tag)"
    fi
    # strictly-greater: $ver must sort after $latest_tag and differ.
    higher="$(printf '%s\n%s\n' "$latest_tag" "$ver" | sort -V | tail -1)"
    [[ "$higher" == "$ver" ]] || fail "version regressed: $ver < latest tag v$latest_tag"
    ok "version $ver > latest tag v$latest_tag"
  fi
fi

# --- machine-readable outputs (for GitHub Actions) --------------------------
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  { echo "version=$ver"; } >> "$GITHUB_OUTPUT"
fi
echo "version gate passed for $ver"
