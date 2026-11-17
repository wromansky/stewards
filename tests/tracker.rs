// `validate --export` against a fake forge CLI and a fake HTTP transport: no `gh`, `glab`, or network.
#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::{append, fixture, json};
use serde_json::{Value, json};
use stewards::forge::{Fake, Out};
use stewards::model::{UnitCard, load_repo};
use stewards::output::CliError;
use stewards::tracker::{Exported, FakeHttp, backend, export};
use stewards::workspace::Repo;

const CARD: &str = ".stewards/units/api.toml";
const TOKEN: &str = "s3cr3t-token-value";
const NOTE: &str = "a ```` run, {noformat} and {NOFORMAT}, @user\n/close\nend";

fn bug(title: &str, severity: Option<&str>) -> String {
    let severity = severity.map_or(String::new(), |s| format!("severity = \"{s}\"\n"));
    format!(
        "\n[[gap]]\nkind = \"bug\"\ntitle = \"{title}\"\nanchor = \"src/api/items.py::create_item\"\nnote = {}\nrepro = \"pytest tests -k count\"\n{severity}",
        toml::Value::String(NOTE.into())
    )
}

/// The single fixture with `[tracker]` of `kind` and `project`, and the given bugs on `api`.
fn setup(kind: &str, project: &str, bugs: &[(&str, Option<&str>)]) -> tempfile::TempDir {
    let dir = fixture("single");
    let d = dir.path();
    append(
        &d.join(".stewards/repo.toml"),
        &format!("[tracker]\nkind = \"{kind}\"\nproject = \"{project}\"\n"),
    );
    for (t, s) in bugs {
        append(&d.join(CARD), &bug(t, *s));
    }
    dir
}

fn repo(d: &Path) -> Repo {
    Repo {
        name: "single".into(),
        root: d.to_path_buf(),
        loaded: Some(load_repo(d).unwrap()),
    }
}

fn issue(d: &Path, title: &str) -> Option<String> {
    let card: UnitCard = toml::from_str(&fs::read_to_string(d.join(CARD)).unwrap()).unwrap();
    card.gaps.into_iter().find(|g| g.title == title)?.issue
}

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let vars: Vec<(String, String)> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
}

/// One export through `cli` and `http` with `vars` as the environment.
fn run(d: &Path, cli: &Fake, http: &FakeHttp, vars: &[(&str, &str)]) -> anyhow::Result<Exported> {
    let e = env(vars);
    export(&repo(d), |t| backend(t, &e, cli, None, http))
}

fn code(e: &anyhow::Error) -> &'static str {
    e.downcast_ref::<CliError>().expect("a CliError").code
}

fn no_secret(text: &str) {
    assert!(!text.contains(TOKEN), "{text}");
    assert!(!text.contains("czNjcjN0"), "{text}"); // base64 of the token's start
}

fn failing(stderr: &str) -> Out {
    Out {
        ok: false,
        stdout: String::new(),
        stderr: stderr.into(),
    }
}

/// Each untrusted line sits indented one space inside a fence longer than its backtick runs.
fn fenced_markdown(body: &str) {
    assert!(body.contains("\n`````text\n"), "{body}");
    assert!(
        body.contains("\n a ```` run, {noformat} and {NOFORMAT}, @user\n /close\n end\n`````\n"),
        "{body}"
    );
    assert!(
        !body
            .lines()
            .any(|l| l.starts_with('/') || l.starts_with('@')),
        "{body}"
    );
    assert!(body.starts_with("Unit api, severity critical.\n"), "{body}");
    assert!(
        body.ends_with("Filed by stewards from .stewards/units/api.toml."),
        "{body}"
    );
}

