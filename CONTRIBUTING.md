# Contributing to Apinata

This is the developer guide; the user-facing page is [README.md](README.md),
and every command, flag and setting is in [GUIDE.md](GUIDE.md).

## Requirements

- Rust 1.99 or newer (the `rust-version` in `Cargo.toml`), with `rustfmt` and
  `clippy`.
- On Debian or Ubuntu, the system packages CI installs:

```sh
sudo apt-get install -y --no-install-recommends pkg-config libfontconfig-dev libfreetype-dev \
  libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libvulkan1 libegl1
```

- For `make windows-check` only: the Windows target and the mingw cross
  compiler (`gcc-mingw-w64-x86-64` on Debian or Ubuntu, `mingw-w64-gcc` on
  Arch).

```sh
rustup target add x86_64-pc-windows-gnu
```

## Build from source

Build and install the app and the CLI for the current user:

```sh
make install
```

- **Linux:** `apinata` and `apinata-app` go to `~/.local/bin`, with a launcher
  entry and icon under `~/.local/share`. Set `PREFIX` in the environment to
  install elsewhere, and `DESTDIR` to stage the files for a package.
- **macOS:** the app goes to `~/Applications/Apinata.app` and `apinata` to
  `~/.local/bin`.
- **Windows** (Git Bash or MSYS2): both programs go to
  `%LOCALAPPDATA%\Programs\Apinata`; add that folder to your `PATH`.

A bare `make` does the same. The MCP server needs no separate install: it is
`apinata mcp`. To remove everything again:

```sh
make uninstall
```

To build without installing, run `cargo build --release`; the binaries land in
`target/release/`.

## Try a change

```sh
make demo                 # builds the app and opens examples/demo on a throwaway profile
cargo run -p apinata-app  # runs the app on your real profile
cargo run -p apinata-cli -- --help
```

`make demo` copies the sample collection to `~/.cache/apinata-demo` on every
run and keeps its profile there, so your real settings and the repository stay
untouched. `DEMO_DRY_RUN=1 make demo` prints the launch line instead. A plain
`cargo run -p apinata-app` uses your real profile and keychain, which is why
`make demo` is the safer way to try a change.

## Linux packages

To build the Linux `.deb`, AppImage and CLI tarball into `target/dist/linux/`
(`OUT_DIR` moves them), install cargo-packager first:

```sh
cargo install --locked cargo-packager --version 0.11.8
make package
```

How a release is cut, signed and checked is in [RELEASING.md](RELEASING.md).

## Layout

- `crates/apinata-core`: the UI-free core. Reading and writing collections,
  variables, sending requests, scripts and assertions all live here.
- `crates/apinata-format`: the YAML reader the app and the team server rules
  share.
- `crates/apinata-cli`: the `apinata` binary, including `apinata mcp`.
- `crates/apinata-mcp`: the MCP server the CLI serves.
- `crates/apinata-app`: the GPUI desktop app, `apinata-app`.
- `crates/apinata-rules`: the push rules for team workspaces on a Potluck
  server.
- `crates/xtask`: repository automation (`cargo xtask`): the Windows
  cross-check, icon exports and packaging config.
- `tools/apinata-pilot`: the gpui-pilot driver for QA, outside the workspace.
- `tools/script-bundles`: rebuilds the JavaScript libraries bundled for
  request scripts from pinned npm versions.
- `scripts/`: install, demo, packaging, release and licence scripts, with
  their tests in `scripts/tests` (run by `cargo test -p xtask`).
- `examples/demo`: the sample collection `make demo` opens.
- `assets/`: the icon sources, the README screenshots and the update signing keys.

The CLI, the MCP server and the app only call `apinata-core`, so a request
behaves the same in all three. Behaviour belongs in core, with a thin call
from each shell.

## Tests and checks

```sh
make check          # the CI gate: fmt-check, lint, test, core-check, readme-images-check
make fmt            # format the workspace
make core-check     # core without default features, and its docs with warnings as errors
make windows-check  # type-checks the headless crates and their tests for Windows
make pilot-check    # fmt, clippy and the tests of tools/apinata-pilot
make icons-check    # the icon exports match the SVG sources
make workflows-lint # actionlint over .github/workflows (Linux x86_64 or arm64)
make licenses-check # every dependency's licence is allowed (needs cargo-about 0.9.3)
```

