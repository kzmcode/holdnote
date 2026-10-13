.PHONY: build test assets bundle run-app clean

build:
	cargo build --release

test:
	cargo test

# Regenerates icon PNGs, menu bar template icons and AppIcon.icns
assets:
	cargo run --release --bin gen_assets
	iconutil -c icns assets/AppIcon.iconset -o assets/AppIcon.icns
	rm -rf assets/AppIcon.iconset

# make bundle [WHISPER=1] to bundle Homebrew's whisper-cli
bundle:
	scripts/bundle.sh $(if $(WHISPER),--bundle-whisper,)

run-app: bundle
	open dist/HoldNote.app

clean:
	cargo clean
	rm -rf dist
