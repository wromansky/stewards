// The forge seam: the remote host picks `gh` or `glab`, and every call goes through the CLI so its
// login and token are the ones used. `next --pr`, `review --pr`, `init --ci` (and task 09's issue
// export) share it. Nothing here reads a PR title or body; of the comments only `id`, `body`,
// and the author's login are read, to find stewards' own.
use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::check::{escape_invisible, one_line};
use crate::git;
use crate::output::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
}

/// An `origin` URL taken apart: https, `ssh://`, or scp-style. Userinfo is dropped here and
/// never printed; a port is kept only for http(s), where it is the web port.
struct Origin {
    host: String,
    port: Option<String>,
    path: String,
}

impl Origin {
    fn parse(url: &str) -> Origin {
        let (http, authority, path) = match url.split_once("://") {
            Some((scheme, rest)) => {
                let (a, p) = rest.split_once('/').unwrap_or((rest, ""));
                (scheme.starts_with("http"), a, p)
            }
            None => {
                let (a, p) = url.split_once([':', '/']).unwrap_or((url, ""));
                (false, a, p)
            }
        };
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let (host, port) = match authority.split_once(':') {
            Some((h, p)) if http => (h, Some(p.to_string())),
            Some((h, _)) => (h, None),
            None => (authority, None),
        };
        let path = path.trim_matches('/');
        Origin {
            host: host.to_ascii_lowercase(),
            port,
            path: path.strip_suffix(".git").unwrap_or(path).to_string(),
        }
    }

    /// `host` or `host:port`, for `--hostname` and web URLs.
    fn hostname(&self) -> String {
        match &self.port {
            Some(p) => format!("{}:{p}", self.host),
            None => self.host.clone(),
        }
    }
}

impl Forge {
    /// The forge of an `origin` URL: exactly `github.com` or `gitlab.com`; any other host only
    /// when `gh auth status --hostname <host>` (else `glab`) succeeds. Otherwise `usage`,
    /// naming the host and path only, never the userinfo.
    pub fn of_origin(cli: &dyn Cli, origin: &str) -> Result<Forge> {
        let o = Origin::parse(origin);
        match o.host.as_str() {
            "github.com" => return Ok(Forge::GitHub),
            "gitlab.com" => return Ok(Forge::GitLab),
            _ => {}
        }
        let host = o.hostname();
        for forge in [Forge::GitHub, Forge::GitLab] {
            let argv = [forge.bin(), "auth", "status", "--hostname", &host];
            if cli.run(&argv, None).is_ok_and(|r| r.ok) {
                return Ok(forge);
            }
        }
        bail!(CliError::new(
            "usage",
            format!(
                "origin `{}/{}` is neither github.com, gitlab.com, nor a host `gh` or `glab` is logged in to",
                one_line(&host),
                one_line(&o.path)
            ),
            "run `gh auth login --hostname <host>` or `glab auth login --hostname <host>`, or set origin to a supported host",
        ))
    }

    pub(crate) fn bin(self) -> &'static str {
        match self {
            Forge::GitHub => "gh",
            Forge::GitLab => "glab",
        }
    }
}

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(stdout: &str) -> Out {
        Out {
            ok: true,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }
}

/// Runs a forge CLI: `argv[0]` is the binary, no shell.
pub trait Cli {
    fn run(&self, argv: &[&str], stdin: Option<&str>) -> Result<Out>;
}

/// The real CLI: repo root as cwd, the environment inherited so `GH_TOKEN`/`GITLAB_TOKEN` work.
pub struct Real {
    pub root: PathBuf,
}

