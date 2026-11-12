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

/// Added lines as `(path, line number in the new file, text)`; a file whose name is C-quoted is skipped.
pub fn diff_added_lines(root: &Path, from: &str, to: &str) -> Result<Vec<(String, u32, String)>> {
    let mut args = DIFF.to_vec();
    args.extend([
        "--no-color",
        "--no-ext-diff",
        "-U0",
        "--end-of-options",
        from,
        to,
    ]);
    let text = run(root, &args, None)?;
    let mut added = Vec::new();
    let mut path: Option<String> = None;
    let mut in_hunk = false;
    let mut line = 0;
    for l in text.lines() {
        if l.starts_with("diff --git ") {
            in_hunk = false;
            path = None;
        } else if !in_hunk && l.starts_with("+++ ") {
            // A C-quoted name (`+++ "b/..."`) does not match `+++ b/`, so its lines are skipped.
            // git appends a tab to the line when the path contains a space.
            path = l
                .strip_prefix("+++ b/")
                .map(|p| p.trim_end_matches('\t').to_string());
        } else if l.starts_with("@@ ") {
            in_hunk = true;
            // "@@ -a,b +c,d @@"
            let new = l
                .split(' ')
                .nth(2)
                .unwrap_or_default()
                .trim_start_matches('+');
            line = new
                .split(',')
                .next()
                .unwrap_or_default()
                .parse()
                .unwrap_or(0);
        } else if in_hunk && let (Some(p), Some(text)) = (&path, l.strip_prefix('+')) {
            added.push((p.clone(), line, text.to_string()));
            line += 1;
        }
    }
    Ok(added)
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
