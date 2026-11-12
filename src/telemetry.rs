// `.stewards/telemetry/<yyyy-mm>.jsonl`: one line per submit, sizes and counts only.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::output::CliError;

#[derive(Debug, Serialize)]
pub struct Record {
    pub unit: Option<String>,
    pub role: &'static str,
    pub mode: &'static str,
    pub task: String,
    pub bytes: usize,
    pub anchors: usize,
    pub pulled_anchors: Vec<String>,
    pub harness: Option<&'static str>,
    /// RFC 3339 UTC; its first seven characters name the file.
    pub timestamp: String,
}

pub fn append(root: &Path, record: &Record) -> Result<()> {
    let rel = format!(".stewards/telemetry/{}.jsonl", &record.timestamp[..7]);
    append_under_stewards(root, &rel, &serde_json::to_string(record)?)
}

/// Every telemetry write: appends `line` to `root/rel`, refusing a path through a symlink, which a
/// hostile repo could commit to point a write outside `.stewards/`.
fn append_under_stewards(root: &Path, rel: &str, line: &str) -> Result<()> {
    let mut path = root.to_path_buf();
    for c in Path::new(rel).components() {
        path.push(c);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(CliError::new(
                "unsafe_path",
                format!("{rel} passes through the symlink {}", path.display()),
                "replace the symlink with a plain file or directory",
            ));
        }
    }
    std::fs::create_dir_all(path.parent().expect("a file has a parent"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// `2026-11-12T00:45:03Z`, by Howard Hinnant's `civil_from_days`; times before the epoch print as
/// the epoch.
pub fn rfc3339(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let (days, rest) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Submits and mean pack bytes per role, and total pulled anchors; null with no telemetry files.
/// simplified: unreadable files and lines are skipped, since telemetry must never fail `status`.
pub fn summary(root: &Path) -> Value {
    let Ok(dir) = std::fs::read_dir(root.join(".stewards/telemetry")) else {
        return Value::Null;
    };
    let mut seen_file = false;
    let mut roles: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut pulled = 0;
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        seen_file = true;
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for v in text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        {
            let Some(role) = v["role"].as_str() else {
                continue;
            };
            let (count, bytes) = roles.entry(role.to_string()).or_default();
            *count += 1;
            *bytes += v["bytes"].as_u64().unwrap_or(0);
            pulled += v["pulled_anchors"].as_array().map_or(0, Vec::len);
        }
    }
    if !seen_file {
        return Value::Null;
    }
    json!({
        "submits": roles.iter().map(|(r, (n, _))| (r.clone(), json!(n))).collect::<serde_json::Map<_, _>>(),
        "mean_pack_bytes": roles.iter().map(|(r, (n, b))| (r.clone(), json!(b / n))).collect::<serde_json::Map<_, _>>(),
        "pulled_anchors": pulled,
    })
}
