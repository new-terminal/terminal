//! The scrollback: labeled rows, newest at the bottom, each one selectable.
//! A source's consecutive rows form a group that shows its label once. A
//! brought-up permission shows as the request block, whose buttons answer
//! it while reply mode is on.

use std::collections::VecDeque;

use gpui_kit::base::{TextView, TextViewStyle, Theme};
use gpui_kit::{
    AnyElement, App, Context, Div, EventEmitter, FontStyle, HighlightStyle, Hsla, IntoElement,
    ListAlignment, ListState, ParentElement as _, Pixels, Render, SharedString, Styled as _,
    Window, div, list, px, relative, rems, transparent_black,
};
use nt_core::{LineKind, Source};

use crate::markup;
use crate::palette::Palette;
use crate::request::{self, Choice, Question};
use crate::typeface::{MONO, SANS};

/// Rows past this many drop from the top, so memory stays bounded however
/// long the app runs.
const MAX_ROWS: usize = 5_000;
/// Rows laid out beyond each edge of the view, so a scroll shows no blank.
const OVERDRAW: Pixels = px(200.);
const APP_LABEL: &str = "nt";
const ECHO_GLYPH: &str = "›";
/// The tool whose line shows its command in a band.
const BAND_TOOL: &str = "Bash";

const LABEL_WIDTH: f32 = 96.;
const LABEL_GAP: f32 = 24.;
const COLUMN_WIDTH: f32 = 880.;
const LABEL_SIZE: f32 = 12.;
const LABEL_LINE: f32 = 20.;
const ECHO_SIZE: f32 = 20.;
const ECHO_LINE: f32 = 28.;
const PROSE_SIZE: f32 = 16.;
const PROSE_LINE: f32 = 1.55;
const PROSE_WIDTH: f32 = 720.;
const PROSE_PAD_Y: f32 = 2.;
const NOTE_SIZE: f32 = 14.;
const NOTE_LINE: f32 = 22.;
const MONO_SIZE: f32 = 13.;
const MONO_LINE: f32 = 20.;
const MARK_GAP: f32 = 8.;
const BAND_PAD_X: f32 = 12.;
const BAND_PAD_Y: f32 = 6.;
const BAND_RADIUS: f32 = 4.;
const BAND_GAP: f32 = 10.;
/// Space above a row: before what the author typed, before another
/// source's rows, between rows of one source, and around the band and the
/// request block.
const TURN_GAP: f32 = 20.;
const GROUP_GAP: f32 = 8.;
const ROW_GAP: f32 = 4.;
const NOTE_GAP: f32 = 2.;
const BLOCK_GAP: f32 = 10.;
/// Moves a label down to sit level with its row's first line.
const NOTE_LABEL_DROP: f32 = 1.;
const PROSE_LABEL_DROP: f32 = 4.;
const BLOCK_LABEL_DROP: f32 = 14.;

#[derive(Clone, Copy, Debug)]
enum Tone {
    Ink,
    Muted,
    Success,
    Error,
    Warning,
}

impl Tone {
    const fn color(self, palette: &Palette) -> Hsla {
        match self {
            Self::Ink => palette.ink,
            Self::Muted => palette.muted,
            Self::Success => palette.success,
            Self::Error => palette.error,
            Self::Warning => palette.warning,
        }
    }
}

/// What one row shows. Text is Markdown that shows the source text as sent.
#[derive(Debug)]
enum Body {
    /// What the author typed.
    Echo(SharedString),
    /// Text the agent wrote.
    Prose(SharedString),
    /// An app, warning, or error line, in the sans family.
    Note {
        markdown: SharedString,
        tone: Tone,
        italic: bool,
    },
    /// A tool, status, or attention line, in the mono family.
    Mono {
        markdown: SharedString,
        tone: Tone,
        marked: bool,
    },
    /// A tool line that carries a command.
    Band {
        tool: SharedString,
        command: SharedString,
    },
    /// One row of request block `request`.
    Request { request: u64, part: Part },
}