#[test]
fn github_files_through_gh_api_and_records_the_url_once() {
    let dir = setup("github", "o/r", &[("count off by one", Some("critical"))]);
    let d = dir.path();
    let cli = Fake::new(vec![
        ("auth status --hostname github.com", Out::ok("")),
        (
            "api -X POST",
            Out::ok(r#"{"html_url":"https://github.com/o/r/issues/7"}"#),
        ),
    ]);
    let http = FakeHttp::new(vec![]);
    let out = run(d, &cli, &http, &[]).unwrap();
    assert_eq!(out.exported.len(), 1);
    assert_eq!(out.exported[0].url, "https://github.com/o/r/issues/7");
    assert_eq!(out.then, "done");
    assert_eq!(
        issue(d, "count off by one").as_deref(),
        Some("https://github.com/o/r/issues/7")
    );
    let calls = cli.calls.borrow();
    assert_eq!(
        calls[1].0,
        "gh api -X POST repos/o/r/issues --input - -H Content-Type: application/json --hostname github.com"
    );
    let sent: Value = serde_json::from_str(calls[1].1.as_deref().unwrap()).unwrap();
    assert_eq!(sent["title"], "count off by one");
    fenced_markdown(sent["body"].as_str().unwrap());
    assert!(http.calls.borrow().is_empty());
    drop(calls);

    let again = Fake::new(vec![]);
    let out = run(d, &again, &http, &[]).unwrap();
    assert!(out.exported.is_empty());
    assert!(again.calls.borrow().is_empty());
}

#[test]
fn gitlab_files_through_glab_api_with_an_encoded_project() {
    let dir = setup("gitlab", "g/sub/p", &[("b", Some("low"))]);
    let d = dir.path();
    let cli = Fake::new(vec![
        ("glab auth status --hostname gitlab.com", Out::ok("")),
        (
            "glab api -X POST",
            Out::ok(r#"{"web_url":"https://gitlab.com/g/sub/p/-/issues/3"}"#),
        ),
    ]);
    run(d, &cli, &FakeHttp::new(vec![]), &[]).unwrap();
    let calls = cli.calls.borrow();
    assert_eq!(
        calls[1].0,
        "glab api -X POST projects/g%2Fsub%2Fp/issues --input - -H Content-Type: application/json --hostname gitlab.com"
    );
    let sent: Value = serde_json::from_str(calls[1].1.as_deref().unwrap()).unwrap();
    assert_eq!(sent["title"], "b");
    assert!(
        sent["description"]
            .as_str()
            .unwrap()
            .contains("\n /close\n")
    );
    assert_eq!(
        issue(d, "b").as_deref(),
        Some("https://gitlab.com/g/sub/p/-/issues/3")
    );
}

#[test]
fn a_forge_cli_without_a_login_is_tracker_auth() {
    let dir = setup("github", "o/r", &[("b", Some("low"))]);
    let cli = Fake::new(vec![("auth status", failing("not logged in"))]);
    let e = run(dir.path(), &cli, &FakeHttp::new(vec![]), &[]).unwrap_err();
    assert_eq!(code(&e), "tracker_auth");
}

#[test]
fn jira_posts_the_issue_with_bearer_or_basic_auth() {
    for (vars, want) in [
        (vec![], format!("Bearer {TOKEN}")),
        (
            vec![("JIRA_EMAIL", "me@x.io")],
            "Basic bWVAeC5pbzpzM2NyM3QtdG9rZW4tdmFsdWU=".to_string(),
        ),
    ] {
        let dir = setup("jira", "ABC", &[("b", Some("medium"))]);
        let d = dir.path();
        let http = FakeHttp::new(vec![(201, r#"{"id":"1","key":"ABC-12"}"#)]);
        let mut all = vec![
            ("JIRA_URL", "https://x.atlassian.net/"),
            ("JIRA_TOKEN", TOKEN),
        ];
        all.extend(vars);
        let out = run(d, &Fake::new(vec![]), &http, &all).unwrap();
        let printed = serde_json::to_string(&out).unwrap();
        no_secret(&printed);
        assert_eq!(out.exported[0].url, "https://x.atlassian.net/browse/ABC-12");
        assert_eq!(
            issue(d, "b").as_deref(),
            Some("https://x.atlassian.net/browse/ABC-12")
        );
        no_secret(&fs::read_to_string(d.join(CARD)).unwrap());
        let calls = http.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, "https://x.atlassian.net/rest/api/2/issue");
        let names: Vec<&str> = calls[0].headers.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["Authorization", "Content-Type", "Accept"]);
        assert_eq!(calls[0].headers[0].1, want);
        let sent: Value = serde_json::from_str(&calls[0].body).unwrap();
        assert_eq!(sent["fields"]["project"], json!({"key": "ABC"}));
        assert_eq!(sent["fields"]["summary"], "b");
        assert_eq!(sent["fields"]["issuetype"], json!({"name": "Bug"}));
        let body = sent["fields"]["description"].as_str().unwrap();
        // only the tool's own {noformat} blocks: three, open and close
        assert_eq!(body.matches("{noformat}").count(), 6, "{body}");
        assert!(
            !body
                .to_lowercase()
                .replace("{noformat}", "")
                .contains("noformat}")
        );
        assert!(
            body.contains("{noformat}\na ```` run,  and , @user\n/close\nend\n{noformat}"),
            "{body}"
        );
    }
}

#[test]
fn jira_auth_and_failures() {
    let vars = [
        ("JIRA_URL", "https://x.atlassian.net"),
        ("JIRA_TOKEN", TOKEN),
    ];
    let dir = setup("jira", "ABC", &[("b", Some("low"))]);
    let d = dir.path();
    let cli = Fake::new(vec![]);
    let e = run(d, &cli, &FakeHttp::new(vec![(401, "no")]), &vars).unwrap_err();
    assert_eq!(code(&e), "tracker_auth");
    no_secret(&format!(
        "{e} {}",
        e.downcast_ref::<CliError>().unwrap().fix
    ));

    let echo = format!("{{\"errorMessages\":[\"bad token {TOKEN}\"]}}\n");
    let e = run(d, &cli, &FakeHttp::new(vec![(500, &echo)]), &vars).unwrap_err();
    assert_eq!(code(&e), "tracker_failed");
    let text = e.to_string();
    assert!(text.contains("HTTP 500") && text.contains("jira"), "{text}");
    no_secret(&text);

    for url in [
        "http://x.atlassian.net",
        "https://u:p@x.atlassian.net",
        "ftp://x",
        "https://x.atlassian.net?a=b",
        "https://x.atlassian.net#f",
        "https://x.atlassian.net\\@evil.io",
    ] {
        let http = FakeHttp::new(vec![]);
        let e = run(d, &cli, &http, &[("JIRA_URL", url), ("JIRA_TOKEN", TOKEN)]).unwrap_err();
        assert_eq!(code(&e), "tracker_auth", "{url}");
        assert!(http.calls.borrow().is_empty());
    }
    let e = run(
        d,
        &cli,
        &FakeHttp::new(vec![]),
        &[("JIRA_URL", "https://x.atlassian.net")],
    )
    .unwrap_err();
    assert_eq!(code(&e), "tracker_auth");
    assert_eq!(issue(d, "b"), None);
}

#[test]
fn linear_finds_the_team_then_creates_the_issue() {
    let dir = setup("linear", "ENG", &[("b", Some("critical"))]);
    let d = dir.path();
    let http = FakeHttp::new(vec![
        (200, r#"{"data":{"teams":{"nodes":[{"id":"team-1"}]}}}"#),
        (
            200,
            r#"{"data":{"issueCreate":{"success":true,"issue":{"url":"https://linear.app/x/issue/ENG-4"}}}}"#,
        ),
    ]);
    let out = run(d, &Fake::new(vec![]), &http, &[("LINEAR_API_KEY", TOKEN)]).unwrap();
    no_secret(&serde_json::to_string(&out).unwrap());
    assert_eq!(
        issue(d, "b").as_deref(),
        Some("https://linear.app/x/issue/ENG-4")
    );
    let calls = http.calls.borrow();
    for c in calls.iter() {
        assert_eq!(c.url, "https://api.linear.app/graphql");
        assert_eq!(
            c.headers,
            common::pairs(&[
                ("Authorization", TOKEN),
                ("Content-Type", "application/json")
            ])
        );
    }
    let team: Value = serde_json::from_str(&calls[0].body).unwrap();
    assert_eq!(team["variables"], json!({"key": "ENG"}));
    assert!(!team["query"].as_str().unwrap().contains("ENG"));
    let made: Value = serde_json::from_str(&calls[1].body).unwrap();
    let input = &made["variables"]["input"];
    assert_eq!(input["teamId"], "team-1");
    assert_eq!(input["title"], "b");
    fenced_markdown(input["description"].as_str().unwrap());
}

#[test]
fn linear_without_the_team_names_the_key() {
    let dir = setup("linear", "ENG", &[("b", Some("low"))]);
    let http = FakeHttp::new(vec![(200, r#"{"data":{"teams":{"nodes":[]}}}"#)]);
    let e = run(
        dir.path(),
        &Fake::new(vec![]),
        &http,
        &[("LINEAR_API_KEY", TOKEN)],
    )
    .unwrap_err();
    assert_eq!(code(&e), "tracker_failed");
    assert!(e.to_string().contains("`ENG`"), "{e}");
    let e = run(dir.path(), &Fake::new(vec![]), &http, &[]).unwrap_err();
    assert_eq!(code(&e), "tracker_auth");
}

#[test]
fn a_failure_keeps_the_issues_already_filed() {
    let dir = setup(
        "jira",
        "ABC",
        &[("first", Some("low")), ("second", Some("low"))],
    );
    let d = dir.path();
    let http = FakeHttp::new(vec![(201, r#"{"key":"ABC-1"}"#), (502, "bad gateway")]);
    let vars = [
        ("JIRA_URL", "https://x.atlassian.net"),
        ("JIRA_TOKEN", TOKEN),
    ];
    let e = run(d, &Fake::new(vec![]), &http, &vars).unwrap_err();
    assert_eq!(code(&e), "tracker_failed");
    assert!(e.to_string().contains("1 issue(s) filed"), "{e}");
    assert_eq!(
        issue(d, "first").as_deref(),
        Some("https://x.atlassian.net/browse/ABC-1")
    );
    assert_eq!(issue(d, "second"), None);
}

#[test]
fn a_returned_url_that_is_not_one_https_line_is_refused() {
    for bad in [
        "javascript:alert(1)",
        "https://a.io/1\nhttps://b.io",
        "https://evil.io/o/r/issues/1",
        "https://github.com/o/r2/issues/1",
        "https://github.com/o/r/issues/1\u{202E}",
    ] {
        let dir = setup("github", "o/r", &[("b", Some("low"))]);
        let reply = json!({ "html_url": bad }).to_string();
        let cli = Fake::new(vec![
            ("auth status", Out::ok("")),
            ("api -X", Out::ok(&reply)),
        ]);
        let e = run(dir.path(), &cli, &FakeHttp::new(vec![]), &[]).unwrap_err();
        assert_eq!(code(&e), "tracker_failed", "{bad}");
        let text = e.to_string();
        assert!(text.contains("an issue may exist"), "{text}");
        assert!(!text.contains('\n') && !text.contains('\u{202E}'), "{text}");
        assert_eq!(issue(dir.path(), "b"), None);
    }
}

#[test]
fn jira_hides_an_echoed_basic_authorization() {
    let dir = setup("jira", "ABC", &[("b", Some("low"))]);
    let basic = "bWVAeC5pbzpzM2NyM3QtdG9rZW4tdmFsdWU=";
    let echo = format!("header was Basic {basic}");
    let vars = [
        ("JIRA_URL", "https://x.atlassian.net"),
        ("JIRA_TOKEN", TOKEN),
        ("JIRA_EMAIL", "me@x.io"),
    ];
    let e = run(
        dir.path(),
        &Fake::new(vec![]),
        &FakeHttp::new(vec![(500, &echo)]),
        &vars,
    )
    .unwrap_err();
    assert_eq!(code(&e), "tracker_failed");
    assert!(!e.to_string().contains(basic), "{e}");
    assert!(e.to_string().contains("[redacted]"), "{e}");
}

#[test]
fn jira_drops_every_noformat_macro_with_its_parameters() {
    let dir = setup("jira", "ABC", &[]);
    let d = dir.path();
    let note = "a{noformat:title=x}b{NoFormat}c{no{noformat}format}d{noformat";
    append(
        &d.join(CARD),
        &format!(
            "\n[[gap]]\nkind = \"bug\"\ntitle = \"n\"\nnote = \"{note}\"\nseverity = \"low\"\n"
        ),
    );
    let http = FakeHttp::new(vec![(201, r#"{"key":"ABC-1"}"#)]);
    let vars = [
        ("JIRA_URL", "https://x.atlassian.net"),
        ("JIRA_TOKEN", TOKEN),
    ];
    run(d, &Fake::new(vec![]), &http, &vars).unwrap();
    let sent: Value = serde_json::from_str(&http.calls.borrow()[0].body).unwrap();
    let body = sent["fields"]["description"].as_str().unwrap();
    assert!(body.contains("{noformat}\nabcd\n{noformat}"), "{body}");
    assert_eq!(
        body.to_lowercase().matches("{noformat").count(),
        6,
        "{body}"
    );
}

/// A tracker double that runs `meddle` while it "creates" the issue.
struct Meddling<F: Fn()>(F);

impl<F: Fn()> stewards::tracker::Issues for Meddling<F> {
    fn create_issue(&self, _: &str, _: &str) -> anyhow::Result<String> {
        (self.0)();
        Ok("https://t.io/i/1".into())
    }
    fn issue_prefix(&self) -> String {
        "https://t.io/".into()
    }
}

#[test]
fn a_card_write_that_fails_after_filing_names_the_url() {
    let by_hand = "issue https://t.io/i/1 was filed but not recorded; set issue = \"https://t.io/i/1\" on that gap in .stewards/units/api.toml by hand";
    // the bug is closed by another writer while the issue is created
    let dir = setup("github", "o/r", &[("b", Some("low"))]);
    let d = dir.path().to_path_buf();
    let card = d.join(CARD);
    let e = export(&repo(&d), |_| {
        Ok(Box::new(Meddling(|| {
            common::edit(&card, "title = \"b\"", "title = \"renamed\"")
        })))
    })
    .unwrap_err();
    assert_eq!(code(&e), "card_changed");
    let c = e.downcast_ref::<CliError>().unwrap();
    assert!(c.message.contains(by_hand), "{}", c.message);
    assert_eq!(c.fix, by_hand);

    // the card stops parsing: a local failure keeps its own code
    let dir = setup("github", "o/r", &[("b", Some("low"))]);
    let d = dir.path().to_path_buf();
    let card = d.join(CARD);
    let e = export(&repo(&d), |_| {
        Ok(Box::new(Meddling(|| {
            fs::write(&card, "not toml [").unwrap()
        })))
    })
    .unwrap_err();
    assert_eq!(code(&e), "internal");
    let c = e.downcast_ref::<CliError>().unwrap();
    assert!(c.message.contains(by_hand), "{}", c.message);
    assert!(!c.fix.contains("again"), "{}", c.fix);
}

#[test]
fn the_url_goes_to_the_judged_bug_not_an_unjudged_namesake() {
    let dir = setup("github", "o/r", &[("dup", None), ("dup", Some("low"))]);
    let d = dir.path();
    let e = export(&repo(d), |_| Ok(Box::new(Meddling(|| ())))).unwrap();
    assert_eq!(e.exported.len(), 1);
    let card: UnitCard = toml::from_str(&fs::read_to_string(d.join(CARD)).unwrap()).unwrap();
    let dups: Vec<_> = card.gaps.iter().filter(|g| g.title == "dup").collect();
    assert_eq!(dups[0].issue, None);
    assert_eq!(dups[1].issue.as_deref(), Some("https://t.io/i/1"));
}

#[test]
fn the_origin_host_is_used_only_for_the_origin_project() {
    for (project, host) in [("o/r", "gh.corp"), ("x/y", "github.com")] {
        let dir = setup("github", project, &[("b", Some("low"))]);
        let reply = json!({ "html_url": format!("https://{host}/{project}/issues/1") }).to_string();
        let cli = Fake::new(vec![
            ("auth status", Out::ok("")),
            ("api -X", Out::ok(&reply)),
        ]);
        let http = FakeHttp::new(vec![]);
        let e = env(&[]);
        export(&repo(dir.path()), |t| {
            backend(t, &e, &cli, Some("https://gh.corp/o/r.git"), &http)
        })
        .unwrap();
        let calls = cli.calls.borrow();
        let post = calls.iter().find(|(c, _)| c.contains("api -X")).unwrap();
        assert!(
            post.0.ends_with(&format!("--hostname {host}")),
            "{}",
            post.0
        );
    }
}

#[test]
fn a_bad_project_is_usage() {
    let dir = setup("github", "--evil/r", &[("b", Some("low"))]);
    let cli = Fake::new(vec![]);
    let e = run(dir.path(), &cli, &FakeHttp::new(vec![]), &[]).unwrap_err();
    assert_eq!(code(&e), "usage");
    assert!(cli.calls.borrow().is_empty());
}

/// The binary with `vars` set and the tracker variables cleared; none of these reach a network.
fn cli(d: &Path, vars: &[(&str, &str)]) -> (i32, Value) {
    let home = tempfile::TempDir::new().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_stewards"));
    c.args(["validate", "--export"])
        .current_dir(d)
        .env("STEWARDS_HOME", home.path());
    for k in ["JIRA_URL", "JIRA_TOKEN", "JIRA_EMAIL", "LINEAR_API_KEY"] {
        c.env_remove(k);
    }
    c.envs(vars.iter().copied());
    json(c.output().unwrap())
}

#[test]
fn the_command_reports_usage_auth_and_unjudged_bugs() {
    let dir = fixture("single");
    let (exit, v) = cli(dir.path(), &[]);
    assert_eq!(exit, 2, "{v}");
    assert_eq!(v["error"]["code"], "usage");
    assert_eq!(
        v["error"]["fix"],
        "add [tracker] kind and project to .stewards/repo.toml"
    );

    let dir = setup("jira", "ABC", &[("b", Some("low"))]);
    let (exit, v) = cli(
        dir.path(),
        &[("JIRA_URL", "http://x.io"), ("JIRA_TOKEN", TOKEN)],
    );
    assert_eq!(exit, 2, "{v}");
    assert_eq!(v["error"]["code"], "tracker_auth");
    no_secret(&v.to_string());

    let dir = setup("linear", "ENG", &[("b", None)]);
    let (exit, v) = cli(dir.path(), &[]);
    assert_eq!(exit, 0, "{v}");
    assert_eq!(v["exported"], json!([]));
    assert!(
        v["then"]
            .as_str()
            .unwrap()
            .starts_with("Bugs without a severity are not exported")
    );
}

#[test]
fn rendered_titles_are_one_escaped_line() {
    use stewards::output::Human;
    let out = Exported {
        exported: vec![stewards::tracker::Filed {
            unit: "api".into(),
            title: "a\nb\u{202E}c".into(),
            url: "https://t.io/i/1".into(),
        }],
        then: "done".into(),
    };
    let text = out.render();
    assert!(text.starts_with("Filed `a b\\u{202E}c` on api"), "{text}");
}
