//! The root view: the scrollback, the prompt, and the status bar, top to
//! bottom, and the speed metrics. In reply mode the scrollback shows the
//! waiting permission as the request block, and the keys and the block's
//! buttons take only its answer.

use std::time::Instant;

use gpui_kit::base::input::{InputEditorStyle, InputEvent, Textarea, TextareaState};
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _, IntoElement,
    Keystroke, ParentElement as _, Render, Styled as _, Subscription, Task, Window, div, px,
};
use nt_core::{
    CoreHandle, Counts, Decision, Event, Events, Label, LineKind, Metric, Reply, Source,
};

use crate::palette::Palette;
use crate::request::{self, Choice};
use crate::scrollback::Scrollback;
use crate::typeface::{MONO, SANS};

const PROMPT_MIN_ROWS: usize = 1;
const PROMPT_MAX_ROWS: usize = 8;
const REPLY_HINT: &str = "Answer with y or n, or press Esc.";
const MARGIN_X: f32 = 56.;
const SCROLLBACK_PAD_TOP: f32 = 14.;
const SCROLLBACK_PAD_BOTTOM: f32 = 8.;
const PROMPT_PAD_TOP: f32 = 6.;
const PROMPT_WIDTH: f32 = 1000.;
const PROMPT_GAP: f32 = 10.;
const PROMPT_ROW_PAD_TOP: f32 = 8.;
const PROMPT_ROW_PAD_BOTTOM: f32 = 10.;
const PROMPT_SIZE: f32 = 15.;
const PROMPT_LINE: f32 = 24.;
const CARET_WIDTH: f32 = 9.;
const CARET_HEIGHT: f32 = 20.;
/// Centers the caret block on the prompt's first line.
const CARET_DROP: f32 = (PROMPT_LINE - CARET_HEIGHT) / 2.;
const STATUS_GAP: f32 = 28.;
const STATUS_PAD_TOP: f32 = 10.;
const STATUS_PAD_BOTTOM: f32 = 14.;
const STATUS_SIZE: f32 = 12.;
const STATUS_LINE: f32 = 20.;
const STATUS_MARK_GAP: f32 = 8.;

pub struct Root {
    core: CoreHandle,
    prompt: Entity<TextareaState>,
    scrollback: Entity<Scrollback>,
    label: Label,
    /// Set while the prompt is in reply mode.
    reply: Option<Reply>,
    counts: Counts,
    launched: Instant,
    cold_start_sent: bool,
    /// Set while the window shows the demo scene, which core events must
    /// not change, and whose prompt and `Tab` send nothing to the core.
    #[cfg(debug_assertions)]
    demo: bool,
    _subscriptions: Vec<Subscription>,
    _pump: Task<()>,
}