#[derive(Debug)]
enum Part {
    Head(SharedString),
    Summary(Question),
    Line {
        markdown: SharedString,
        first: bool,
        last: bool,
    },
    Foot,
}

#[derive(Debug)]
enum Label {
    Hidden,
    Echo,
    Source(SharedString),
}

/// Who a run of rows speaks for. A new run starts a group.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Speaker {
    Author,
    Source(String),
}

#[derive(Debug)]
struct Row {
    /// Unique for the life of the app, so each row keeps its own text state
    /// and selection while rows above it drop.
    id: u64,
    label: Label,
    gap: Pixels,
    body: Body,
}

/// The newest push, kept while it could be a request block's text: the core
/// sends a bring-up's block as an app line from the target just before the
/// prompt event that starts reply mode.
#[derive(Debug)]
struct Pending {
    target: String,
    text: String,
    rows: usize,
    speaker_before: Option<Speaker>,
}

pub struct Scrollback {
    rows: VecDeque<Row>,
    next_id: u64,
    next_request: u64,
    list: ListState,
    speaker: Option<Speaker>,
    pending: Option<Pending>,
    /// The request block whose buttons answer, while reply mode is on.
    answering: Option<u64>,
}

impl EventEmitter<Choice> for Scrollback {}

impl Scrollback {
    pub fn new() -> Self {
        Self {
            rows: VecDeque::new(),
            next_id: 0,
            next_request: 0,
            list: ListState::new(0, ListAlignment::Bottom, OVERDRAW),
            speaker: None,
            pending: None,
            answering: None,
        }
    }

    /// Adds a line from the core, labeled with its source.
    pub fn push_line(&mut self, source: &Source, kind: LineKind, text: &str) {
        let (name, from_app) = match source {
            Source::App => (APP_LABEL, true),
            Source::Target(name) => (name.as_str(), false),
        };
        let prefix = match kind {
            LineKind::Attention => format!("? {name}  "),
            LineKind::Done => format!("✓ {name} "),
            LineKind::Stopped => format!("■ {name} "),
            LineKind::Failed => format!("✗ {name} failed: "),
            _ => String::new(),
        };
        let indent = " ".repeat(prefix.chars().count());
        let bodies = lines(text)
            .enumerate()
            .map(|(index, line)| {
                let lead = if index == 0 { &prefix } else { &indent };
                body(kind, from_app, &format!("{lead}{line}"))
            })
            .collect();
        let speaker_before = self.speaker.clone();
        let rows = self.push(Speaker::Source(name.to_owned()), bodies);
        self.pending = (kind == LineKind::App && !from_app).then(|| Pending {
            target: name.to_owned(),
            text: text.to_owned(),
            rows,
            speaker_before,
        });
    }

    /// Adds the author's own submitted text.
    pub fn push_echo(&mut self, text: &str) {
        let bodies = lines(text)
            .map(|line| Body::Echo(markup::echo(line).into()))
            .collect();
        self.push(Speaker::Author, bodies);
        self.pending = None;
    }

    /// Shows the permission that reply mode answers as the request block,
    /// and lets its buttons answer. When the newest push is `target`'s
    /// block text, the block takes its place and shows that text.
    pub fn raise_request(&mut self, target: &str, question: &str) {
        let text = match self.pending.take() {
            Some(pending) if pending.target == target && pending.rows <= self.rows.len() => {
                let start = self.rows.len() - pending.rows;
                self.rows.truncate(start);
                self.list.splice(start..start + pending.rows, 0);
                self.speaker = pending.speaker_before;
                Some(pending.text)
            }
            _ => None,
        };
        let request = self.next_request;
        self.next_request += 1;
        let mut parts = vec![
            Part::Head(target.to_owned().into()),
            Part::Summary(Question::parse(question)),
        ];
        if let Some(text) = &text {
            let count = lines(text).count();
            parts.extend(lines(text).enumerate().map(|(index, line)| Part::Line {
                markdown: markup::plain(line).into(),
                first: index == 0,
                last: index + 1 == count,
            }));
        }
        parts.push(Part::Foot);
        let bodies = parts
            .into_iter()
            .map(|part| Body::Request { request, part })
            .collect();
        self.push(Speaker::Source(target.to_owned()), bodies);
        self.answering = Some(request);
    }

