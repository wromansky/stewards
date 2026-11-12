use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stewards::llm::{FieldError, Mode, direct, output_format, parse, prepare_baml_env, role_text};
use stewards::model::Role;
use stewards::output::CliError;
use tempfile::TempDir;

const SHAPES: [(Role, Mode); 7] = [
    (Role::Maintainer, Mode::Review),
    (Role::Maintainer, Mode::Explain),
    (Role::Developer, Mode::Review),
    (Role::Tester, Mode::Review),
    (Role::Documenter, Mode::Review),
    (Role::Validator, Mode::Review),
    (Role::Architect, Mode::Review),
];

fn with_common(mut v: Value) -> Value {
    v["pulled_anchors"] = json!([]);
    v["notes"] = json!("none");
    v
}

fn review() -> Value {
    with_common(json!({
        "verdict": "changes",
        "findings": [{"file": "src/a.rs", "line": 3, "rung": "one_line", "cut": "loop", "replace_with": "iter().sum()"}],
        "card_patch": {"gaps": [{"kind": "gap", "title": "t", "note": "n"}]},
        "tester_needed": true
    }))
}

fn explain() -> Value {
    with_common(json!({
        "explanation": "it stores items",
        "tasks": [{"title": "t", "change": "c", "files": ["src/a.rs"]}],
        "card_patch": null
    }))
}

fn developer() -> Value {
    with_common(json!({"check": null, "summary": "done", "blocked_on": null}))
}

fn tester() -> Value {
    with_common(json!({
        "scenario_patch": [{"id": "s", "interface": "i", "given": "g", "expect": "e", "level": "unit"}],
        "results": [{"scenario": "s", "pass": false, "output": "boom"}],
        "bugs": [{"title": "b", "repro": "cargo test s", "evidence": "boom"}],
        "tests_written": []
    }))
}

fn documenter() -> Value {
    with_common(json!({"doc": {"overview": "o", "how_it_works": "h", "how_to_change": "c"}}))
}

fn validator() -> Value {
    with_common(json!({"real": true, "severity": "critical", "reason": "r", "evidence": ["e"]}))
}

fn architect() -> Value {
    with_common(json!({
        "plan": {"title": "p", "steps": [{"unit": "api", "change": "c", "interfaces": [], "check": "cargo test"}]},
        "flows": [], "decisions": [{"title": "t", "why": "w"}], "unit_proposals": [], "explanation": "x"
    }))
}

fn errors(role: Role, mode: Mode, text: &str) -> Vec<FieldError> {
    parse(role, mode, text).expect_err("answer should be rejected")
}

/// The one error, after checking it names `field` and carries an example.
fn only_error(role: Role, mode: Mode, text: &str, field: &str) -> FieldError {
    let errs = errors(role, mode, text);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!(errs[0].field, field, "{errs:?}");
    assert!(!errs[0].example.is_empty(), "{errs:?}");
    errs[0].clone()
}

#[test]
fn maintainer_review_answer_parses() {
    parse(Role::Maintainer, Mode::Review, &review().to_string()).unwrap();
}

#[test]
fn maintainer_explain_answer_parses() {
    parse(Role::Maintainer, Mode::Explain, &explain().to_string()).unwrap();
}

#[test]
fn developer_answer_parses() {
    parse(Role::Developer, Mode::Review, &developer().to_string()).unwrap();
}

#[test]
fn tester_answer_parses() {
    parse(Role::Tester, Mode::Review, &tester().to_string()).unwrap();
}

#[test]
fn documenter_answer_parses() {
    parse(Role::Documenter, Mode::Review, &documenter().to_string()).unwrap();
}

#[test]
fn validator_answer_parses() {
    parse(Role::Validator, Mode::Review, &validator().to_string()).unwrap();
}

#[test]
fn architect_answer_parses() {
    parse(Role::Architect, Mode::Review, &architect().to_string()).unwrap();
}

