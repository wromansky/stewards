// BAML calls, brief rendering, and answer parsing.
//
// baml_src/ is the one source of the role prompts and answer schemas. Normal commands read the
// rendered text in src/llm_schema/ and validate answers with serde through the generated types;
// only `direct` loads the BAML native library.
use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::Role;
use crate::output::CliError;

#[path = "baml_client/mod.rs"]
#[allow(clippy::all)]
mod baml_client;
use baml_client::sync_client::B;
pub use baml_client::types;

/// Only the maintainer has two modes; other roles ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Review,
    Explain,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Answer {
    MaintainerReview(types::MaintainerReviewAnswer),
    MaintainerExplain(types::MaintainerExplainAnswer),
    Developer(types::DeveloperAnswer),
    Tester(types::TesterAnswer),
    Documenter(types::DocumenterAnswer),
    Validator(types::ValidatorAnswer),
    Architect(types::ArchitectAnswer),
    Review(types::ReviewAnswer),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub problem: String,
    pub example: String,
}

/// The role's prompt text from baml_src, with `<unit>` left for the caller to fill.
pub fn role_text(role: Role, mode: Mode) -> &'static str {
    match (role, mode) {
        (Role::Maintainer, Mode::Review) => include_str!("llm_schema/maintainer-review.role.txt"),
        (Role::Maintainer, Mode::Explain) => include_str!("llm_schema/maintainer-explain.role.txt"),
        (Role::Developer, _) => include_str!("llm_schema/developer.role.txt"),
        (Role::Tester, _) => include_str!("llm_schema/tester.role.txt"),
        (Role::Documenter, _) => include_str!("llm_schema/documenter.role.txt"),
        (Role::Validator, _) => include_str!("llm_schema/validator.role.txt"),
        (Role::Architect, _) => include_str!("llm_schema/architect.role.txt"),
        (Role::Reviewer, _) => include_str!("llm_schema/reviewer.role.txt"),
    }
}

/// BAML's `ctx.output_format` for the role's answer shape.
pub fn output_format(role: Role, mode: Mode) -> &'static str {
    match (role, mode) {
        (Role::Maintainer, Mode::Review) => include_str!("llm_schema/maintainer-review.schema.txt"),
        (Role::Maintainer, Mode::Explain) => {
            include_str!("llm_schema/maintainer-explain.schema.txt")
        }
        (Role::Developer, _) => include_str!("llm_schema/developer.schema.txt"),
        (Role::Tester, _) => include_str!("llm_schema/tester.schema.txt"),
        (Role::Documenter, _) => include_str!("llm_schema/documenter.schema.txt"),
        (Role::Validator, _) => include_str!("llm_schema/validator.schema.txt"),
        (Role::Architect, _) => include_str!("llm_schema/architect.schema.txt"),
        (Role::Reviewer, _) => include_str!("llm_schema/reviewer.schema.txt"),
    }
}

/// Strictly validates `text` as the role's answer: exactly one JSON object, no unknown fields.
pub fn parse(role: Role, mode: Mode, text: &str) -> Result<Answer, Vec<FieldError>> {
    let example = example(role, mode);
    let answer = match (role, mode) {
        (Role::Maintainer, Mode::Review) => typed(text, &example).map(Answer::MaintainerReview),
        (Role::Maintainer, Mode::Explain) => typed(text, &example).map(Answer::MaintainerExplain),
        (Role::Developer, _) => typed(text, &example).map(Answer::Developer),
        (Role::Tester, _) => typed(text, &example).map(Answer::Tester),
        (Role::Documenter, _) => typed(text, &example).map(Answer::Documenter),
        (Role::Validator, _) => typed(text, &example).map(Answer::Validator),
        (Role::Architect, _) => typed(text, &example).map(Answer::Architect),
        (Role::Reviewer, _) => typed(text, &example).map(Answer::Review),
    }?;
    let value: Value = serde_json::from_str(text).expect("typed() accepted it as one JSON value");
    // The generated types do not deny unknown fields, so compare the input with what serde kept.
    let known = serde_json::to_value(&answer).expect("answer serializes");
    let mut unknown = vec![];
    unknown_fields(&value, &known, &mut vec![], &mut unknown);
    if unknown.is_empty() {
        Ok(answer)
    } else {
        Err(unknown
            .into_iter()
            .map(|path| FieldError {
                problem: format!("unknown field `{}`", display(&path)),
                example: example_at(&example, &path[..path.len() - 1]),
                field: display(&path),
            })
            .collect())
    }
}