    /// Takes the buttons away once reply mode ends.
    pub const fn settle_request(&mut self) {
        self.answering = None;
    }

    /// Appends `bodies` as one push by `speaker`, and returns how many rows
    /// it added. Each echo starts its own group, so every request the author
    /// types begins a new turn.
    fn push(&mut self, speaker: Speaker, bodies: Vec<Body>) -> usize {
        let starts_group = speaker == Speaker::Author || self.speaker.as_ref() != Some(&speaker);
        let first_new = self.rows.len();
        for (index, body) in bodies.into_iter().enumerate() {
            let label = match (&speaker, index) {
                (Speaker::Author, 0) => Label::Echo,
                (Speaker::Source(name), 0) if starts_group => Label::Source(name.clone().into()),
                _ => Label::Hidden,
            };
            let gap = if self.rows.is_empty() {
                px(0.)
            } else {
                px(gap_above(&body, index == 0 && starts_group))
            };
            self.rows.push_back(Row {
                id: self.next_id,
                label,
                gap,
                body,
            });
            self.next_id += 1;
        }
        self.speaker = Some(speaker);
        let added = self.rows.len() - first_new;
        self.list.splice(first_new..first_new, added);

        let excess = self.rows.len().saturating_sub(MAX_ROWS);
        if excess > 0 {
            self.rows.drain(..excess);
            self.list.splice(0..excess, 0);
        }
        added
    }

    fn render_row(&self, index: usize, cx: &Context<'_, Self>) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let palette = *cx.global::<Palette>();
        let content = self.render_body(row, &palette, cx);
        div()
            .flex()
            .w_full()
            .gap(px(LABEL_GAP))
            .pt(row.gap)
            .child(label_cell(&row.label, &row.body, &palette))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(px(COLUMN_WIDTH))
                    .child(content),
            )
            .into_any_element()
    }

    fn render_body(&self, row: &Row, palette: &Palette, cx: &Context<'_, Self>) -> Div {
        let text = |markdown: &SharedString, color: Hsla, cx: &App| {
            text_view(row.id, markdown.clone(), color, cx)
        };
        match &row.body {
            Body::Echo(markdown) => div()
                .font_family(SANS)
                .text_size(px(ECHO_SIZE))
                .line_height(px(ECHO_LINE))
                .child(text(markdown, palette.ink, cx)),
            Body::Prose(markdown) => div()
                .max_w(px(PROSE_WIDTH))
                .py(px(PROSE_PAD_Y))
                .font_family(SANS)
                .text_size(px(PROSE_SIZE))
                .line_height(relative(PROSE_LINE))
                .child(text(markdown, palette.ink, cx)),
            Body::Note {
                markdown,
                tone,
                italic,
            } => {
                let note = div()
                    .font_family(SANS)
                    .text_size(px(NOTE_SIZE))
                    .line_height(px(NOTE_LINE));
                let note = if *italic { note.italic() } else { note };
                note.child(text(markdown, tone.color(palette), cx))
            }
            Body::Mono {
                markdown,
                tone,
                marked,
            } => {
                let line = mono().child(text(markdown, tone.color(palette), cx));
                if *marked {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(MARK_GAP))
                        .child(request::mark(palette))
                        .child(line)
                } else {
                    line
                }
            }
            Body::Band { tool, command } => div().flex().child(
                mono()
                    .flex()
                    .gap(px(BAND_GAP))
                    .px(px(BAND_PAD_X))
                    .py(px(BAND_PAD_Y))
                    .rounded(px(BAND_RADIUS))
                    .bg(palette.band)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(palette.muted)
                            .child(tool.clone()),
                    )
                    .child(div().min_w_0().child(text(command, palette.ink, cx))),
            ),
            Body::Request { request, part } => match part {
                Part::Head(target) => request::head(target, palette),
                Part::Summary(question) => request::summary(question, palette),
                Part::Line {
                    markdown,
                    first,
                    last,
                } => request::well_line(
                    text(markdown, palette.block_ink, cx),
                    *first,
                    *last,
                    palette,
                ),
                Part::Foot => request::foot(
                    *request,
                    self.answering == Some(*request),
                    palette,
                    |choice| cx.listener(move |_, _, _, cx| cx.emit(choice)),
                ),
            },
        }
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

