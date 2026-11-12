// Git CLI wrappers: ls-files, snapshot, ls-tree, diffs, head.
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
    let temp = TempIndex(std::env::temp_dir().join(format!(
        "stewards-index-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    match std::fs::copy(&real, &temp.0) {
        Ok(_) => {}
        // no index yet: git starts from an empty one
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("copy {}", real.display())),
    }
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

pub fn has_object(root: &Path, sha: &str) -> bool {
    !sha.starts_with('-') && run(root, &["cat-file", "-e", sha], None).is_ok()
}

const DIFF: [&str; 4] = ["diff", "--no-renames", "--src-prefix=a/", "--dst-prefix=b/"];

pub fn diff_names(root: &Path, from: &str, to: &str) -> Result<Vec<String>> {
    let mut args = DIFF.to_vec();
    args.extend(["--name-only", "-z", "--end-of-options", from, to]);
    Ok(nul_split(&run(root, &args, None)?))
}

pub fn diff_file(root: &Path, from: &str, to: &str, path: &str) -> Result<String> {
    let mut args = DIFF.to_vec();
    args.extend([
        "--no-color",
        "--no-ext-diff",
        "-U3",
        "--end-of-options",
        from,
        to,
        "--",
        path,
    ]);
    run(root, &args, None)
}

/// Added lines as `(path, line number in the new file, text)`.
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
            if l.starts_with("+++ \"") {
                bail!("cannot parse C-quoted path in diff line: {l}");
            }
            // git appends a tab to the line when the path contains a space
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

pub fn head(root: &Path) -> Result<String> {
    Ok(run(root, &["rev-parse", "HEAD"], None)?.trim().to_string())
}

/// Current branch name (`HEAD` when detached), then the last commit subject on the next line
/// (none before the first commit).
pub fn task_text(root: &Path) -> Result<String> {
    let branch = run(root, &["branch", "--show-current"], None)?;
    let branch = branch.trim();
    let head = run_raw(root, &["rev-parse", "--verify", "--quiet", "HEAD"], None)?;
    let subject = match head.status.code() {
        Some(0) => run(root, &["log", "-1", "--format=%s"], None)?,
        // --quiet: exit 1 with no message means HEAD has no commit yet
        Some(1) => String::new(),
        _ => bail!(
            "git rev-parse HEAD: {}",
            String::from_utf8_lossy(&head.stderr).trim()
        ),
    };
    let branch = if branch.is_empty() { "HEAD" } else { branch };
    Ok(format!("{branch}\n{}", subject.trim()).trim().to_string())
}
