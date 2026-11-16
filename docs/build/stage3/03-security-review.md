## Review

Read-only review. I did not run anything. I could not verify the action SHAs, the library hashes, or the AUR host key against the network. The host key matches the fingerprint in the comment as I remember it for aur.archlinux.org.

### Correct
- **Triggers:** only `push: tags: v*` (release.yml:2-4). There is no `pull_request` or `pull_request_target`, so a fork PR cannot reach a secret.
- **Permissions:** `permissions: {}` at the top (:6) and an explicit block per job. Only `release` has `contents: write`. Every checkout uses `persist-credentials: false`.
- **Script injection:** `github.ref_name` and the matrix values reach `run:` only through `env:`. The tag is checked against `v<d>.<d>.<d>` and `[0-9A-Za-z.+-]` (:45-46) before use. Nothing is interpolated inline.
- **Library hashes:** the expected sha256 values are in the workflow in the repo. A compromised download can only fail the job, never update a hash. `shasum -c -` is the last command in its pipeline, so a mismatch fails under `set -eu` (:73-75).
- **Order and AUR hash:** release comes first and the crate second. A release can be deleted and a crate cannot, so the order is right. The AUR hash is computed from the crate actually fetched from crates.io.
- **AUR key handling:** the key is written with `umask 077` and removed by a trap. The host key is pinned with `StrictHostKeyChecking=yes` and a private `UserKnownHostsFile`. The push is not forced. The AUR job runs no repo-supplied code once the key is on disk.
- **`install.sh` validation:** the tag is whitelisted before any URL or path is built (:36-44). There is no `eval`, and downloaded text only reaches `awk` and a string comparison. The checksum is verified before `tar`, and nothing is installed on a mismatch.
- **`bump.sh`:** `version` is `[0-9.]` only and `sha` is 64 lowercase hex digits. Neither contains `/` or `&`, so there is no sed injection.
- **`prepare_baml_env`:** an existing `BAML_LIBRARY_PATH` is always respected, including an empty one (llm.rs:307-310).
- **`Cargo.toml` include:** `/src/`, `/baml_src/`, `Cargo.toml`, `Cargo.lock`, `LICENSE` and `README.md` cover what the build needs. `include_str!` only reads `src/llm_schema`, and there is no `build.rs`. I found no token-like strings in `src/`. Fixtures, evals, `.stewards/` and `docs/` are not shipped.

### Findings, by severity

**P1 — `CARGO_REGISTRY_TOKEN` is visible to dependency build scripts (release.yml:145-148).**
- `cargo publish --locked` runs the verification build with the token in its environment. Every dependency's `build.rs` and proc-macro, including baml-sys, inherits it. A compromised crate in the dependency tree could read it.
- Smallest fix: add a step `cargo package --locked` with no secret, then run `cargo publish --locked --no-verify` with the token.
- Also use a crates.io token scoped to this crate (`publish-update`, plus `publish-new` for the first release).

**P1 — Secrets are not tied to a protected context (release.yml:134-148, :152-195).**
- Any user who can push a tag `v*` on any commit or branch runs that commit's workflow. That workflow can print `secrets.CARGO_REGISTRY_TOKEN` and `secrets.AUR_SSH_PRIVATE_KEY`.
- Nothing checks that the tagged commit is on `main`.
- Smallest fix:
  - Put both secrets in a GitHub environment (`environment: release`) on the `publish` and `aur` jobs, with required reviewers.
  - Add a tag ruleset on `v*` that restricts who can create tags.
  - Add one check in `release` that the tag commit is an ancestor of `origin/main`.
- RELEASING.md step 7 should say this, because it currently says only that the secrets "must exist in the repository settings".

**P1 — Bundled-library lookup trusts the exe's grandparent directory (llm.rs:285-293).**
- The code takes `<exe dir>/../lib/stewards/libbaml_cffi.*` and loads it with `dlopen` as soon as the file exists.
- It does not check that the exe lives in a `bin/` directory.
- Run `/tmp/x/stewards`, or `~/Downloads/stewards`, and it loads `/tmp/lib/stewards/…` or `~/lib/stewards/…`. Another local user can plant `/tmp/lib/stewards/libbaml_cffi.so` and get code execution as the victim on `init --draft --direct`.
- On Linux `current_exe` is a resolved path, so a symlinked exe is fine. On macOS it may be the unresolved path (not verified), which moves the lookup to the symlink's directory.
- Smallest fix: return `None` unless `exe.parent().file_name() == Some("bin")`, and canonicalize the exe first.
- A stricter option is to require the library not to be group- or world-writable.

**P2 — Cargo/AUR token blast radius.** Use a dedicated AUR account or key limited to this package, not the maintainer's main key.

