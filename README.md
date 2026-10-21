# stewards

A team of agents that already knows your repo, for any coding-agent harness.

`stewards` is a Rust CLI that agents call through their shell tool. It keeps
per-unit knowledge (structure, contracts, judgment) in `.stewards/`, computes
the exact context each role needs, and delegates work to a maintainer, tester,
documenter, validator, or architect. The same ritual runs in every harness
that reads `AGENTS.md`.

Status: design complete, build in progress. See [DESIGN.md](DESIGN.md).
