// Git CLI wrappers: ls-files, snapshot, ls-tree, diffs, head commit.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};

fn run_raw(root: &Path, args: &[&str], index: Option<&Path>) -> Result<Output> {
    let mut cmd = Command::new("git");
    // quotepath off so patch headers carry raw UTF-8 paths
    cmd.args(["-c", "core.quotepath=false"])
        .args(args)
        .current_dir(root);
    if let Some(index) = index {
        cmd.env("GIT_INDEX_FILE", index);
    }
    cmd.output().context("cannot run git")
}

fn run(root: &Path, args: &[&str], index: Option<&Path>) -> Result<String> {
    let out = run_raw(root, args, index)?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn nul_split(s: &str) -> Vec<String> {
    s.split('\0')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// Tracked files, as recorded in the index.
pub fn ls_files(root: &Path) -> Result<Vec<String>> {
    Ok(nul_split(&run(root, &["ls-files", "-z"], None)?))
}

/// Removes the temporary index when dropped.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Tree sha of the working tree: tracked, modified, and untracked-not-ignored files.
/// Works on a copy of the index; the real index, HEAD, and refs are untouched.
pub fn snapshot(root: &Path) -> Result<String> {
    let real = root.join(run(root, &["rev-parse", "--git-path", "index"], None)?.trim());
    // pid + counter: unique across processes and across threads of one process
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "stewards-index-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // create_new: never follow or reuse whatever already sits at the path
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("create {}", path.display()))?;
    let temp = TempIndex(path);
    match std::fs::File::open(&real) {
        Ok(mut index) => {
            std::io::copy(&mut index, &mut file)
                .with_context(|| format!("copy {}", real.display()))?;
            // Git re-reads a file whose mtime is not older than the index's own ("racily clean").
            // A fresh copy would look newer than every file and hide a same-size edit made in the
            // same second as the last `git add`, so the copy keeps the real index's mtime.
            file.set_modified(index.metadata()?.modified()?)
                .with_context(|| format!("set mtime of {}", temp.0.display()))?;
        }
        // no index yet: start from an empty one (git rejects a zero-byte index file)
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            run(root, &["read-tree", "--empty"], Some(&temp.0))?;
        }
        Err(e) => return Err(e).with_context(|| format!("copy {}", real.display())),
    }
    drop(file);
    run(root, &["add", "-A"], Some(&temp.0))?;
    Ok(run(root, &["write-tree"], Some(&temp.0))?
        .trim()
        .to_string())
}

/// `(path, blob sha)` for every file in `tree`.
pub fn ls_tree(root: &Path, tree: &str) -> Result<Vec<(String, String)>> {
    let out = run(
        root,
        &["ls-tree", "-r", "-z", "--end-of-options", tree],
        None,
    )?;
    nul_split(&out)
        .into_iter()
        .map(|entry| {
            // "<mode> <type> <sha>\t<path>"
            let (meta, path) = entry
                .split_once('\t')
                .with_context(|| format!("unexpected ls-tree entry {entry:?}"))?;
            let sha = meta
                .split(' ')
                .nth(2)
                .with_context(|| format!("unexpected ls-tree entry {entry:?}"))?;
            Ok((path.to_string(), sha.to_string()))
        })
        .collect()
}

/// True when `sha` resolves to a tree (or a commit's tree); a blob or an unknown name is false.
pub fn has_object(root: &Path, sha: &str) -> bool {
    !sha.starts_with('-')
        && run(root, &["cat-file", "-e", &format!("{sha}^{{tree}}")], None).is_ok()
}

/// True when `sha` names a tree object itself; a commit, a blob, or an unknown name is false.
pub fn is_tree(root: &Path, sha: &str) -> bool {
    run(root, &["cat-file", "-t", "--end-of-options", sha], None).is_ok_and(|t| t.trim() == "tree")
}

const DIFF: [&str; 4] = ["diff", "--no-renames", "--src-prefix=a/", "--dst-prefix=b/"];

pub fn diff_names(root: &Path, from: &str, to: &str) -> Result<Vec<String>> {
    let mut args = DIFF.to_vec();
    args.extend(["--name-only", "-z", "--end-of-options", from, to]);
    Ok(nul_split(&run(root, &args, None)?))
}

pub fn diff_file(root: &Path, from: &str, to: &str, path: &str) -> Result<String> {
    // literal: a tracked name like `*` must not match other files as a glob
    let path = format!(":(literal){path}");
    let mut args = DIFF.to_vec();
    args.extend([
        "--no-color",
        "--no-ext-diff",
        "-U3",
        "--end-of-options",
        from,
        to,
        "--",
        &path,
    ]);
    run(root, &args, None)
}

fn diff_u0(root: &Path, from: &str, to: &str) -> Result<String> {
    let mut args = DIFF.to_vec();
    args.extend([
        "--no-color",
        "--no-ext-diff",
        "-U0",
        "--end-of-options",
        from,
        to,
    ]);
    run(root, &args, None)
}

