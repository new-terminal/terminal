//! The root view: the scrollback, the prompt, and the status bar, top to
//! bottom, and the speed metrics.

use std::time::Instant;

use gpui_kit::base::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Task, Window, div, px,
};
use nt_core::{CoreHandle, Counts, Event, Events, Label, LineKind, Metric, Source};

use crate::palette::Palette;
use crate::scrollback::Scrollback;

const PROMPT_MIN_ROWS: usize = 1;
const PROMPT_MAX_ROWS: usize = 8;
const FONT: &str = "Menlo";
const EDGE_PADDING: f32 = 8.;
const STATUS_SEPARATOR: &str = " │ ";

pub struct Root {
    core: CoreHandle,
    prompt: Entity<TextareaState>,
    scrollback: Entity<Scrollback>,
    label: Label,
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
        let scrollback = cx.new(|_| Scrollback::new());
        let pump = cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update_in(cx, |this, _, cx| this.apply(event, cx))
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
            counts: Counts::default(),
            launched,
            cold_start_sent: false,
            _subscriptions: vec![on_enter, on_key],
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

    fn apply(&mut self, event: Event, cx: &mut Context<'_, Self>) {
        match event {
            Event::Line { source, kind, text } => self.push_line(&source, kind, &text, cx),
            Event::Prompt { label } => self.label = label,
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
            .child(
                div()
                    .flex()
                    .gap(px(EDGE_PADDING))
                    .child(
                        div()
                            .text_color(palette.dim)
                            .child(prompt_label(&self.label)),
                    )
                    .child(div().flex_1().child(Textarea::new(&self.prompt))),
            )
            .child(
                div()
                    .text_color(palette.dim)
                    .child(status_line(self.counts)),
            )
    }
}

const fn prompt_label(label: &Label) -> &'static str {
    match label {
        Label::NoTarget => "no target ›",
    }
}

fn status_line(counts: Counts) -> String {
    [
        count(counts.projects, "project"),
        count(counts.workspaces, "workspace"),
    ]
    .join(STATUS_SEPARATOR)
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
