enum Verdict {
  Approve @alias("approve")
  Changes @alias("changes")
}

enum Rung {
  NotNeeded @alias("not_needed")
  AlreadyInRepo @alias("already_in_repo")
  Stdlib @alias("stdlib")
  Platform @alias("platform")
  InstalledDependency @alias("installed_dependency")
  OneLine @alias("one_line")
  Untraced @alias("untraced")
  NoCheck @alias("no_check")
}

enum GapKind {
  Gap @alias("gap")
  Future @alias("future")
  Bug @alias("bug")
}

enum Severity {
  Critical @alias("critical")
  Medium @alias("medium")
  Low @alias("low")
}

class Finding {
  file string @description("max 300 chars")
  line int
  rung Rung
  cut string @description("max 2000 chars")
  replace_with string @description("max 2000 chars")
}

class Interface {
  id string @description("max 120 chars")
  anchor string @description("path::Name, the bare identifier with no keyword; max 300 chars")
  contract string @description("max 300 chars")
}

class Anchor {
  path string @description("max 300 chars")
  name string @description("the bare identifier with no keyword; max 300 chars")
}

class Gap {
  kind GapKind
  title string @description("max 120 chars")
  anchor string? @description("path::Name, the bare identifier with no keyword; max 300 chars")
  note string @description("max 2000 chars")
  repro string? @description("max 300 chars")
}

// Lists upsert by key (interface id, anchor path and name, gap title); removal is only through the
// remove lists. Absent or null leaves that part unchanged.
class CardPatch {
  interface Interface[]? @description("entries to add or update, by id; the rest of the card is kept; max 200 items")
  anchors Anchor[]? @description("entries to add, by path and name; the rest of the card is kept; max 200 items")
  gaps Gap[]? @description("entries to add or update, by title; the rest of the card is kept; max 200 items")
  remove_interface string[]? @description("ids of interface entries to remove; max 200 items, each max 120 chars")
  remove_anchors string[]? @description("path::name of anchors to remove; max 200 items, each max 300 chars")
  remove_gaps string[]? @description("titles of gaps to remove; max 200 items, each max 120 chars")
}

class MaintainerReviewAnswer {
  verdict Verdict
  findings Finding[] @description("max 200 items; all cut and replace_with together max 20000 chars")
  card_patch CardPatch?
  tester_needed bool
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}

class Task {
  title string @description("max 120 chars")
  change string @description("max 20000 chars")
  files string[] @description("max 200 items, each max 300 chars")
  check string? @description("max 300 chars")
}

class MaintainerExplainAnswer {
  explanation string @description("max 4000 chars")
  tasks Task[] @description("max 200 items")
  card_patch CardPatch?
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}

class CheckRun {
  command string @description("max 300 chars")
  pass bool
  output string @description("max 2000 chars")
}

class DeveloperAnswer {
  check CheckRun?
  summary string @description("max 2000 chars")
  blocked_on string? @description("max 2000 chars")
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}

class Scenario {
  id string @description("max 120 chars")
  interface string @description("max 120 chars")
  given string @description("max 300 chars")
  expect string @description("max 300 chars")
  level string @description("max 120 chars")
  command string? @description("max 300 chars")
}

class ScenarioResult {
  scenario string @description("max 120 chars")
  pass bool
  output string @description("max 2000 chars")
}

class Bug {
  title string @description("max 120 chars")
  repro string @description("max 300 chars")
  evidence string @description("max 2000 chars")
  interface string? @description("max 120 chars")
}

class TesterAnswer {
  scenario_patch Scenario[] @description("max 200 items")
  results ScenarioResult[] @description("max 200 items")
  bugs Bug[] @description("max 200 items")
  tests_written string[] @description("max 200 items, each max 300 chars")
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}

class Doc {
  overview string @description("max 2000 chars")
  how_it_works string @description("max 2000 chars")
  how_to_change string @description("max 2000 chars")
}

class DocumenterAnswer {
  doc Doc
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}

class ValidatorAnswer {
  real bool
  severity Severity
  reason string @description("max 2000 chars")
  evidence string[] @description("max 200 items, each max 2000 chars")
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}

class PlanStep {
  unit string @description("max 120 chars")
  change string @description("max 2000 chars")
  interfaces string[] @description("max 200 items, each max 120 chars")
  check string @description("max 300 chars")
}

class Plan {
  title string @description("max 120 chars")
  steps PlanStep[] @description("max 200 items")
}

class Flow {
  name string @description("max 120 chars")
  units string[] @description("max 200 items, each max 120 chars")
  entry string @description("path::Name, the bare identifier with no keyword; max 300 chars")
  check string? @description("max 300 chars")
}

class Decision {
  title string @description("max 120 chars")
  why string @description("max 2000 chars")
}

class UnitProposal {
  name string @description("max 120 chars")
  purpose string @description("max 300 chars")
  paths string[] @description("max 200 items, each max 300 chars")
}

class ArchitectAnswer {
  plan Plan?
  flows Flow[] @description("max 200 items")
  decisions Decision[] @description("max 200 items")
  unit_proposals UnitProposal[] @description("max 200 items")
  explanation string @description("max 4000 chars")
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}

class UnitReview {
  unit string @description("one of the units under review; max 120 chars")
  verdict Verdict
  findings Finding[] @description("max 200 items; all cut and replace_with together max 20000 chars")
  card_patch CardPatch?
  check CheckRun? @description("the test command as run, and the tail of its output; required when the pack shows a test command, null when it shows none; a failing check cannot approve")
  scenario_patch Scenario[]? @description("only when the contract changed; upserts by id; max 200 items")
  bugs Bug[]? @description("defects found and not fixed here; max 200 items")
  doc Doc? @description("all three sections as they should read; required when the unit has no docs; otherwise only when what a section says is no longer true")
}

class ReviewAnswer {
  snapshot string @description("the snapshot the brief gives, copied exactly")
  units UnitReview[] @description("every unit under review exactly once, no other; max 200 items")
  pulled_anchors string[] @description("max 200 items, each max 300 chars")
  notes string @description("max 2000 chars")
}
