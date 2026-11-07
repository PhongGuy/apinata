# One command surface for contributors and CI: `make check` is the full merge gate.
CARGO ?= cargo

# A bare `make` installs the app and the CLI for the current user.
.DEFAULT_GOAL := install
export PREFIX DESTDIR DEMO_DRY_RUN

.PHONY: check core-check fmt fmt-check lint test pilot-check package package-release clean icons-check windows-check workflows-lint licenses licenses-check readme-images-check install uninstall demo

check: fmt-check lint test core-check readme-images-check

fmt:
	$(CARGO) fmt

fmt-check:
	$(CARGO) fmt --check

lint:
	$(CARGO) clippy --all-targets --all-features --locked -- -Dwarnings

# The app's tests must never open a window on the developer's display, so both display
# variables are unset for the test run.
test:
	env -u WAYLAND_DISPLAY -u DISPLAY $(CARGO) test --workspace --locked

# Core must build without the keychain feature, and its public docs must have no broken links.
core-check:
	$(CARGO) check -p apinata-core --no-default-features --locked
	RUSTDOCFLAGS=-Dwarnings $(CARGO) doc --no-deps -p apinata-core --locked

# The pilot package sits outside the workspace because its host crate is private, so CI never builds it.
PILOT := --manifest-path tools/apinata-pilot/Cargo.toml
pilot-check:
	$(CARGO) fmt --check $(PILOT)
	$(CARGO) clippy $(PILOT) --all-targets --all-features -- -Dwarnings
	env -u WAYLAND_DISPLAY -u DISPLAY $(CARGO) test $(PILOT)

package:
	scripts/package-linux.sh

# PREFIX (default ~/.local) and DESTDIR pass through to the script; see scripts/install.sh.
install:
	bash scripts/install.sh install

uninstall:
	bash scripts/install.sh uninstall

# Opens examples/demo in the app with a throwaway profile; DEMO_DRY_RUN=1 only prints the launch.
demo:
	bash scripts/demo.sh

# Run through bash so Windows never depends on the exec bit or the shebang.
package-release:
	bash scripts/release.sh package

clean:
	$(CARGO) clean

icons-check:
	$(CARGO) xtask icons --check

windows-check:
	$(CARGO) xtask check-windows --require

workflows-lint:
	scripts/actionlint.sh

# Not part of `make check`, which must not need cargo-about; run both before a release.
licenses:
	bash scripts/licenses.sh generate

licenses-check:
	bash scripts/licenses.sh check

# Every local image the README names must exist once assets/screenshots/ does.
readme-images-check:
	bash scripts/readme-images-check.sh
