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

make_asset_source() {
    releases=$(curl -fsSL "$1")
    {
        echo "["
        suffix=","
        for arch in x86_64 aarch64; do
            file="${2//\{\@\}/$arch}"      
            asset=$(jq -c --arg file "$file" '.assets[] | select(.name == $file)' <<<"$releases")
            url=$(jq -r '.browser_download_url' <<<"$asset")
            digest=$(jq -r '.digest' <<<"$asset")
            if [[ "$digest" != "null" ]]; then
                algo=${digest%%:*}
                hash=${digest#*:}    
            else 
                algo="sha512"
                curl -L -o "$file" "$url"
                hash=$(sha512sum "$file" | awk '{print $1}')
            fi           
            cat <<EOF
  {
    "type": "$3",
    "url": "$url",
    "$algo": "$hash",
    "$4": "$5",
    "only-arches": ["$arch"]
  }$suffix
EOF
        suffix=""
        done
        echo "]"

    } > "$6"
}

make_asset_source \
"https://api.github.com/repos/denoland/rusty_v8/releases/tags/v$(grep -oPm1 '"dest": "cargo/vendor/v8-\K[0-9]+([\.\d]+)' cargo-sources.json)" \
"librusty_v8_release_{@}-unknown-linux-gnu.a.gz" \
"file" \
"dest-filename" \
"librusty.a.gz" \
"librusty.json"

make_asset_source \
"https://api.github.com/repos/DioxusLabs/dioxus/releases/tags/v$(grep -oPm1 'cargo install dioxus-cli@\K[0-9]+([\.\d]+)' $CUR_DIR/.github/workflows/release.yml)" \
"dx-{@}-unknown-linux-gnu.zip" \
"archive" \
"dest" \
"dioxus-cli" \
"dioxus-cli.json"



cp "pnpm-sources.json" "$CUR_DIR/packaging/flatpak/pnpm-sources.json"

cp "cargo-sources.json" "$CUR_DIR/packaging/flatpak/cargo-sources.json"

cp "dioxus-cli.json" "$CUR_DIR/packaging/flatpak/dioxus-cli.json"

cp "librusty.json" "$CUR_DIR/packaging/flatpak/librusty.json"