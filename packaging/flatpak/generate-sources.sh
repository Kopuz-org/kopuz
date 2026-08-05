#!/usr/bin/env bash

set -euo pipefail

CUR_DIR="$PWD"


if [[ "$CUR_DIR" == */packaging/flatpak ]]; then
    CUR_DIR="$(dirname "$(dirname "$CUR_DIR")")"
fi

WORKDIR="$(mktemp -d -t kopuz-flatpak-XXXXXX)"

cd "$WORKDIR"


python -m venv venv

source ./venv/bin/activate

pip install pipx

pipx install git+https://github.com/flatpak/flatpak-builder-tools.git#subdirectory=node --force

flatpak-node-generator npm "$CUR_DIR/package-lock.json" -o "pnpm-sources.json"

pip install flatpak-cargo-generator

flatpak-cargo-generator "$CUR_DIR/Cargo.lock" -o "cargo-sources.json"

#dioxus_releases=$(curl -fsSL "https://api.github.com/repos/DioxusLabs/dioxus/releases/latest")

#{
#    echo "["
#    suffix=","
#    for arch in x86_64 aarch64; do
#        file="dx-${arch}-unknown-linux-gnu.zip"
#        asset=$(jq -c --arg file "$file" '.assets[] | select(.name == $file)' <<<"$dioxus_releases")
#        url=$(jq -r '.browser_download_url' <<<"$asset")
#        digest=$(jq -r '.digest' <<<"$asset")
#        algo=${digest%%:*}
#        hash=${digest#*:}       
#        cat <<EOF
#  {
#    "type": "archive",
#    "url": "$url",
#    "$algo": "$hash",
#    "dest": "dioxus-cli",
#    "only-arches": ["$arch"]
#  }$suffix
#EOF
#    suffix=""
#    done
#    echo "]"
#} > dioxus-cli.json

# latest doesnt work so pinned to v130.0.7

#rusty_v8_releases=$(curl -fsSL "https://api.github.com/repos/denoland/rusty_v8/releases/latest")

#{
#    echo "["
#    suffix=","
#    for arch in x86_64 aarch64; do
#        file="librusty_v8_release_${arch}-unknown-linux-gnu.a.gz"        
#        asset=$(jq -c --arg file "$file" '.assets[] | select(.name == $file)' <<<"$rusty_v8_releases")
#        url=$(jq -r '.browser_download_url' <<<"$asset")
#        digest=$(jq -r '.digest' <<<"$asset")
#        algo=${digest%%:*}
#        hash=${digest#*:}       
#        cat <<EOF
#  {
#    "type": "file",
#    "url": "$url",
#    "$algo": "$hash",
#    "dest-filename": "librusty.a.gz",
#    "only-arches": ["$arch"]
#  }$suffix
#EOF
#    suffix=""
#    done
#    echo "]"

#} > librusty.json

cp "pnpm-sources.json" "$CUR_DIR/packaging/flatpak/pnpm-sources.json"

cp "cargo-sources.json" "$CUR_DIR/packaging/flatpak/cargo-sources.json"

cp "dioxus-cli.json" "$CUR_DIR/packaging/flatpak/dioxus-cli.json"

#cp "librusty.json" "$CUR_DIR/packaging/flatpak/librusty.json"