#[derive(Clone)]
enum Step {
    Key(String),
    Index(usize),
}

/// Deserializes straight from the text, not through a `Value`, so a duplicate key is an error
/// rather than a silent last-wins.
fn typed<T: serde::de::DeserializeOwned>(
    text: &str,
    example: &Value,
) -> Result<T, Vec<FieldError>> {
    let mut de = serde_json::Deserializer::from_str(text);
    let answer = serde_path_to_error::deserialize(&mut de).map_err(|e| {
        let mut path: Vec<Step> = e
            .path()
            .iter()
            .filter_map(|s| match s {
                serde_path_to_error::Segment::Seq { index } => Some(Step::Index(*index)),
                serde_path_to_error::Segment::Map { key } => Some(Step::Key(key.clone())),
                _ => None,
            })
            .collect();
        let mut problem = e.into_inner().to_string();
        // serde reports a missing or duplicate field at its parent; name the field itself.
        let named = ["missing field `", "duplicate field `"]
            .iter()
            .find_map(|p| problem.strip_prefix(p)?.split('`').next());
        if let Some(name) = named {
            path.push(Step::Key(name.into()));
        } else if path.is_empty() {
            problem = format!("not a single JSON object: {problem}");
        }
        vec![FieldError {
            field: display(&path),
            example: example_at(example, &path),
            problem,
        }]
    })?;
    de.end().map_err(|e| {
        vec![FieldError {
            field: "(document)".into(),
            problem: format!("text after the JSON object: {e}"),
            example: example.to_string(),
        }]
    })?;
    Ok(answer)
}