/// The lines of `text`, without a trailing carriage return.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

fn body(kind: LineKind, from_app: bool, line: &str) -> Body {
    let note = |tone, italic| Body::Note {
        markdown: markup::with_code(line).into(),
        tone,
        italic,
    };
    let mono = |tone, marked| Body::Mono {
        markdown: markup::plain(line).into(),
        tone,
        marked,
    };
    match kind {
        LineKind::App if from_app => note(Tone::Muted, true),
        LineKind::Warning => note(Tone::Warning, true),
        LineKind::Error => note(Tone::Error, false),
        LineKind::AgentText => Body::Prose(markup::plain(line).into()),
        LineKind::Tool => match line
            .strip_prefix(BAND_TOOL)
            .and_then(|rest| rest.strip_prefix(' '))
        {
            Some(command) => Body::Band {
                tool: BAND_TOOL.into(),
                command: markup::plain(command).into(),
            },
            None => mono(Tone::Muted, false),
        },
        LineKind::App | LineKind::Stopped => mono(Tone::Muted, false),
        LineKind::Attention => mono(Tone::Ink, true),
        LineKind::Done => mono(Tone::Success, false),
        LineKind::Failed => mono(Tone::Error, false),
    }
}

const fn gap_above(body: &Body, starts_group: bool) -> f32 {
    match body {
        Body::Echo(_) if starts_group => TURN_GAP,
        Body::Echo(_) => 0.,
        _ if starts_group => GROUP_GAP,
        Body::Note { .. } => NOTE_GAP,
        Body::Band { .. }
        | Body::Request {
            part: Part::Head(_),
            ..
        } => BLOCK_GAP,
        Body::Request { .. } => 0.,
        Body::Prose(_) | Body::Mono { .. } => ROW_GAP,
    }
}

fn label_cell(label: &Label, body: &Body, palette: &Palette) -> Div {
    let cell = div()
        .w(px(LABEL_WIDTH))
        .flex_shrink_0()
        .text_right()
        .truncate()
        .text_color(palette.muted);
    match label {
        Label::Hidden => cell,
        Label::Echo => cell
            .font_family(SANS)
            .text_size(px(ECHO_SIZE))
            .line_height(px(ECHO_LINE))
            .child(ECHO_GLYPH),
        Label::Source(name) => cell
            .pt(px(label_drop(body)))
            .font_family(MONO)
            .text_size(px(LABEL_SIZE))
            .line_height(px(LABEL_LINE))
            .child(name.clone()),
    }
}

const fn label_drop(body: &Body) -> f32 {
    match body {
        Body::Note { .. } => NOTE_LABEL_DROP,
        Body::Prose(_) => PROSE_LABEL_DROP,
        Body::Band { .. } => BAND_PAD_Y,
        Body::Request { .. } => BLOCK_LABEL_DROP,
        Body::Echo(_) | Body::Mono { .. } => 0.,
    }
}

fn mono() -> Div {
    div()
        .font_family(MONO)
        .text_size(px(MONO_SIZE))
        .line_height(px(MONO_LINE))
}

/// A selectable view of `markdown`. Inline code stays upright and unshaded,
/// so a path reads as part of its sentence.
fn text_view(id: u64, markdown: SharedString, color: Hsla, cx: &App) -> AnyElement {
    let style = TextViewStyle::from_theme(&Theme::global(cx))
        .with_foreground(color)
        .with_paragraph_gap(rems(0.))
        .with_inline_code(HighlightStyle {
            font_style: Some(FontStyle::Normal),
            background_color: Some(transparent_black()),
            ..HighlightStyle::default()
        });
    TextView::markdown(("scrollback-row", id), markdown)
        .style(style)
        .selectable(true)
        .into_any_element()
}
