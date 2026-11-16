# Task 09: `validate --export` to GitHub, GitLab, Jira, Linear

Repo: stewards (Rust), branch off `main` after tasks 06 and 07 are merged. Follow
`docs/build/common.md`. Model: Opus 5.5, thinking medium. Security
review on the patch before merge.

Read DESIGN.md: Verification and operations › Validation (`--export`), Schemas › `[tracker]` and the
gap's `issue`, Trust and safety › Secrets and external text ("Tracker export sends only fields the
tool composed. No secrets in any stewards file; tracker auth from CLIs and env."), Agent-facing
contracts (`tracker_auth`), Docs templates (gaps are rendered). Then `src/validate.rs` (task 06),
`src/forge.rs` (task 07: the `gh`/`glab` seam and `tracker_auth`), `src/model.rs` (`Tracker`,
`Gap`), the card write and lost-update helpers in `src/submit.rs`, the fence helper used by task 07.

## Why
"`--export` creates an issue on GitHub, GitLab, Jira, or Linear (`repo.toml` `[tracker]`; auth from
`gh`, `glab`, `JIRA_URL` and `JIRA_TOKEN`, `LINEAR_API_KEY`; Jira and Linear through their REST and
GraphQL APIs), once per bug: the issue URL is recorded on the gap."

## Build
1. **`stewards validate --export`** on the current repo: every bug gap with a `severity` and no
   `issue`, in units.toml then card order. For each: create the issue, then at once write its URL to
   the gap (`Repo::write`, lost-update check), so a later failure never loses a filed issue. Stop at
   the first failure. A crash between creating and writing can still file one issue twice: record
   that limit in DESIGN.md.
2. **Composed text only.** Title: the gap title (one line, capped as the card caps it). Body:
   `Unit <unit>, severity <severity>, anchor <anchor>`, the repro and the note each in a code fence
   longer than any backtick run in it, and `Filed by stewards from .stewards/units/<unit>.toml.`
   DECIDED BY PLANNER: Jira gets the same in plain text with `{noformat}` blocks (any `{noformat}`
   in the text removed). Nothing else from the repo is sent.
3. **Backends** behind one small trait in `src/tracker.rs` (`create_issue(title, body) -> url`):
   - GitHub: `gh issue create -R <project> --title=<title> --body-file -` (body on stdin) through
     task 07's CLI seam; the URL is the last stdout line.
   - GitLab: `glab issue create -R <project>` with title and description (check `glab`'s docs for
     how to pass the description without a shell; stdin if it reads it), same seam.
   - Jira: `POST <JIRA_URL>/rest/api/2/issue`, `{"fields": {"project": {"key"}, "summary",
     "description", "issuetype": {"name": "Bug"}}}`; the URL is `<JIRA_URL>/browse/<key>`. Auth per
     OQ 5 until answered: `JIRA_EMAIL` set → Basic `email:token` (`base64` is already in the tree via
     `ureq`), else Bearer `JIRA_TOKEN`. DECIDED BY PLANNER: `JIRA_URL` must be `https://` with no user
     info, else `tracker_auth`.
   - Linear: `POST https://api.linear.app/graphql` (`Authorization: <LINEAR_API_KEY>`): the team id
     by key (`teams(filter: {key: {eq: $key}})`), then `issueCreate(input: {teamId, title,
     description})` returning `issue { url }`; values only as GraphQL variables.
   HTTP through `ureq` (already compiled for `baml-sys`; add it as a direct dependency at the locked
   version and features, `tls`, so nothing new builds), behind a transport trait (`post(url,
   headers, json) -> (status, body)`) with a recorded-response fake; 30 s timeout.
4. **Errors.** No `[tracker]` → `usage` (`add [tracker] kind and project to .stewards/repo.toml`).
   Missing env, `gh`/`glab` not logged in, HTTP 401/403 → `tracker_auth`. Any other failure → new
   code `tracker_failed` (DECIDED BY PLANNER: `internal` says "report a bug", wrong for a tracker
   outage): message = kind, HTTP status, the tracker's error text on one line, at most 300
   characters, and how many issues this call already filed and recorded; fix = `check [tracker]
   project and the tracker, then run stewards validate --export again; filed issues are recorded on
   the cards`. Never a token, header, or request body in any message. Add the row to the error table
   and `guide.rs`.
5. **A returned URL** is checked before it is stored: `https://`, one line, capped; else
   `tracker_failed`.
6. **Output**: `{exported[] {unit, title, url}, then}`; `then` = `done` when nothing is left, or, when
   unjudged bugs exist, `Bugs without a severity are not exported: run stewards review --roles to
   judge them, then stewards validate --export again.` Exit 0.
7. **DESIGN.md**: Validation, Trust and safety (what is sent, the double-file limit), error table.

## Tests (no network: the CLI seam and the transport are fakes with recorded responses)
Per kind: the exact request (URL, method, header names, body; the token present but never in any
output) and the URL written to the right gap; a second run exports nothing; a card with two bugs
where the second fails: the first URL is on the card, the error is `tracker_failed`, exit 2; 401 →
`tracker_auth`; 500 → `tracker_failed` whose message holds the status and not the token; Linear team
not found → `tracker_failed` naming the key; `JIRA_URL=http://…` → `tracker_auth`; no `[tracker]` →
`usage`; a note holding backticks, `{noformat}`, and `@user` stays inside its block; a returned URL
that is `javascript:` or multi-line is refused. Real end to end: GitHub only, with the supervisor's
ok, on a scratch repo; GitLab, Jira, Linear cannot be run here (PLAN.md): say so.

## Files
Own: `src/tracker.rs` (new), `src/validate.rs` (`--export`), `tests/tracker.rs` (new); shared
append: `src/lib.rs`, Cargo.toml (one line), `src/guide.rs` and DESIGN.md error rows, DESIGN.md
sections above. Use `src/forge.rs` as task 07 left it; if it needs a change, keep it minimal and
say so. New files: `src/tracker.rs`, `tests/tracker.rs` → suggested unit `cli` (or the `forge` unit
if the supervisor made one).
