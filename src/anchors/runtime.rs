// Tree-sitter grammars loaded at run time from `<stewards home>/grammars/<lang>/`: a compiled
// library (`lib.so`, `lib.dylib` on macOS), its `tags.scm`, and an optional `extensions` file.
// Read only from the user's own stewards home, never from a repository.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libloading::os::unix;
use libloading::{Library, Symbol};
use tree_sitter::{LANGUAGE_VERSION, Language, MIN_COMPATIBLE_LANGUAGE_VERSION, Query};
use tree_sitter_language::LanguageFn;

use crate::user;

/// A loaded grammar. Statics are never dropped, so the library stays loaded for the process.
pub(super) struct Runtime {
    pub(super) language: Language,
    pub(super) query: Query,
    _library: Library,
}

/// One directory under `grammars/`, loaded the first time one of its extensions is looked up
/// (or `unusable` is asked).
struct Dir {
    path: PathBuf,
    exts: Vec<String>,
    loaded: OnceLock<Result<Runtime, String>>,
}

impl Dir {
    fn load(&self) -> &Result<Runtime, String> {
        self.loaded.get_or_init(|| load(&self.path))
    }
}

/// The grammar directories in name order, read once per process.
fn dirs() -> &'static [Dir] {
    static DIRS: OnceLock<Vec<Dir>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let Some(Ok(entries)) = user::grammars_dir().map(std::fs::read_dir) else {
            return vec![];
        };
        let mut dirs: Vec<Dir> = entries
            .filter_map(|e| {
                let path = e.ok()?.path();
                // `is_dir` follows links: the stewards home is the user's own
                if !path.is_dir() {
                    return None;
                }
                let name = path.file_name()?.to_str()?.to_string();
                let exts = match std::fs::read_to_string(path.join("extensions")) {
                    Ok(text) => text
                        .lines()
                        .map(|l| l.trim().trim_start_matches('.').to_string())
                        .filter(|l| !l.is_empty())
                        .collect(),
                    Err(_) => vec![name],
                };
                Some(Dir {
                    path,
                    exts,
                    loaded: OnceLock::new(),
                })
            })
            .collect();
        dirs.sort_by(|a, b| a.path.cmp(&b.path));
        dirs
    })
}

fn regular_file(path: &Path) -> Result<(), String> {
    // `metadata` follows links
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => Ok(()),
        _ => Err(format!("{} is not a regular file", path.display())),
    }
}

fn load(dir: &Path) -> Result<Runtime, String> {
    let lib = dir.join(if cfg!(target_os = "macos") {
        "lib.dylib"
    } else {
        "lib.so"
    });
    let tags = dir.join("tags.scm");
    regular_file(&lib)?;
    regular_file(&tags)?;
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let symbol = format!("tree_sitter_{}", name.replace('-', "_"));
    // SAFETY: the user put this library in their own stewards home, so its initializers and its
    // `tree_sitter_<lang>` function are trusted to run and to return a `TSLanguage`.
    let (library, language) = unsafe {
        // RTLD_NOW: an unresolved symbol fails here, as a warning, not later mid-parse as an abort.
        let library: Library = unix::Library::open(Some(&lib), unix::RTLD_NOW | unix::RTLD_LOCAL)
            .map_err(|e| format!("cannot load {}: {e}", lib.display()))?
            .into();
        let entry: Symbol<unsafe extern "C" fn() -> *const ()> = library
            .get(symbol.as_bytes())
            .map_err(|_| format!("{} exports no `{symbol}`", lib.display()))?;
        if (*entry)().is_null() {
            return Err(format!("{} `{symbol}` returned null", lib.display()));
        }
        let language = Language::from(LanguageFn::from_raw(*entry));
        (library, language)
    };
    let abi = language.abi_version();
    if !(MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION).contains(&abi) {
        return Err(format!(
            "{} has tree-sitter ABI {abi}, this build reads {MIN_COMPATIBLE_LANGUAGE_VERSION}..{LANGUAGE_VERSION}",
            lib.display()
        ));
    }
    let source = std::fs::read_to_string(&tags)
        .map_err(|e| format!("cannot read {}: {e}", tags.display()))?;
    let query = Query::new(&language, &source)
        .map_err(|e| format!("{} does not compile for this grammar: {e}", tags.display()))?;
    Ok(Runtime {
        language,
        query,
        _library: library,
    })
}

/// The usable grammar for a file extension; `None` when no directory claims it or the first that
/// does can't be used. (simplified: two directories claiming one extension, the first by name wins.)
pub(super) fn lookup(ext: &str) -> Option<&'static Runtime> {
    let dir = dirs().iter().find(|d| d.exts.iter().any(|e| e == ext))?;
    dir.load().as_ref().ok()
}

/// Each grammar directory that can't be used, with the reason; loads every directory.
pub fn unusable() -> Vec<(&'static Path, &'static str)> {
    dirs()
        .iter()
        .filter_map(|d| Some((d.path.as_path(), d.load().as_ref().err()?.as_str())))
        .collect()
}
