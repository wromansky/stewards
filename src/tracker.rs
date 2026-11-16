// `validate --export`: each judged bug filed once as an issue on the repo's `[tracker]`, its URL
// written to the gap at once. Only text the tool composes is sent; tokens come from the CLIs and
// the environment and never reach output, an error, or a file.
use std::cell::RefCell;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::check::{escape_invisible, one_line};
use crate::forge::{self, Cli, Forge};
use crate::model::{GapKind, Tracker, TrackerKind, UnitCard};
use crate::output::{CliError, Human, with_note};
use crate::submit::{self, ID_CAP, LINE_CAP};
use crate::workspace::Repo;

const LINEAR_URL: &str = "https://api.linear.app/graphql";
/// Characters of one repro, note, or anchor sent; the card caps them lower.
const TEXT_SENT: usize = 4000;
const UNJUDGED: &str = "Bugs without a severity are not exported: run stewards review --roles to judge them, then stewards validate --export again.";

/// Creates one issue and returns its web URL.
pub trait Issues {
    fn create_issue(&self, title: &str, body: &str) -> Result<String>;
    /// What every issue URL of this tracker starts with; a returned URL that does not is refused.
    fn issue_prefix(&self) -> String;
}

/// One HTTP POST: `(status, body)`.
pub trait Transport {
    fn post(&self, url: &str, headers: &[(&str, &str)], json: &str) -> Result<(u16, String)>;
}

/// `ureq`: https only, 30 s, no redirect followed (so no Authorization header reaches another host).
pub struct Http;

impl Transport for Http {
    fn post(&self, url: &str, headers: &[(&str, &str)], json: &str) -> Result<(u16, String)> {
        if !url.starts_with("https://") {
            bail!("refusing a URL that is not https");
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .redirects(0)
            .https_only(true)
            .build();
        let mut req = agent.post(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        match req.send_string(json) {
            Ok(r) => Ok((r.status(), r.into_string()?)),
            Err(ureq::Error::Status(code, r)) => Ok((code, r.into_string().unwrap_or_default())),
            Err(ureq::Error::Transport(t)) => {
                // a bad header's error quotes the header line, token included
                let mut text = t.to_string();
                for (_, v) in headers {
                    text = text.replace(v, "[redacted]");
                }
                bail!("{text}")
            }
        }
    }
}

/// A request as the fake transport saw it.
#[derive(Debug, Clone)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Recorded responses for tests, returned in order; every request is kept in `calls`.
pub struct FakeHttp {
    responses: RefCell<Vec<(u16, String)>>,
    pub calls: RefCell<Vec<Request>>,
}

impl FakeHttp {
    pub fn new(responses: Vec<(u16, &str)>) -> FakeHttp {
        FakeHttp {
            responses: RefCell::new(
                responses
                    .into_iter()
                    .rev()
                    .map(|(s, b)| (s, b.to_string()))
                    .collect(),
            ),
            calls: RefCell::new(vec![]),
        }
    }
}

impl Transport for FakeHttp {
    fn post(&self, url: &str, headers: &[(&str, &str)], json: &str) -> Result<(u16, String)> {
        self.calls.borrow_mut().push(Request {
            url: url.into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: json.into(),
        });
        match self.responses.borrow_mut().pop() {
            Some(r) => Ok(r),
            None => bail!("fake transport: no response recorded"),
        }
    }
}

fn kind_name(k: TrackerKind) -> &'static str {
    match k {
        TrackerKind::Github => "github",
        TrackerKind::Gitlab => "gitlab",
        TrackerKind::Jira => "jira",
        TrackerKind::Linear => "linear",
    }
}

const FAILED_FIX: &str = "check [tracker] project and the tracker, then run stewards validate --export again; filed issues are recorded on the cards";

/// One line of untrusted text as shown: invisible characters escaped, at most 300 characters.
fn shown(text: &str) -> String {
    one_line(&escape_invisible(text))
        .chars()
        .take(LINE_CAP)
        .collect()
}

/// `tracker_failed`: the kind, the status, and the tracker's text on one escaped line of at most
/// 300 characters, with each of `secrets` blanked out of it (longest first).
fn failed(kind: &str, status: Option<u16>, text: &str, secrets: &[&str]) -> anyhow::Error {
    let mut text = text.to_string();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        text = text.replace(s, "[redacted]");
    }
    let text = shown(&text);
    let status = status.map(|s| format!(" HTTP {s}:")).unwrap_or_default();
    CliError::new(
        "tracker_failed",
        format!("{kind}:{status} {text}"),
        FAILED_FIX,
    )
    .into()
}