impl Root {
    pub fn new(
        core: CoreHandle,
        events: Events,
        launched: Instant,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        // The input's own caret takes the accent, like the caret block before
        // it. Every other color stays unset, so it follows the theme.
        let accent = cx.global::<Palette>().accent;
        let prompt = cx.new(|cx| {
            let mut prompt = TextareaState::new(window, cx)
                .submit_on_enter(true)
                .auto_grow(PROMPT_MIN_ROWS, PROMPT_MAX_ROWS);
            prompt.set_editor_style(InputEditorStyle {
                caret: accent,
                ..InputEditorStyle::default()
            });
            prompt
        });
        let on_enter = cx.subscribe_in(
            &prompt,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    this.submit(window, cx);
                }
            },
        );
        let keypress_core = core.clone();
        let on_key = cx.observe_keystrokes(move |_, _, window, _| {
            let pressed = Instant::now();
            let core = keypress_core.clone();
            window.on_next_frame(move |_, _| {
                core.metric(Metric::KeypressToFrame(pressed.elapsed()));
            });
        });
        let root = cx.entity().downgrade();
        let on_answer_key = cx.intercept_keystrokes(move |event, window, cx| {
            let handled = root
                .update(cx, |root, cx| root.take_key(&event.keystroke, window, cx))
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        let scrollback = cx.new(|_| Scrollback::new());
        let on_button = cx.subscribe_in(
            &scrollback,
            window,
            |this, _, choice: &Choice, window, cx| this.choose(*choice, window, cx),
        );
        let pump = cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.apply(event, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            core,
            prompt,
            scrollback,
            label: Label::NoTarget,
            reply: None,
            counts: Counts::default(),
            launched,
            cold_start_sent: false,
            #[cfg(debug_assertions)]
            demo: false,
            _subscriptions: vec![on_enter, on_key, on_answer_key, on_button],
            _pump: pump,
        }
    }

    pub fn focus_prompt(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        window.focus(&self.prompt_focus(cx), cx);
    }

    /// Fills the window with the demo scene and keeps core events from
    /// changing it.
    #[cfg(debug_assertions)]
    pub fn show_demo(&mut self, cx: &mut Context<'_, Self>) {
        self.demo = true;
        self.label = crate::demo::label();
        self.counts = crate::demo::counts();
        self.scrollback.update(cx, |scrollback, cx| {
            crate::demo::stage(scrollback);
            cx.notify();
        });
        cx.notify();
    }

    fn prompt_focus(&self, cx: &App) -> FocusHandle {
        self.prompt.focus_handle(cx)
    }

    fn submit(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        #[cfg(debug_assertions)]
        if self.demo {
            return;
        }
        let text = self.prompt.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let submitted = Instant::now();
        self.scrollback.update(cx, |scrollback, cx| {
            scrollback.push_echo(&text);
            cx.notify();
        });
        self.prompt
            .update(cx, |prompt, cx| prompt.set_value("", window, cx));
        self.core.submit(text);
        let core = self.core.clone();
        window.on_next_frame(move |_, _| {
            core.metric(Metric::SubmitToEcho(submitted.elapsed()));
        });
        cx.notify();
    }

    /// Handles the keys that reply mode and `Tab` own, before the input
    /// sees them. Returns whether the key was taken. `⌘` keys always pass,
    /// so `⌘.` still stops every agent and `⌘Q` still quits.
    fn take_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if keystroke.modifiers.platform {
            return false;
        }
        #[cfg(debug_assertions)]
        if self.demo {
            return false;
        }
        let plain = !keystroke.modifiers.modified();
        if self.reply.is_none() {
            let empty = self.prompt.read(cx).value().is_empty();
            if keystroke.key == "tab" && plain && empty {
                self.core.bring_up();
                return true;
            }
            return false;
        }
        let choice = match keystroke.key.as_str() {
            "y" if plain => Choice::Allow,
            "n" if plain => Choice::Deny,
            "escape" if plain => Choice::Later,
            _ => {
                self.push_line(&Source::App, LineKind::App, REPLY_HINT, cx);
                return true;
            }
        };
        self.choose(choice, window, cx);
        true
    }

    /// Answers the permission reply mode shows, the same way from a key or
    /// a button. Does nothing outside reply mode.
    fn choose(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(reply) = &self.reply else {
            return;
        };
        let item = reply.item;
        match choice {
            Choice::Allow => self.core.answer(item, Decision::Allow),
            Choice::Deny => self.core.answer(item, Decision::Deny),
            Choice::Later => self.core.later(item),
        }
        self.leave_reply_mode(window, cx);
    }

    /// Takes the request block's buttons away, and focuses the input again
    /// with the text it held.
    fn leave_reply_mode(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.reply = None;
        self.scrollback.update(cx, |scrollback, cx| {
            scrollback.settle_request();
            cx.notify();
        });
        self.focus_prompt(window, cx);
        cx.notify();
    }

    fn apply(&mut self, event: Event, window: &mut Window, cx: &mut Context<'_, Self>) {
        #[cfg(debug_assertions)]
        if self.demo {
            return;
        }
        match event {
            Event::Line { source, kind, text } => self.push_line(&source, kind, &text, cx),
            Event::Prompt { label, reply } => {
                if let Some(new) = &reply
                    && self.reply.as_ref().map(|shown| shown.item) != Some(new.item)
                {
                    let target = target_name(&label);
                    self.scrollback.update(cx, |scrollback, cx| {
                        scrollback.raise_request(target, &new.question);
                        cx.notify();
                    });
                }
                self.label = label;
                if reply.is_none() && self.reply.is_some() {
                    self.leave_reply_mode(window, cx);
                }
                self.reply = reply;
            }
            Event::Status(counts) => self.counts = counts,
            Event::Fatal(text) => self.push_line(&Source::App, LineKind::Error, &text, cx),
        }
        cx.notify();
    }

    fn push_line(&self, source: &Source, kind: LineKind, text: &str, cx: &mut Context<'_, Self>) {
        self.scrollback.update(cx, |scrollback, cx| {
            scrollback.push_line(source, kind, text);
            cx.notify();
        });
    }

    fn send_cold_start_once(&mut self, window: &Window, cx: &App) {
        if self.cold_start_sent || !self.prompt_focus(cx).is_focused(window) {
            return;
        }
        self.cold_start_sent = true;
        let core = self.core.clone();
        let launched = self.launched;
        window.on_next_frame(move |_, _| {
            core.metric(Metric::ColdStart(launched.elapsed()));
        });
    }

    /// The one prompt: the target, the caret block, and the input, over a
    /// single rule.
    fn render_prompt(&self, palette: &Palette) -> Div {
        let label_color = match self.label {
            Label::NoTarget => palette.muted,
            Label::Project(_) | Label::Workspace { .. } => palette.ink,
        };
        div()
            .flex_shrink_0()
            .px(px(MARGIN_X))
            .pt(px(PROMPT_PAD_TOP))
            .child(
                div()
                    .w_full()
                    .max_w(px(PROMPT_WIDTH))
                    .flex()
                    .items_start()
                    .gap(px(PROMPT_GAP))
                    .pt(px(PROMPT_ROW_PAD_TOP))
                    .pb(px(PROMPT_ROW_PAD_BOTTOM))
                    .border_b_2()
                    .border_color(palette.ink)
                    .font_family(MONO)
                    .text_size(px(PROMPT_SIZE))
                    .line_height(px(PROMPT_LINE))
                    .child(
                        div()
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_color(label_color)
                            .child(prompt_label(&self.label)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .mt(px(CARET_DROP))
                            .w(px(CARET_WIDTH))
                            .h(px(CARET_HEIGHT))
                            .bg(palette.accent),
                    )
                    .child(div().flex_1().min_w_0().child(Textarea::new(&self.prompt))),
            )
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.send_cold_start_once(window, cx);
        let palette = *cx.global::<Palette>();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.ground)
            .text_color(palette.ink)
            .font_family(SANS)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(MARGIN_X))
                    .pt(px(SCROLLBACK_PAD_TOP))
                    .pb(px(SCROLLBACK_PAD_BOTTOM))
                    .child(self.scrollback.clone()),
            )
            .child(self.render_prompt(&palette))
            .child(status_bar(self.counts, &palette))
    }
}