**P2 — `install.sh` redirects (install.sh:30, 58, 59).** `curl -L` accepts redirects to http. Add `--proto '=https' --tlsv1.2`. The checksum comes from the same origin, so this only keeps an attacker from controlling both files. The writer already admits the checksum cannot protect against a tampered release.

**P2 — `install.sh` runs top-to-bottom, so `curl | sh` can run a truncated script (install.sh:1-91).** Wrap the body in `main() { …; }` and call `main "$@"` on the last line.

**P2 — `install.sh` copies are not atomic and the temp name is predictable (install.sh:70-72).**
- `stewards.new` has a fixed name. With a shared or world-writable `STEWARDS_PREFIX`, a pre-planted symlink makes `cp` write through it.
- `cp` rewrites `libbaml_cffi` in place. A running stewards that has it mapped can crash.
- Smallest fix: `rm -f "$prefix/bin/stewards.new"` before the copy, and use the same `.new` then `mv` pattern for the library.

**P2 — `install.sh` prefix and root handling (install.sh:7).**
- A relative `STEWARDS_PREFIX` ends up in the printed PATH advice, which puts a relative directory on PATH.
- Under `sudo`, `$HOME` may still be the invoking user's, so root creates `~/.local` directories there.
- Smallest fix: `case $prefix in /*) ;; *) fail … ;; esac`, and refuse `id -u` = 0 unless `STEWARDS_PREFIX` is set.

**P2 — The trap does not exit (install.sh:55).** After a HUP, INT or TERM trap, `sh` resumes the script. Use `trap 'rm -rf "$tmp"' EXIT` plus `trap 'exit 1' HUP INT TERM`.

**P2 — PKGBUILD can install empty completions and man page without failing (PKGBUILD:35-38).**
- `stewards … 2>/dev/null | install -Dm644 /dev/stdin …` has no pipefail, so only `install`'s exit status counts.
- Smallest fix: write each file to `$srcdir` first, then `install` it.
- The committed copy also has `sha256sums=('SKIP'` for the crate. The workflow replaces it for the AUR, but anyone using the in-repo PKGBUILD gets no check.

**P2 — Workflow checks are narrow.**
- **`_bamlver` drift:** nothing checks the PKGBUILD `_bamlver` and the library sha against `BAML_VERSION`.
- **`baml-sys` pin:** the `baml = "=…"` check (:49) does not cover `baml-sys`.
- **Archive names:** the check at :125 counts four archives but does not check their names.
- **AUR pull:** `git pull … || true` (:190) hides auth and network errors. The non-forced push still fails safely.
- **Build tools:** `stable` Rust and `brew install protobuf` are unpinned.
- **`+` build metadata:** a tag like `v1.2.3+meta` is not treated as a prerelease, passes the tag check, and then fails in `bump.sh`. This happens after the crate is already published.
- **Provenance:** adding `actions/attest-build-provenance` (SHA-pinned) would give users something to verify beyond `SHA256SUMS`.

**P2 — Licensing: Apache-2.0 notices are missing.**
- **Where the notices are missing:** the archives ship only `LICENSE` (the repo's MIT text) and `README.md` (release.yml assemble step, `cp LICENSE README.md`). The AUR package installs only `/usr/share/licenses/stewards/LICENSE`, and `license=('MIT')` omits Apache-2.0.
- **What §4 requires for a redistributed Apache-2.0 work:**
  - (a) A copy of the Apache-2.0 licence text. It is missing from the archives, the AUR package and the crate.
  - (b) For modified files, a notice that you changed them. This is not needed because the library is unmodified.
  - (c) Retention of the copyright, patent, trademark and attribution notices from the source. I could not see BAML's repo to confirm what they are.
  - (d) If BAML has a `NOTICE` file, a readable copy of its attribution text, distributed as a file, in the NOTICE text, or in documentation. I could not check whether upstream has one.
- **What is shipped from BAML:** the release `.so` is shipped as a bare binary with no notice files. The `baml` 0.226.3 crate in the local registry also has no LICENSE or NOTICE, and its Cargo.toml says `Apache-2.0`.
- **Statically linked crates:** the `stewards` binary and `libbaml_cffi` statically link many other crates (MIT, Apache and others) whose notices should also be shipped.
- **Smallest fix:** vendor BAML's `LICENSE` and `NOTICE` (if any) from the 0.226.3 tag into the repo.
  - Ship them as `licenses/baml/` in the archive.
  - Install them to `/usr/share/licenses/stewards/` in the PKGBUILD and set `license=('MIT' 'Apache-2.0')`.
  - Add a `THIRD-PARTY.md` generated with `cargo about` or `cargo-bundle-licenses`.

### Merge verdict: CHANGES
Two things block it: the publish token reaching dependency build scripts, and secrets that any pushed tag can use. The unvalidated library path should be fixed in the same pass. The rest are P2 and can follow.

Not checked: the pinned SHAs and library hashes against real sources, the workflow itself, and BAML's upstream NOTICE.