Command: `bash evals/repros/flaky-client-clock-test/repro.sh`

Expected behavior: the "createItem stamps the current time" test compares the
item's `createdAt` and `Date.now()` after truncating both to whole seconds.
It passes most of the time; it fails only when the second boundary falls
between the api's timestamp and the client's timestamp: a window of a few
tens of milliseconds in each second, so a plain run fails only now and then.
The repro preloads `tick.mjs`, which moves the test's clock one second on,
so the failing case shows on every run. After the fix, the test no longer compares against wall
clock and always passes.
