Command (run from `evals/fixture/cli`):

    cargo run --quiet -- create

Expected failing output: a Rust panic:

    thread 'main' panicked at src/main.rs:<line>:<col>:
    called `Option::unwrap()` on a `None` value

After the fix, the same command prints a usage error and exits 2 instead of
panicking.