fn prompt_label(label: &Label) -> String {
    match label {
        Label::NoTarget => "no target ›".to_owned(),
        Label::Project(name) => format!("{name} ›"),
        Label::Workspace { name, project } => format!("{name} ({project}) ›"),
    }
}

/// The name of the target that `label` shows; empty with no target.
fn target_name(label: &Label) -> &str {
    match label {
        Label::NoTarget => "",
        Label::Project(name) | Label::Workspace { name, .. } => name,
    }
}

/// `working` shows once any agent has started, so `0 working` confirms a
/// stop. `needs you` and `failed` show only when above 0. The key hints
/// sit at the right end.
fn status_bar(counts: Counts, palette: &Palette) -> Div {
    let mut bar = div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(STATUS_GAP))
        .px(px(MARGIN_X))
        .pt(px(STATUS_PAD_TOP))
        .pb(px(STATUS_PAD_BOTTOM))
        .font_family(MONO)
        .text_size(px(STATUS_SIZE))
        .line_height(px(STATUS_LINE))
        .text_color(palette.muted)
        .child(count(counts.projects, "project"))
        .child(count(counts.workspaces, "workspace"));
    if counts.any_agent_started {
        bar = bar.child(format!("{} working", counts.working));
    }
    if counts.needs_you > 0 {
        bar = bar.child(
            div()
                .flex()
                .items_center()
                .gap(px(STATUS_MARK_GAP))
                .text_color(palette.ink)
                .child(request::mark(palette))
                .child(format!("{} needs you", counts.needs_you)),
        );
    }
    if counts.failed > 0 {
        bar = bar.child(format!("{} failed", counts.failed));
    }
    let mut keys = Vec::new();
    if counts.needs_you > 0 {
        keys.push("⇥ answer");
    }
    if counts.agents_alive > 0 {
        keys.push("⌘. stop");
    }
    for (index, key) in keys.into_iter().enumerate() {
        let hint = div().child(key);
        bar = bar.child(if index == 0 { hint.ml_auto() } else { hint });
    }
    bar
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
