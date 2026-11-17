// The forge seam against recorded responses: no `gh`, `glab`, or network.
use serde_json::{Value, json};
use stewards::forge::{Comment, Fake, Forge, Out, PrComment, post_comment};
use stewards::output::CliError;

fn code(e: &anyhow::Error) -> &'static str {
    e.downcast_ref::<CliError>().expect("a CliError").code
}

fn failing(stderr: &str) -> Out {
    Out {
        ok: false,
        stdout: String::new(),
        stderr: stderr.into(),
    }
}

#[test]
fn the_forge_comes_from_the_origin_host() {
    let none = Fake::new(vec![]);
    for (url, want) in [
        ("https://github.com/a/b.git", Forge::GitHub),
        ("git@github.com:a/b.git", Forge::GitHub),
        ("https://gitlab.com/a/b", Forge::GitLab),
        ("git@gitlab.com:a/b.git", Forge::GitLab),
    ] {
        assert_eq!(Forge::of_origin(&none, url).unwrap(), want, "{url}");
    }
    // the exact hosts need no CLI call; a lookalike is not accepted by name
    assert!(none.calls.borrow().is_empty());
    let e = Forge::of_origin(&none, "https://notgithub.com/a/b").unwrap_err();
    assert_eq!(code(&e), "usage");
}

#[test]
fn another_host_is_the_forge_whose_cli_is_logged_in_to_it() {
    let url = "https://gitlab.corp.example/a/b.git";
    let glab = Fake::new(vec![
        ("gh auth status", failing("no")),
        (
            "glab auth status --hostname gitlab.corp.example",
            Out::ok(""),
        ),
    ]);
    assert_eq!(Forge::of_origin(&glab, url).unwrap(), Forge::GitLab);
    let gh = Fake::new(vec![(
        "gh auth status --hostname gitlab.corp.example",
        Out::ok(""),
    )]);
    assert_eq!(Forge::of_origin(&gh, url).unwrap(), Forge::GitHub);
    let neither = Fake::new(vec![("auth status", failing("no"))]);
    assert_eq!(code(&Forge::of_origin(&neither, url).unwrap_err()), "usage");
}

#[test]
fn the_unknown_host_error_never_shows_userinfo() {
    let none = Fake::new(vec![]);
    let e = Forge::of_origin(&none, "https://user:secret@example.com/a/b.git").unwrap_err();
    let text = format!("{e} {}", e.downcast_ref::<CliError>().unwrap().fix);
    assert!(text.contains("example.com/a/b"), "{text}");
    assert!(
        !text.contains("secret") && !text.contains("user:"),
        "{text}"
    );
}

