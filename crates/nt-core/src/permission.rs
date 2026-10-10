//! The rule that lets an agent edit inside its target with no question.
//! Only `Edit` and `Write` qualify, and only for a path that stays inside
//! the target both as sent and after every link resolves, and that is not
//! a setup file. Every other request goes to the author.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// File names that Claude Code or a tool it guards loads as its own
/// configuration, in any directory. Compared without case, because macOS
/// file systems ignore case and the CLI lowercases names for its own check.
const SETUP_NAMES: [&str; 9] = [
    "CLAUDE.md",
    "CLAUDE.local.md",
    "AGENTS.md",
    "bunfig.toml",
    "lefthook.yml",
    "lefthook.yaml",
    "gradle-wrapper.properties",
    "maven-wrapper.properties",
    "pyrightconfig.json",
];
const AUTO_ALLOWED_TOOLS: [&str; 2] = ["Edit", "Write"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    AutoAllow,
    /// `setup` is true when either path is a setup file.
    AskAuthor {
        setup: bool,
    },
}

/// Decides one `can_use_tool` request. `target` is the stored target path,
/// which is never resolved again, so a target replaced by a link makes
/// every edit ask.
pub fn decide(tool: &str, input: &Value, target: &Path) -> Verdict {
    let ask = Verdict::AskAuthor { setup: false };
    if !AUTO_ALLOWED_TOOLS.contains(&tool) {
        return ask;
    }
    let Some(asked) = input["file_path"].as_str().map(Path::new) else {
        return ask;
    };
    let has_parent_part = asked.components().any(|part| part == Component::ParentDir);
    if !asked.is_absolute() || has_parent_part {
        return ask;
    }
    let Some(asked_below) = below(asked, target) else {
        return ask;
    };
    let resolved = resolve(asked);
    let resolved_below = resolved.as_deref().and_then(|path| below(path, target));
    let setup = is_setup(asked_below) || resolved_below.is_some_and(is_setup);
    if resolved_below.is_none() || setup {
        return Verdict::AskAuthor { setup };
    }
    Verdict::AutoAllow
}

/// The parts of `path` below `target`, when `path` lies strictly inside it.
fn below<'a>(path: &'a Path, target: &Path) -> Option<&'a Path> {
    path.strip_prefix(target)
        .ok()
        .filter(|rest| rest.components().next().is_some())
}

/// `path` with every link followed, the last part included. A missing tail
/// joins the canonical form of its deepest existing ancestor. `None` when a
/// part exists but does not resolve, such as a link that leads nowhere.
fn resolve(path: &Path) -> Option<PathBuf> {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(canonical) = fs::canonicalize(current) {
            return Some(
                missing
                    .iter()
                    .rev()
                    .fold(canonical, |path, part| path.join(part)),
            );
        }
        if fs::symlink_metadata(current).is_ok() {
            return None;
        }
        missing.push(current.file_name()?);
        current = current.parent()?;
    }
}

/// A part that starts with `.`, or a last part that is a setup name.
fn is_setup(below_target: &Path) -> bool {
    let parts: Vec<String> = below_target
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let dot_part = parts.iter().any(|part| part.starts_with('.'));
    let setup_name = parts.last().is_some_and(|name| {
        let name = fold_case(name);
        SETUP_NAMES.iter().any(|setup| fold_case(setup) == name)
    });
    dot_part || setup_name
}

/// Lowercase, plus the one fold APFS applies that `to_lowercase` does not:
/// long s (U+017F) matches `s`, so `pyrightconfig.j\u{17f}on` opens
/// `pyrightconfig.json`.
fn fold_case(name: &str) -> String {
    name.to_lowercase().replace('\u{17f}', "s")
}