`make check` is what a change must pass. Its `test` step runs
`cargo test --workspace` with `DISPLAY` and `WAYLAND_DISPLAY` unset, so the
app's tests never open a window on your desktop. `make core-check` matters
whenever core changes: CI fails a broken doc link in core's public API.
`make windows-check` skips with a message when the target or mingw is missing;
CI runs it with `--require`, so there it fails instead.

Each crate has one integration test binary, `integration`: `tests/main.rs`
declares every `tests/*.rs` file as a module, and `autotests = false` stops
cargo from building them one by one. A new test file must be added to
`tests/main.rs`, or it never runs. Run one module with:

```sh
cargo test -p apinata-core --test integration open::
```

Tests must talk only to `127.0.0.1` and never touch your real keychain, config
or data directories. CI cross-builds the tests for Windows, so tests that need
Unix-only helpers carry `#[cfg(unix)]`.

## CI

`.github/workflows/ci.yml` runs on every push to `master` and every pull
request:

- **gate**, on Ubuntu: `make check icons-check windows-check workflows-lint`,
  the same targets you can run locally;
- **zizmor**: audits the workflows for injection, credential and pinning
  mistakes.

Two more jobs run only when started by hand from the Actions tab: clippy on
macOS, and the core git stop tests on Windows.

## Conventions

- Commit messages follow
  [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):
  `<type>(<scope>): <description>`, for example
  `fix(cli): keep the exit code when a report fails to write`. The types are
  `feat`, `fix`, `chore`, `refactor`, `docs` and `test`. Keep each commit to
  one change.
- One pull request per issue, with `Closes #N` in its body.
- Public types in `apinata-core` are `#[non_exhaustive]`, and no dependency's
  type appears in a public signature.
- Comments explain the intent and the one non-obvious constraint in a line or
  three; they do not restate the code.
- A new behaviour or fix comes with a test that fails without it.
- A change is done when `make check` passes.

## Issues and pull requests

Open an issue from one of the forms on the
[new issue page](https://github.com/PhongGuy/apinata/issues/new/choose):

- **Bug**: something is broken or gives a wrong result;
- **Feature**: a new ability, or a change to how something works;
- **Looks wrong or feels clunky**: layout, wording, focus or keyboard flow;
- **Accessibility barrier**: something hard or impossible to use with a
  keyboard, screen reader, zoom or other assistive technology;
- **Task**: maintenance such as a refactor, a dependency bump or CI.

Blank issues are off. Remove tokens, passwords and other secrets from anything
you paste.

To send a change:

1. Comment on the issue you want to take, or open one first for anything
   larger than a small fix, so the design can be agreed before you build it.
2. Fork the repository and branch from `master`.
3. Make the change with its tests, and run `make check` (plus
   `make pilot-check` if you touched the app or `tools/apinata-pilot`).
4. Open a pull request against `master` with a plain-words title and
   `Closes #N` in the body, saying what changed and why.

CI runs on every pull request; a first-time contributor's run waits for
approval.

## Pilot QA

`tools/apinata-pilot` runs the real app under
[gpui-pilot](https://github.com/PhongGuy/gpui-pilot), so an MCP client such as
an AI coding assistant can click, type, read the element tree and take
screenshots. Install the server at the tag the pilot crate builds against
(`gpui-pilot-host` in `tools/apinata-pilot/Cargo.toml`):

```sh
cargo install --locked --git https://github.com/PhongGuy/gpui-pilot --tag v0.6.0 gpui-pilot
```

`.mcp.json` registers it as `gpui-pilot mcp`, and `gpui-pilot.toml` says how
to build and launch the app:

- **headless** builds the pilot crate and runs the app without a display, on a
  fresh data directory per launch. Secrets go to `pilot-secrets.json` in that
  directory, never your keychain.
- **session** builds `apinata-app` and runs it in a sandboxed desktop session.

Launch arguments are collection roots, each an absolute path to an existing
directory. Keep `CARGO_TARGET_DIR` unset while you use the pilot: it moves the
binary away from the path `gpui-pilot.toml` names, and the launch fails.

The pilot crate sits outside the workspace, so `make check` and CI never build
it; run `make pilot-check` before sending a change to it.
