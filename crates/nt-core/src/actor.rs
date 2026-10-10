//! The actor: one thread that owns all core state and handles one action at
//! a time.

use std::any::Any;
use std::convert::Infallible;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::grammar::{self, IntentKind, LINE_LIMIT_KB, LIST_WORDS, Name, Parsed};
use crate::home::{self, Home};
use crate::log::AppLog;
use crate::metrics::{KeypressSamples, KeypressSummary, whole_ms_rounded_up};
use crate::{Action, Counts, Event, Label, LineKind, Metric, Source};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs the actor on its own thread. The thread drops `closed` when it ends,
/// which is how [`crate::CoreHandle::wait_closed`] sees the end.
pub fn spawn(
    app_home: PathBuf,
    actions: mpsc::Receiver<Action>,
    events: async_channel::Sender<Event>,
    closed: mpsc::Sender<Infallible>,
) {
    thread::Builder::new()
        .name("nt-core".to_owned())
        .spawn(move || {
            // The events sender stays outside the unwind boundary so a panic
            // can still tell the window that the core stopped.
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                run(&app_home, &actions, &events);
            }));
            if let Err(payload) = outcome {
                let _ = events.try_send(Event::Fatal(format!(
                    "New Terminal's core stopped: {}. Quit and relaunch.",
                    panic_message(payload.as_ref())
                )));
            }
            drop(closed);
        })
        .expect("start the core thread");
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else {
        "unknown panic"
    }
}

fn run(app_home: &Path, actions: &mpsc::Receiver<Action>, events: &async_channel::Sender<Event>) {
    let home = match home::prepare(app_home) {
        Ok(home) => home,
        Err(failure) => {
            let _ = events.try_send(Event::Fatal(format!(
                "Cannot create the app home {}: {}.",
                failure.path.display(),
                failure.error
            )));
            return;
        }
    };
    let log_path = home.app_log();
    let log = match AppLog::open(&log_path) {
        Ok(log) => log,
        Err(error) => {
            let _ = events.try_send(Event::Fatal(format!(
                "Cannot open {}: {error}.",
                log_path.display()
            )));
            return;
        }
    };
    let mut actor = Actor {
        home,
        log,
        log_failed: false,
        events: events.clone(),
        keypresses: KeypressSamples::default(),
    };
    actor.launch();
    for action in actions {
        if actor.handle(action) == Flow::End {
            return;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    End,
}

#[derive(Debug)]
struct Actor {
    home: Home,
    log: AppLog,
    log_failed: bool,
    events: async_channel::Sender<Event>,
    keypresses: KeypressSamples,
}

impl Actor {
    fn launch(&mut self) {
        let build = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let pid = std::process::id().to_string();
        let home = self.home.root.display().to_string();
        self.log(
            "launch",
            &[
                ("version", VERSION),
                ("build", build),
                ("pid", &pid),
                ("home", &home),
            ],
        );
        self.emit(Event::Prompt {
            label: Label::NoTarget,
        });
        self.emit(Event::Status(Counts::default()));
        self.line(
            LineKind::App,
            "No projects yet. Add one: add project <name> <path>".to_owned(),
        );
    }

    fn handle(&mut self, action: Action) -> Flow {
        match action {
            Action::Submit(line) => self.submit(&line),
            Action::StopAll => self.line(LineKind::Error, "No agents are running.".to_owned()),
            Action::Metric(metric) => self.metric(metric),
            Action::Quit => {
                self.quit();
                return Flow::End;
            }
        }
        Flow::Continue
    }

    fn submit(&self, line: &str) {
        if let Some(refusal) = refusal(grammar::parse(line)) {
            self.line(LineKind::Error, refusal);
        }
    }

    fn metric(&mut self, metric: Metric) {
        match metric {
            Metric::ColdStart(elapsed) => self.log_duration("cold_start_ms", elapsed),
            Metric::SubmitToEcho(elapsed) => self.log_duration("submit_to_echo_ms", elapsed),
            Metric::KeypressToFrame(elapsed) => {
                if let Some(summary) = self.keypresses.push(elapsed) {
                    self.log_keypresses(summary);
                }
            }
        }
    }

    fn quit(&mut self) {
        if let Some(summary) = self.keypresses.flush() {
            self.log_keypresses(summary);
        }
        self.log("quit", &[]);
        self.log("quit done", &[]);
    }

    fn log_duration(&mut self, name: &str, elapsed: Duration) {
        let ms = whole_ms_rounded_up(elapsed).to_string();
        self.log("metric", &[(name, &ms)]);
    }

    fn log_keypresses(&mut self, summary: KeypressSummary) {
        let p50 = summary.p50_ms.to_string();
        let p95 = summary.p95_ms.to_string();
        let count = summary.count.to_string();
        self.log(
            "metric keypress_to_frame_ms",
            &[("p50", &p50), ("p95", &p95), ("n", &count)],
        );
    }

    /// Writes one `app.log` line. The first failed write shows one error
    /// line, so a full disk or a removed log is not silent, and later
    /// failures do not flood the scrollback.
    fn log(&mut self, event: &str, fields: &[(&str, &str)]) {
        if let Err(error) = self.log.write(event, fields)
            && !self.log_failed
        {
            self.log_failed = true;
            let text = format!(
                "Could not write {}: {error}. Later log lines may be lost.",
                self.home.app_log().display()
            );
            self.line(LineKind::Error, text);
        }
    }

    fn line(&self, kind: LineKind, text: String) {
        self.emit(Event::Line {
            source: Source::App,
            kind,
            text,
        });
    }

    fn emit(&self, event: Event) {
        // An unbounded channel fails only when the window dropped its
        // receiver, and then nobody is left to show the event.
        let _ = self.events.try_send(event);
    }
}

/// The one error line for a submitted line, or `None` when the line asks for
/// nothing.
fn refusal(parsed: Parsed) -> Option<String> {
    let text = match parsed {
        Parsed::Empty => return None,
        Parsed::TooLong { kb } => {
            format!("This line is {kb} KB. The limit is {LINE_LIMIT_KB} KB.")
        }
        Parsed::List => not_built(LIST_WORDS),
        Parsed::Intent(kind) => not_built(kind.words()),
        Parsed::IntentUsage(kind) => usage(kind).to_owned(),
        Parsed::MentionOnly(name)
        | Parsed::Request {
            mention: Some(name),
            ..
        } => unknown_name(&name),
        Parsed::Request {
            mention: None,
            text,
        } => format!(
            "No target. Start the line with a mention, for example: @<name> {}",
            text.trim_end()
        ),
        Parsed::BadMention(token) => format!("Not a valid name: @{token}."),
        Parsed::SeveralMentions => {
            "One target per request in this version. Send one line per target.".to_owned()
        }
    };
    Some(text)
}

fn not_built(intent_words: &str) -> String {
    format!("{intent_words} is not built yet. Nothing changed.")
}

fn unknown_name(name: &Name) -> String {
    format!("Unknown name @{name}. Known names: none yet.")
}

const fn usage(kind: IntentKind) -> &'static str {
    match kind {
        IntentKind::AddProject => {
            "Usage: add project <name> <path>, for example: add project terminal ~/code/new-terminal/terminal"
        }
        IntentKind::NewWorkspace => {
            "Usage: new workspace <name> @<project>, for example: new workspace fix-readme @terminal"
        }
        IntentKind::ArchiveWorkspace => {
            "Usage: archive workspace <name>, for example: archive workspace fix-readme"
        }
    }
}