#[test]
fn wrong_enum_value_names_the_field() {
    let mut v = review();
    v["findings"][0]["rung"] = json!("NotNeeded");
    let e = only_error(
        Role::Maintainer,
        Mode::Review,
        &v.to_string(),
        "findings[0].rung",
    );
    assert!(e.problem.contains("not_needed"), "{e:?}");
    assert_eq!(e.example, "\"stdlib\"");
}

#[test]
fn missing_required_field_names_the_field() {
    let mut v = developer();
    v.as_object_mut().unwrap().remove("summary");
    let e = only_error(Role::Developer, Mode::Review, &v.to_string(), "summary");
    assert!(e.problem.contains("missing field"), "{e:?}");
}

#[test]
fn wrong_type_names_the_field() {
    let mut v = validator();
    v["real"] = json!("yes");
    let e = only_error(Role::Validator, Mode::Review, &v.to_string(), "real");
    assert_eq!(e.example, "true");
}

#[test]
fn duplicate_field_is_rejected() {
    let text = review().to_string();
    let text = format!("{},\"verdict\":\"approve\"}}", &text[..text.len() - 1]);
    let e = only_error(Role::Maintainer, Mode::Review, &text, "verdict");
    assert!(e.problem.contains("duplicate field"), "{e:?}");
}

#[test]
fn nullable_string_field_has_a_string_example() {
    let mut v = developer();
    v["blocked_on"] = json!(7);
    let e = only_error(Role::Developer, Mode::Review, &v.to_string(), "blocked_on");
    assert!(e.example.starts_with('"'), "{e:?}");
}

#[test]
fn unknown_field_is_rejected() {
    let mut v = tester();
    v["results"][0]["duration"] = json!(3);
    let e = only_error(
        Role::Tester,
        Mode::Review,
        &v.to_string(),
        "results[0].duration",
    );
    assert!(e.problem.contains("unknown field"), "{e:?}");
}

#[test]
fn prose_around_the_json_is_rejected() {
    let text = format!("Here is my answer:\n{}\nThanks!", documenter());
    only_error(Role::Documenter, Mode::Review, &text, "(document)");
}

#[test]
fn text_after_the_json_is_rejected() {
    let text = format!("{}\nDone.", documenter());
    only_error(Role::Documenter, Mode::Review, &text, "(document)");
}

