//! The scrollback: labeled lines, newest at the end, each one selectable.

use std::collections::VecDeque;

use gpui_kit::base::{TextView, TextViewStyle, Theme};
use gpui_kit::{
    AnyElement, App, Context, FollowMode, IntoElement, ListAlignment, ListState, Pixels, Render,
    SharedString, Styled as _, Window, div, list, px, rems,
};
use nt_core::{LineKind, Source};

use crate::palette::Palette;

/// Rows past this many drop from the top, so memory stays bounded however
/// long the app runs.
const MAX_ROWS: usize = 5_000;
/// Rows laid out beyond each edge of the view, so a scroll shows no blank.
const OVERDRAW: Pixels = px(200.);
const ECHO_PREFIX: &str = "› ";
const NO_BREAK_SPACE: char = '\u{a0}';

#[derive(Clone, Copy, Debug)]
enum Tone {
    Text,
    Error,
}

#[derive(Debug)]
struct Row {
    /// Unique for the life of the app, so each row keeps its own text state
    /// and selection while rows above it drop.
    id: u64,
    markdown: SharedString,
    tone: Tone,
}

pub struct Scrollback {
    rows: VecDeque<Row>,
    next_id: u64,
    list: ListState,
}

impl Scrollback {
    pub fn new() -> Self {
        let list = ListState::new(0, ListAlignment::Top, OVERDRAW);
        list.set_follow_mode(FollowMode::Tail);
        Self {
            rows: VecDeque::new(),
            next_id: 0,
            list,
        }
    }

    /// Adds a line from the core, labeled with its source.
    pub fn push_line(&mut self, source: &Source, kind: LineKind, text: &str) {
        let label = match source {
            Source::App => "nt",
            Source::Target(name) => name,
        };
        let tone = match kind {
            LineKind::App => Tone::Text,
            LineKind::Error => Tone::Error,
        };
        self.push(&format!("{label} │ "), text, tone);
    }

    /// Adds the author's own submitted text.
    pub fn push_echo(&mut self, text: &str) {
        self.push(ECHO_PREFIX, text, Tone::Text);
    }

    /// One row per line of `text`. The first row starts with `prefix`, and
    /// later rows are indented to line up under it.
    fn push(&mut self, prefix: &str, text: &str, tone: Tone) {
        let indent = " ".repeat(prefix.chars().count());
        let first_new = self.rows.len();
        for (index, line) in text.split('\n').enumerate() {
            let lead = if index == 0 { prefix } else { &indent };
            let line = line.strip_suffix('\r').unwrap_or(line);
            self.rows.push_back(Row {
                id: self.next_id,
                markdown: as_plain_markdown(&format!("{lead}{line}")).into(),
                tone,
            });
            self.next_id += 1;
        }
        self.list
            .splice(first_new..first_new, self.rows.len() - first_new);

        let excess = self.rows.len().saturating_sub(MAX_ROWS);
        if excess > 0 {
            self.rows.drain(..excess);
            self.list.splice(0..excess, 0);
        }
    }

    fn render_row(&self, index: usize, cx: &App) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let palette = cx.global::<Palette>();
        let color = match row.tone {
            Tone::Text => palette.text,
            Tone::Error => palette.error,
        };
        let style = TextViewStyle::from_theme(&Theme::global(cx))
            .with_foreground(color)
            .with_paragraph_gap(rems(0.));
        TextView::markdown(("scrollback-row", row.id), row.markdown.clone())
            .style(style)
            .selectable(true)
            .into_any_element()
    }
}

impl Render for Scrollback {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        list(
            self.list.clone(),
            cx.processor(|this, index: usize, _, cx| this.render_row(index, cx)),
        )
        .size_full()
    }
}

/// Markdown that shows `text` exactly as typed. The row view parses
/// Markdown, so every ASCII punctuation mark is escaped, and leading spaces
/// and tabs become no-break spaces, which Markdown does not strip.
fn as_plain_markdown(text: &str) -> String {
    let mut markdown = String::with_capacity(text.len());
    let mut leading = true;
    for c in text.chars() {
        if leading && (c == ' ' || c == '\t') {
            markdown.push(NO_BREAK_SPACE);
            continue;
        }
        leading = false;
        if c.is_ascii_punctuation() {
            markdown.push('\\');
        }
        markdown.push(c);
    }
    markdown
}
