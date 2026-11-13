// Stands in for the second ticking over between the api's timestamp and the test's own reading.
const now = Date.now;
Date.now = () => now() + 1000;
