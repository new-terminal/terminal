//! `state.json`: the registry and the target, kept across launches. The
//! author may edit it by hand while the app is quit, so a load checks it as
//! untrusted input and refuses to change a file it cannot read whole.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::grammar::{NAME_RULE, Name};
use crate::home::PRIVATE_FILE_MODE;
use crate::paths::{self, Rules};
use crate::registry::{Project, Registry};

/// The only format version this build reads and writes.
const VERSION: u32 = 1;

/// The file as written. Unknown fields fail the load, so a hand edit with a
/// typo is reported, not dropped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateFile {
    version: u32,
    target: Option<String>,
    projects: Vec<ProjectEntry>,
}

impl StateFile {
    pub const fn new(target: Option<String>, projects: Vec<ProjectEntry>) -> Self {
        Self {
            version: VERSION,
            target,
            projects,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEntry {
    pub name: String,
    pub path: PathBuf,
}

/// Read first and on its own, so a file from a newer build reports its
/// version rather than the first field this build does not know.
#[derive(Deserialize)]
struct VersionOnly {
    version: u32,
}

/// What the load needs to know about a stored path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnDisk {
    Missing,
    Present { canonical: PathBuf, is_dir: bool },
}

/// Looks up `path` on disk for [`check`].
pub fn look(path: &Path) -> OnDisk {
    match fs::canonicalize(path) {
        Ok(canonical) => OnDisk::Present {
            is_dir: canonical.is_dir(),
            canonical,
        },
        Err(_) => OnDisk::Missing,
    }
}

/// A state file that passed every load check.
#[derive(Debug)]
pub struct Loaded {
    pub registry: Registry,
    /// `None` when the file saved no target, or one that names no entry.
    pub target: Option<Name>,
    /// One line for each problem that does not stop the load.
    pub warnings: Vec<String>,
}

/// Reads `path`. `Ok(None)` when the file does not exist. The error is the
/// problem as the author should read it, with the parser's line and column
/// where it has them.
pub fn read(path: &Path) -> Result<Option<StateFile>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let VersionOnly { version } =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if version != VERSION {
        return Err(format!(
            "version {version}, but this New Terminal reads only version {VERSION}"
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Checks a parsed file against the name rule and path rules 1 and 3 to 6,
/// with `look` for the file system facts. Rule 2 gives a warning. Rule 7
/// waits for the captured environment, so it runs at agent start only.
pub fn check(
    file: StateFile,
    home_dir: &Path,
    app_home: &Path,
    look: impl Fn(&Path) -> OnDisk,
) -> Result<Loaded, String> {
    let entries = named_entries(&file.projects)?;
    let mut projects = Vec::with_capacity(entries.len());
    let mut warnings = Vec::new();
    for (index, (name, path)) in entries.iter().enumerate() {
        let on_disk = look(path);
        if let OnDisk::Present { canonical, .. } = &on_disk
            && canonical != path
        {
            return Err(format!(
                "project {name}: its path, {}, resolves to {}",
                path.display(),
                canonical.display()
            ));
        }
        let rules = Rules {
            home_dir,
            app_home,
            claude_config_dir: None,
            other_projects: entries
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, (name, path))| (name, *path))
                .collect(),
        };
        paths::check_stored(path, &rules).map_err(|rule| format!("project {name}: {rule}"))?;
        match on_disk {
            OnDisk::Missing => {
                warnings.push(format!("Project {name}: {} is missing.", path.display()));
            }
            OnDisk::Present { is_dir: false, .. } => {
                warnings.push(format!(
                    "Project {name}: {} is not a directory.",
                    path.display()
                ));
            }
            OnDisk::Present { is_dir: true, .. } => {}
        }
        projects.push(Project {
            name: name.clone(),
            path: path.to_path_buf(),
        });
    }
    let registry = Registry::from_projects(projects);
    let target = match file.target {
        None => None,
        Some(saved) => {
            let found = Name::parse(&saved).filter(|name| registry.find(name).is_some());
            if found.is_none() {
                warnings.push(format!(
                    "Saved target {saved} names no project or workspace. No target."
                ));
            }
            found
        }
    };
    Ok(Loaded {
        registry,
        target,
        warnings,
    })
}

/// Each entry's name, checked against the name rule and the other names,
/// with its path, checked to be absolute.
fn named_entries(entries: &[ProjectEntry]) -> Result<Vec<(Name, &Path)>, String> {
    let mut names = BTreeSet::new();
    let mut named = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(name) = Name::parse(&entry.name) else {
            return Err(format!(
                "project {:?} breaks the name rule: {NAME_RULE}",
                entry.name
            ));
        };
        if !names.insert(name.clone()) {
            return Err(format!("the name {name} is used by more than one entry"));
        }
        if !entry.path.is_absolute() {
            return Err(format!(
                "project {name}: its path, {}, is not absolute",
                entry.path.display()
            ));
        }
        named.push((name, entry.path.as_path()));
    }
    Ok(named)
}

/// Writes `file` to `temp` with mode 0600, flushes it to disk, and renames
/// it over `path`. On any error the old file at `path` stays whole.
pub fn save(path: &Path, temp: &Path, file: &StateFile) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(file).map_err(io::Error::other)?;
    text.push('\n');
    let mut out = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(PRIVATE_FILE_MODE)
        .open(temp)?;
    out.write_all(text.as_bytes())?;
    out.sync_all()?;
    fs::rename(temp, path)
}
