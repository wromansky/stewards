# stewards

A team of agents that already knows your repo, for any coding-agent harness.

`stewards` is a Rust CLI that agents call through their shell tool. It keeps
per-unit knowledge (structure, contracts, judgment) in `.stewards/`, computes
the exact context each role needs, and delegates work to a maintainer, tester,
documenter, validator, or architect. The same ritual runs in every harness
that reads `AGENTS.md`.

Status: design complete, build in progress. See [DESIGN.md](DESIGN.md).

## Harnesses that do not read AGENTS.md

stewards writes its block to `AGENTS.md` only, and does not know which
harness you use. A harness that does not read `AGENTS.md` needs one line in
its own instruction file pointing at it: a line telling the agent to read
`AGENTS.md`, or that harness's import syntax. List that file in
`.stewards/repo.toml` as `instruction_files = ["<path>", ...]`, and
`stewards check` warns while a listed file is missing or does not mention
`AGENTS.md`.
