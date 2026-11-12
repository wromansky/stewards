# evals

Fixture and tasks for measuring coding agents against `stewards`.

## Directories
- `fixture/api/` — Python in-memory items store (create, list, get) with a CLI.
  Tests: `cd fixture/api && python3 -m unittest`
- `fixture/client/` — TypeScript client for the api.
  Tests: `cd fixture/client && node --test client.test.ts`
- `fixture/cli/` — Rust CLI that shells out to the api.
  Build/lint: `cd fixture/cli && cargo build && cargo clippy -- -D warnings`
- `repros/` — one reference reproduction and patch per seeded bug.
- `tasks/` — the six task prompts.
- `arms.sh` — prepares bare, block-only, and full eval arms.

## Arms
    arms.sh bare DIR        fixture only
    arms.sh block-only DIR  fixture plus the AGENTS.md block
    arms.sh full DIR        fixture plus `stewards init` (needs stewards on PATH)

## Flow
create-item: `cli create <name>` or `client create-item <name>` calls
`api create <name>`, which appends the item to the in-memory store.
