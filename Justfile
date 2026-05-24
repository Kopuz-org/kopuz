default:
    @just --list

tailwind:
    tailwindcss -i ./tailwind.css -o ./crates/kopuz/assets/tailwind.css --content './crates/kopuz/**/*.rs,./crates/components/**/*.rs,./crates/pages/**/*.rs,./crates/hooks/**/*.rs,./crates/player/**/*.rs,./crates/reader/**/*.rs'

serve: tailwind
    dx serve

test:
    cargo nextest run --workspace

test-doc:
    cargo test --workspace --doc

test-ci: test test-doc

build: tailwind
    dx build --package kopuz --release
    @echo ""
    @echo "Build complete!"

run-release: build
    target/dx/kopuz/release/linux/app/kopuz

clean:
    cargo clean
    rm -rf target/dx dist build-dir .flatpak-builder

flatpak:
    @chmod +x packaging/flatpak/build-flatpak.sh
    ./packaging/flatpak/build-flatpak.sh

flatpak-install: flatpak

flatpak-run:
    flatpak run com.temidaradev.kopuz
