# Task 03: completions, man page, release pipeline

Repo: stewards (Rust), branch off `main`. Follow `docs/build/common.md`.
Model: Sonnet 5.5, thinking high. Security review on the patch (release secrets, downloaded
binaries) before merge.

Read DESIGN.md: Shape › Stack and LLM modes (`BAML_LIBRARY_PATH`, `STEWARDS_BAML_DOWNLOAD`),
Verification and operations › Distribution and Versioning, Release and versioning (line ~1842),
Facts › BAML Rust (the `libbaml_cffi` download URL, no published checksum), the Command surface row
`completions <shell>`. Then `src/main.rs`, `src/completions.rs`, `src/llm.rs` `prepare_baml_env`,
`.github/workflows/ci.yml`, `tests/skeleton.rs`.

## Why
0.1.0 ships as `cargo install stewards`, GitHub release archives with the matching
`libbaml_cffi` (only `--direct`, used by `init --draft --direct`, loads it), an install script, and
an AUR package, with shell completions and a man page.

## Build
1. **`stewards completions <shell>`** (`bash zsh fish elvish powershell`, a clap value enum so a
   bad shell is the usual `usage` error) prints the script from `clap_complete` on stdout. New
   dependency `clap_complete`: generating five shells' scripts by hand would drift from clap. Every
   new command prints what to do next: one line on stderr (stdout is the script), e.g. `save it:
   stewards completions bash > ~/.local/share/bash-completion/completions/stewards`.
2. **`stewards man`** DECIDED BY PLANNER (DESIGN.md says "at build time"; a command is smaller than a
   build script that would need the CLI moved into the library): prints the roff page from
   `clap_mangen` (new dependency, same reason) on stdout, the save hint on stderr. Add the row to
   the Command surface table and change the Release sentence to "the release workflow generates
   them with `stewards completions` and `stewards man`".
3. **Bundled library lookup.** DECIDED BY PLANNER (DESIGN.md says the install script "sets
   `BAML_LIBRARY_PATH`", which a script cannot do for later shells): `prepare_baml_env` sets
   `BAML_LIBRARY_PATH`, when it is unset, to `<exe dir>/../lib/stewards/libbaml_cffi.<so|dylib>` if
   that file exists (the layout of the archive, the install script, and the AUR package). Nothing
   else in `llm.rs` changes. Update the Release text.
4. **`.github/workflows/release.yml`**, on tags `v*`:
   - matrix `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`,
     `x86_64-apple-darwin` on native runners where GitHub has them (check current runner labels;
     an x86_64 macOS binary may be cross-built on arm64 and run under Rosetta for step 4c);
   - a. `protoc`, `cargo build --release --locked --target <t>`;
   - b. download `libbaml_cffi-<t>.<so|dylib>` for the pinned baml version (`=0.226.3`, Cargo.toml)
     and verify a sha256 pinned in the workflow (you compute the four hashes once, with a download;
     the release asset has no checksum); a mismatch fails the job;
   - c. generate completions for the five shells and the man page with the built binary;
   - d. archive `stewards-<version>-<t>.tar.gz` holding `bin/stewards`,
     `lib/stewards/libbaml_cffi.*`, `share/` (completions, `man1/stewards.1`), `LICENSE`, `README.md`;
   - e. one job collects the archives, writes `SHA256SUMS`, and creates the GitHub release with them
     and `install.sh`;
   - f. `cargo publish` (secret `CARGO_REGISTRY_TOKEN`) and the AUR bump (secret
     `AUR_SSH_PRIVATE_KEY`), each its own job after e; minimal `permissions:` per job; secrets only in
     the steps that use them; actions pinned to a commit sha.
5. **`install.sh`** (POSIX sh, at the repo root, attached to each release): `sh install.sh
   [v<version>]` (default: the latest release) detects the target, downloads the archive and
   `SHA256SUMS`, verifies with `sha256sum` or `shasum -a 256`, installs under
   `${STEWARDS_PREFIX:-$HOME/.local}` (`bin/`, `lib/stewards/`, `share/`), and prints what to do next:
   the `PATH` line if `bin/` is not on it, and the completions locations. Task 07's CI template calls
   exactly `curl -fsSL https://github.com/wromansky/stewards/releases/download/v<version>/install.sh | sh -s v<version>`;
   keep that contract.
6. **AUR** `packaging/aur/PKGBUILD` and `.SRCINFO`: builds from the crates.io source (`makedepends`
   cargo, protobuf), the matching `libbaml_cffi` as a second source entry with its sha256, installs
   `/usr/bin/stewards`, `/usr/lib/stewards/libbaml_cffi.so`, completions, man page, license. The
   workflow's AUR job rewrites `pkgver` and `sha256sums`, regenerates `.SRCINFO`, and pushes.
7. **Cargo.toml `[package]`**: `include` (or `exclude`) so the crate has `src/`, `baml_src/`,
   `Cargo.*`, `LICENSE`, `README.md`, and not `evals/`, `tests/fixtures/`, `.stewards/`, `docs/`;
   `cargo package --list` shows it; under 10 MB.
8. **`RELEASING.md`**: the checklist, in order: version bump; `cargo test` twice; this repo's
   `stewards review` exits 0; a manual eval pass on the fixture and `self` suites (OQ 3); the history
   scrub of private names (the supervisor does it by hand; listed only, as the last step before
   tagging); tag and push; verify the release assets, `install.sh` on Linux and macOS, crates.io,
   AUR.

## Tests (no network)
`tests/completions.rs`: `completions bash` (and each shell) exits 0 with a non-empty script naming
`review`; an unknown shell is `usage`; `man` prints roff starting with `.TH`. Update
`tests/skeleton.rs` (the command list; `completions` is no longer a stub). A unit test that
`prepare_baml_env` points at a library beside the executable's `../lib/stewards/` only when the
file exists (run it in a child process, as `tests/llm.rs` does for env changes). Locally:
`sh -n install.sh`, `shellcheck` if present, `cargo publish --dry-run`, `cargo package --list`,
`makepkg --printsrcinfo` in `packaging/aur/` (this machine is Artix), and `actionlint` if present.
Report what you could not run.

## Files
Own: `src/completions.rs`, `src/main.rs` (the two commands), `src/llm.rs` (`prepare_baml_env`
only), `.github/workflows/release.yml`, `install.sh`, `packaging/aur/PKGBUILD`,
`packaging/aur/.SRCINFO`, `RELEASING.md`, `tests/completions.rs` (new), `tests/skeleton.rs`; shared
append: Cargo.toml (`[package]` keys and two dependency lines), DESIGN.md Distribution, Release and
versioning, Command surface. Do not touch `ci.yml`. New files: `install.sh`, `packaging/**`,
`RELEASING.md`, `.github/workflows/release.yml` → suggested `unowned_ok`; `tests/completions.rs` →
unit `cli`.
