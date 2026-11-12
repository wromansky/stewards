Command (run from `evals/fixture/client`):

    node client.ts create-item ok

Expected failing output (stderr): the successful call still prints

    retrying api call (attempt 1 of 2)

even though nothing failed and no retry happened. After the fix, a
successful call prints no retry message.
