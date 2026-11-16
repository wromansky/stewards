# Task 07: PR/MR comments and the CI template

Repo: stewards (Rust), branch off `main` after tasks 03 and 04 are merged. Follow
`docs/build/common.md`. Model: Sonnet 5.5, thinking high. Security
review on the patch (text from cards goes to a public page; tokens) before merge.

Read DESIGN.md: Verification and operations › Git and PR (line ~434), Command surface (`next`,
`review --pr`, `init --ci`), Trust and safety › Writes and › Secrets and external text,
Agent-facing contracts (`tracker_auth`), Release and versioning. Then `src/next.rs` `run`,
`src/review.rs` `run`, `src/init.rs` `run`, `src/git.rs` `origin_url`, the fence helpers in
`src/brief.rs`/`src/pack.rs`, `install.sh` (task 03), `tests/next.rs`
(`pr_flag_is_not_implemented`), `tests/init.rs` (`ci_is_not_implemented…`).

## Why
"`next --pr <n>` posts the jobs as a PR or MR comment updated in place; … CI templates for GitHub
Actions and GitLab CI. The remote host picks `gh` or `glab`." All three are stubs. None is
per-harness: they talk to forges through their CLIs.

## Build
1. **`src/forge.rs`**: the forge from `git::origin_url` (DECIDED BY PLANNER: host `github.com` or
   containing `github` → GitHub/`gh`; containing `gitlab` → GitLab/`glab`; else `usage` naming the
   origin and the two supported hosts). A small trait for running the CLI (`argv`, optional stdin →
   status, stdout, stderr) with the real implementation (no shell, repo root as cwd, inherited env so
   `GH_TOKEN`/`GITLAB_TOKEN` work) and a recorded-response fake for tests. Not logged in (`gh auth
   status` / `glab auth status` fails) or the binary missing → `tracker_auth`, fix text from the
   error table. Task 09 reuses this seam for issue export; keep it to what both need.
2. **`next --pr <n>`**: the normal report (all other flags apply), also posted as one comment on
   PR/MR `n`: a first line `<!-- stewards:next -->`, a title, then the `--human` rendering inside a
   code fence longer than any backtick run in it (reuse the existing fence helper), so card and file
   text cannot ping people, link, or format. Updated in place: DECIDED BY PLANNER (updating needs
   its own earlier comment): list the PR's comments, take the first whose author is the logged-in
   user and whose body starts with the marker, and edit it; else create one. GitHub through `gh api`
   (`repos/{owner}/{repo}/issues/<n>/comments`, `--paginate`; edit with `PATCH
   …/issues/comments/<id>`; body on stdin); GitLab through `glab api` (MR notes). No other comment
   field is read, kept, or printed; the PR title and body are never read. The report gains
   `pr_comment {url, updated}`. Exit code as `next`.
3. **`review --pr <n>`**, OQ 1 option (b) until answered: `review` exactly as today, then the same
   posting with the marker `<!-- stewards:review -->`: what this call reported (each finding and note
   as `file:line: cut → replace_with`, per unit verdicts), the jobs listed, and the `then` line. If
   the answer is (a), remove the flag, its skeleton test argument, and the DESIGN.md row instead.
4. **`init --ci`**: writes the template for the origin's forge, through `Repo::write`:
   `.github/workflows/stewards.yml` (GitHub Actions) or `.gitlab/stewards.yml` (GitLab CI; DECIDED BY
   PLANNER: never touches `.gitlab-ci.yml`, prints the `include:` lines to add). The job: checkout;
   install with `curl -fsSL https://github.com/wromansky/stewards/releases/download/v<version>/install.sh | sh -s v<version>`
   (`<version>` = this binary's `CARGO_PKG_VERSION`; task 03's contract); `stewards check`. On pull
   requests / merge requests, before the check, `stewards next --no-brief --no-run --pr <number>`
   allowed to fail (the check decides the job), with the token it needs (GitHub: `GH_TOKEN` from
   `GITHUB_TOKEN` and the least `permissions:` that let it comment; GitLab: a `GITLAB_TOKEN` CI
   variable the user creates, named in the `then` line). An existing file with the same bytes is
   left; a different one is `usage` (`remove it to rewrite`). Unless a unit owns the path, it is
   added to `repo.toml` `unowned_ok` (`toml_edit`, layout kept), else `review` would ask for it.
   Output: `{written, unowned_ok_added, then}`; `then`: `Commit <path> and .stewards/repo.toml. CI
   runs stewards check on every push and fails while a unit changed since its review: run stewards
   review until it exits 0 before you push.` plus the GitLab include and token lines.
   Note in DESIGN.md: CI has no stamped trees (they are local unreachable objects), so `next` there
   lists due units with every owned file as changed and no hunks; `check` needs none.
5. **DESIGN.md**: Git and PR, Command surface rows, Trust and safety › Secrets and external text (the
   marker-and-author rule replaces "never … comments"), Writes (the `unowned_ok` edit).

## Tests (no network; the fake records `gh`/`glab` responses, plus one CLI test with a fake `gh`
script first on `PATH`)
Forge from https and ssh origins of github.com, gitlab.com, a self-hosted GitLab, and an unknown
host; `next --pr` creates a comment when none is its own and edits its own when present (a marker
comment by another user is not edited); the posted body keeps card text with backticks, `@user`,
and `[x](y)` inside the fence; not logged in → `tracker_auth`; `review --pr` posts the reported
findings; `init --ci` per forge writes the right file, adds `unowned_ok`, is a no-op the second
time, refuses a different existing file, and the template holds `stewards check` and the install
line with the crate version. Replace the two `not_implemented` tests. If the supervisor approves a
scratch GitHub repo, one real `next --pr` run there; otherwise say it was not run.

## Files
Own: `src/forge.rs` (new), `src/next.rs` (`run` and the posting call only), `src/review.rs` (`run`
only), `src/init.rs` (`--ci` only), `tests/forge.rs` (new), `tests/next.rs`, `tests/review.rs`,
`tests/init.rs`, `tests/skeleton.rs` (only if OQ 1 is (a)); shared append: `src/lib.rs`, DESIGN.md
sections above. Do not touch `validate.rs` or tracker code (task 09). New files: `src/forge.rs`,
`tests/forge.rs` → suggested unit `cli` (or a new `forge` unit shared with task 09).
