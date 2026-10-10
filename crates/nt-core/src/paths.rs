//! The path rules for a project path, and the check on a workspace's
//! isolated copy. An agent edits inside its target with no question, so
//! these rules keep a target away from the author's home, git and Claude
//! Code setup, credentials, and New Terminal's own home.

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
/// The full Unicode case folds that `to_lowercase` leaves out and that end
/// in ASCII. APFS applies them when it compares names, so `.ſſh`, `.ßh`, and
/// `.conﬁg` each open the ASCII directory.
const ASCII_FOLDS: [(char, &str); 9] = [
    ('\u{17f}', "s"),
    ('\u{df}', "ss"),
    ('\u{fb00}', "ff"),
    ('\u{fb01}', "fi"),
    ('\u{fb02}', "fl"),
    ('\u{fb03}', "ffi"),
    ('\u{fb04}', "ffl"),
    ('\u{fb05}', "st"),
    ('\u{fb06}', "st"),
];

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

/// `path` with a leading home directory written as `~`, as the author
/// would type it.
pub fn with_tilde(path: &Path, home_dir: &Path) -> String {
    match path.strip_prefix(home_dir) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
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
    check_placement(path, &canonical, rules)?;
    check_claude_config(&canonical, rules.claude_config_dir)?;
    Ok(canonical)
}

/// Why a workspace's isolated copy cannot take an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyProblem {
    Missing,
    /// The copy exists, but its canonical form is this other path.
    Moved(PathBuf),
    /// The copy is not at `<home>/workspaces/<name>/<project>`, shown here.
    NotAt(PathBuf),
    Rule(PathRule),
}

/// The check at agent start for a workspace. The copy stands in for path
/// rules 1 to 6, because the app made it under its own home: it must
/// exist, be canonical, and be `expected`. Rule 7 still applies, so a
/// `CLAUDE_CONFIG_DIR` inside a copy cannot take edits with no question.
pub fn check_copy(
    copy: &Path,
    expected: &Path,
    claude_config_dir: Option<&OsStr>,
) -> Result<(), CopyProblem> {
    let canonical = fs::canonicalize(copy).map_err(|_| CopyProblem::Missing)?;
    if canonical != copy {
        return Err(CopyProblem::Moved(canonical));
    }
    if copy != expected {
        return Err(CopyProblem::NotAt(expected.to_path_buf()));
    }
    check_claude_config(&canonical, claude_config_dir).map_err(CopyProblem::Rule)
}

/// Rule 7.
fn check_claude_config(
    canonical: &Path,
    claude_config_dir: Option<&OsStr>,
) -> Result<(), PathRule> {
    let Some(value) = claude_config_dir.filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let config_dir = PathBuf::from(value);
    let config_dir = fs::canonicalize(&config_dir).unwrap_or(config_dir);
    if canonical.starts_with(&config_dir) || config_dir.starts_with(canonical) {
        return Err(rule(
            7,
            format!(
                "the path must not be, hold, or sit inside CLAUDE_CONFIG_DIR ({})",
                value.display()
            ),
        ));
    }
    Ok(())
}

/// Runs rules 3 to 6 on a path read from the state file. A stored path is
/// canonical when it exists, so it stands for both forms that rule 5
/// tests. Rule 7 waits for the captured environment and is not run here,
/// so `rules.claude_config_dir` is not read.
pub fn check_stored(stored: &Path, rules: &Rules<'_>) -> Result<(), PathRule> {
    check_placement(stored, stored, rules)
}

/// Rules 3 to 6. Rule 5 tests `given` and `canonical`, so a link cannot
/// hide a guarded part on either side.
fn check_placement(given: &Path, canonical: &Path, rules: &Rules<'_>) -> Result<(), PathRule> {
    if canonical.parent().is_none() || rules.home_dir.starts_with(canonical) {
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
    if let Some(part) = guarded_part(given).or_else(|| guarded_part(canonical)) {
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
    Ok(())
}

const fn rule(number: u8, text: String) -> PathRule {
    PathRule { number, text }
}

/// `name` folded so that two names APFS opens as one file fold to the same
/// text, as far as a name that folds to ASCII goes. Every rule that compares
/// a path part with a listed ASCII name compares folded text.
pub fn fold_case(name: &str) -> String {
    let mut folded = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        match ASCII_FOLDS.iter().find(|(from, _)| *from == c) {
            Some((_, to)) => folded.push_str(to),
            None => folded.push(c),
        }
    }
    folded
}

/// The first guarded part of `path`, folded, so it reads as the listed
/// entry it matched.
fn guarded_part(path: &Path) -> Option<String> {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(fold_case(&part.to_string_lossy())),
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