fn gh_fake(comments: Value) -> Fake {
    Fake::new(vec![
        ("auth status", Out::ok("")),
        ("api user", Out::ok(r#"{"login":"me"}"#)),
        ("--paginate", Out::ok(&comments.to_string())),
        (
            "-X",
            Out::ok(r#"{"html_url":"https://github.com/o/r/pull/3#issuecomment-9"}"#),
        ),
    ])
}

const ORIGIN: &str = "https://github.com/o/r.git";

fn comment(text: &str) -> Comment<'_> {
    Comment {
        pr: 3,
        marker: "<!-- stewards:next -->",
        title: "### t",
        text,
    }
}

fn post(fake: &Fake, text: &str) -> PrComment {
    post_comment(fake, Forge::GitHub, ORIGIN, &comment(text), None).unwrap()
}

fn last_call(fake: &Fake) -> (String, Value) {
    let (line, stdin) = fake.calls.borrow().last().cloned().unwrap();
    (line, serde_json::from_str(&stdin.unwrap()).unwrap())
}

fn body_of(fake: &Fake) -> String {
    last_call(fake).1["body"].as_str().unwrap().to_string()
}

#[test]
fn creates_a_comment_when_none_is_its_own_and_never_edits_another_users() {
    let others = json!([
        {"id": 1, "body": "<!-- stewards:next -->\nhers", "user": {"login": "someone"}},
        {"id": 2, "body": "mine, but no marker", "user": {"login": "me"}},
    ]);
    let fake = gh_fake(others);
    let c = post(&fake, "jobs");
    assert!(!c.updated);
    assert_eq!(c.url, "https://github.com/o/r/pull/3#issuecomment-9");
    let (line, _) = last_call(&fake);
    assert!(
        line.starts_with("gh api -X POST repos/o/r/issues/3/comments"),
        "{line}"
    );
}

#[test]
fn every_call_names_the_origins_repo_and_host() {
    let fake = Fake::new(vec![
        ("auth status", Out::ok("")),
        ("api user", Out::ok(r#"{"login":"me"}"#)),
        ("--paginate", Out::ok("[]")),
        ("-X", Out::ok(r#"{"html_url":"u"}"#)),
    ]);
    post_comment(
        &fake,
        Forge::GitHub,
        "git@ghe.corp.example:own-er/re.po_1.git",
        &comment("x"),
        None,
    )
    .unwrap();
    let calls = fake.calls.borrow();
    assert_eq!(calls.len(), 4);
    for (line, _) in calls.iter() {
        assert!(line.ends_with("--hostname ghe.corp.example"), "{line}");
        assert!(!line.contains("{owner}") && !line.contains(":id"), "{line}");
    }
    assert!(
        calls[3]
            .0
            .contains("repos/own-er/re.po_1/issues/3/comments")
    );
}

#[test]
fn an_origin_path_that_is_not_a_plain_owner_and_repo_is_usage() {
    for url in [
        "https://github.com/-x/r.git",
        "https://github.com/a/b/c.git",
        "https://github.com/a/..",
        "https://github.com/a%2Fb/r",
        "https://github.com/a",
    ] {
        let fake = gh_fake(json!([]));
        let e = post_comment(&fake, Forge::GitHub, url, &comment("x"), None).unwrap_err();
        assert_eq!(code(&e), "usage", "{url}");
        assert!(fake.calls.borrow().is_empty(), "{url}");
    }
}

#[test]
fn edits_its_own_comment_in_place_across_pages() {
    // two pages back to back, as `--paginate` prints them
    let pages = format!(
        "{}{}",
        json!([{"id": 1, "body": "x", "user": {"login": "me"}}]),
        json!([{"id": 5, "body": "<!-- stewards:next -->\nold", "user": {"login": "me"}}])
    );
    let fake = Fake::new(vec![
        ("auth status", Out::ok("")),
        ("api user", Out::ok(r#"{"login":"me"}"#)),
        ("--paginate", Out::ok(&pages)),
        ("-X", Out::ok(r#"{"html_url":"u"}"#)),
    ]);
    let c = post(&fake, "jobs");
    assert!(c.updated);
    let (line, _) = last_call(&fake);
    assert!(
        line.starts_with("gh api -X PATCH repos/o/r/issues/comments/5"),
        "{line}"
    );
}

#[test]
fn card_text_stays_inside_a_fence_longer_than_its_backtick_runs() {
    let fake = gh_fake(json!([]));
    let text = "see `x` and ```` four, @user ping, [x](http://evil.example)";
    post(&fake, text);
    let body = body_of(&fake);
    let mut lines = body.lines();
    assert_eq!(lines.next(), Some("<!-- stewards:next -->"));
    assert_eq!(lines.next(), Some("### t"));
    // the longest run is four backticks, so the fence is five
    assert!(body.contains("\n`````text\n"), "{body}");
    assert!(body.contains(&format!(" {text}")));
    assert!(body.trim_end().ends_with("`````"));
}

#[test]
fn no_line_of_the_body_starts_a_quick_action() {
    let fake = gh_fake(json!([]));
    post(&fake, "```\n/merge\n/approve\n\n/close");
    let body = body_of(&fake);
    assert!(body.lines().all(|l| !l.starts_with('/')), "{body}");
    assert!(body.contains("\n /merge\n"), "{body}");
}

#[test]
fn invisible_characters_in_the_text_are_shown_not_posted() {
    let fake = gh_fake(json!([]));
    post(&fake, "a\u{202E}b\u{200B}c");
    let body = body_of(&fake);
    assert!(body.contains("a\\u{202E}b\\u{200B}c"), "{body}");
    assert!(!body.contains('\u{202E}') && !body.contains('\u{200B}'));
}

#[test]
fn the_body_stays_under_the_cap_whatever_the_backtick_runs() {
    for text in [
        format!("{}\n{}", "`".repeat(3000), "x".repeat(200_000)),
        "é".repeat(100_000),
        "`".repeat(70_000),
    ] {
        let fake = gh_fake(json!([]));
        post(&fake, &text);
        let body = body_of(&fake);
        assert!(body.len() < 65_536, "{}", body.len());
        assert!(body.trim_end().ends_with('`'));
    }
}

#[test]
fn not_logged_in_is_tracker_auth() {
    let fake = Fake::new(vec![("auth status", failing("You are not logged in"))]);
    let e = post_comment(&fake, Forge::GitHub, ORIGIN, &comment("x"), None).unwrap_err();
    assert_eq!(code(&e), "tracker_auth");
    assert_eq!(fake.calls.borrow().len(), 1);
}

#[test]
fn under_actions_a_refused_user_call_means_the_bot_author() {
    let comments = json!([
        {"id": 8, "body": "<!-- stewards:next -->\nold", "user": {"login": "github-actions[bot]"}},
        {"id": 9, "body": "<!-- stewards:next -->\nhers", "user": {"login": "someone"}},
    ]);
    let rules = |c: &Value| {
        vec![
            ("auth status", Out::ok("")),
            ("api user", failing("HTTP 403")),
            ("--paginate", Out::ok(&c.to_string())),
            ("-X", Out::ok(r#"{"html_url":"u"}"#)),
        ]
    };
    let fake = Fake::new(rules(&comments));
    let c = post_comment(&fake, Forge::GitHub, ORIGIN, &comment("x"), Some("true")).unwrap();
    assert!(c.updated);
    assert!(last_call(&fake).0.contains("comments/8"));
    // only under Actions
    let fake = Fake::new(rules(&comments));
    assert!(post_comment(&fake, Forge::GitHub, ORIGIN, &comment("x"), None).is_err());
    let fake = Fake::new(rules(&comments));
    assert!(post_comment(&fake, Forge::GitHub, ORIGIN, &comment("x"), Some("false")).is_err());
}

#[test]
fn gitlab_uses_glab_merge_request_notes_of_the_origins_project() {
    let fake = Fake::new(vec![
        ("auth status", Out::ok("")),
        ("api user", Out::ok(r#"{"username":"me"}"#)),
        (
            "--paginate",
            Out::ok(r#"[{"id":4,"body":"<!-- stewards:next -->","author":{"username":"me"}}]"#),
        ),
        ("-X", Out::ok(r#"{"id":4}"#)),
    ]);
    let mut c = comment("x");
    c.pr = 7;
    let done = post_comment(
        &fake,
        Forge::GitLab,
        "https://u:tok@gitlab.corp.example:8443/g/sub/p.git",
        &c,
        None,
    )
    .unwrap();
    assert!(done.updated);
    assert_eq!(
        done.url,
        "https://gitlab.corp.example:8443/g/sub/p/-/merge_requests/7#note_4"
    );
    let (line, _) = last_call(&fake);
    assert!(
        line.starts_with("glab api -X PUT projects/g%2Fsub%2Fp/merge_requests/7/notes/4"),
        "{line}"
    );
    assert!(
        line.ends_with("--hostname gitlab.corp.example:8443"),
        "{line}"
    );
    assert!(!line.contains("tok"));
}
