# Releasing stewards

Do the steps in order. Nothing here is automatic until the tag is pushed.

## One-time repository setup

The owner does this by hand in the GitHub settings before the first tag; the workflow cannot do it and does not check it.

- [ ] Create the environment `release` (Settings > Environments) with a required reviewer (not only the person who pushes the tag).
- [ ] Add the secret `CARGO_REGISTRY_TOKEN` to that environment, not to the repository. Use a crates.io token scoped to the `stewards` crate (`publish-new` for the first release, then `publish-update`).
- [ ] Add the secret `AUR_SSH_PRIVATE_KEY` to that environment, not to the repository. Use a dedicated AUR account or key that can push this one package, not the maintainer's main key.
- [ ] Add a tag ruleset (Settings > Rules) on `v*` that restricts who can create, update, and delete these tags.
- [ ] Leave no repository-level secret with either name. The `publish` and `aur` jobs read them from the `release` environment only.

## Each release

1. **Version bump.** Set `version` in `Cargo.toml` (the tag must be `v<version>`; the release workflow refuses a mismatch) and refresh `Cargo.lock` with `cargo build`. If `baml` changes, update `BAML_VERSION` and the four `lib_sha256` values in `.github/workflows/release.yml` and the library URL and sha256 in `packaging/aur/PKGBUILD` and `packaging/aur/.SRCINFO` (the workflow refuses a mismatch): download `libbaml_cffi-<target>.<so|dylib>` from the BAML release for each target and run `sha256sum` on it. Also replace `licenses/baml/LICENSE` and `licenses/baml/SOURCE` with BAML's files at that tag (add its `NOTICE` if the repository has one).
2. **Tests.** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, then `cargo test` twice, both green.
3. **Package.** `cargo package --list` shows only `src/`, `baml_src/`, `licenses/`, `Cargo.*`, `LICENSE`, `README.md`; `cargo publish --dry-run` passes.
4. **Review.** `stewards review` in this repo exits 0.
5. **Evals.** Run the `fixture` and `self` suites by hand (`stewards eval --suite fixture`, `stewards eval --suite self`) and read the results (OQ 3). Evals call models and cost money; they are not in the workflow.
6. **History scrub.** Remove private names from the history. This is done by hand by the maintainer; it is listed here only because it must be the last step before tagging.
7. **Tag and push.** `git tag v<version> && git push origin main v<version>`. The `release` workflow builds the four archives (each with `licenses/` and a `cargo-about` `THIRD-PARTY-LICENSES.html`), checks the BAML library against the pinned sha256, refuses a tag whose commit is not on `main`, attests build provenance, and creates the GitHub release with `SHA256SUMS` and `install.sh`. Then the `publish` job (crate; `CARGO_REGISTRY_TOKEN`) and the `aur` job (`AUR_SSH_PRIVATE_KEY`) each wait for a reviewer to approve the `release` environment. The one-time setup above must be done first.
8. **Verify the release.**
   - The release has four `stewards-<version>-<target>.tar.gz`, `SHA256SUMS`, and `install.sh`.
   - `curl -fsSL https://github.com/wromansky/stewards/releases/download/v<version>/install.sh | sh -s v<version>` on Linux and on macOS installs, and `stewards --version` prints `<version>`.
   - `gh attestation verify stewards-<version>-<target>.tar.gz --repo wromansky/stewards` passes for a downloaded archive.
   - The archive and `/usr/share/licenses/stewards/` (AUR) hold `licenses/baml/LICENSE`; the archive also holds `THIRD-PARTY-LICENSES.html`.
   - `stewards init --draft --direct` loads the bundled library (with a key set).
   - `cargo install stewards` works from crates.io.
   - The AUR package `stewards` shows the new `pkgver`; `makepkg -si` builds it.

## Known limits

- The Linux archives are built on Ubuntu 24.04 (glibc 2.39): older distributions use `cargo install stewards`. The macOS archives are built on `macos-15` (arm64) and `macos-15-intel`.
- The AUR package does not ship `THIRD-PARTY-LICENSES.html` (it builds from the crate, which cannot hold a file generated after the tag); it does ship BAML's licence.
- `cargo publish` and the AUR push cannot be undone: if the workflow fails after the release exists, fix forward with the next patch version. A failed AUR job can be re-run on its own.
- Prerelease tags (`v0.2.0-rc.1`) make a GitHub prerelease and a crates.io prerelease, and skip the AUR.
- `SHA256SUMS` is fetched from the same place as the archive, so it detects a corrupt download, not a tampered release.