#[test]
fn output_format_names_every_field() {
    let common = ["pulled_anchors:", "notes:"];
    let fields: [&[&str]; 7] = [
        &[
            "verdict:",
            "findings:",
            "file:",
            "line:",
            "rung:",
            "cut:",
            "replace_with:",
            "card_patch:",
            "interface:",
            "anchors:",
            "gaps:",
            "kind:",
            "title:",
            "note:",
            "repro:",
            "tester_needed:",
        ],
        &[
            "explanation:",
            "tasks:",
            "title:",
            "change:",
            "files:",
            "check:",
            "card_patch:",
        ],
        &[
            "check:",
            "command:",
            "pass:",
            "output:",
            "summary:",
            "blocked_on:",
        ],
        &[
            "scenario_patch:",
            "id:",
            "interface:",
            "given:",
            "expect:",
            "level:",
            "command:",
            "results:",
            "scenario:",
            "pass:",
            "output:",
            "bugs:",
            "repro:",
            "evidence:",
            "tests_written:",
        ],
        &["doc:", "overview:", "how_it_works:", "how_to_change:"],
        &["real:", "severity:", "reason:", "evidence:"],
        &[
            "plan:",
            "steps:",
            "unit:",
            "interfaces:",
            "flows:",
            "units:",
            "entry:",
            "decisions:",
            "why:",
            "unit_proposals:",
            "purpose:",
            "paths:",
            "explanation:",
        ],
    ];
    for ((role, mode), fields) in SHAPES.into_iter().zip(fields) {
        let text = output_format(role, mode);
        for f in fields.iter().chain(&common) {
            assert!(
                text.contains(f),
                "{role:?} {mode:?} schema lacks {f}:\n{text}"
            );
        }
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn role_text_is_the_design_text() {
    let design = include_str!("../DESIGN.md");
    let section = &design[design.find("## Role prompts").unwrap()..];
    let section = &section[..section.find("\n## ").unwrap()];
    let labels = [
        "Maintainer, review:",
        "Maintainer, explain:",
        "Developer:",
        "Tester:",
        "Documenter:",
        "Validator:",
        "Architect:",
    ];
    for ((role, mode), label) in SHAPES.into_iter().zip(labels) {
        let quoted: Vec<&str> = section
            .lines()
            .skip_while(|l| *l != label)
            .skip(1)
            .skip_while(|l| l.is_empty())
            .map_while(|l| l.strip_prefix("> "))
            .collect();
        assert!(!quoted.is_empty(), "no quoted block after {label}");
        assert_eq!(
            collapse(role_text(role, mode)),
            collapse(&quoted.join("\n")),
            "{label}"
        );
    }
}

#[test]
fn non_maintainer_roles_ignore_the_mode() {
    assert_eq!(
        output_format(Role::Tester, Mode::Explain),
        output_format(Role::Tester, Mode::Review)
    );
}

/// The values the schema text offers for `field`: quoted inline, or listed in a named enum block.
fn schema_values(schema: &str, field: &str) -> Vec<String> {
    let line = schema
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("{field}:")))
        .unwrap();
    let values: Vec<String> = line
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(String::from)
        .collect();
    if !values.is_empty() {
        return values;
    }
    // A named enum is listed as `Name\n----\n- value` above the schema.
    let name = line.split(':').nth(1).unwrap().trim().trim_end_matches(',');
    schema
        .split(&format!("{name}\n----\n"))
        .nth(1)
        .unwrap()
        .lines()
        .map_while(|l| l.strip_prefix("- "))
        .map(String::from)
        .collect()
}

#[test]
fn every_enum_value_in_the_schema_text_parses() {
    // (answer, role, field as the schema text names it, JSON pointer to it in the answer)
    let cases = [
        (review(), Role::Maintainer, "verdict", "/verdict"),
        (review(), Role::Maintainer, "rung", "/findings/0/rung"),
        (
            review(),
            Role::Maintainer,
            "kind",
            "/card_patch/gaps/0/kind",
        ),
        (validator(), Role::Validator, "severity", "/severity"),
    ];
    for (answer, role, field, pointer) in cases {
        let values = schema_values(output_format(role, Mode::Review), field);
        assert!(values.len() >= 2, "{field}: {values:?}");
        for value in values {
            let mut v = answer.clone();
            *v.pointer_mut(pointer).unwrap() = json!(value);
            assert!(
                parse(role, Mode::Review, &v.to_string()).is_ok(),
                "{field} = {value}"
            );
        }
    }
}

/// Must match `llm_sources_hash` in src/llm.rs: baml_src/*.baml and src/llm_schema/*.txt.
fn llm_sources_hash(root: &Path) -> String {
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

const REFRESH: &str = "run `baml-cli generate --from baml_src` (0.226.3, with BAML_LIBRARY_PATH set), then \
     `cargo test --lib llm::tests::refresh_llm_schema -- --ignored`";

#[test]
fn generated_client_and_rendered_text_match_baml_src() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_map =
        std::fs::read_to_string(root.join("src/baml_client/baml_source_map.rs")).unwrap();
    for entry in std::fs::read_dir(root.join("baml_src")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let embedded = format!("m.insert({name:?}.to_string(), {text:?}.to_string());");
        assert!(
            source_map.contains(&embedded),
            "src/baml_client is stale for baml_src/{name}: {REFRESH}"
        );
    }
    let recorded = std::fs::read_to_string(root.join("src/llm_schema/llm_sources.sha256")).unwrap();
    assert_eq!(
        recorded.trim(),
        llm_sources_hash(root),
        "baml_src or src/llm_schema changed without a refresh (never edit src/llm_schema by hand): {REFRESH}"
    );
    // The on_generate hook's post-processing, so a regenerate without it is caught.
    let enums = std::fs::read_to_string(root.join("src/baml_client/types/enums.rs")).unwrap();
    assert_eq!(
        enums.matches("pub enum ").count(),
        enums
            .matches("#[serde(rename_all = \"snake_case\")]")
            .count(),
        "every generated enum needs the snake_case rename: {REFRESH}"
    );
}

