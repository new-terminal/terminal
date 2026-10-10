//! New Terminal's core. It owns every rule, parse, and decision, and it has
//! no UI dependency. A window talks to it only through [`start`], which
//! returns a [`CoreHandle`] for actions and an [`Events`] stream for what to
//! show.

mod actor;
mod agent;
mod attention;
mod block;
mod env;
mod git;
mod grammar;
mod home;
mod log;
mod metrics;
mod paths;
mod permission;
mod registry;
mod state;
mod stop;
mod stream;
mod worker;
mod workspace;

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use actor::Message;
pub use attention::ItemId;

const DEBUG_HOME: &str = ".new-terminal-dev";
const RELEASE_HOME: &str = ".new-terminal";

/// Starts the core on its own thread and returns at once.
///
/// The core creates `app_home` and its `logs/` directories if they are
/// missing. When it cannot, the first event is [`Event::Fatal`] and the core
/// ends.
pub fn start(app_home: PathBuf) -> (CoreHandle, Events) {
    let (inbox, messages) = mpsc::channel();
    let (events_tx, events_rx) = async_channel::unbounded();
    let (closed_tx, closed_rx) = mpsc::channel();
    actor::spawn(app_home, inbox.clone(), messages, events_tx, closed_tx);
    let handle = CoreHandle {
        inbox,
        closed: Arc::new(Mutex::new(closed_rx)),
    };
    (handle, Events(events_rx))
}

/// The app home for this build: `~/.new-terminal-dev` for debug builds and
/// `~/.new-terminal` for release builds, so a debug build never touches the
/// author's state. `None` when the user has no home directory.
pub fn default_app_home() -> Option<PathBuf> {
    let name = if cfg!(debug_assertions) {
        DEBUG_HOME
    } else {
        RELEASE_HOME
    };
    std::env::home_dir().map(|home| home.join(name))
}

/// Sends actions to the core. A send never blocks or fails, and after the
/// core ends a send does nothing.
#[derive(Clone, Debug)]
pub struct CoreHandle {
    inbox: mpsc::Sender<Message>,
    closed: Arc<Mutex<mpsc::Receiver<Infallible>>>,
}

impl CoreHandle {
    /// Sends one line the author typed, exactly as typed.
    pub fn submit(&self, text: String) {
        self.send(Action::Submit {
            text,
            at: Instant::now(),
        });
    }

    /// Brings up the first waiting item. A permission starts reply mode
    /// through [`Event::Prompt`]. A finished turn shows its last text again
    /// and becomes the target.
    pub fn bring_up(&self) {
        self.send(Action::BringUp);
    }

    /// Answers the permission `item` and ends reply mode. Answers nothing
    /// when `item` no longer waits, and says so.
    pub fn answer(&self, item: ItemId, decision: Decision) {
        self.send(Action::Answer { item, decision });
    }

    /// Moves `item` to the end of its group and ends reply mode.
    pub fn later(&self, item: ItemId) {
        self.send(Action::Later(item));
    }

    /// Ends every agent process, and ends reply mode.
    pub fn stop_all(&self) {
        self.send(Action::StopAll);
    }

    /// Ends the core after it writes its final log lines. The core ignores
    /// every later action.
    pub fn quit(&self) {
        self.send(Action::Quit);
    }

    pub fn metric(&self, metric: Metric) {
        self.send(Action::Metric(metric));
    }

    /// Blocks until the core has ended or `limit` passes. Returns at once if
    /// the core already ended.
    pub fn wait_closed(&self, limit: Duration) -> Closed {
        let closed = self.closed.lock().unwrap_or_else(PoisonError::into_inner);
        match closed.recv_timeout(limit) {
            Err(mpsc::RecvTimeoutError::Timeout) => Closed::TimedOut,
            Err(mpsc::RecvTimeoutError::Disconnected) => Closed::Ended,
            Ok(never) => match never {},
        }
    }

    fn send(&self, action: Action) {
        // The only failure is a core that has already ended, and an ended
        // core ignores actions by contract.
        let _ = self.inbox.send(Message::Action(action));
    }
}

/// The author's answer to a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

/// How [`CoreHandle::wait_closed`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closed {
    Ended,
    TimedOut,
}

/// A speed measurement taken by the window, logged to `app.log`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// From process start to the first frame with the prompt focused.
    ColdStart(Duration),
    /// From a key event to the next drawn frame.
    KeypressToFrame(Duration),
    /// From a submit to the frame that shows its echo line.
    SubmitToEcho(Duration),
}

/// What the core asks the window to show. `None` from [`Events::recv`] means
/// the core stopped.
#[derive(Debug)]
pub struct Events(async_channel::Receiver<Event>);

impl Events {
    pub async fn recv(&self) -> Option<Event> {
        self.0.recv().await.ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// One scrollback line.
    Line {
        source: Source,
        kind: LineKind,
        text: String,
    },
    /// The prompt label to show before the input. With `reply` set, the
    /// prompt is in reply mode: it shows the question in place of the input
    /// and takes only an answer for that item.
    Prompt {
        label: Label,
        reply: Option<Reply>,
    },
    Status(Counts),
    /// The core stopped and accepts no more actions. The text says why.
    Fatal(String),
}

/// Who a scrollback line speaks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    App,
    Target(String),
}

/// The permission that reply mode answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub item: ItemId,
    /// `Allow <tool>: <detail> (<n> lines)`.
    pub question: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    App,
    Error,
    /// Text the agent wrote.
    AgentText,
    /// A tool the agent called.
    Tool,
    Warning,
    /// A request that waits for the author.
    Attention,
    /// A finished turn.
    Done,
    /// An agent that the stop steps ended.
    Stopped,
    /// An agent that ended on its own.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Label {
    NoTarget,
    /// The target is the project with this name.
    Project(String),
    /// The target is the workspace `name`, an isolated copy of `project`.
    Workspace {
        name: String,
        project: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub projects: usize,
    pub workspaces: usize,
    /// Agents in a turn, including those that wait on a permission.
    pub working: usize,
    /// Waiting permissions plus finished turns.
    pub needs_you: usize,
    /// Targets whose last agent ended on its own and that got no request
    /// since.
    pub failed: usize,
    /// Agent processes not yet seen to exit.
    pub agents_alive: usize,
    /// Whether any agent has started since launch.
    pub any_agent_started: bool,
}

#[derive(Debug)]
enum Action {
    /// `at` is when the window took the line, for the speed metrics.
    Submit {
        text: String,
        at: Instant,
    },
    BringUp,
    Answer {
        item: ItemId,
        decision: Decision,
    },
    Later(ItemId),
    StopAll,
    Quit,
    Metric(Metric),
}
