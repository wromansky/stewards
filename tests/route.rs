#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use common::fixture;
use stewards::model::{UnitDef, load_repo};
use stewards::route::Router;

fn unit(name: &str, paths: &[&str]) -> UnitDef {
    UnitDef {
        name: name.into(),
        purpose: String::new(),
        paths: paths.iter().map(|p| p.to_string()).collect(),
    }
}

fn router(units: &[UnitDef]) -> Router {
    Router::new(units, "docs/units").unwrap()
}

#[test]
fn plain_glob_routes_to_its_unit() {
    let r = router(&[
        unit("api", &["src/api/**"]),
        unit("store", &["src/store/**"]),
    ]);
    assert_eq!(r.owner("src/store/db.py"), Some("store"));
    assert_eq!(r.owner("Makefile"), None);
}

#[test]
fn more_specific_glob_wins_whatever_the_unit_order() {
    let broad = unit("broad", &["src/**"]);
    let narrow = unit("narrow", &["src/api/**"]);
    let r = router(&[broad.clone(), narrow.clone()]);
    assert_eq!(r.owner("src/api/a.py"), Some("narrow"));
    let r = router(&[narrow, broad]);
    assert_eq!(r.owner("src/api/a.py"), Some("narrow"));
}

#[test]
fn fewer_wildcards_win_at_equal_prefix() {
    let r = router(&[unit("deep", &["src/**/*.py"]), unit("flat", &["src/*.py"])]);
    assert_eq!(r.owner("src/a.py"), Some("flat"));
}

#[test]
fn equal_specificity_falls_to_file_order_and_shows_in_overlaps() {
    let r = router(&[unit("first", &["src/*.py"]), unit("second", &["src/*.py"])]);
    assert_eq!(r.owner("src/a.py"), Some("first"));
    let files = ["src/a.py".to_string(), "src/b.rs".to_string()];
    assert_eq!(
        r.overlaps(&files),
        [(
            "src/a.py".to_string(),
            vec!["first".to_string(), "second".to_string()]
        )]
    );
}

#[test]
fn a_more_specific_match_is_not_an_overlap() {
    let r = router(&[unit("broad", &["src/**"]), unit("narrow", &["src/api/**"])]);
    assert!(r.overlaps(&["src/api/a.py".to_string()]).is_empty());
}

#[test]
fn tool_owned_paths_have_no_owner() {
    let r = router(&[unit("all", &["**"])]);
    assert_eq!(r.owner(".stewards/units.toml"), None);
    assert_eq!(r.owner("docs/units/api.md"), None);
    assert_eq!(r.owner("docs/units.md"), Some("all"));
    assert!(r.overlaps(&[".stewards/repo.toml".to_string()]).is_empty());
}

#[test]
fn dot_slash_and_backslashes_are_normalised() {
    let r = router(&[unit("api", &["src/api/**"])]);
    assert_eq!(r.owner("./src/api/a.py"), Some("api"));
    assert_eq!(r.owner("src\\api\\a.py"), Some("api"));
    assert_eq!(r.owner(".\\.stewards\\repo.toml"), None);
}

#[test]
fn star_stays_in_one_directory_and_double_star_crosses() {
    let r = router(&[unit("flat", &["src/*.py"]), unit("deep", &["lib/**/*.py"])]);
    assert_eq!(r.owner("src/a.py"), Some("flat"));
    assert_eq!(r.owner("src/x/a.py"), None);
    assert_eq!(r.owner("lib/x/y/a.py"), Some("deep"));
}

#[test]
fn bad_glob_is_invalid_toml() {
    let e = Router::new(&[unit("api", &["src/[a"])], "docs/units")
        .err()
        .unwrap();
    assert_eq!(
        e.downcast_ref::<stewards::output::CliError>().unwrap().code,
        "invalid_toml"
    );
}

#[test]
fn single_fixture_routes_as_its_units_toml_says() {
    let repo = fixture("single");
    let loaded = load_repo(repo.path()).unwrap();
    let r = Router::new(&loaded.units, &loaded.repo.docs_path).unwrap();
    for (path, unit) in [
        ("src/api/items.py", Some("api")),
        ("tests/api/test_items.py", Some("api")),
        ("src/store/db.py", Some("store")),
        ("src/store/README.md", Some("store")),
        ("tests/store/test_db.py", Some("store")),
        ("Makefile", None),
        (".stewards/units.toml", None),
    ] {
        assert_eq!(r.owner(path), unit, "{path}");
    }
}

#[cfg(unix)]
#[test]
fn route_command_follows_a_symlinked_directory_into_the_repo() {
    let repo = common::fixture("single");
    let outside = tempfile::TempDir::new().unwrap();
    let link = outside.path().join("api-link");
    std::os::unix::fs::symlink(repo.path().join("src/api"), &link).unwrap();
    let out = common::stewards(
        repo.path(),
        &["route", link.join("items.py").to_str().unwrap()],
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["routes"][0]["unit"], "api", "{v}");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn route_command_rejects_a_path_outside_the_repo() {
    let repo = common::fixture("single");
    let out = common::stewards(repo.path(), &["route", "../elsewhere.py"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "path_outside_repo");
    assert_eq!(out.status.code(), Some(2));
}