fn auth(message: impl Into<String>, fix: &str) -> anyhow::Error {
    CliError::new("tracker_auth", message, fix).into()
}

/// The body of a 2xx response; 401/403 → `tracker_auth`, anything else → `tracker_failed`.
fn checked(kind: TrackerKind, (status, body): (u16, String), secrets: &[&str]) -> Result<String> {
    let name = kind_name(kind);
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err(auth(
            format!("{name} refused the credentials (HTTP {status})"),
            match kind {
                TrackerKind::Jira => {
                    "set `JIRA_TOKEN` (and `JIRA_EMAIL` for Jira Cloud) to a token that may create issues"
                }
                _ => "set `LINEAR_API_KEY` to a key that may create issues",
            },
        )),
        _ => Err(failed(name, Some(status), &body, secrets)),
    }
}

/// GitHub or GitLab through the forge CLI's `api`: the body goes as JSON on stdin.
struct Forged<'a> {
    cli: &'a dyn Cli,
    forge: Forge,
    host: String,
    project: String,
}

impl Issues for Forged<'_> {
    fn create_issue(&self, title: &str, body: &str) -> Result<String> {
        let (path, payload, key) = match self.forge {
            Forge::GitHub => (
                format!("repos/{}/issues", self.project),
                json!({"title": title, "body": body}),
                "html_url",
            ),
            Forge::GitLab => (
                format!("projects/{}/issues", forge::gitlab_id(&self.project)),
                json!({"title": title, "description": body}),
                "web_url",
            ),
        };
        let args = [
            "-X",
            "POST",
            &path,
            "--input",
            "-",
            "-H",
            "Content-Type: application/json",
        ];
        let reply = forge::api(
            self.cli,
            self.forge,
            &self.host,
            &args,
            Some(&payload.to_string()),
        );
        let name = if self.forge == Forge::GitHub {
            "github"
        } else {
            "gitlab"
        };
        let reply = reply.map_err(|e| {
            let text = e.to_string();
            if text.contains("HTTP 401") || text.contains("HTTP 403") {
                auth(
                    format!(
                        "`{} api` was refused: {}",
                        self.forge.bin(),
                        one_line(&text)
                    ),
                    &forge::login_fix(self.forge.bin()),
                )
            } else {
                failed(name, None, &text, &[])
            }
        })?;
        let v: Value = serde_json::from_str(&reply)
            .map_err(|_| failed(name, None, "the reply is not JSON", &[]))?;
        match v[key].as_str() {
            Some(u) => Ok(u.to_string()),
            None => Err(failed(name, None, &format!("the reply has no {key}"), &[])),
        }
    }

    fn issue_prefix(&self) -> String {
        match self.forge {
            Forge::GitHub => format!("https://{}/{}/issues/", self.host, self.project),
            Forge::GitLab => format!("https://{}/{}/-/issues/", self.host, self.project),
        }
    }
}

struct Jira<'a> {
    http: &'a dyn Transport,
    base: String,
    authorization: String,
    /// The whole Authorization value, its credential part, and the token: blanked out of errors.
    secrets: Vec<String>,
    project: String,
}

impl Jira<'_> {
    fn secrets(&self) -> Vec<&str> {
        self.secrets.iter().map(String::as_str).collect()
    }
}

impl Issues for Jira<'_> {
    fn create_issue(&self, title: &str, body: &str) -> Result<String> {
        let payload = json!({"fields": {
            "project": {"key": self.project},
            "summary": title,
            "description": body,
            "issuetype": {"name": "Bug"},
        }});
        let reply = self.http.post(
            &format!("{}/rest/api/2/issue", self.base),
            &[
                ("Authorization", &self.authorization),
                ("Content-Type", "application/json"),
                ("Accept", "application/json"),
            ],
            &payload.to_string(),
        );
        let reply = reply.map_err(|e| failed("jira", None, &e.to_string(), &self.secrets()))?;
        let reply = checked(TrackerKind::Jira, reply, &self.secrets())?;
        let v: Value = serde_json::from_str(&reply).unwrap_or_default();
        match v["key"].as_str().filter(|k| forge::plain_name(k)) {
            Some(k) => Ok(format!("{}/browse/{k}", self.base)),
            None => Err(failed("jira", None, "the reply has no issue key", &[])),
        }
    }

    fn issue_prefix(&self) -> String {
        format!("{}/browse/", self.base)
    }
}

struct Linear<'a> {
    http: &'a dyn Transport,
    key: String,
    team: String,
}