fn unknown_fields(input: &Value, known: &Value, path: &mut Vec<Step>, out: &mut Vec<Vec<Step>>) {
    match (input, known) {
        (Value::Object(input), Value::Object(known)) => {
            for (key, v) in input {
                path.push(Step::Key(key.clone()));
                match known.get(key) {
                    Some(k) => unknown_fields(v, k, path, out),
                    None => out.push(path.clone()),
                }
                path.pop();
            }
        }
        (Value::Array(input), Value::Array(known)) => {
            for (n, (v, k)) in input.iter().zip(known).enumerate() {
                path.push(Step::Index(n));
                unknown_fields(v, k, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn display(path: &[Step]) -> String {
    let mut out = String::new();
    for step in path {
        match step {
            Step::Key(k) if out.is_empty() => out.push_str(k),
            Step::Key(k) => out.push_str(&format!(".{k}")),
            Step::Index(n) => out.push_str(&format!("[{n}]")),
        }
    }
    if out.is_empty() {
        "(document)".into()
    } else {
        out
    }
}

/// The example value at `path`, or at its deepest prefix the example has.
fn example_at(example: &Value, path: &[Step]) -> String {
    let mut at = example;
    for step in path {
        let next = match step {
            Step::Key(k) => at.get(k),
            Step::Index(_) => at.get(0),
        };
        match next {
            Some(v) => at = v,
            None => break,
        }
    }
    at.to_string()
}

/// A short valid answer; each list holds one element so every field path has an example.
fn example(role: Role, mode: Mode) -> Value {
    let card_patch = json!({
        "interface": [{"id": "create-item", "anchor": "src/api.rs::create_item", "contract": "POST /items returns 201"}],
        "anchors": [{"path": "src/api.rs", "name": "router"}],
        "gaps": [{"kind": "gap", "title": "no paging", "anchor": "src/api.rs::list", "note": "list returns everything", "repro": "cargo test list_pages"}],
        "remove_interface": ["delete-item"],
        "remove_anchors": ["src/api.rs::old_router"],
        "remove_gaps": ["no auth"]
    });
    let mut v = match (role, mode) {
        (Role::Maintainer, Mode::Review) => json!({
            "verdict": "changes",
            "findings": [{"file": "src/api.rs", "line": 12, "rung": "stdlib", "cut": "the hand-rolled join", "replace_with": "slice::join"}],
            "card_patch": card_patch,
            "tester_needed": false
        }),
        (Role::Maintainer, Mode::Explain) => json!({
            "explanation": "create_item validates then stores (src/api.rs::create_item).",
            "tasks": [{"title": "add priority", "change": "accept a priority field", "files": ["src/api.rs"], "check": "cargo test api"}],
            "card_patch": card_patch
        }),
        (Role::Developer, _) => json!({
            "check": {"command": "cargo test api", "pass": true, "output": "1 passed"},
            "summary": "added the priority field",
            "blocked_on": "need src/store.rs to add the column"
        }),
        (Role::Tester, _) => json!({
            "scenario_patch": [{"id": "create-ok", "interface": "create-item", "given": "a valid body", "expect": "201", "level": "integration", "command": "cargo test create_ok"}],
            "results": [{"scenario": "create-ok", "pass": true, "output": "ok"}],
            "bugs": [{"title": "count off by one", "repro": "cargo test count", "evidence": "expected 2, got 3", "interface": "create-item"}],
            "tests_written": ["tests/api.rs"]
        }),
        (Role::Documenter, _) => json!({
            "doc": {"overview": "Serves items over HTTP.", "how_it_works": "Requests enter at src/api.rs::router.", "how_to_change": "Start at the router; keep status codes stable."}
        }),
        (Role::Validator, _) => json!({
            "real": true,
            "severity": "medium",
            "reason": "the count is wrong with no error",
            "evidence": ["expected 2, got 3"]
        }),
        (Role::Architect, _) => json!({
            "plan": {"title": "add priority end to end", "steps": [{"unit": "api", "change": "accept priority", "interfaces": ["create-item"], "check": "cargo test api"}]},
            "flows": [{"name": "create-item", "units": ["api", "store"], "entry": "src/api.rs::create_item", "check": "cargo test create_item"}],
            "decisions": [{"title": "store has no HTTP knowledge", "why": "keeps storage testable"}],
            "unit_proposals": [{"name": "auth", "purpose": "login and tokens", "paths": ["src/auth/**"]}],
            "explanation": "Two steps, api then store."
        }),
        (Role::Reviewer, _) => json!({
            "snapshot": "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
            "units": [{
                "unit": "api",
                "verdict": "changes",
                "findings": [{"file": "src/api.rs", "line": 12, "rung": "stdlib", "cut": "the hand-rolled join", "replace_with": "slice::join"}],
                "card_patch": card_patch,
                "check": {"command": "cargo test api", "pass": true, "output": "1 passed"},
                "scenario_patch": [{"id": "create-ok", "interface": "create-item", "given": "a valid body", "expect": "201", "level": "integration", "command": "cargo test create_ok"}],
                "bugs": [{"title": "count off by one", "repro": "cargo test count", "evidence": "expected 2, got 3", "interface": "create-item"}],
                "doc": {"overview": "Serves items over HTTP.", "how_it_works": "Requests enter at src/api.rs::router.", "how_to_change": "Start at the router; keep status codes stable."}
            }]
        }),
    };
    v["pulled_anchors"] = json!(["src/api.rs::router"]);
    v["notes"] = json!("");
    v
}

/// Makes the BAML library download opt-in (`STEWARDS_BAML_DOWNLOAD`), keeps BAML from logging
/// prompts and replies to stderr unless `BAML_LOG` is set, and drops `BOUNDARY_*` variables so
/// nothing is sent to Boundary's cloud.
///
/// # Safety
/// Mutates the process environment: call it before any other thread exists.
pub unsafe fn prepare_baml_env() {
    let boundary: Vec<_> = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| k.to_string_lossy().starts_with("BOUNDARY_"))
        .collect();
    // SAFETY: the caller guarantees no other thread exists.
    unsafe {
        if std::env::var_os("STEWARDS_BAML_DOWNLOAD").is_none() {
            std::env::set_var("BAML_LIBRARY_DISABLE_DOWNLOAD", "true");
        }
        if std::env::var_os("BAML_LOG").is_none() {
            std::env::set_var("BAML_LOG", "off");
        }
        for k in boundary {
            std::env::remove_var(k);
        }
    }
}

/// `direct`'s three guards for the role, in order: a `[models.direct]` entry naming a supported
/// provider, that provider's key, the native library. Returns the provider, model, and key.
pub fn direct_guards(role: Role) -> Result<(&'static str, String, String)> {
    let config = crate::user::load_config()?;
    let role_name = role.as_str();
    let no_model = |message: String| {
        CliError::new(
            "no_direct_model",
            message,
            format!(
                "set [models.direct] {role_name} = \"<provider>/<model>\" in ~/.stewards/config.toml (providers: anthropic, openai, google-ai)"
            ),
        )
    };
    let value = config
        .model("direct", role_name)
        .ok_or_else(|| no_model(format!("no model for the {role_name} in [models.direct]")))?;
    let (provider, model, key_var) = match value.split_once('/') {
        Some(("anthropic", m)) if !m.is_empty() => ("anthropic", m, "ANTHROPIC_API_KEY"),
        Some(("openai", m)) if !m.is_empty() => ("openai", m, "OPENAI_API_KEY"),
        Some(("google-ai", m)) if !m.is_empty() => ("google-ai", m, "GOOGLE_API_KEY"),
        _ => {
            return Err(no_model(format!(
                "[models.direct] {role_name} = \"{value}\" is not <provider>/<model> with a supported provider"
            ))
            .into());
        }
    };
    let key = std::env::var(key_var)
        .ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| {
            CliError::new(
                "no_api_key",
                format!("--direct with {provider} needs {key_var}, which is not set"),
                format!("export {key_var}"),
            )
        })?;
    load_runtime()?;
    Ok((provider, model.to_string(), key))
}

/// Calls the role's model through BAML (`--direct`) and returns the answer as JSON text.
/// `unit` fills the role text; the architect and the reviewer ignore it.
pub fn direct(role: Role, mode: Mode, unit: &str, brief_text: &str) -> Result<String> {
    let (provider, model, key) = direct_guards(role)?;

    let mut registry = baml::ClientRegistry::new();
    // An explicit api_key: without it BAML sends the literal `$OPENAI_API_KEY`.
    let options = HashMap::from([
        ("model".to_string(), json!(model)),
        ("api_key".to_string(), json!(key)),
    ]);
    registry.add_llm_client("Direct", provider, options);
    registry.set_primary_client("Direct");
    let answer = match (role, mode) {
        (Role::Maintainer, Mode::Review) => Answer::MaintainerReview(
            B.MaintainerReview
                .with_client_registry(&registry)
                .call(unit, brief_text)
                .map_err(call_failed)?,
        ),
        (Role::Maintainer, Mode::Explain) => Answer::MaintainerExplain(
            B.MaintainerExplain
                .with_client_registry(&registry)
                .call(unit, brief_text)
                .map_err(call_failed)?,
        ),
        (Role::Developer, _) => Answer::Developer(
            B.Developer
                .with_client_registry(&registry)
                .call(unit, brief_text)
                .map_err(call_failed)?,
        ),
        (Role::Tester, _) => Answer::Tester(
            B.Tester
                .with_client_registry(&registry)
                .call(unit, brief_text)
                .map_err(call_failed)?,
        ),
        (Role::Documenter, _) => Answer::Documenter(
            B.Documenter
                .with_client_registry(&registry)
                .call(unit, brief_text)
                .map_err(call_failed)?,
        ),
        (Role::Validator, _) => Answer::Validator(
            B.Validator
                .with_client_registry(&registry)
                .call(unit, brief_text)
                .map_err(call_failed)?,
        ),
        (Role::Architect, _) => Answer::Architect(
            B.Architect
                .with_client_registry(&registry)
                .call(brief_text)
                .map_err(call_failed)?,
        ),
        (Role::Reviewer, _) => Answer::Review(
            B.Reviewer
                .with_client_registry(&registry)
                .call(brief_text)
                .map_err(call_failed)?,
        ),
    };
    Ok(serde_json::to_string(&answer)?)
}

/// Fixed text: BAML's error can carry the brief, the reply, or the provider's response.
fn call_failed(_: baml::BamlError) -> CliError {
    CliError::new(
        "direct_call_failed",
        "the model call through BAML failed",
        "rerun with BAML_LOG=info to see the provider error",
    )
}

/// Loads the native library; the generated client panics when it is missing, so catch that.
fn load_runtime() -> Result<(), CliError> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let loaded = std::panic::catch_unwind(baml_client::init);
    std::panic::set_hook(hook);
    loaded.map_err(|_| {
        use std::env::consts::{ARCH, DLL_EXTENSION, DLL_PREFIX, OS};
        let target = match OS {
            "macos" => "apple-darwin",
            "windows" => "pc-windows-msvc",
            _ => "unknown-linux-gnu",
        };
        CliError::new(
            "baml_runtime_unavailable",
            format!(
                "the BAML native library 0.226.3 ({DLL_PREFIX}baml_cffi-{ARCH}-{target}.{DLL_EXTENSION}) was not found or could not load"
            ),
            "set BAML_LIBRARY_PATH to the library, or set STEWARDS_BAML_DOWNLOAD=1 to let stewards download it",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::Path;

    fn texts(request: Result<baml::HTTPRequest, baml::BamlError>) -> Vec<String> {
        let body = request.unwrap().body().json().unwrap();
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["content"][0]["text"].as_str().unwrap().to_string())
            .collect()
    }

    /// Rewrites src/llm_schema/ from baml_src through the compiled client; needs no key, but
    /// needs BAML_LIBRARY_PATH set to the native library and fails at once without it, so it
    /// never triggers the download. Run after `baml-cli generate`.
    #[test]
    #[ignore]
    fn refresh_llm_schema() {
        assert!(
            std::env::var_os("BAML_LIBRARY_PATH").is_some(),
            "set BAML_LIBRARY_PATH to libbaml_cffi 0.226.3 to refresh src/llm_schema"
        );
        let shapes = [
            (
                "maintainer-review",
                B.MaintainerReviewSchema.build_request(),
                B.MaintainerReview.build_request("<unit>", "BRIEF"),
            ),
            (
                "maintainer-explain",
                B.MaintainerExplainSchema.build_request(),
                B.MaintainerExplain.build_request("<unit>", "BRIEF"),
            ),
            (
                "developer",
                B.DeveloperSchema.build_request(),
                B.Developer.build_request("<unit>", "BRIEF"),
            ),
            (
                "tester",
                B.TesterSchema.build_request(),
                B.Tester.build_request("<unit>", "BRIEF"),
            ),
            (
                "documenter",
                B.DocumenterSchema.build_request(),
                B.Documenter.build_request("<unit>", "BRIEF"),
            ),
            (
                "validator",
                B.ValidatorSchema.build_request(),
                B.Validator.build_request("<unit>", "BRIEF"),
            ),
            (
                "architect",
                B.ArchitectSchema.build_request(),
                B.Architect.build_request("BRIEF"),
            ),
            (
                "reviewer",
                B.ReviewerSchema.build_request(),
                B.Reviewer.build_request("BRIEF"),
            ),
        ];
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/llm_schema");
        for (stem, schema, role) in shapes {
            let schema = texts(schema).concat();
            let role = texts(role);
            assert_eq!(role[1], "BRIEF", "{stem}: the brief is the user message");
            let role_text = role[0]
                .strip_suffix(&schema)
                .unwrap_or_else(|| panic!("{stem}: the system message ends with the schema"))
                .trim_end();
            std::fs::write(
                dir.join(format!("{stem}.schema.txt")),
                format!("{schema}\n"),
            )
            .unwrap();
            std::fs::write(
                dir.join(format!("{stem}.role.txt")),
                format!("{role_text}\n"),
            )
            .unwrap();
        }
        std::fs::write(
            dir.join("llm_sources.sha256"),
            format!("{}\n", llm_sources_hash()),
        )
        .unwrap();
    }

    #[test]
    fn no_example_is_null() {
        for (role, mode) in [
            (Role::Maintainer, Mode::Review),
            (Role::Maintainer, Mode::Explain),
            (Role::Developer, Mode::Review),
            (Role::Tester, Mode::Review),
            (Role::Documenter, Mode::Review),
            (Role::Validator, Mode::Review),
            (Role::Architect, Mode::Review),
            (Role::Reviewer, Mode::Review),
        ] {
            assert!(
                !example(role, mode).to_string().contains("null"),
                "{role:?} {mode:?}"
            );
        }
    }

    #[test]
    fn every_example_is_a_valid_answer() {
        for role in [
            Role::Developer,
            Role::Maintainer,
            Role::Tester,
            Role::Documenter,
            Role::Validator,
            Role::Architect,
            Role::Reviewer,
        ] {
            for mode in [Mode::Review, Mode::Explain] {
                let text = example(role, mode).to_string();
                assert!(parse(role, mode, &text).is_ok(), "{role:?} {mode:?}");
            }
        }
    }

    /// Must match `llm_sources_hash` in tests/llm.rs.
    fn llm_sources_hash() -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = vec![];
        for dir in ["baml_src", "src/llm_schema"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|e| e == "baml" || e == "txt") {
                    files.push(path.strip_prefix(root).unwrap().to_path_buf());
                }
            }
        }
        files.sort();
        let mut hash = Sha256::new();
        for f in files {
            hash.update(f.as_os_str().as_encoded_bytes());
            hash.update([0]);
            hash.update(std::fs::read(root.join(f)).unwrap());
        }
        hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}
