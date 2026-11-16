#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::path::Path;

use common::{run, stewards};

#[test]
fn every_shell_prints_a_script_naming_review_and_a_save_hint() {
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let out = stewards(Path::new("/"), &["completions", shell]);
        let script = String::from_utf8(out.stdout).unwrap();
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert_eq!(out.status.code(), Some(0), "{shell}: {stderr}");
        assert!(script.contains("review"), "{shell}");
        assert!(
            stderr.starts_with(&format!("save it: stewards completions {shell} > ")),
            "{shell}: {stderr}"
        );
    }
}

#[test]
fn an_unknown_shell_is_a_usage_error() {
    let (code, v) = run(Path::new("/"), &["completions", "tcsh"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage");
    assert!(v["error"]["message"].as_str().unwrap().contains("tcsh"));
}

#[test]
fn man_prints_roff_naming_the_commands() {
    let out = stewards(Path::new("/"), &["man"]);
    let page = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0));
    // clap_mangen writes two preamble lines before the title.
    assert!(
        page.lines().any(|l| l.starts_with(".TH stewards 1")),
        "{page}"
    );
    assert!(page.contains("completions"));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .starts_with("save it: stewards man > ")
    );
}

/// A fake release served by `curl` and `uname` shims on PATH: no network, and always Linux x86_64.
struct FakeRelease {
    dir: tempfile::TempDir,
}

const ARCHIVE: &str = "stewards-0.0.1-x86_64-unknown-linux-gnu.tar.gz";

impl FakeRelease {
    /// `sums_hash` is what SHA256SUMS claims for the archive; `None` for the real hash.
    fn new(sums_hash: Option<&str>) -> Self {
        use sha2::{Digest, Sha256};
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let top = root.join("stewards-0.0.1-x86_64-unknown-linux-gnu");
        for (path, text) in [
            ("bin/stewards", "#!/bin/sh\necho fake stewards\n"),
            ("lib/stewards/libbaml_cffi.so", "lib"),
            ("share/man/man1/stewards.1", ".TH stewards 1\n"),
        ] {
            std::fs::create_dir_all(top.join(path).parent().unwrap()).unwrap();
            std::fs::write(top.join(path), text).unwrap();
        }
        std::fs::set_permissions(
            top.join("bin/stewards"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let tar = std::process::Command::new("tar")
            .args(["-czf", ARCHIVE, "stewards-0.0.1-x86_64-unknown-linux-gnu"])
            .current_dir(root)
            .status()
            .unwrap();
        assert!(tar.success());
        let hash: String = Sha256::digest(std::fs::read(root.join(ARCHIVE)).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let claimed = sums_hash.unwrap_or(&hash);
        std::fs::write(root.join("SHA256SUMS"), format!("{claimed}  {ARCHIVE}\n")).unwrap();
        let shims = root.join("shims");
        std::fs::create_dir(&shims).unwrap();
        // The shim serves files from the release directory in place of GitHub's download URLs.
        let curl = format!(
            r#"#!/bin/sh
out=
for a in "$@"; do url=$a; done
while [ "$#" -gt 1 ]; do
  [ "$1" = -o ] && out=$2
  shift
done
echo "$url" >> {root}/urls
case $url in
  https://github.com/wromansky/stewards/releases/latest) printf '%s' https://github.com/wromansky/stewards/releases/tag/v0.0.1 ;;
  https://github.com/wromansky/stewards/releases/download/v0.0.1/*) cp {root}/"${{url##*/}}" "$out" ;;
  *) echo "unexpected url $url" >&2; exit 22 ;;
esac
"#,
            root = root.display()
        );
        for (name, body) in [
            ("curl", curl.as_str()),
            (
                "uname",
                "#!/bin/sh\nif [ \"$1\" = -s ]; then echo Linux; else echo x86_64; fi\n",
            ),
        ] {
            let path = shims.join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
        }
        Self { dir }
    }

    /// Runs `sh install.sh <args>` with the shims first on PATH; returns the output and the prefix.
    fn install(&self, args: &[&str]) -> (std::process::Output, tempfile::TempDir) {
        let prefix = tempfile::TempDir::new().unwrap();
        let path = format!(
            "{}:{}",
            self.dir.path().join("shims").display(),
            std::env::var("PATH").unwrap()
        );
        let out = std::process::Command::new("sh")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
            .args(args)
            .env("PATH", path)
            .env("STEWARDS_PREFIX", prefix.path())
            .output()
            .unwrap();
        (out, prefix)
    }

    fn urls(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("urls")).unwrap_or_default()
    }
}

#[test]
fn install_sh_installs_a_verified_release_and_says_what_to_do_next() {
    let release = FakeRelease::new(None);
    let (out, prefix) = release.install(&["v0.0.1"]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let p = prefix.path();
    assert!(p.join("bin/stewards").is_file());
    assert!(p.join("lib/stewards/libbaml_cffi.so").is_file());
    assert!(p.join("share/man/man1/stewards.1").is_file());
    assert!(stdout.contains("export PATH="), "{stdout}");
    assert!(stdout.contains("zsh:"), "{stdout}");
}

#[test]
fn install_sh_without_a_version_installs_the_latest_release() {
    let release = FakeRelease::new(None);
    let (out, prefix) = release.install(&[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(prefix.path().join("bin/stewards").is_file());
    assert!(release.urls().contains("/releases/latest"));
}

#[test]
fn install_sh_refuses_a_checksum_mismatch_and_installs_nothing() {
    let release = FakeRelease::new(Some(&"0".repeat(64)));
    let (out, prefix) = release.install(&["v0.0.1"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    assert!(!prefix.path().join("bin").exists());
}

#[test]
fn install_sh_refuses_a_version_that_is_not_a_plain_version() {
    let release = FakeRelease::new(None);
    for bad in ["v1/../x", "latest", "v0.0.1;id", "v1..2"] {
        let (out, prefix) = release.install(&[bad]);
        assert!(!out.status.success(), "{bad}");
        assert!(!prefix.path().join("bin").exists(), "{bad}");
    }
    assert_eq!(release.urls(), "");
}