impl Linear<'_> {
    /// One GraphQL call: values only as variables; a GraphQL error → `tracker_failed`.
    fn call(&self, query: &str, variables: Value) -> Result<Value> {
        let reply = self.http.post(
            LINEAR_URL,
            &[
                ("Authorization", &self.key),
                ("Content-Type", "application/json"),
            ],
            &json!({"query": query, "variables": variables}).to_string(),
        );
        let reply = reply.map_err(|e| failed("linear", None, &e.to_string(), &[&self.key]))?;
        let reply = checked(TrackerKind::Linear, reply, &[&self.key])?;
        let v: Value = serde_json::from_str(&reply)
            .map_err(|_| failed("linear", None, "the reply is not JSON", &[]))?;
        if let Some(e) = v["errors"].get(0) {
            let text = e["message"].as_str().unwrap_or("GraphQL error");
            return Err(failed("linear", None, text, &[&self.key]));
        }
        Ok(v["data"].clone())
    }
}

impl Issues for Linear<'_> {
    fn create_issue(&self, title: &str, body: &str) -> Result<String> {
        // simplified: the team is looked up once per issue; cache it if exports grow large
        let teams = self.call(
            "query($key: String!) { teams(filter: {key: {eq: $key}}) { nodes { id } } }",
            json!({"key": self.team}),
        )?;
        let Some(team) = teams["teams"]["nodes"][0]["id"].as_str() else {
            return Err(failed(
                "linear",
                None,
                &format!("no team with key `{}`", self.team),
                &[],
            ));
        };
        let made = self.call(
            "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { url } } }",
            json!({"input": {"teamId": team, "title": title, "description": body}}),
        )?;
        match made["issueCreate"]["issue"]["url"].as_str() {
            Some(u) => Ok(u.to_string()),
            None => Err(failed("linear", None, "the reply has no issue url", &[])),
        }
    }

    fn issue_prefix(&self) -> String {
        "https://linear.app/".into()
    }
}

/// Standard base64, for Jira's Basic auth.
fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The backend for `t`. `env` reads a variable (blank counts as unset); `origin` is the repo's
/// `origin` URL, whose host is used when it is the tracker's forge.
pub fn backend<'a>(
    t: &Tracker,
    env: &dyn Fn(&str) -> Option<String>,
    cli: &'a dyn Cli,
    origin: Option<&str>,
    http: &'a dyn Transport,
) -> Result<Box<dyn Issues + 'a>> {
    let var = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    let bad_project = |shape: &str| -> anyhow::Error {
        CliError::new(
            "usage",
            format!(
                "[tracker] project `{}` is not {shape}",
                one_line(&escape_invisible(&t.project))
            ),
            format!("set [tracker] project in .stewards/repo.toml to {shape}"),
        )
        .into()
    };
    let parts: Vec<&str> = t.project.split('/').collect();
    let plain = parts.iter().all(|p| forge::plain_name(p));
    Ok(match t.kind {
        TrackerKind::Github | TrackerKind::Gitlab => {
            let (forge, default, shape, ok) = if t.kind == TrackerKind::Github {
                (
                    Forge::GitHub,
                    "github.com",
                    "<owner>/<repo>",
                    parts.len() == 2,
                )
            } else {
                (
                    Forge::GitLab,
                    "gitlab.com",
                    "<group>/<project>",
                    parts.len() >= 2,
                )
            };
            if !(ok && plain) {
                return Err(bad_project(shape));
            }
            // origin's host only when origin is this very project on this forge
            let host = match origin.map(|o| (o, forge::host_and_path(o))) {
                Some((o, (host, path)))
                    if path == t.project && Forge::of_origin(cli, o).ok() == Some(forge) =>
                {
                    host
                }
                _ => default.to_string(),
            };
            let status = cli.run(&[forge.bin(), "auth", "status", "--hostname", &host], None)?;
            if !status.ok {
                return Err(auth(
                    format!("`{} auth status` reports no login for {host}", forge.bin()),
                    &forge::login_fix(forge.bin()),
                ));
            }
            Box::new(Forged {
                cli,
                forge,
                host,
                project: t.project.clone(),
            })
        }
        TrackerKind::Jira => {
            if !(parts.len() == 1 && plain) {
                return Err(bad_project("a Jira project key"));
            }
            let fix =
                "set `JIRA_URL` (https://<site>) and `JIRA_TOKEN`, and `JIRA_EMAIL` for Jira Cloud";
            let (Some(url), Some(token)) = (var("JIRA_URL"), var("JIRA_TOKEN")) else {
                return Err(auth("`JIRA_URL` or `JIRA_TOKEN` is not set", fix));
            };
            let base = url.trim().trim_end_matches('/').to_string();
            let authority = base
                .strip_prefix("https://")
                .map(|r| r.split('/').next().unwrap_or(""));
            let ok = authority.is_some_and(|a| !a.is_empty() && !a.contains('@'))
                && !base
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || "?#\\".contains(c));
            if !ok {
                return Err(auth(
                    "`JIRA_URL` is not an https URL without user info",
                    fix,
                ));
            }
            let (scheme, credential) = match var("JIRA_EMAIL") {
                Some(email) => ("Basic", base64(format!("{email}:{token}").as_bytes())),
                None => ("Bearer", token.clone()),
            };
            let authorization = format!("{scheme} {credential}");
            Box::new(Jira {
                http,
                base,
                secrets: vec![authorization.clone(), credential, token],
                authorization,
                project: t.project.clone(),
            })
        }
        TrackerKind::Linear => {
            if !(parts.len() == 1 && plain) {
                return Err(bad_project("a Linear team key"));
            }
            let Some(key) = var("LINEAR_API_KEY") else {
                return Err(auth(
                    "`LINEAR_API_KEY` is not set",
                    "set `LINEAR_API_KEY` to a Linear API key",
                ));
            };
            Box::new(Linear {
                http,
                key,
                team: t.project.clone(),
            })
        }
    })
}