/// Runs the gated test `name` of this binary in a child with a scrubbed env: no provider key,
/// no BAML library on the search path, downloads off, and any HTTP sent to a closed port.
/// `envs` are set and `remove` unset after those defaults.
fn child(name: &str, envs: &[(&str, &str)], remove: &[&str]) -> std::process::Output {
    for dir in ["/usr/lib", "/usr/local/lib"] {
        let found = std::fs::read_dir(dir).into_iter().flatten().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("libbaml_cffi")
        });
        assert!(
            !found,
            "a BAML library in {dir} would let the child reach a model"
        );
    }
    let cache = TempDir::new().unwrap();
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([name, "--exact", "--nocapture", "--test-threads=1"]);
    for var in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GOOGLE_API_KEY",
        "BAML_LIBRARY_PATH",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("STEWARDS_LLM_CHILD", "1")
        .env("BAML_LIBRARY_DISABLE_DOWNLOAD", "true")
        .env("BAML_CACHE_DIR", cache.path())
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("BAML_LOG", "off")
        .envs(envs.iter().copied());
    for var in remove {
        cmd.env_remove(var);
    }
    let out = cmd.output().unwrap();
    assert_eq!(
        std::fs::read_dir(cache.path()).unwrap().count(),
        0,
        "the child cached a library"
    );
    out
}

fn is_child() -> bool {
    std::env::var("STEWARDS_LLM_CHILD").is_ok_and(|v| v == "1")
}

#[test]
fn child_parse() {
    if is_child() {
        parse(Role::Maintainer, Mode::Review, &review().to_string()).unwrap();
        println!("CHILD parsed");
    }
}

#[test]
fn parse_needs_no_baml_library() {
    let out = child("child_parse", &[], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("CHILD parsed"),
        "{stdout}"
    );
}

#[test]
fn child_direct() {
    if is_child() {
        match direct(Role::Maintainer, Mode::Review, "api", "brief") {
            Ok(_) => println!("CHILD called"),
            Err(e) => println!(
                "CHILD {}",
                serde_json::to_string(e.downcast_ref::<CliError>().unwrap()).unwrap()
            ),
        }
    }
}

