serve:
	dx serve

tailwind:
	npx @tailwindcss/cli -i ./tailwind.css -o ./crates/kopuz/assets/tailwind.css --content './crates/kopuz/**/*.rs,./crates/components/**/*.rs,./crates/pages/**/*.rs,./crates/hooks/**/*.rs,./crates/player/**/*.rs,./crates/reader/**/*.rs'

build: tailwind
	dx build --package kopuz --release
	@echo ""
	@echo "Build complete!"

run-release:
	cd target/dx/kopuz/release/linux/app && ./kopuz

flatpak:
	@chmod +x packaging/flatpak/build-flatpak.sh
	./packaging/flatpak/build-flatpak.sh

flatpak-install: flatpak

flatpak-run:
	flatpak run com.temidaradev.kopuz

clean:
	cargo clean
	rm -rf target/dx dist build-dir .flatpak-builder

