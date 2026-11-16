const wt = args.wt;
const stepModel = args.stepModel || "anthropic/claude-haiku-5-5:high";
const reviewModel = args.reviewModel || "deepseek/deepseek-flash";
const log = [];
if (args.pre) {
  const p = await runs.all([{ key: "pre", label: "Pre-step writer", agent: "worker", model: args.preModel || "anthropic/claude-sonnet-5-5:high", cwd: args.wt, task: args.pre }]);
  if (!p[0] || !p[0].ok) { return { ended: "pre-step failed", log: [((p[0] && p[0].output) || "").slice(-600)] }; }
}

let lastKey = "";
let repeats = 0;
for (let round = 1; round <= 20; round++) {
  const step = await runs.all([{ key: "step-" + round, label: "Loop step " + round, agent: "worker", model: stepModel, cwd: wt, task: "Read docs/build/loop-step.md and do the task in this worktree (" + wt + ")." }]);
  const out = ((step[0] && step[0].output) || "").trim();
  const lines = out.split("\n").map((l) => l.trim().replace(/^[`*\-\s]+|[`*\s]+$/g, "")).filter((l) => l.length > 0);
  const tail = lines.slice(-30);
  const handoffs = tail.filter((l) => l.startsWith("HANDOFF:")).map((l) => l.slice("HANDOFF:".length).trim());
  const last = tail.length ? tail[tail.length - 1] : "";
  const stop = tail.filter((l) => l.startsWith("STOP:"));
  log.push({ round: round, step: out.slice(-600), handoffs: handoffs.length });
  if (!step[0] || !step[0].ok) { return { ended: "step failed", round: round, log: log }; }
  if (stop.length) { return { ended: stop[0], round: round, log: log }; }
  if (handoffs.length === 0) { if (last === "DONE") { return { ended: "done", round: round, log: log }; } return { ended: "step gave no verdict", round: round, log: log }; }
  const key = handoffs.join("|");
  if (key === lastKey) { repeats++; } else { repeats = 0; lastKey = key; }
  if (repeats >= 2) { return { ended: "the same handoffs were listed three times", round: round, log: log }; }
  for (let i = 0; i < handoffs.length; i += 3) {
    const batch = handoffs.slice(i, i + 3);
    const rev = await runs.all(batch.map((h, j) => ({ key: "review-" + round + "-" + (i + j + 1), label: "Reviewer " + round + "." + (i + j + 1), agent: "worker", model: reviewModel, cwd: wt, task: h })));
    for (const r of rev) { log.push({ round: round, reviewer: ((r.output || "").replace(/\s+/g, " ")).slice(-360), ok: !!r.ok }); }
  }
}
return { ended: "20 rounds without finishing", log: log };