/// The CliError `direct` returns in a child whose ~/.stewards/config.toml is `config`.
fn direct_error(config: &str, envs: &[(&str, &str)]) -> Value {
    let home = TempDir::new().unwrap();
    std::fs::write(home.path().join("config.toml"), config).unwrap();
    let home_path = home.path().to_str().unwrap();
    let mut envs = envs.to_vec();
    envs.push(("STEWARDS_HOME", home_path));
    let out = child("child_direct", &envs, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find_map(|l| Some(l.split_once("CHILD ")?.1))
        .unwrap_or_else(|| panic!("no result from the child: {stdout}"));
    serde_json::from_str(line).unwrap_or_else(|_| panic!("direct returned `{line}`"))
}

#[test]
fn direct_without_a_configured_model_is_no_direct_model() {
    let e = direct_error("[models.direct]\ntester = \"anthropic/m\"\n", &[]);
    assert_eq!(e["code"], "no_direct_model");
    assert!(
        e["fix"]
            .as_str()
            .unwrap()
            .contains("[models.direct] maintainer = "),
        "{e}"
    );
}

#[test]
fn direct_with_an_unsupported_provider_is_no_direct_model() {
    let e = direct_error("[models.direct]\nmaintainer = \"mistral/large\"\n", &[]);
    assert_eq!(e["code"], "no_direct_model");
    assert!(
        e["message"].as_str().unwrap().contains("mistral/large"),
        "{e}"
    );
}

#[test]
fn direct_without_a_key_is_no_api_key() {
    let e = direct_error("[models.direct]\nmaintainer = \"anthropic/m\"\n", &[]);
    assert_eq!(e["code"], "no_api_key");
    assert_eq!(e["fix"], "export ANTHROPIC_API_KEY");
}

#[test]
fn direct_with_an_empty_key_is_no_api_key() {
    let e = direct_error(
        "[models.direct]\nmaintainer = \"openai/m\"\n",
        &[("OPENAI_API_KEY", "")],
    );
    assert_eq!(e["code"], "no_api_key");
    assert_eq!(e["fix"], "export OPENAI_API_KEY");
}

#[test]
fn direct_with_a_blank_key_is_no_api_key() {
    let e = direct_error(
        "[models.direct]\nmaintainer = \"google-ai/m\"\n",
        &[("GOOGLE_API_KEY", " \t ")],
    );
    assert_eq!(e["code"], "no_api_key");
    assert_eq!(e["fix"], "export GOOGLE_API_KEY");
}

#[test]
fn direct_without_the_baml_library_is_baml_runtime_unavailable() {
    let e = direct_error(
        "[models.direct]\nmaintainer = \"anthropic/m\"\n",
        &[("ANTHROPIC_API_KEY", "sk-test-not-a-key")],
    );
    assert_eq!(e["code"], "baml_runtime_unavailable");
    let message = e["message"].as_str().unwrap();
    assert!(
        message.contains("0.226.3") && message.contains("baml_cffi"),
        "{e}"
    );
    assert!(!message.contains("Searched"), "{e}");
    assert!(
        e["fix"]
            .as_str()
            .unwrap()
            .contains("STEWARDS_BAML_DOWNLOAD=1"),
        "{e}"
    );
}

#[test]
fn child_prepared_env() {
    if is_child() {
        // SAFETY: this child runs one test with --test-threads=1; libtest's main thread only waits.
        unsafe { prepare_baml_env() };
        for var in [
            "BAML_LOG",
            "BAML_LIBRARY_DISABLE_DOWNLOAD",
            "BOUNDARY_API_KEY",
        ] {
            println!("CHILD {var}={:?}", std::env::var(var).ok());
        }
        let e = direct(Role::Maintainer, Mode::Review, "api", "brief").unwrap_err();
        println!("CHILD code={}", e.downcast_ref::<CliError>().unwrap().code);
    }
}

// simplified: no CLI command reaches `direct` yet, so this checks prepare_baml_env (what main runs
// first) in a child test process; move it to the binary once a command takes --direct.
#[test]
fn prepared_env_turns_off_baml_logging_and_download_before_direct() {
    let home = TempDir::new().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        "[models.direct]\nmaintainer = \"anthropic/m\"\n",
    )
    .unwrap();
    let out = child(
        "child_prepared_env",
        &[
            ("STEWARDS_HOME", home.path().to_str().unwrap()),
            ("ANTHROPIC_API_KEY", "sk-test-not-a-key"),
            ("BOUNDARY_API_KEY", "x"),
        ],
        &[
            "BAML_LOG",
            "BAML_LIBRARY_DISABLE_DOWNLOAD",
            "STEWARDS_BAML_DOWNLOAD",
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in [
        "CHILD BAML_LOG=Some(\"off\")",
        "CHILD BAML_LIBRARY_DISABLE_DOWNLOAD=Some(\"true\")",
        "CHILD BOUNDARY_API_KEY=None",
        "CHILD code=baml_runtime_unavailable",
    ] {
        assert!(stdout.contains(line), "{line}: {stdout}");
    }
    assert!(!stderr.contains("Downloading"), "{stderr}");
}
