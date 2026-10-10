//! The root view: the scrollback, the prompt, and the status bar, top to
//! bottom, and the speed metrics. In reply mode the prompt shows a waiting
//! permission's question in place of the input and takes only its answer.

use std::time::Instant;

use gpui_kit::base::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, IntoElement, Keystroke,
    ParentElement as _, Render, Styled as _, Subscription, Task, Window, div, px,
};
use nt_core::{
    CoreHandle, Counts, Decision, Event, Events, Label, LineKind, Metric, Reply, Source,
};

use crate::palette::Palette;
use crate::scrollback::Scrollback;

const PROMPT_MIN_ROWS: usize = 1;
const PROMPT_MAX_ROWS: usize = 8;
const FONT: &str = "Menlo";
const EDGE_PADDING: f32 = 8.;
const STATUS_SEPARATOR: &str = " │ ";
const STATUS_KEYS_GAP: &str = "  ";
const REPLY_HINT: &str = "Answer with y or n, or press Esc.";

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
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .submit_on_enter(true)
                .auto_grow(PROMPT_MIN_ROWS, PROMPT_MAX_ROWS)
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
            _subscriptions: vec![on_enter, on_key, on_answer_key],
            _pump: pump,
        }
    }

    pub fn focus_prompt(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        window.focus(&self.prompt_focus(cx), cx);
    }

    fn prompt_focus(&self, cx: &App) -> FocusHandle {
        self.prompt.focus_handle(cx)
    }

    fn submit(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
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
        let plain = !keystroke.modifiers.modified();
        let Some(reply) = &self.reply else {
            let empty = self.prompt.read(cx).value().is_empty();
            if keystroke.key == "tab" && plain && empty {
                self.core.bring_up();
                return true;
            }
            return false;
        };
        let item = reply.item;
        match keystroke.key.as_str() {
            "y" if plain => self.core.answer(item, Decision::Allow),
            "n" if plain => self.core.answer(item, Decision::Deny),
            "escape" if plain => self.core.later(item),
            _ => {
                self.push_line(&Source::App, LineKind::App, REPLY_HINT, cx);
                return true;
            }
        }
        self.leave_reply_mode(window, cx);
        true
    }

    /// Shows the input again, with the text it held, and focuses it.
    fn leave_reply_mode(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.reply = None;
        self.focus_prompt(window, cx);
        cx.notify();
    }

    fn apply(&mut self, event: Event, window: &mut Window, cx: &mut Context<'_, Self>) {
        match event {
            Event::Line { source, kind, text } => self.push_line(&source, kind, &text, cx),
            Event::Prompt { label, reply } => {
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
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.send_cold_start_once(window, cx);
        let palette = *cx.global::<Palette>();
        div()
            .size_full()
            .flex()
            .flex_col()
            .p(px(EDGE_PADDING))
            .gap(px(EDGE_PADDING))
            .bg(palette.background)
            .text_color(palette.text)
            .font_family(FONT)
            .child(div().flex_1().min_h_0().child(self.scrollback.clone()))
            .child(match &self.reply {
                Some(reply) => div()
                    .text_color(palette.accent)
                    .child(reply_question(&self.label, reply)),
                None => div()
                    .flex()
                    .gap(px(EDGE_PADDING))
                    .child(
                        div()
                            .text_color(match self.label {
                                Label::NoTarget => palette.dim,
                                Label::Project(_) => palette.text,
                            })
                            .child(prompt_label(&self.label)),
                    )
                    .child(div().flex_1().child(Textarea::new(&self.prompt))),
            })
            .child(
                div()
                    .text_color(palette.dim)
                    .child(status_line(self.counts)),
            )
    }
}

fn prompt_label(label: &Label) -> String {
    match label {
        Label::NoTarget => "no target ›".to_owned(),
        Label::Project(name) => format!("{name} ›"),
    }
}

/// Reply mode's prompt line, which names the keys that answer.
fn reply_question(label: &Label, reply: &Reply) -> String {
    let target = match label {
        Label::NoTarget => "",
        Label::Project(name) => name,
    };
    format!(
        "? {target}  {}  [y] yes  [n] no  [esc] later",
        reply.question
    )
}

/// `working` shows once any agent has started, so `0 working` confirms a
/// stop. `needs you` and `failed` show only when above 0.
fn status_line(counts: Counts) -> String {
    let mut parts = vec![
        count(counts.projects, "project"),
        count(counts.workspaces, "workspace"),
    ];
    if counts.any_agent_started {
        parts.push(format!("{} working", counts.working));
    }
    if counts.needs_you > 0 {
        parts.push(format!("{} needs you", counts.needs_you));
    }
    if counts.failed > 0 {
        parts.push(format!("{} failed", counts.failed));
    }
    let mut keys = Vec::new();
    if counts.needs_you > 0 {
        keys.push("⇥ answer");
    }
    if counts.agents_alive > 0 {
        keys.push("⌘. stop");
    }
    let mut line = parts.join(STATUS_SEPARATOR);
    if !keys.is_empty() {
        line.push_str(STATUS_KEYS_GAP);
        line.push_str(&keys.join(STATUS_KEYS_GAP));
    }
    line
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
