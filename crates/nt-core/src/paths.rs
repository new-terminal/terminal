//! The path rules for a project path. An agent edits inside its target with
//! no question, so these rules keep a target away from the author's home,
//! git and Claude Code setup, credentials, and New Terminal's own home.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::grammar::Name;

/// Directories Claude Code guards against edits.
const GUARDED_DIRECTORIES: [&str; 9] = [
    ".git",
    ".claude",
    ".vscode",
    ".idea",
    ".husky",
    ".cargo",
    ".devcontainer",
    ".yarn",
    ".mvn",
];
/// Directories Claude Code lists as holding credentials.
const CREDENTIAL_DIRECTORIES: [&str; 6] = [".ssh", ".aws", ".azure", ".gnupg", ".kube", ".docker"];
/// `.config/git` holds the global git config, which can name commands git
/// runs. A target at `.config` itself would hold it too.
const CONFIG_DIRECTORY: &str = ".config";
const GIT_UNDER_CONFIG: &str = "git";

/// A broken path rule: its number and the text that explains it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathRule {
    pub number: u8,
    pub text: String,
}

impl fmt::Display for PathRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "path rule {}, {}", self.number, self.text)
    }
}

/// What the rules compare a path with.
#[derive(Debug)]
pub struct Rules<'a> {
    /// The user's home directory, canonical.
    pub home_dir: &'a Path,
    /// New Terminal's home, canonical.
    pub app_home: &'a Path,
    /// `CLAUDE_CONFIG_DIR` from the captured environment, as set.
    pub claude_config_dir: Option<&'a OsStr>,
    /// Every project except the one the path belongs to.
    pub other_projects: Vec<(&'a Name, &'a Path)>,
}

/// Replaces a leading `~` with the home directory.
pub fn expand_tilde(raw: &str, home_dir: &Path) -> PathBuf {
    if raw == "~" {
        home_dir.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home_dir.join(rest)
    } else {
        PathBuf::from(raw)
    }
}

/// Runs the 7 rules in order and returns the canonical path, or the first
/// rule the path breaks.
pub fn check(path: &Path, rules: &Rules<'_>) -> Result<PathBuf, PathRule> {
    if !path.is_absolute() {
        return Err(rule(1, "the path must be absolute".to_owned()));
    }
    let canonical = fs::canonicalize(path)
        .ok()
        .filter(|canonical| canonical.is_dir())
        .ok_or_else(|| {
            rule(
                2,
                format!("{} is not an existing directory", path.display()),
            )
        })?;
    if canonical.parent().is_none() || rules.home_dir.starts_with(&canonical) {
        return Err(rule(
            3,
            "the path must not be /, your home directory, or a directory that holds it".to_owned(),
        ));
    }
    if canonical.starts_with(rules.app_home) {
        return Err(rule(
            4,
            format!(
                "the path must not be inside New Terminal's home ({})",
                rules.app_home.display()
            ),
        ));
    }
    if let Some(part) = guarded_part(&canonical) {
        return Err(rule(
            5,
            format!(
                "the path must not include a directory Claude Code guards or a credential directory ({part})"
            ),
        ));
    }
    if let Some((name, _)) = rules
        .other_projects
        .iter()
        .find(|(_, other)| *other == canonical)
    {
        return Err(rule(6, format!("the path is already project {name}")));
    }
    if let Some(value) = rules.claude_config_dir.filter(|value| !value.is_empty()) {
        let config_dir = PathBuf::from(value);
        let config_dir = fs::canonicalize(&config_dir).unwrap_or(config_dir);
        if canonical.starts_with(&config_dir) || config_dir.starts_with(&canonical) {
            return Err(rule(
                7,
                format!(
                    "the path must not be, hold, or sit inside CLAUDE_CONFIG_DIR ({})",
                    value.display()
                ),
            ));
        }
    }
    Ok(canonical)
}

const fn rule(number: u8, text: String) -> PathRule {
    PathRule { number, text }
}

/// The first guarded part of `path`, compared without case, as the author
/// would recognize it.
fn guarded_part(path: &Path) -> Option<String> {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    let listed = parts.iter().find(|part| {
        GUARDED_DIRECTORIES.contains(&part.as_str())
            || CREDENTIAL_DIRECTORIES.contains(&part.as_str())
    });
    if let Some(part) = listed {
        return Some(part.clone());
    }
    let git_under_config = parts
        .windows(2)
        .any(|pair| pair[0] == CONFIG_DIRECTORY && pair[1] == GIT_UNDER_CONFIG);
    if git_under_config {
        return Some(format!("{CONFIG_DIRECTORY}/{GIT_UNDER_CONFIG}"));
    }
    (parts.last().map(String::as_str) == Some(CONFIG_DIRECTORY))
        .then(|| CONFIG_DIRECTORY.to_owned())
}
