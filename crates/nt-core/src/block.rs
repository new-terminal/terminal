//! The request block: everything a permission request would run or write,
//! shown in full above the question so `y` allows only what the author saw.
//! No field is ever cut. Characters that could hide or reorder other text
//! show as escapes.

use std::fmt::Write as _;

use serde_json::{Map, Value};

/// A block past either limit is too long to review at the prompt.
const MAX_LINES: usize = 2_000;
const MAX_BYTES: usize = 256 * BYTES_PER_KB;
const BYTES_PER_KB: usize = 1024;

/// Zero-width and direction marks, which can hide or reorder text on
/// screen. Control characters other than tab and newline escape too.
const HIDING_MARKS: [(char, char); 6] = [
    ('\u{200b}', '\u{200f}'),
    ('\u{202a}', '\u{202e}'),
    ('\u{2060}', '\u{2060}'),
    ('\u{2066}', '\u{2069}'),
    ('\u{061c}', '\u{061c}'),
    ('\u{feff}', '\u{feff}'),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// The block's lines joined with newlines.
    Fits(String),
    TooLong {
        lines: usize,
        kb: usize,
    },
}

/// Builds the block for `tool` with its `input` as received.
pub fn build(tool: &str, input: &Value) -> Block {
    let lines = match (tool, input.as_object()) {
        ("Bash", Some(fields)) => bash(fields),
        ("Edit", Some(fields)) => edit(fields),
        ("Write", Some(fields)) => write(fields),
        _ => pretty_json(input),
    };
    let lines: Vec<String> = lines.iter().map(|line| escape(line)).collect();
    let bytes = lines.iter().map(String::len).sum::<usize>() + lines.len().saturating_sub(1);
    if lines.len() > MAX_LINES || bytes > MAX_BYTES {
        return Block::TooLong {
            lines: lines.len(),
            kb: bytes.div_ceil(BYTES_PER_KB),
        };
    }
    Block::Fits(lines.join("\n"))
}

/// Replaces every control character other than tab and newline, and every
/// zero-width or direction mark, with `\u{xxxx}`.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        let hides = (c.is_control() && c != '\t' && c != '\n')
            || HIDING_MARKS
                .iter()
                .any(|(first, last)| (*first..=*last).contains(&c));
        if hides {
            let _ = write!(escaped, "\\u{{{:04x}}}", u32::from(c));
        } else {
            escaped.push(c);
        }
    }
    escaped
}

fn bash(fields: &Map<String, Value>) -> Vec<String> {
    let mut shown = Shown::new(fields);
    let mut lines = Vec::new();
    if let Some(command) = shown.text("command") {
        lines.extend(split_lines(command, ""));
    }
    if shown.flag("run_in_background") == Some(true) {
        lines.push("runs in the background".to_owned());
    }
    lines.extend(shown.rest());
    lines
}

fn edit(fields: &Map<String, Value>) -> Vec<String> {
    let mut shown = Shown::new(fields);
    let mut lines = Vec::new();
    if let Some(path) = shown.text("file_path") {
        lines.push(path.to_owned());
    }
    if let Some(old) = shown.text("old_string") {
        lines.extend(split_lines(old, "-"));
    }
    if let Some(new) = shown.text("new_string") {
        lines.extend(split_lines(new, "+"));
    }
    if shown.flag("replace_all") == Some(true) {
        lines.push("every match".to_owned());
    }
    lines.extend(shown.rest());
    lines
}

fn write(fields: &Map<String, Value>) -> Vec<String> {
    let mut shown = Shown::new(fields);
    let mut lines = Vec::new();
    if let Some(path) = shown.text("file_path") {
        lines.push(path.to_owned());
    }
    if let Some(content) = shown.text("content") {
        lines.extend(split_lines(content, "+"));
    }
    lines.extend(shown.rest());
    lines
}

/// The fields of one request, marked off as the rules above show them, so
/// every field they do not show follows as JSON.
struct Shown<'a> {
    fields: &'a Map<String, Value>,
    used: Vec<&'static str>,
}

impl<'a> Shown<'a> {
    const fn new(fields: &'a Map<String, Value>) -> Self {
        Self {
            fields,
            used: Vec::new(),
        }
    }

    /// The field when it is a string. A field of another type stays for
    /// the JSON part.
    fn text(&mut self, key: &'static str) -> Option<&'a str> {
        let text = self.fields.get(key)?.as_str()?;
        self.used.push(key);
        Some(text)
    }

    /// The field when it is a boolean. A field of another type stays for
    /// the JSON part.
    fn flag(&mut self, key: &'static str) -> Option<bool> {
        let flag = self.fields.get(key)?.as_bool()?;
        self.used.push(key);
        Some(flag)
    }

    /// Every field not shown yet, as indented JSON.
    fn rest(&self) -> Vec<String> {
        let rest: Map<String, Value> = self
            .fields
            .iter()
            .filter(|(key, _)| !self.used.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if rest.is_empty() {
            return Vec::new();
        }
        pretty_json(&Value::Object(rest))
    }
}

/// Every line, so a trailing newline or a carriage return stays visible.
fn split_lines(text: &str, mark: &str) -> Vec<String> {
    text.split('\n')
        .map(|line| format!("{mark}{line}"))
        .collect()
}

fn pretty_json(value: &Value) -> Vec<String> {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|_| value.to_string())
        .lines()
        .map(str::to_owned)
        .collect()
}
