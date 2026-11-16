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
            ("licenses/baml/LICENSE", "apache"),
            ("LICENSE", "mit"),
            ("THIRD-PARTY-LICENSES.html", "<html>"),
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
echo "$*" >> {root}/curl-args
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
            ("id", "#!/bin/sh\necho ${FAKE_UID:-1000}\n"),
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

    /// Runs `sh install.sh <args>` with the shims first on PATH and `envs` set; returns the output
    /// and the prefix, which is `STEWARDS_PREFIX` unless `envs` sets it.
    fn install_with(
        &self,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> (std::process::Output, tempfile::TempDir) {
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
            .envs(envs.iter().copied())
            .output()
            .unwrap();
        (out, prefix)
    }

    fn install(&self, args: &[&str]) -> (std::process::Output, tempfile::TempDir) {
        self.install_with(args, &[])
    }

    fn curl_args(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("curl-args")).unwrap_or_default()
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
    for f in ["LICENSE", "THIRD-PARTY-LICENSES.html", "baml/LICENSE"] {
        assert!(p.join("share/licenses/stewards").join(f).is_file(), "{f}");
    }
    // Installed by rename: no temp file is left, and the files are not writable by others.
    for dir in ["bin", "lib/stewards"] {
        let names: Vec<_> = std::fs::read_dir(p.join(dir))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|n| !n.contains('.') || n.ends_with(".so")),
            "{names:?}"
        );
    }
    let mode = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(p.join("lib/stewards/libbaml_cffi.so"))
            .unwrap()
            .permissions(),
    );
    assert_eq!(mode & 0o777, 0o755);
    assert!(stdout.contains("export PATH="), "{stdout}");
    assert!(stdout.contains("zsh:"), "{stdout}");
}

#[test]
fn install_sh_replaces_a_symlink_at_the_destination_instead_of_writing_through_it() {
    let release = FakeRelease::new(None);
    let victim = tempfile::TempDir::new().unwrap();
    let target = victim.path().join("target");
    std::fs::write(&target, "keep").unwrap();
    let prefix = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(prefix.path().join("bin")).unwrap();
    std::os::unix::fs::symlink(&target, prefix.path().join("bin/stewards")).unwrap();
    let (out, _) = release.install_with(
        &["v0.0.1"],
        &[("STEWARDS_PREFIX", prefix.path().to_str().unwrap())],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    assert!(!prefix.path().join("bin/stewards").is_symlink());
}

#[test]
fn install_sh_fetches_over_https_only() {
    let release = FakeRelease::new(None);
    let (out, _prefix) = release.install(&[]);
    assert!(out.status.success());
    let calls = release.curl_args();
    // The latest-release lookup, the archive, and SHA256SUMS.
    assert_eq!(calls.lines().count(), 3, "{calls}");
    for line in calls.lines() {
        assert!(
            line.contains("--proto =https") && line.contains("--tlsv1.2"),
            "{line}"
        );
        assert!(line.contains("--proto-redir =https"), "{line}");
    }
}

#[test]
fn install_sh_needs_an_absolute_prefix() {
    let release = FakeRelease::new(None);
    let home = tempfile::TempDir::new().unwrap();
    for prefix in ["relative/dir", "."] {
        let (out, _) = release.install_with(&["v0.0.1"], &[("STEWARDS_PREFIX", prefix)]);
        assert!(!out.status.success(), "{prefix}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("absolute"),
            "{prefix}"
        );
    }
    // No prefix: $HOME/.local, which must itself be absolute.
    let (out, _) = release.install_with(&["v0.0.1"], &[("STEWARDS_PREFIX", ""), ("HOME", "rel")]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("absolute"));
    let (out, _) = release.install_with(
        &["v0.0.1"],
        &[
            ("STEWARDS_PREFIX", ""),
            ("HOME", home.path().to_str().unwrap()),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.path().join(".local/bin/stewards").is_file());
    assert_eq!(
        release
            .urls()
            .lines()
            .filter(|l| l.ends_with(".tar.gz"))
            .count(),
        1
    );
}

#[test]
fn install_sh_refuses_root_unless_a_prefix_is_set() {
    let release = FakeRelease::new(None);
    let home = tempfile::TempDir::new().unwrap();
    let (out, _) = release.install_with(
        &["v0.0.1"],
        &[
            ("STEWARDS_PREFIX", ""),
            ("FAKE_UID", "0"),
            ("HOME", home.path().to_str().unwrap()),
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("STEWARDS_PREFIX"));
    assert!(!home.path().join(".local").exists());
    assert_eq!(release.urls(), "");
    let (out, prefix) = release.install_with(&["v0.0.1"], &[("FAKE_UID", "0")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(prefix.path().join("bin/stewards").is_file());
}

#[test]
fn install_sh_runs_nothing_until_its_last_line() {
    let script =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh")).unwrap();
    assert_eq!(script.lines().last(), Some("main \"$@\""));
    assert_eq!(script.lines().filter(|l| l.starts_with("main ")).count(), 1);
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

#[test]
fn bump_sh_rewrites_the_version_and_crate_hash_in_pkgbuild_and_srcinfo() {
    let aur = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/aur");
    let dir = tempfile::TempDir::new().unwrap();
    for f in ["bump.sh", "PKGBUILD", ".SRCINFO"] {
        std::fs::copy(aur.join(f), dir.path().join(f)).unwrap();
    }
    let baml = "44ffbd33d0c3b3a79a228a815c657b47c3c0151258da9f62b24d62491467d8ac";
    for (version, sha) in [("0.2.0", "a".repeat(64)), ("1.10.3", "b".repeat(64))] {
        let out = std::process::Command::new("sh")
            .args(["bump.sh", version, &sha])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let read = |f: &str| std::fs::read_to_string(dir.path().join(f)).unwrap();
        let (pkgbuild, srcinfo) = (read("PKGBUILD"), read(".SRCINFO"));
        for line in [
            format!("pkgver={version}"),
            "pkgrel=1".to_string(),
            format!("sha256sums=('{sha}'"),
            format!("            '{baml}')"),
        ] {
            assert!(pkgbuild.lines().any(|l| l == line), "{line}\n{pkgbuild}");
        }
        for line in [
            format!("\tpkgver = {version}"),
            "\tpkgrel = 1".to_string(),
            format!(
                "\tsource = stewards-{version}.tar.gz::https://static.crates.io/crates/stewards/stewards-{version}.crate"
            ),
            format!("\tsha256sums = {sha}"),
            format!("\tsha256sums = {baml}"),
        ] {
            assert!(srcinfo.lines().any(|l| l == line), "{line}\n{srcinfo}");
        }
        assert!(!dir.path().join("PKGBUILD.bak").exists());
    }
}
