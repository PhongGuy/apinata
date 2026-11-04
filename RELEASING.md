# Releasing Apinata

A release is a `v<version>` tag. Pushing it runs `.github/workflows/release.yml`, which checks the tag, builds every platform, writes one `SHA256SUMS` over all artifacts, signs the update manifest when one is configured and drafts a GitHub release. A human reviews the draft and publishes it.

## Signing

### macOS

There is no Apple Developer account and no notarisation. The `.app` inside the `.dmg` and the CLI binary are ad-hoc signed.

On first open, Gatekeeper blocks the app. Right-click the app and choose Open, or run:

```sh
xattr -dr com.apple.quarantine /Applications/Apinata.app
```

For the CLI, run `xattr -d com.apple.quarantine apinata`.

### Windows

Windows artifacts are signed with a self-signed code-signing certificate, created once on the maintainer's machine. With OpenSSL:

```sh
openssl req -x509 -newkey rsa:3072 -sha256 -days 1825 -noenc -subj "/CN=Apinata" -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=codeSigning" -keyout apinata-code.key -out apinata-code.crt
openssl pkcs12 -export -inkey apinata-code.key -in apinata-code.crt -name Apinata -out apinata-code.pfx
base64 -w0 apinata-code.pfx | gh secret set WINDOWS_CERTIFICATE
gh secret set WINDOWS_CERTIFICATE_PASSWORD
```

The last command prompts for the export password. On Windows, `New-SelfSignedCertificate -Type CodeSigningCert` plus `Export-PfxCertificate` does the same. Keep the key and the `.pfx` offline.

SmartScreen still warns, because the certificate chains to no trusted root. Without both secrets, the installer and the `.zip` carry `-unsigned` in their names; with only one of them set, the job fails.

### Linux

Linux has no OS signing. Verify downloads with:

```sh
sha256sum -c SHA256SUMS
```

## Stable download names

The draft also carries version-free copies of the installers, so `https://github.com/PhongGuy/apinata/releases/latest/download/<name>` works for every release: `apinata-linux-x86_64.AppImage`, `apinata-linux-aarch64.AppImage`, `apinata-linux-amd64.deb`, `apinata-linux-arm64.deb`, `apinata-macos-aarch64.dmg`, `apinata-macos-x86_64.dmg` and `apinata-windows-setup.exe` (also for an `-unsigned` installer). They are copies with the same bytes as the versioned files, built by `scripts/release.sh stable dist stable` and uploaded after everything else. They are not in `SHA256SUMS` (verify the versioned file instead), and the updater and `latest.json` use only the versioned names.

## Updates

The app checks a signed `latest.json` and installs the release it names. The workflow writes the manifest after `SHA256SUMS`, signs it, and uploads it to the draft after every other artifact, with `latest.json.sig` last.

### Keys

Generate the key pair once on the maintainer's machine. Store the private key online only as the repository secret below, and keep the key file itself offline as the backup, since the secret is its only online copy:

```sh
cargo packager signer generate --path apinata-update.key
gh secret set CARGO_PACKAGER_SIGN_PRIVATE_KEY < apinata-update.key
gh secret set CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD
```

Repository secrets reach any workflow run. The stronger option is a GitHub environment whose deployment rules allow only version tags, holding the two signing secrets instead, with the checksums job set to that environment.

Append the public key, the `.pub` line, to `assets/update/keys.txt` and commit it. The app trusts every key listed there, so a build only accepts manifests signed by a key it was shipped with.

### Variables

Set these repository variables. Builds without them are unconfigured and never offer updates:

- `APINATA_UPDATE_BASE_URL`: the public `releases/download` URL, for example `https://github.com/<owner>/<repo>/releases/download`. Artifact URLs in the manifest are this value plus `/<tag>/<file>`. Without it, the manifest is neither built nor signed; with it, only tag runs build and sign one, so a dry run from a branch never produces a signed manifest.
- `APINATA_UPDATE_MANIFEST_URL`: where the app fetches `latest.json`, compiled into the app.
- `APINATA_DOWNLOAD_PAGE_URL`: the page the app opens when it cannot install in place.

### Publishing

The draft lives in this private repository, which anonymous clients cannot reach. When `APINATA_UPDATE_BASE_URL` points at a public repository, mirror every update artifact (the AppImage, the `.app.tar.gz`, the installers and raw CLI binaries) plus `latest.json`, and then `latest.json.sig`, to that repository's release `<tag>`. Upload the signature last, so no client sees a manifest before the files it names.

### Rotation and compromise

To rotate, generate a new pair, append its public key to `keys.txt`, ship a release signed by the old key, and only then switch the secrets to the new key. Remove the old line in a later release. If a private key leaks, every client that still lists it would accept a manifest an attacker signs with it, so ship a release that removes its line from `keys.txt` and is signed by another key those clients already trust. Clients that trust only the leaked key cannot verify releases signed only by the new key, so tell their users to reinstall from the download page.

### Release notes

On macOS an in-place update replaces the app bundle, and the system may ask for keychain access on first launch afterwards. Mention this in the release notes.

## Rehearsal

Dispatch a dry run from `master`. It builds the chosen platforms and drafts nothing:

```sh
gh workflow run release.yml --ref master -f dry_run=true -f platforms=linux,windows
gh run watch <id> --exit-status
```

The `release-<version>` artifact holds every file plus `SHA256SUMS`. A non-dry run must build every platform; the guard refuses a subset.

## Tagging

1. Refresh the public suffix list the cookie jar ships with: run
   `scripts/update-public-suffix-list.sh`, which replaces
   `crates/apinata-core/vendor/public_suffix_list.dat` and its `THIRD-PARTY.md` row
   together, then run `make check` and commit both as
   `chore(core): update the public suffix list`.
2. Set `[workspace.package] version` in `Cargo.toml` and commit.
3. `git tag v<version>`
4. `git push origin v<version>`

The guard fails on a tag that does not match the workspace version, before any build starts.

## Draft review

1. Download the draft's files and run `sha256sum -c SHA256SUMS`.
2. Mount the `.dmg` and run `codesign -v` on the `.app`.
3. Check the Windows installer's signature, or its `-unsigned` name.
4. Install the `.deb` on stock Ubuntu 24.04.
5. Publish the release in the GitHub UI.

## Actions minutes

Each job is billed in whole minutes; Windows bills 2x and macOS 10x.

- A CI run takes 10 to 12 billed minutes (measured on two pushes).
- A cold Linux release rehearsal measured 24 billed minutes for the x86_64 leg; the aarch64 leg failed in package verify after 16 (since fixed). ASSUMED: a full cold Linux pair is about 50; warm-cache times are not measured yet.
- ASSUMED: a full release is about 500: Linux about 50, Windows 25 x 2, macOS 2 x 20 x 10.
- ASSUMED: a `linux,windows` rehearsal is about 100.

Every job's `timeout-minutes` caps what a hung run can cost (`guard` 5, `gate` 45, `build` 90, `checksums` 45), per Decision 11 of plan 4e.
