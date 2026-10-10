//! The request block: a brought-up permission drawn as one solid block in
//! the scrollback, with the request in full and, while reply mode is on,
//! buttons that answer it. The scrollback lays the block out as rows, so a
//! long request scrolls like any other text.

use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};

use crate::palette::Palette;
use crate::typeface::{MONO, SANS};

const PAD_X: f32 = 20.;
const PAD_TOP: f32 = 14.;
const PAD_BOTTOM: f32 = 16.;
const GAP: f32 = 12.;
const RADIUS: f32 = 6.;
const HEAD_GAP: f32 = 10.;
const HEAD_SIZE: f32 = 12.;
const HEAD_LINE: f32 = 20.;
const SUMMARY_SIZE: f32 = 15.;
const SUMMARY_LINE: f32 = 22.;
/// About one space at the summary size.
const SUMMARY_WORD_GAP: f32 = 4.;
const CODE_SIZE: f32 = 14.;
const WELL_LINE: f32 = 22.;
const WELL_PAD_X: f32 = 12.;
const WELL_PAD_Y: f32 = 8.;
const WELL_RADIUS: f32 = 4.;
const BUTTONS_GAP: f32 = 22.;
const BUTTON_HINT_GAP: f32 = 8.;
const BUTTON_SIZE: f32 = 13.;
const BUTTON_LINE: f32 = 20.;
const BUTTON_PAD_X: f32 = 14.;
const BUTTON_PAD_Y: f32 = 4.;
const BUTTON_RADIUS: f32 = 5.;
const HINT_SIZE: f32 = 12.;
const MARK_WIDTH: f32 = 7.;
const MARK_HEIGHT: f32 = 12.;
const NEEDS_YOU: &str = "needs you";
/// The tool name and its line count frame the question's detail.
const HEAD_PREFIX: &str = "Allow ";
const HEAD_END: &str = ": ";
const COUNT_START: &str = " (";

/// The answer a key or a button gives in reply mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Allow,
    Deny,
    Later,
}

impl Choice {
    const ALL: [Self; 3] = [Self::Allow, Self::Deny, Self::Later];

    const fn word(self) -> &'static str {
        match self {
            Self::Allow => "yes",
            Self::Deny => "no",
            Self::Later => "later",
        }
    }

    /// The key that gives the same answer, as the hint beside the button.
    const fn key(self) -> &'static str {
        match self {
            Self::Allow => "y",
            Self::Deny => "n",
            Self::Later => "esc",
        }
    }

    const fn element_name(self) -> &'static str {
        match self {
            Self::Allow => "request-yes",
            Self::Deny => "request-no",
            Self::Later => "request-later",
        }
    }
}

/// The question split for display: `Allow Bash:`, the command or path, and
/// `(1 line)`. A question in another form shows whole as the detail.
#[derive(Clone, Debug)]
pub struct Question {
    head: SharedString,
    detail: SharedString,
    count: SharedString,
}

impl Question {
    pub fn parse(question: &str) -> Self {
        let (rest, count) = match question.rfind(COUNT_START) {
            Some(start) if question.ends_with(')') => {
                (&question[..start], question[start..].trim_start())
            }
            _ => (question, ""),
        };
        let (head, detail) = match rest.split_once(HEAD_END) {
            Some((head, detail)) if head.starts_with(HEAD_PREFIX) => (format!("{head}:"), detail),
            _ => (String::new(), rest),
        };
        Self {
            head: head.into(),
            detail: detail.to_owned().into(),
            count: count.to_owned().into(),
        }
    }
}

/// The small solid mark that flags what waits for the author.
pub fn mark(palette: &Palette) -> Div {
    div()
        .flex_shrink_0()
        .w(px(MARK_WIDTH))
        .h(px(MARK_HEIGHT))
        .bg(palette.accent)
}

/// The block's top: the mark, "needs you", and the target.
pub fn head(target: &SharedString, palette: &Palette) -> Div {
    shell(palette)
        .rounded_t(px(RADIUS))
        .pt(px(PAD_TOP))
        .flex()
        .items_center()
        .gap(px(HEAD_GAP))
        .font_family(MONO)
        .text_size(px(HEAD_SIZE))
        .line_height(px(HEAD_LINE))
        .child(mark(palette))
        .child(div().text_color(palette.attention).child(NEEDS_YOU))
        .child(div().text_color(palette.block_muted).child(target.clone()))
}

pub fn summary(question: &Question, palette: &Palette) -> Div {
    let mut words = div()
        .flex()
        .flex_wrap()
        .items_baseline()
        .gap_x(px(SUMMARY_WORD_GAP));
    if !question.head.is_empty() {
        words = words.child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(question.head.clone()),
        );
    }
    words = words.child(
        div()
            .font_family(MONO)
            .text_size(px(CODE_SIZE))
            .child(question.detail.clone()),
    );
    if !question.count.is_empty() {
        words = words.child(
            div()
                .text_color(palette.block_muted)
                .child(question.count.clone()),
        );
    }
    shell(palette)
        .pt(px(GAP))
        .font_family(SANS)
        .text_size(px(SUMMARY_SIZE))
        .line_height(px(SUMMARY_LINE))
        .child(words)
}

/// One line of the request text, inside the block's tinted well.
pub fn well_line(text: AnyElement, first: bool, last: bool, palette: &Palette) -> Div {
    let mut well = div()
        .bg(palette.block_band)
        .px(px(WELL_PAD_X))
        .font_family(MONO)
        .text_size(px(CODE_SIZE))
        .line_height(px(WELL_LINE))
        .child(text);
    let mut row = shell(palette);
    if first {
        well = well.rounded_t(px(WELL_RADIUS)).pt(px(WELL_PAD_Y));
        row = row.pt(px(GAP));
    }
    if last {
        well = well.rounded_b(px(WELL_RADIUS)).pb(px(WELL_PAD_Y));
    }
    row.child(well)
}

/// The block's bottom, with the answer buttons while they answer.
/// `listener` makes each button's click handler.
pub fn foot<L>(
    request: u64,
    answering: bool,
    palette: &Palette,
    listener: impl Fn(Choice) -> L,
) -> Div
where
    L: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let foot = shell(palette).rounded_b(px(RADIUS)).pb(px(PAD_BOTTOM));
    if !answering {
        return foot;
    }
    let buttons = Choice::ALL.into_iter().map(|choice| {
        let border = if choice == Choice::Allow {
            palette.block_ink
        } else {
            palette.block_line
        };
        div()
            .flex()
            .items_center()
            .gap(px(BUTTON_HINT_GAP))
            .child(
                div()
                    .id(ElementId::from((choice.element_name(), request)))
                    .px(px(BUTTON_PAD_X))
                    .py(px(BUTTON_PAD_Y))
                    .rounded(px(BUTTON_RADIUS))
                    .border_1()
                    .border_color(border)
                    .font_family(SANS)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(BUTTON_SIZE))
                    .line_height(px(BUTTON_LINE))
                    .cursor_pointer()
                    .on_click(listener(choice))
                    .child(choice.word()),
            )
            .child(
                div()
                    .font_family(MONO)
                    .text_size(px(HINT_SIZE))
                    .text_color(palette.block_muted)
                    .child(choice.key()),
            )
    });
    foot.pt(px(GAP)).child(
        div()
            .flex()
            .items_center()
            .gap(px(BUTTONS_GAP))
            .children(buttons),
    )
}

fn shell(palette: &Palette) -> Div {
    div()
        .bg(palette.block)
        .text_color(palette.block_ink)
        .px(px(PAD_X))
}