impl Cli for Real {
    fn run(&self, argv: &[&str], stdin: Option<&str>) -> Result<Out> {
        let mut child = Command::new(argv[0])
            .args(&argv[1..])
            .current_dir(&self.root)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                CliError::new(
                    "tracker_auth",
                    format!("cannot run `{}`: {e}", argv[0]),
                    login_fix(argv[0]),
                )
            })?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // the CLI reads its stdin before it prints, so writing first cannot deadlock
            pipe.write_all(text.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        Ok(Out {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

pub(crate) fn login_fix(bin: &str) -> String {
    format!("install `{bin}` and run `{bin} auth login`, or set its token variable")
}

/// A recorded-response CLI for tests: the response of the first rule whose key occurs in the
/// space-joined argv; every call is kept in `calls` as `(argv joined, stdin)`.
pub struct Fake {
    rules: Vec<(String, Out)>,
    pub calls: RefCell<Vec<(String, Option<String>)>>,
}

impl Fake {
    pub fn new(rules: Vec<(&str, Out)>) -> Fake {
        Fake {
            rules: rules.into_iter().map(|(k, o)| (k.to_string(), o)).collect(),
            calls: RefCell::new(vec![]),
        }
    }
}

impl Cli for Fake {
    fn run(&self, argv: &[&str], stdin: Option<&str>) -> Result<Out> {
        let line = argv.join(" ");
        self.calls
            .borrow_mut()
            .push((line.clone(), stdin.map(String::from)));
        let rule = self.rules.iter().find(|(k, _)| line.contains(k.as_str()));
        let Some((_, out)) = rule else {
            bail!("fake CLI: no response recorded for `{line}`")
        };
        Ok(Out {
            ok: out.ok,
            stdout: out.stdout.clone(),
            stderr: out.stderr.clone(),
        })
    }
}

/// A comment this call created or edited.
#[derive(Debug, Clone, Serialize)]
pub struct PrComment {
    pub url: String,
    pub updated: bool,
}

/// What `post_comment` posts and where.
pub struct Comment<'a> {
    pub pr: u64,
    pub marker: &'a str,
    pub title: &'a str,
    pub text: &'a str,
}

/// Bytes the whole comment body stays under (GitHub's cap is 65536).
const BODY_BYTES: usize = 65_000;

pub(crate) fn api(
    cli: &dyn Cli,
    forge: Forge,
    host: &str,
    args: &[&str],
    stdin: Option<&str>,
) -> Result<String> {
    let mut argv = vec![forge.bin(), "api"];
    argv.extend(args);
    // the repo and host come from origin, not from the CLI's default remote
    argv.extend(["--hostname", host]);
    let out = cli.run(&argv, stdin)?;
    if !out.ok {
        let why: String = one_line(&out.stderr).chars().take(300).collect();
        bail!("`{} api` failed: {why}", forge.bin());
    }
    Ok(out.stdout)
}

/// `[A-Za-z0-9._-]+`, not starting with `-`, not `.` or `..`.
pub(crate) fn plain_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// `text` with invisible characters shown as `\u{XXXX}` and each line indented by one space, so
/// no line starts a GitLab quick action; for a fence.
pub(crate) fn indented(text: &str) -> String {
    escape_invisible(text)
        .replace('\r', " ")
        .lines()
        .map(|l| format!(" {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A GitLab project path as the API's URL-encoded project id.
pub(crate) fn gitlab_id(path: &str) -> String {
    path.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// The `host` or `host:port` and the repo path of an `origin` URL, without its userinfo.
pub(crate) fn host_and_path(origin: &str) -> (String, String) {
    let o = Origin::parse(origin);
    (o.hostname(), o.path)
}

/// The comment body: `marker`, `title`, then `text` in a code fence longer than any backtick run
/// in it, each line indented by one space (GitLab runs a line starting with `/` as a quick
/// action, even inside a fence), invisible characters shown as `\u{XXXX}`, cut so the whole body stays under `BODY_BYTES`.
fn body(marker: &str, title: &str, text: &str) -> String {
    let indented = indented(text);
    let longest = indented
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = longest.max(2) + 1;
    let room = BODY_BYTES.saturating_sub(marker.len() + title.len() + 2 * fence + 16);
    let mut end = indented.len().min(room);
    while !indented.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{marker}\n{title}\n\n{}",
        crate::pack::fenced("text", &indented[..end])
    )
}

/// Posts `c` as the one comment of its marker on PR/MR `c.pr`, edited in place when the
/// logged-in user already has one; the comment is found by author and marker, nothing else of
/// any comment is read. `origin` names the repo and host. `github_actions` is the value of the
/// `GITHUB_ACTIONS` variable: under `GITHUB_TOKEN`, `gh api user` is refused and the author is
/// `github-actions[bot]`.
pub fn post_comment(
    cli: &dyn Cli,
    forge: Forge,
    origin: &str,
    c: &Comment,
    github_actions: Option<&str>,
) -> Result<PrComment> {
    let o = Origin::parse(origin);
    let host = o.hostname();
    let pr = c.pr;
    let (user_key, author_key, list, edit) = match forge {
        Forge::GitHub => {
            let ok = o
                .path
                .split_once('/')
                .is_some_and(|(owner, repo)| plain_name(owner) && plain_name(repo));
            if !ok {
                bail!(CliError::new(
                    "usage",
                    format!("origin path `{}` is not <owner>/<repo>", one_line(&o.path)),
                    "set origin to https://github.com/<owner>/<repo>.git",
                ));
            }
            (
                "login",
                "user",
                format!("repos/{}/issues/{pr}/comments", o.path),
                format!("repos/{}/issues/comments/", o.path),
            )
        }
        Forge::GitLab => {
            let enc = gitlab_id(&o.path);
            (
                "username",
                "author",
                format!("projects/{enc}/merge_requests/{pr}/notes"),
                format!("projects/{enc}/merge_requests/{pr}/notes/"),
            )
        }
    };
    let auth = cli.run(&[forge.bin(), "auth", "status", "--hostname", &host], None)?;
    if !auth.ok {
        bail!(CliError::new(
            "tracker_auth",
            format!("`{} auth status` reports no login", forge.bin()),
            login_fix(forge.bin()),
        ));
    }
    let me = match api(cli, forge, &host, &["user"], None) {
        Ok(u) => {
            let u: Value = serde_json::from_str(&u)?;
            match u[user_key].as_str() {
                Some(m) => m.to_string(),
                None => bail!("`{} api user` named no login", forge.bin()),
            }
        }
        Err(_) if forge == Forge::GitHub && github_actions == Some("true") => {
            "github-actions[bot]".to_string()
        }
        Err(e) => return Err(e),
    };
    // `--paginate` prints one JSON array per page, back to back
    let listing = api(cli, forge, &host, &["--paginate", &list], None)?;
    let mine = serde_json::Deserializer::from_str(&listing)
        .into_iter::<Vec<Value>>()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .find(|n| {
            n[author_key][user_key].as_str() == Some(me.as_str())
                && n["body"].as_str().is_some_and(|b| b.starts_with(c.marker))
        })
        .and_then(|n| n["id"].as_u64());

    let payload = json!({ "body": body(c.marker, c.title, c.text) }).to_string();
    let (method, path) = match mine {
        Some(id) => (
            if forge == Forge::GitHub {
                "PATCH"
            } else {
                "PUT"
            },
            format!("{edit}{id}"),
        ),
        None => ("POST", list),
    };
    let reply = api(
        cli,
        forge,
        &host,
        &[
            "-X",
            method,
            &path,
            "--input",
            "-",
            "-H",
            "Content-Type: application/json",
        ],
        Some(&payload),
    )?;
    let reply: Value = serde_json::from_str(&reply)?;
    let url = match forge {
        Forge::GitHub => reply["html_url"].as_str().unwrap_or_default().to_string(),
        Forge::GitLab => format!(
            "https://{host}/{}/-/merge_requests/{pr}#note_{}",
            o.path, reply["id"]
        ),
    };
    Ok(PrComment {
        url,
        updated: mine.is_some(),
    })
}

/// `post_comment` through the real CLI for the forge of the repo's `origin`.
pub fn post(root: &Path, pr: u64, marker: &str, title: &str, text: &str) -> Result<PrComment> {
    let Some(origin) = git::origin_url(root) else {
        bail!(CliError::new(
            "usage",
            "the repo has no `origin` remote to pick `gh` or `glab` from",
            "add an origin remote on GitHub or GitLab",
        ));
    };
    let cli = Real {
        root: root.to_path_buf(),
    };
    let forge = Forge::of_origin(&cli, &origin)?;
    let comment = Comment {
        pr,
        marker,
        title,
        text,
    };
    let actions = std::env::var("GITHUB_ACTIONS").ok();
    post_comment(&cli, forge, &origin, &comment, actions.as_deref())
}

/// What a failed post reports in `pr_comment_error`: one line, cut as `api` cuts a CLI error.
pub fn error_line(e: &anyhow::Error) -> String {
    one_line(&e.to_string()).chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_shapes() {
        for (url, want, port) in [
            ("https://github.com/a/b.git", ("github.com", "a/b"), None),
            ("git@gitlab.com:a/b.git", ("gitlab.com", "a/b"), None),
            ("ssh://git@git.corp:2222/a/b", ("git.corp", "a/b"), None),
            (
                "https://u:p@git.corp:8443/a/b",
                ("git.corp", "a/b"),
                Some("8443"),
            ),
        ] {
            let o = Origin::parse(url);
            assert_eq!((o.host.as_str(), o.path.as_str()), want, "{url}");
            assert_eq!(o.port.as_deref(), port, "{url}");
        }
    }
}
