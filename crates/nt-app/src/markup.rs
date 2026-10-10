//! Turns one line of core or author text into the Markdown that a
//! scrollback text view parses. The text always shows exactly as sent: every
//! ASCII punctuation mark is escaped, and only the spans named here gain a
//! style.

const NO_BREAK_SPACE: char = '\u{a0}';
const CODE_MARK: char = '`';
const MENTION_MARK: char = '@';
/// Git prints a short commit with at least 7 hex digits; a full one has 40.
const COMMIT_DIGITS: std::ops::RangeInclusive<usize> = 7..=40;
/// Punctuation that ends a sentence or a clause, not a path.
const TRAILING_PUNCTUATION: &[char] = &['.', ',', ':', ';', ')'];
const LEADING_PUNCTUATION: &[char] = &['('];

/// `line` with no added style.
pub fn plain(line: &str) -> String {
    let mut markdown = String::with_capacity(line.len());
    push_escaped(&mut markdown, line);
    keep_visible(markdown)
}

/// An echo line: every `@name` token is strong.
pub fn echo(line: &str) -> String {
    let mut markdown = String::with_capacity(line.len());
    for (word, is_space) in Words::new(line) {
        let is_mention = !is_space && word.len() > 1 && word.starts_with(MENTION_MARK);
        if is_mention {
            markdown.push_str("**");
            push_escaped(&mut markdown, word);
            markdown.push_str("**");
        } else {
            push_escaped(&mut markdown, word);
        }
    }
    keep_visible(markdown)
}

/// An app line: paths, commits, and branch names are inline code, which
/// the scrollback sets in the mono family.
pub fn with_code(line: &str) -> String {
    let mut markdown = String::with_capacity(line.len());
    let mut preceding = Preceding::default();
    for (word, is_space) in Words::new(line) {
        if is_space {
            push_escaped(&mut markdown, word);
            continue;
        }
        let after_lead = word.trim_start_matches(LEADING_PUNCTUATION);
        let core = after_lead.trim_end_matches(TRAILING_PUNCTUATION);
        let code = !core.is_empty() && !core.contains(CODE_MARK) && preceding.is_code(core);
        if code {
            let lead = &word[..word.len() - after_lead.len()];
            let tail = &after_lead[core.len()..];
            push_escaped(&mut markdown, lead);
            markdown.push(CODE_MARK);
            markdown.push_str(core);
            markdown.push(CODE_MARK);
            push_escaped(&mut markdown, tail);
        } else {
            push_escaped(&mut markdown, word);
        }
        preceding = preceding.after(core, code);
    }
    keep_visible(markdown)
}

/// The two words before the one being judged, as far as [`with_code`]
/// needs them.
#[derive(Clone, Copy, Default)]
struct Preceding<'a> {
    last: &'a str,
    last_is_code: bool,
    code_before_last: bool,
}

impl<'a> Preceding<'a> {
    const fn after(self, word: &'a str, is_code: bool) -> Self {
        Self {
            last: word,
            last_is_code: is_code,
            code_before_last: self.last_is_code,
        }
    }

    /// A path or branch (it has a `/` or starts at `~`), a commit, the
    /// name after "branch", or the branch a code word was made "from".
    fn is_code(self, word: &str) -> bool {
        let is_path = word.contains('/') || word.starts_with('~');
        let is_commit = COMMIT_DIGITS.contains(&word.len())
            && word
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
            && word.chars().any(|c| c.is_ascii_digit())
            && word.chars().any(|c| c.is_ascii_alphabetic());
        let names_a_branch =
            self.last == "branch" || (self.last == "from" && self.code_before_last);
        is_path || is_commit || names_a_branch
    }
}

fn push_escaped(markdown: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            markdown.push('\\');
        }
        markdown.push(c);
    }
}

/// Markdown drops leading spaces and an empty paragraph has no height, so
/// leading spaces and tabs become no-break spaces and an empty line becomes
/// one.
fn keep_visible(markdown: String) -> String {
    let body = markdown.trim_start_matches([' ', '\t']);
    let indent = markdown.len() - body.len();
    if indent == 0 && !body.is_empty() {
        return markdown;
    }
    let mut kept = String::with_capacity(markdown.len() + indent);
    kept.extend(std::iter::repeat_n(
        NO_BREAK_SPACE,
        indent.max(usize::from(body.is_empty())),
    ));
    kept.push_str(body);
    kept
}

/// Splits a line into runs of whitespace and runs of anything else, in
/// order, each with whether it is whitespace.
struct Words<'a> {
    rest: &'a str,
}

impl<'a> Words<'a> {
    const fn new(line: &'a str) -> Self {
        Self { rest: line }
    }
}

impl<'a> Iterator for Words<'a> {
    type Item = (&'a str, bool);

    fn next(&mut self) -> Option<Self::Item> {
        let first = self.rest.chars().next()?;
        let is_space = first.is_whitespace();
        let end = self
            .rest
            .find(|c: char| c.is_whitespace() != is_space)
            .unwrap_or(self.rest.len());
        let (word, rest) = self.rest.split_at(end);
        self.rest = rest;
        Some((word, is_space))
    }
}