/// The new side `(start, count)` of a `@@ -a,b +c,d @@` header; a missing count is 1.
fn hunk_new_side(header: &str) -> (u32, u32) {
    let new = header
        .split(' ')
        .nth(2)
        .unwrap_or_default()
        .trim_start_matches('+');
    let (start, count) = new.split_once(',').unwrap_or((new, "1"));
    (start.parse().unwrap_or(0), count.parse().unwrap_or(0))
}

/// The path of a `+++ b/<path>` line; `None` for a C-quoted name (`+++ "b/..."`) or `/dev/null`.
fn new_path(line: &str) -> Option<String> {
    // git appends a tab to the line when the path contains a space.
    line.strip_prefix("+++ b/")
        .map(|p| p.trim_end_matches('\t').to_string())
}

/// Added lines as `(path, line number in the new file, text)`; a file whose name is C-quoted is skipped.
pub fn diff_added_lines(root: &Path, from: &str, to: &str) -> Result<Vec<(String, u32, String)>> {
    let text = diff_u0(root, from, to)?;
    let mut added = Vec::new();
    let mut path: Option<String> = None;
    let mut in_hunk = false;
    let mut line = 0;
    for l in text.lines() {
        if l.starts_with("diff --git ") {
            in_hunk = false;
            path = None;
        } else if !in_hunk && l.starts_with("+++ ") {
            path = new_path(l);
        } else if l.starts_with("@@ ") {
            in_hunk = true;
            line = hunk_new_side(l).0;
        } else if in_hunk && let (Some(p), Some(text)) = (&path, l.strip_prefix('+')) {
            added.push((p.clone(), line, text.to_string()));
            line += 1;
        }
    }
    Ok(added)
}

/// Each hunk's new side as `(path, start, count)`: count 0 is a pure deletion after line
/// `start`. A deleted file, a file whose name is C-quoted, and a binary file have none.
pub fn diff_hunks(root: &Path, from: &str, to: &str) -> Result<Vec<(String, u32, u32)>> {
    let text = diff_u0(root, from, to)?;
    let mut hunks = Vec::new();
    let mut path: Option<String> = None;
    let mut in_hunk = false;
    for l in text.lines() {
        if l.starts_with("diff --git ") {
            in_hunk = false;
            path = None;
        } else if !in_hunk && l.starts_with("+++ ") {
            path = new_path(l);
        } else if l.starts_with("@@ ") {
            in_hunk = true;
            if let Some(p) = &path {
                let (start, count) = hunk_new_side(l);
                hunks.push((p.clone(), start, count));
            }
        }
    }
    Ok(hunks)
}

/// The text of blob `sha`, invalid UTF-8 replaced.
pub fn blob_text(root: &Path, sha: &str) -> Result<String> {
    run(root, &["cat-file", "blob", "--end-of-options", sha], None)
}

/// HEAD's commit sha; `None` before the first commit, an error for any other failure.
pub fn head_commit(root: &Path) -> Result<Option<String>> {
    let head = run_raw(root, &["rev-parse", "--verify", "--quiet", "HEAD"], None)?;
    match head.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&head.stdout).trim().to_string(),
        )),
        // --quiet: exit 1 with no message means HEAD has no commit yet
        Some(1) => Ok(None),
        _ => bail!(
            "git rev-parse HEAD: {}",
            String::from_utf8_lossy(&head.stderr).trim()
        ),
    }
}

/// The URL of the `origin` remote; `None` when there is none.
pub fn origin_url(root: &Path) -> Option<String> {
    let url = run(root, &["remote", "get-url", "origin"], None).ok()?;
    Some(url.trim().to_string()).filter(|u| !u.is_empty())
}

/// Current branch name (`HEAD` when detached), then the last commit subject on the next line
/// (none before the first commit).
pub fn task_text(root: &Path) -> Result<String> {
    let branch = run(root, &["branch", "--show-current"], None)?;
    let branch = branch.trim();
    let subject = match head_commit(root)? {
        Some(_) => run(root, &["log", "-1", "--format=%s"], None)?,
        None => String::new(),
    };
    let branch = if branch.is_empty() { "HEAD" } else { branch };
    Ok(format!("{branch}\n{}", subject.trim()).trim().to_string())
}

/// The files each of the last `n` commits touched (merge commits list none); empty with no commits.
pub fn recent_commit_files(root: &Path, n: usize) -> Result<Vec<Vec<String>>> {
    if run(root, &["rev-parse", "--verify", "-q", "HEAD"], None).is_err() {
        return Ok(vec![]);
    }
    let limit = format!("-{n}");
    // \x01 on its own line starts each commit; no path is named that
    let out = run(
        root,
        &[
            "log",
            &limit,
            "--no-renames",
            "--name-only",
            "--format=%x01",
        ],
        None,
    )?;
    Ok(out
        .split('\x01')
        .skip(1)
        .map(|c| {
            c.lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .collect())
}
