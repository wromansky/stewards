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
  file string
  line int
  rung Rung
  cut string
  replace_with string
}

class Interface {
  id string
  anchor string @description("path::Name, the bare identifier with no keyword")
  contract string
}

class Anchor {
  path string
  name string @description("the bare identifier with no keyword")
}

class Gap {
  kind GapKind
  title string
  anchor string? @description("path::Name, the bare identifier with no keyword")
  note string
  repro string?
}

// Lists upsert by key (interface id, anchor path and name, gap title); removal is only through the
// remove lists. Absent or null leaves that part unchanged.
class CardPatch {
  interface Interface[]? @description("entries to add or update, by id; the rest of the card is kept")
  anchors Anchor[]? @description("entries to add, by path and name; the rest of the card is kept")
  gaps Gap[]? @description("entries to add or update, by title; the rest of the card is kept")
  remove_interface string[]? @description("ids of interface entries to remove")
  remove_anchors string[]? @description("path::name of anchors to remove")
  remove_gaps string[]? @description("titles of gaps to remove")
}

class MaintainerReviewAnswer {
  verdict Verdict
  findings Finding[]
  card_patch CardPatch?
  tester_needed bool
  pulled_anchors string[]
  notes string
}

class Task {
  title string
  change string
  files string[]
  check string?
}

class MaintainerExplainAnswer {
  explanation string
  tasks Task[]
  card_patch CardPatch?
  pulled_anchors string[]
  notes string
}

class CheckRun {
  command string
  pass bool
  output string
}

class DeveloperAnswer {
  check CheckRun?
  summary string
  blocked_on string?
  pulled_anchors string[]
  notes string
}

class Scenario {
  id string
  interface string
  given string
  expect string
  level string
  command string?
}

class ScenarioResult {
  scenario string
  pass bool
  output string
}

class Bug {
  title string
  repro string
  evidence string
  interface string?
}

class TesterAnswer {
  scenario_patch Scenario[]
  results ScenarioResult[]
  bugs Bug[]
  tests_written string[]
  pulled_anchors string[]
  notes string
}

class Doc {
  overview string
  how_it_works string
  how_to_change string
}

class DocumenterAnswer {
  doc Doc
  pulled_anchors string[]
  notes string
}

class ValidatorAnswer {
  real bool
  severity Severity
  reason string
  evidence string[]
  pulled_anchors string[]
  notes string
}

class PlanStep {
  unit string
  change string
  interfaces string[]
  check string
}

class Plan {
  title string
  steps PlanStep[]
}

class Flow {
  name string
  units string[]
  entry string @description("path::Name, the bare identifier with no keyword")
  check string?
}

class Decision {
  title string
  why string
}

class UnitProposal {
  name string
  purpose string
  paths string[]
}

class ArchitectAnswer {
  plan Plan?
  flows Flow[]
  decisions Decision[]
  unit_proposals UnitProposal[]
  explanation string
  pulled_anchors string[]
  notes string
}
