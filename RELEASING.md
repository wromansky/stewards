# Releasing stewards

Do the steps in order. Nothing here is automatic until the tag is pushed.

1. **Version bump.** Set `version` in `Cargo.toml` (the tag must be `v<version>`; the release workflow refuses a mismatch) and refresh `Cargo.lock` with `cargo build`. If `baml` changes, update `BAML_VERSION` and the four `lib_sha256` values in `.github/workflows/release.yml` and the library URL and sha256 in `packaging/aur/PKGBUILD` and `packaging/aur/.SRCINFO`: download `libbaml_cffi-<target>.<so|dylib>` from the BAML release for each target and run `sha256sum` on it.
2. **Tests.** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, then `cargo test` twice, both green.
3. **Package.** `cargo package --list` shows only `src/`, `baml_src/`, `Cargo.*`, `LICENSE`, `README.md`; `cargo publish --dry-run` passes.
4. **Review.** `stewards review` in this repo exits 0.
5. **Evals.** Run the `fixture` and `self` suites by hand (`stewards eval --suite fixture`, `stewards eval --suite self`) and read the results (OQ 3). Evals call models and cost money; they are not in the workflow.
6. **History scrub.** Remove private names from the history. This is done by hand by the maintainer; it is listed here only because it must be the last step before tagging.
7. **Tag and push.** `git tag v<version> && git push origin main v<version>`. The `release` workflow builds the four archives, checks the BAML library against the pinned sha256, creates the GitHub release with `SHA256SUMS` and `install.sh`, then publishes the crate (secret `CARGO_REGISTRY_TOKEN`) and bumps the AUR package (secret `AUR_SSH_PRIVATE_KEY`). Both secrets must exist in the repository settings before the tag.
8. **Verify the release.**
   - The release has four `stewards-<version>-<target>.tar.gz`, `SHA256SUMS`, and `install.sh`.
   - `curl -fsSL https://github.com/wromansky/stewards/releases/download/v<version>/install.sh | sh -s v<version>` on Linux and on macOS installs, and `stewards --version` prints `<version>`.
   - `stewards init --draft --direct` loads the bundled library (with a key set).
   - `cargo install stewards` works from crates.io.
   - The AUR package `stewards` shows the new `pkgver`; `makepkg -si` builds it.

## Known limits

- The Linux archives are built on Ubuntu 24.04 (glibc 2.39): older distributions use `cargo install stewards`. The macOS archives are built on `macos-15` (arm64) and `macos-15-intel`.
- `cargo publish` and the AUR push cannot be undone: if the workflow fails after the release exists, fix forward with the next patch version. A failed AUR job can be re-run on its own.
- Prerelease tags (`v0.2.0-rc.1`) make a GitHub prerelease and a crates.io prerelease, and skip the AUR.
- `SHA256SUMS` is fetched from the same place as the archive, so it detects a corrupt download, not a tampered release.
