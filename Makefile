.PHONY: build test audit assets bundle run-app clean verify release docs-check

build:
	cargo build --release

test:
	cargo test

# Dependency vulnerability scan (install once: cargo install cargo-audit --locked)
audit:
	cargo audit

# Regenerates icon PNGs, menu bar template icons and AppIcon.icns
assets:
	cargo run --release --bin gen_assets
	iconutil -c icns assets/AppIcon.iconset -o assets/AppIcon.icns
	rm -rf assets/AppIcon.iconset

# make bundle [WHISPER=1] to bundle Homebrew's whisper-cli
bundle:
	scripts/bundle.sh $(if $(WHISPER),--bundle-whisper,)

# Pre-release checks of dist/HoldNote.app (make verify [TRANSCRIBE=1])
verify:
	scripts/verify-bundle.sh $(if $(TRANSCRIBE),--transcribe,)

# make release VERSION=x.y.z [DRY_RUN=1] [NO_WHISPER=1] [TAP_DIR=path]
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=x.y.z [CHANGES=file.md] [DRY_RUN=1]" >&2; exit 2; }
	scripts/release.sh $(VERSION) $(if $(DRY_RUN),--dry-run,) $(if $(NO_WHISPER),--no-whisper,) $(if $(TAP_DIR),--tap-dir $(TAP_DIR),) $(if $(CHANGES),--changes $(CHANGES),)

run-app: bundle
	open dist/HoldNote.app

clean:
	cargo clean
	rm -rf dist

# Broken relative links / #anchors in README.md and docs/
docs-check:
	scripts/check-docs.py
