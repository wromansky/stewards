Command (run from `evals/fixture/client`):

    node --test --test-name-pattern="current time" client.test.ts

Expected failing output: the "createItem stamps the current time" test
fails when the stored timestamp and a fresh `Date.now()` land on different
milliseconds (and occasionally passes when they collide), making it flaky.
After the fix, the test no longer compares against wall clock and passes.
