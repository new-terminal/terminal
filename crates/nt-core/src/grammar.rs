//! The prompt grammar: what one submitted line asks for. A pure function of
//! the line, so the registry and the target stay with the actor.

use std::fmt;

/// The longest line the prompt accepts.
pub const LINE_LIMIT_KB: usize = 100;
const BYTES_PER_KB: usize = 1024;
const LINE_LIMIT_BYTES: usize = LINE_LIMIT_KB * BYTES_PER_KB;
const NAME_MAX_CHARS: usize = 32;

/// A project or workspace name: lowercase ASCII letters, digits, and hyphens,
/// 1 to 32 characters, starting with a letter or a digit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Name(String);

impl Name {
    /// `None` when `text` breaks the name rule. No case folding happens here.
    pub fn parse(text: &str) -> Option<Self> {
        let mut chars = text.chars();
        let starts_well = chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        let rest_valid = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        let length_valid = text.len() <= NAME_MAX_CHARS;
        (starts_well && rest_valid && length_valid).then(|| Self(text.to_owned()))
    }
}

/// The name rule, worded for a refusal.
pub const NAME_RULE: &str = "a name uses lowercase letters, digits, and hyphens, 1 to 32 characters, and starts with a letter or a digit";

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The whole line that asks for the list of projects and workspaces.
pub const LIST_WORDS: &str = "list";

/// An app intent that takes arguments: a line the app handles itself and
/// never sends to an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentKind {
    AddProject,
    NewWorkspace,
    ArchiveWorkspace,
}

impl IntentKind {
    /// The words that start the intent, as the author types them.
    pub const fn words(self) -> &'static str {
        match self {
            Self::AddProject => "add project",
            Self::NewWorkspace => "new workspace",
            Self::ArchiveWorkspace => "archive workspace",
        }
    }
}

/// A well-formed app intent with its arguments as typed. Names are not
/// checked here, so a refusal can name the rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// `path` is the rest of the line, so it may hold spaces.
    AddProject { name: String, path: String },
    /// `project` is lowercase, as a mention, with its `@` removed.
    NewWorkspace { name: String, project: String },
    /// `name` is lowercase, as a mention, with its `@` removed.
    ArchiveWorkspace { name: String },
}

/// What one line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Empty,
    TooLong {
        kb: usize,
    },
    /// The line is [`LIST_WORDS`] alone.
    List,
    Intent(Intent),
    /// The line starts with an intent's words, but its arguments do not fit.
    IntentUsage(IntentKind),
    MentionOnly(Name),
    Request {
        mention: Option<Name>,
        text: String,
    },
    /// A leading `@` token that is not a valid name. Holds the token
    /// without its `@`, as typed.
    BadMention(String),
    SeveralMentions,
}

pub fn parse(line: &str) -> Parsed {
    let line = line.trim_start();
    if line.is_empty() {
        return Parsed::Empty;
    }
    if line.len() > LINE_LIMIT_BYTES {
        return Parsed::TooLong {
            kb: line.len().div_ceil(BYTES_PER_KB),
        };
    }
    if let Some(intent) = parse_intent(line) {
        return intent;
    }
    parse_mentions(line)
}

fn parse_intent(line: &str) -> Option<Parsed> {
    if line.trim_end().eq_ignore_ascii_case(LIST_WORDS) {
        return Some(Parsed::List);
    }
    let (first, rest) = split_word(line)?;
    let (second, arguments) = split_word(rest)?;
    let typed = format!("{first} {second}");
    let starts_with = |kind: IntentKind| typed.eq_ignore_ascii_case(kind.words());
    let (kind, intent) = if starts_with(IntentKind::AddProject) {
        (IntentKind::AddProject, add_project(arguments))
    } else if starts_with(IntentKind::NewWorkspace) {
        (IntentKind::NewWorkspace, new_workspace(arguments))
    } else if starts_with(IntentKind::ArchiveWorkspace) {
        (IntentKind::ArchiveWorkspace, archive_workspace(arguments))
    } else {
        return None;
    };
    Some(intent.map_or(Parsed::IntentUsage(kind), Parsed::Intent))
}

/// `<name> @<project>`, where the `@` is optional.
fn new_workspace(arguments: &str) -> Option<Intent> {
    let mut words = arguments.split_whitespace();
    let (Some(name), Some(project), None) = (words.next(), words.next(), words.next()) else {
        return None;
    };
    let project = project.strip_prefix('@').unwrap_or(project);
    Some(Intent::NewWorkspace {
        name: name.to_owned(),
        project: project.to_lowercase(),
    })
}

/// `<name>`, where the `@` is optional.
fn archive_workspace(arguments: &str) -> Option<Intent> {
    let mut words = arguments.split_whitespace();
    let (Some(name), None) = (words.next(), words.next()) else {
        return None;
    };
    let name = name.strip_prefix('@').unwrap_or(name);
    Some(Intent::ArchiveWorkspace {
        name: name.to_lowercase(),
    })
}

/// `<name> <path>`, where the path is the rest of the line, so it may hold
/// spaces.
fn add_project(arguments: &str) -> Option<Intent> {
    let (name, path) = split_word(arguments)?;
    let path = path.trim_end();
    (!path.is_empty()).then(|| Intent::AddProject {
        name: name.to_owned(),
        path: path.to_owned(),
    })
}

fn parse_mentions(line: &str) -> Parsed {
    let mut mentions = Vec::new();
    let mut rest = line;
    while let Some((word, after)) = split_word(rest)
        && let Some(token) = word.strip_prefix('@')
    {
        let Some(name) = Name::parse(&token.to_lowercase()) else {
            return Parsed::BadMention(token.to_owned());
        };
        mentions.push(name);
        rest = after;
    }
    if mentions.len() > 1 {
        return Parsed::SeveralMentions;
    }
    let mention = mentions.pop();
    match mention {
        Some(name) if rest.trim_end().is_empty() => Parsed::MentionOnly(name),
        mention => Parsed::Request {
            mention,
            text: rest.to_owned(),
        },
    }
}

/// The first whitespace-separated word of `text`, and what follows it with
/// leading whitespace removed. `None` when `text` holds no word.
fn split_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let (word, rest) = text.split_at(end);
    Some((word, rest.trim_start()))
}