/// `text` as sent: invisible characters shown, cut to `TEXT_SENT` characters.
fn sent(text: &str) -> String {
    escape_invisible(text).chars().take(TEXT_SENT).collect()
}

/// `text` with every `{noformat...}` (any case, any parameters up to the next `}`, or to the end)
/// removed, until none is left: a removal can join the halves of another.
fn without_noformat(text: &str) -> String {
    let mut s = text.to_string();
    while let Some(at) = s.to_ascii_lowercase().find("{noformat") {
        let end = s[at..].find('}').map_or(s.len(), |i| at + i + 1);
        s.replace_range(at..end, "");
    }
    s
}

/// A severity as the card holds it, or `unrecognized` for anything that is not a plain word.
fn severity_word(s: &str) -> &str {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase()) {
        s
    } else {
        "unrecognized"
    }
}

/// The issue body: markdown with each untrusted text in an indented fence, or for Jira plain
/// text with `{noformat}` blocks.
pub fn body(kind: TrackerKind, unit: &str, severity: &str, gap: &crate::model::Gap) -> String {
    let parts = [
        ("Anchor", gap.anchor.as_deref().unwrap_or("(none)")),
        ("Repro", gap.repro.as_deref().unwrap_or("(none)")),
        ("Note", gap.note.as_str()),
    ];
    let mut out = format!("Unit {unit}, severity {}.\n\n", severity_word(severity));
    for (label, text) in parts {
        let text = sent(text);
        if kind == TrackerKind::Jira {
            out += &format!(
                "{label}:\n{{noformat}}\n{}\n{{noformat}}\n\n",
                without_noformat(&text)
            );
        } else {
            out += &format!(
                "{label}:\n{}\n",
                crate::pack::fenced("text", &forge::indented(&text))
            );
        }
    }
    out + &format!("Filed by stewards from .stewards/units/{unit}.toml.")
}

/// The issue title: the gap title on one line, capped as the card caps it.
fn title(gap: &crate::model::Gap) -> String {
    one_line(&escape_invisible(&gap.title))
        .chars()
        .take(ID_CAP)
        .collect()
}

/// A URL the tracker returned, fit to store: printable ASCII with no space, at most 300
/// characters, under `prefix` (ASCII case aside) and longer than it. Else `tracker_failed`
/// saying an issue may exist, with the returned text shown escaped.
fn checked_url(kind: TrackerKind, prefix: &str, url: &str, rel: &str) -> Result<String> {
    let ok = url.len() > prefix.len()
        && url.len() <= LINE_CAP
        && url.bytes().all(|b| b.is_ascii_graphic())
        && url[..prefix.len()].eq_ignore_ascii_case(prefix);
    if !ok {
        return Err(CliError::new(
            "tracker_failed",
            format!(
                "{}: the tracker returned `{}`, not an issue URL under {prefix}; an issue may exist",
                kind_name(kind),
                shown(url)
            ),
            format!("look for the new issue in the tracker and set issue = \"<its URL>\" on that gap in {rel} by hand"),
        )
        .into());
    }
    Ok(url.to_string())
}

