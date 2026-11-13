Command (run from `evals/fixture/cli`):

    cargo run --quiet -- create widget

Expected failing output (stdout): the created item has no `name` field

    {"id":"1","createdAt":...}

because the api now returns `label` but the client still reads `name`.
After the fix, the output includes `"name":"widget"`.