#[derive(Debug, Serialize)]
pub struct Filed {
    pub unit: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct Exported {
    pub exported: Vec<Filed>,
    pub then: String,
}

impl Human for Exported {
    fn render(&self) -> String {
        let mut out: Vec<String> = self
            .exported
            .iter()
            .map(|f| format!("Filed `{}` on {}: {}", shown(&f.title), f.unit, f.url))
            .collect();
        if out.is_empty() {
            out.push("Nothing to export.".into());
        }
        out.push(self.then.clone());
        out.join("\n")
    }
}

/// Files every bug with a severity and no issue, in units.toml then card order, writing each
/// URL to its gap as soon as the issue exists; stops at the first failure. `open` builds the
/// backend, only when there is something to file.
pub fn export<'a>(
    repo: &Repo,
    open: impl FnOnce(&Tracker) -> Result<Box<dyn Issues + 'a>>,
) -> Result<Exported> {
    let loaded = repo.loaded()?;
    let Some(tracker) = loaded.repo.tracker.clone() else {
        bail!(CliError::new(
            "usage",
            "the repo has no [tracker]",
            "add [tracker] kind and project to .stewards/repo.toml",
        ));
    };
    let mut todo = vec![];
    let mut unjudged = false;
    for u in &loaded.units {
        for g in loaded.card(&u.name).gaps {
            if g.kind != GapKind::Bug || g.issue.is_some() {
                continue;
            }
            match &g.severity {
                Some(s) => todo.push((u.name.clone(), s.clone(), g)),
                None => unjudged = true,
            }
        }
    }
    let then = if unjudged { UNJUDGED } else { "done" }.to_string();
    let mut exported = vec![];
    if todo.is_empty() {
        return Ok(Exported { exported, then });
    }
    let issues = open(&tracker)?;
    for (unit, severity, gap) in todo {
        let filed = file_one(repo, &*issues, tracker.kind, &unit, &severity, &gap);
        match filed {
            Ok(f) => exported.push(f),
            Err(e) => {
                let n = exported.len();
                return Err(with_note(
                    e,
                    format!("{n} issue(s) filed and recorded by this call before it stopped"),
                ));
            }
        }
    }
    Ok(Exported { exported, then })
}

fn file_one(
    repo: &Repo,
    issues: &dyn Issues,
    kind: TrackerKind,
    unit: &str,
    severity: &str,
    gap: &crate::model::Gap,
) -> Result<Filed> {
    let rel = format!(".stewards/units/{unit}.toml");
    let url = issues.create_issue(&title(gap), &body(kind, unit, severity, gap))?;
    let url = checked_url(kind, &issues.issue_prefix(), &url, &rel)?;
    record_issue(repo, &rel, gap, &url).map_err(|e| {
        let by_hand = format!(
            "issue {url} was filed but not recorded; set issue = \"{url}\" on that gap in {rel} by hand"
        );
        let (code, message) = match e.downcast_ref::<CliError>() {
            Some(c) => (c.code, c.message.clone()),
            None => ("internal", format!("{e:#}")),
        };
        anyhow::Error::from(CliError::new(code, format!("{message}; {by_hand}"), by_hand))
    })?;
    Ok(Filed {
        unit: unit.to_string(),
        title: gap.title.clone(),
        url,
    })
}

/// Writes `url` as the issue of the first judged open bug titled as `gap` on the card as it is
/// now, so a change made since the export started is kept.
fn record_issue(repo: &Repo, rel: &str, gap: &crate::model::Gap, url: &str) -> Result<()> {
    let before = submit::record(&repo.root, &[rel.to_string()])?;
    let lost = || {
        anyhow::Error::from(CliError::new(
            "card_changed",
            format!("the bug `{}` is no longer open on {rel}", shown(&gap.title)),
            "",
        ))
    };
    let Some(Some(bytes)) = before.first().map(|(_, b)| b.clone()) else {
        return Err(lost());
    };
    let mut card: UnitCard = toml::from_str(&String::from_utf8_lossy(&bytes))?;
    let open = card.gaps.iter_mut().find(|g| {
        g.kind == GapKind::Bug && g.title == gap.title && g.issue.is_none() && g.severity.is_some()
    });
    let Some(g) = open else {
        return Err(lost());
    };
    g.issue = Some(url.to_string());
    submit::check_unchanged(&repo.root, &before, &[rel])?;
    repo.write(rel, toml::to_string(&card)?.as_bytes())
}
