//! The actor: one thread that owns all core state and handles one message
//! at a time. It never waits on a child process or a pipe. Workers and pipe
//! threads report through its inbox, and the stop-step deadlines and the
//! exit poll ride on its receive time limit.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::fs::File;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use nix::sys::signal::Signal;

use crate::agent::{self, AgentHandle, AgentId, PipeEvent};
use crate::attention::{FinishedTurn, Item, ItemId, Permission, Queue};
use crate::block::{self, Block};
use crate::env::{self, Environment};
use crate::grammar::{
    self, Intent, IntentKind, LINE_LIMIT_KB, LIST_WORDS, NAME_RULE, Name, Parsed,
};
use crate::home::{self, Home, LockError};
use crate::log::{AgentLog, AppLog, Direction};
use crate::metrics::{KeypressSamples, KeypressSummary, whole_ms_rounded_up};
use crate::paths::{self, CopyProblem, Rules};
use crate::permission::{self, Verdict};
use crate::registry::{Entry, NO_PROJECTS, Project, Registry};
use crate::state::{self, StateFile};
use crate::stop::{SignalSent, Step, StopSteps};
use crate::stream::{self, Effect, PermissionRequest};
use crate::worker::{self, Call, ChildDone};
use crate::workspace::{self, Create, Next};
use crate::{Action, Counts, Decision, Event, Label, LineKind, Metric, Reply, Source};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often the actor looks for agent exits while any agent lives.
const EXIT_POLL: Duration = Duration::from_millis(100);
/// An exit shows once the agent's output ends, or this long after the exit
/// without it, so the last output shows before the exit line.
const EXIT_SHOW_WAIT: Duration = Duration::from_secs(1);
/// At quit, how long the actor waits for child calls to report once it has
/// told them to stop. It covers the environment capture's SIGTERM grace, so
/// its SIGKILL still goes before the process ends.
const QUIT_CALL_WAIT: Duration = Duration::from_secs(1);
const STDERR_TAIL_LINES: usize = 20;
const EXPECTED_PERMISSION_MODE: &str = "default";
const STOPPED_BY_USER: &str = "Stopped by the user";
const DECLINED: &str = "Declined at the prompt";
const ASK_IN_TEXT: &str = "New Terminal takes questions only as text. Ask them in your reply with their options, then end your turn.";
const TOO_LONG: &str = "Too long to review at the prompt. Make a smaller change.";
const NO_LONGER_WAITING: &str = "That request is no longer waiting.";
/// Its answers would need a second reply mode, so the agent asks in text.
const ASK_USER_QUESTION: &str = "AskUserQuestion";
const READING_ENVIRONMENT: &str =
    "Reading your shell environment. Send the line again in a moment.";
/// Names every git call in `app.log`, which adds `git_ms` to its line.
const GIT_CALL_PREFIX: &str = "git-";
const WILL_NOT_CHANGE: &str =
    "New Terminal will not change this file. Fix or move it, then relaunch.";

/// Everything that reaches the actor.
#[derive(Debug)]
pub enum Message {
    Action(Action),
    Pipe(AgentId, PipeEvent),
    CallDone(CallPurpose, ChildDone),
}

/// Why a child call ran, so its result reaches the right handler.
#[derive(Debug)]
pub enum CallPurpose {
    EnvCapture,
    ClaudeVersion(PathBuf),
    AddProject {
        name: Name,
        path: PathBuf,
    },
    /// A git call of the `new workspace` that holds this name.
    Create(Name),
}

/// Runs the actor on its own thread. The thread drops `closed` when it ends,
/// which is how [`crate::CoreHandle::wait_closed`] sees the end.
pub fn spawn(
    app_home: PathBuf,
    inbox: mpsc::Sender<Message>,
    messages: mpsc::Receiver<Message>,
    events: async_channel::Sender<Event>,
    closed: mpsc::Sender<Infallible>,
) {
    thread::Builder::new()
        .name("nt-core".to_owned())
        .spawn(move || {
            // The events sender stays outside the unwind boundary so a panic
            // can still tell the window that the core stopped. The unwind
            // drops every agent handle, which sends SIGTERM to its group.
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                run(&app_home, inbox, &messages, &events);
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

fn run(
    app_home: &Path,
    inbox: mpsc::Sender<Message>,
    messages: &mpsc::Receiver<Message>,
    events: &async_channel::Sender<Event>,
) {
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
    let home_dir = std::env::home_dir()
        .map(|dir| std::fs::canonicalize(&dir).unwrap_or(dir))
        .unwrap_or_default();
    let mut actor = Actor {
        home,
        home_dir,
        log,
        log_failed: false,
        events: events.clone(),
        inbox,
        keypresses: KeypressSamples::default(),
        quit_flag: Arc::new(AtomicBool::new(false)),
        calls_running: 0,
        env: EnvState::Reading,
        state_lock: None,
        refusal: None,
        registry: Registry::default(),
        adding: Vec::new(),
        creating: BTreeMap::new(),
        target: None,
        saved_target: None,
        agents: BTreeMap::new(),
        next_agent: 0,
        attention: Queue::default(),
        replying: None,
        started_targets: BTreeSet::new(),
        failed_targets: BTreeSet::new(),
        stop_all_at: None,
        quitting: None,
        sent_counts: Counts::default(),
    };
    actor.launch();
    loop {
        let message = match actor.next_wake() {
            Some(wake) => {
                match messages.recv_timeout(wake.saturating_duration_since(Instant::now())) {
                    Ok(message) => Some(message),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            None => match messages.recv() {
                Ok(message) => Some(message),
                Err(mpsc::RecvError) => return,
            },
        };
        if let Some(message) = message {
            actor.handle(message);
        }
        if actor.tick(Instant::now()) == Flow::End {
            return;
        }
        actor.send_counts();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    End,
}

#[derive(Debug)]
enum EnvState {
    Reading,
    Ready {
        env: Arc<Environment>,
        claude: Option<PathBuf>,
    },
}

#[derive(Debug)]
struct Quitting {
    /// Set once every agent has exited and the quit flag is up.
    calls_until: Option<Instant>,
}

/// One agent process and what the actor knows about it.
#[derive(Debug)]
struct Agent {
    id: AgentId,
    target: Name,
    /// The project path, or the workspace's isolated copy.
    path: PathBuf,
    in_workspace: bool,
    handle: AgentHandle,
    log: Option<AgentLog>,
    log_failed: bool,
    submitted: Instant,
    first_line_seen: bool,
    init_seen: bool,
    /// Working, or waiting on a permission.
    in_turn: bool,
    /// The last text block of the current turn.
    last_text: String,
    /// `tool_use_id`s of `AskUserQuestion` requests the app denied. Their
    /// error results stay in the agent log only.
    hidden_results: BTreeSet<String>,
    stop: Option<StopSteps>,
    interrupted: bool,
    writer_alive: bool,
    stdout_ended: bool,
    stderr_ended: bool,
    exit: Option<(ExitStatus, Instant)>,
    stderr_tail: VecDeque<String>,
}

impl Agent {
    const fn is_stopping(&self) -> bool {
        self.stop.is_some() || self.exit.is_some()
    }
}

#[derive(Debug)]
struct Actor {
    home: Home,
    /// The user's home directory, canonical.
    home_dir: PathBuf,
    log: AppLog,
    log_failed: bool,
    events: async_channel::Sender<Event>,
    inbox: mpsc::Sender<Message>,
    keypresses: KeypressSamples,
    /// Tells every running child call to stop, once quit has ended the
    /// agents.
    quit_flag: Arc<AtomicBool>,
    calls_running: usize,
    env: EnvState,
    /// Held for the life of the core, so no second process writes the
    /// state file.
    state_lock: Option<File>,
    /// When set, every line fails with this text and the state file is
    /// never written: the lock is held elsewhere, or the file could not be
    /// read whole.
    refusal: Option<String>,
    registry: Registry,
    /// Projects whose `add project` waits on its git call. Their names and
    /// paths are taken.
    adding: Vec<(Name, PathBuf)>,
    /// Each `new workspace` in progress. Its name is taken until it ends.
    creating: BTreeMap<Name, Create>,
    target: Option<Name>,
    /// The target a relaunch would restore from the file as last read or
    /// written.
    saved_target: Option<Name>,
    agents: BTreeMap<AgentId, Agent>,
    next_agent: u64,
    attention: Queue,
    /// The permission that reply mode answers, as last sent in
    /// [`Event::Prompt`].
    replying: Option<ItemId>,
    /// Targets that had an agent in this run.
    started_targets: BTreeSet<Name>,
    failed_targets: BTreeSet<Name>,
    stop_all_at: Option<Instant>,
    quitting: Option<Quitting>,
    sent_counts: Counts,
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
        self.open_state();
        self.show_prompt(None);
        self.sent_counts = self.counts();
        self.emit(Event::Status(self.sent_counts));
        self.run_call(env::capture_call(), CallPurpose::EnvCapture);
    }

    /// Takes the state lock, then loads the state file and restores the
    /// registry and the target. A held lock or a file that fails a load
    /// check starts refusal mode.
    fn open_state(&mut self) {
        match self.home.lock() {
            Ok(file) => self.state_lock = Some(file),
            Err(LockError::Held) => {
                let path = self.home.lock_file().display().to_string();
                self.log("lock held", &[("path", &path)]);
                let home = self.tilde(&self.home.root);
                return self.refuse(format!(
                    "Another New Terminal is using {home}. Quit it first."
                ));
            }
            Err(LockError::Failed(error)) => {
                let path = self.tilde(&self.home.lock_file());
                return self.refuse(format!("Cannot lock {path}: {error}."));
            }
        }
        let path = self.home.state_file();
        let loaded = state::read(&path).and_then(|file| {
            file.map(|file| state::check(file, &self.home_dir, &self.home.root, state::look))
                .transpose()
        });
        match loaded {
            Ok(Some(loaded)) => {
                for warning in loaded.warnings {
                    self.app_line(LineKind::Warning, warning);
                }
                self.registry = loaded.registry;
                self.target.clone_from(&loaded.target);
                self.saved_target = loaded.target;
            }
            Ok(None) => {}
            Err(problem) => {
                let shown = path.display().to_string();
                self.log(
                    "state load error",
                    &[("path", &shown), ("problem", &problem)],
                );
                let path = self.tilde(&path);
                self.refuse(format!("Cannot read {path}: {problem}."));
                return self.app_line(LineKind::Error, WILL_NOT_CHANGE.to_owned());
            }
        }
        self.warn_leftover_copies();
        if self.registry.project_count() == 0 {
            self.app_line(LineKind::App, NO_PROJECTS.to_owned());
        }
    }

    /// A copy that no workspace names is left over from a create that the
    /// app did not finish, such as one cut off by a quit.
    fn warn_leftover_copies(&self) {
        let checkouts = self.registry.checkouts().collect();
        for copy in workspace::leftover_copies(&self.home.workspaces(), &checkouts) {
            let copy = self.tilde(&copy);
            self.app_line(
                LineKind::Warning,
                format!("{copy} is left over from an interrupted create."),
            );
        }
    }

    /// Shows `text` and fails every later line with it.
    fn refuse(&mut self, text: String) {
        self.app_line(LineKind::Error, text.clone());
        self.refusal = Some(text);
    }

    fn tilde(&self, path: &Path) -> String {
        paths::with_tilde(path, &self.home_dir)
    }

    /// Writes `file` over `state.json` and logs how long it took.
    fn save_state(&mut self, file: &StateFile) -> io::Result<()> {
        let started = Instant::now();
        state::save(&self.home.state_file(), &self.home.state_temp(), file)?;
        let ms = whole_ms_rounded_up(started.elapsed()).to_string();
        self.log("state saved", &[("state_save_ms", &ms)]);
        Ok(())
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Action(action) => self.handle_action(action),
            Message::Pipe(id, event) => self.pipe(id, event),
            Message::CallDone(purpose, done) => self.call_done(purpose, &done),
        }
    }

    fn handle_action(&mut self, action: Action) {
        if self.quitting.is_some() {
            return;
        }
        match action {
            Action::Submit { text, at } => self.submit(&text, at),
            Action::BringUp => {
                if let Some(item) = self.attention.first() {
                    self.bring_up(item);
                }
            }
            Action::Answer { item, decision } => self.answer(item, decision),
            Action::Later(item) => self.later(item),
            Action::StopAll => self.stop_all(),
            Action::Metric(metric) => self.metric(metric),
            Action::Quit => self.quit(),
        }
    }

    fn submit(&mut self, line: &str, at: Instant) {
        let parsed = grammar::parse(line);
        if parsed == Parsed::Empty {
            return;
        }
        if let Some(refusal) = &self.refusal {
            return self.app_line(LineKind::Error, refusal.clone());
        }
        let refusal = match parsed {
            Parsed::Empty => return,
            Parsed::List => return self.list(),
            Parsed::Intent(Intent::AddProject { name, path }) => {
                return self.add_project(&name, &path);
            }
            Parsed::Intent(Intent::NewWorkspace { name, project }) => {
                return self.new_workspace(&name, &project);
            }
            Parsed::MentionOnly(name) => {
                if self.mention(name.clone())
                    && let Some(item) = self.attention.for_target(&name)
                {
                    self.bring_up(item);
                }
                return;
            }
            Parsed::Request { mention, text } => {
                if mention.is_none_or(|name| self.mention(name)) {
                    self.request(&text, at);
                }
                return;
            }
            Parsed::TooLong { kb } => {
                format!("This line is {kb} KB. The limit is {LINE_LIMIT_KB} KB.")
            }
            Parsed::Intent(intent) => not_built(intent.kind().words()),
            Parsed::IntentUsage(kind) => usage(kind).to_owned(),
            Parsed::BadMention(token) => format!("Not a valid name: @{token}."),
            Parsed::SeveralMentions => {
                "One target per request in this version. Send one line per target.".to_owned()
            }
        };
        self.app_line(LineKind::Error, refusal);
    }

    fn list(&mut self) {
        self.log("intent", &[("kind", LIST_WORDS)]);
        for line in self.registry.list_lines(&self.home_dir) {
            self.app_line(LineKind::App, line);
        }
    }

    /// Sets the target when `name` is registered. Otherwise refuses the line
    /// and returns `false`.
    fn mention(&mut self, name: Name) -> bool {
        if self.creating.contains_key(&name) {
            self.app_line(LineKind::Error, being_created(&name));
            return false;
        }
        if self.registry.find(&name).is_none() {
            let known = self.known_names();
            let known = if known.is_empty() {
                "none yet".to_owned()
            } else {
                known.join(", ")
            };
            self.app_line(
                LineKind::Error,
                format!("Unknown name @{name}. Known names: {known}."),
            );
            return false;
        }
        self.set_target(name);
        true
    }

    /// Applies the target at once, then saves it.
    fn set_target(&mut self, name: Name) {
        let field = name.to_string();
        self.target = Some(name);
        self.show_prompt(None);
        let saved = self.saved_target == self.target || self.save_target();
        self.log(
            "target",
            &[
                ("name", &field),
                ("saved", if saved { "yes" } else { "no" }),
            ],
        );
    }

    /// Saves the live target. A failed save keeps it, because the label
    /// already shows it and the next line goes there, and warns what a
    /// relaunch would restore.
    fn save_target(&mut self) -> bool {
        let file = self.registry.to_state(self.target.as_ref());
        match self.save_state(&file) {
            Ok(()) => {
                self.saved_target.clone_from(&self.target);
                true
            }
            Err(error) => {
                let after = self.saved_target.as_ref().map_or_else(
                    || "there is no target".to_owned(),
                    |name| format!("the target is {name}"),
                );
                self.app_line(
                    LineKind::Warning,
                    format!("Could not save the target: {error}. After a relaunch {after}."),
                );
                false
            }
        }
    }

    /// Sends the prompt label for the target, in reply mode for `reply`.
    /// Reply mode shows the item's target, and the target itself never
    /// changes for it, so ending reply mode restores the target as it stood
    /// when the bring-up started.
    fn show_prompt(&mut self, reply: Option<(Name, Reply)>) {
        self.replying = reply.as_ref().map(|(_, reply)| reply.item);
        let (label, reply) = match reply {
            Some((target, reply)) => (self.label_for(&target), Some(reply)),
            None => (
                self.target
                    .as_ref()
                    .map_or(Label::NoTarget, |name| self.label_for(name)),
                None,
            ),
        };
        self.emit(Event::Prompt { label, reply });
    }

    fn label_for(&self, name: &Name) -> Label {
        match self.registry.find(name) {
            Some(Entry::Workspace(workspace)) => Label::Workspace {
                name: workspace.name.to_string(),
                project: workspace.project.to_string(),
            },
            Some(Entry::Project(_)) | None => Label::Project(name.to_string()),
        }
    }

    fn end_reply_mode(&mut self) {
        if self.replying.is_some() {
            self.show_prompt(None);
        }
    }

    /// Shows a permission's request block and starts reply mode for it, or
    /// shows a finished turn's last text again and makes its target the
    /// target.
    fn bring_up(&mut self, item: ItemId) {
        if let Some(permission) = self.attention.permission(item) {
            let target = permission.target.clone();
            let block = permission.block.clone();
            let question = permission.request.summary.clone();
            self.agent_line(&target, LineKind::App, block);
            return self.show_prompt(Some((target, Reply { item, question })));
        }
        if let Some(Item::FinishedTurn(turn)) = self.attention.remove(item) {
            for line in turn.last_text.lines() {
                self.agent_line(&turn.target, LineKind::AgentText, line.to_owned());
            }
            if self.target.as_ref() != Some(&turn.target) {
                self.set_target(turn.target);
            }
        }
    }

    fn answer(&mut self, item: ItemId, decision: Decision) {
        if self.attention.permission(item).is_none() {
            return self.no_longer_waiting();
        }
        let Some(Item::Permission(permission)) = self.attention.remove(item) else {
            return self.no_longer_waiting();
        };
        let Some(mut agent) = self.agents.remove(&permission.agent) else {
            return self.no_longer_waiting();
        };
        let (line, field) = match decision {
            Decision::Allow => (
                agent::allow_line(&permission.request.request_id, &permission.request.input),
                "user-allow",
            ),
            Decision::Deny => (
                agent::deny_line(&permission.request.request_id, DECLINED),
                "user-deny",
            ),
        };
        self.send_answer(&mut agent, line);
        self.log_decision(
            &agent,
            &permission.request,
            field,
            permission.setup,
            Some(item),
        );
        self.agents.insert(permission.agent, agent);
        self.end_reply_mode();
    }

    fn later(&mut self, item: ItemId) {
        if !self.attention.later(item) {
            return self.no_longer_waiting();
        }
        self.end_reply_mode();
    }

    fn no_longer_waiting(&mut self) {
        self.app_line(LineKind::Error, NO_LONGER_WAITING.to_owned());
        self.end_reply_mode();
    }

    fn known_names(&self) -> Vec<String> {
        self.registry
            .known_names()
            .into_iter()
            .map(ToString::to_string)
            .collect()
    }

    fn add_project(&mut self, typed_name: &str, typed_path: &str) {
        self.log("intent", &[("kind", "add-project"), ("name", typed_name)]);
        let EnvState::Ready { env, .. } = &self.env else {
            return self.app_line(LineKind::Error, READING_ENVIRONMENT.to_owned());
        };
        let env = Arc::clone(env);
        let Some(name) = Name::parse(typed_name) else {
            return self.app_line(
                LineKind::Error,
                format!("Cannot add project {typed_name}: {NAME_RULE}."),
            );
        };
        if let Some(holder) = self.registry.holder(&name) {
            return self.app_line(LineKind::Error, format!("{name} is taken by {holder}."));
        }
        if self.adding.iter().any(|(adding, _)| *adding == name) {
            return self.app_line(
                LineKind::Error,
                format!("{name} is being added. Wait for it to finish."),
            );
        }
        if self.creating.contains_key(&name) {
            return self.app_line(LineKind::Error, taken_by_create(&name));
        }
        let path = paths::expand_tilde(typed_path, &self.home_dir);
        let other_projects = self
            .registry
            .projects()
            .chain(
                self.adding
                    .iter()
                    .map(|(name, path)| (name, path.as_path())),
            )
            .collect();
        let rules = Rules {
            home_dir: &self.home_dir,
            app_home: &self.home.root,
            claude_config_dir: env.claude_config_dir(),
            other_projects,
        };
        let path = match paths::check(&path, &rules) {
            Ok(path) => path,
            Err(rule) => {
                return self.app_line(
                    LineKind::Error,
                    format!("Cannot add project {name}: {rule}."),
                );
            }
        };
        self.adding.push((name.clone(), path.clone()));
        let call = Call {
            name: "git-branch",
            argv: vec![
                "git".into(),
                "-C".into(),
                path.clone().into(),
                "rev-parse".into(),
                "--abbrev-ref".into(),
                "HEAD".into(),
            ],
            env: Some(env),
            limit: worker::GIT_LIMIT,
            term_grace: None,
        };
        self.run_call(call, CallPurpose::AddProject { name, path });
    }

    fn add_project_done(&mut self, name: Name, path: PathBuf, done: &ChildDone) {
        self.adding.retain(|(adding, _)| *adding != name);
        let kind = if done.succeeded() && !done.stdout_cut {
            format!(
                "git, branch {}",
                String::from_utf8_lossy(&done.stdout).trim()
            )
        } else {
            "not a git repository".to_owned()
        };
        let text = format!("Added project {name}: {} ({kind})", path.display());
        if let Err(error) = self.save_registry(self.registry.with_project(Project { name, path })) {
            return self.app_line(LineKind::Error, error);
        }
        self.app_line(LineKind::App, text);
    }

    /// Saves `registry` with the live target, and swaps it in only once
    /// the save succeeds. The error is the line to show.
    fn save_registry(&mut self, registry: Registry) -> Result<(), String> {
        let file = registry.to_state(self.target.as_ref());
        if let Err(error) = self.save_state(&file) {
            let path = self.tilde(&self.home.state_file());
            return Err(format!("Could not save {path}: {error}. Nothing changed."));
        }
        self.registry = registry;
        self.saved_target.clone_from(&self.target);
        Ok(())
    }

    fn new_workspace(&mut self, typed_name: &str, typed_project: &str) {
        self.log("intent", &[("kind", "new-workspace"), ("name", typed_name)]);
        let EnvState::Ready { env, .. } = &self.env else {
            return self.app_line(LineKind::Error, READING_ENVIRONMENT.to_owned());
        };
        let env = Arc::clone(env);
        let Some(name) = Name::parse(typed_name) else {
            return self.app_line(
                LineKind::Error,
                format!("Cannot create workspace {typed_name}: {NAME_RULE}."),
            );
        };
        let refusal = if let Some(holder) = self.registry.holder(&name) {
            Some(format!("{name} is taken by {holder}."))
        } else if self.adding.iter().any(|(adding, _)| *adding == name) {
            Some(format!(
                "{name} is taken by project {name}, which is being added."
            ))
        } else if self.creating.contains_key(&name) {
            Some(taken_by_create(&name))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            return self.app_line(LineKind::Error, refusal);
        }
        let Some(project) = Name::parse(typed_project)
            .and_then(|project| self.registry.project(&project))
            .cloned()
        else {
            let projects: Vec<String> = self
                .registry
                .projects()
                .map(|(name, _)| name.to_string())
                .collect();
            let projects = if projects.is_empty() {
                "none yet".to_owned()
            } else {
                projects.join(", ")
            };
            return self.app_line(
                LineKind::Error,
                format!("Unknown project @{typed_project}. Projects: {projects}."),
            );
        };
        let (create, next) = Create::start(workspace::Request {
            name,
            project: project.name,
            project_path: project.path,
            workspaces_dir: self.home.workspaces(),
            home_dir: self.home_dir.clone(),
            env,
        });
        self.drive_create(create, next);
    }

    /// Runs a create's next steps until it waits on a git call or ends.
    /// While it waits, it holds its name in `creating`.
    fn drive_create(&mut self, mut create: Create, mut next: Next) {
        loop {
            next = match next {
                Next::Run(call) => {
                    let name = create.name().clone();
                    self.run_call(call, CallPurpose::Create(name.clone()));
                    self.creating.insert(name, create);
                    return;
                }
                Next::Save(entry) => {
                    match self.save_registry(self.registry.with_workspace(entry)) {
                        Ok(()) => create.saved(),
                        Err(error) => create.save_failed(error),
                    }
                }
                Next::Done { created, lines } => {
                    let kind = if created {
                        LineKind::App
                    } else {
                        LineKind::Error
                    };
                    for line in lines {
                        self.app_line(kind, line);
                    }
                    return;
                }
            };
        }
    }

    fn request(&mut self, text: &str, at: Instant) {
        let Some(target) = self.target.clone() else {
            let example = self
                .registry
                .known_names()
                .first()
                .map_or_else(|| "<name>".to_owned(), ToString::to_string);
            return self.app_line(
                LineKind::Error,
                format!(
                    "No target. Start the line with a mention, for example: @{example} {}",
                    text.trim_end()
                ),
            );
        };
        if let Some(item) = self.attention.permission_for(&target) {
            self.app_line(
                LineKind::Error,
                format!("{target} is waiting for your answer."),
            );
            return self.bring_up(item);
        }
        if matches!(self.env, EnvState::Reading) {
            return self.app_line(LineKind::Error, READING_ENVIRONMENT.to_owned());
        }
        let live = self
            .agents
            .values()
            .find(|agent| agent.target == target)
            .map(|agent| agent.id);
        let id = match live {
            Some(id) => {
                let agent = &self.agents[&id];
                if agent.is_stopping() {
                    return self.app_line(
                        LineKind::Error,
                        format!("{target} is stopping. Send the line again in a moment."),
                    );
                }
                if agent.in_turn {
                    return self.app_line(
                        LineKind::Error,
                        format!("{target} is working. Wait, or press ⌘. to stop it."),
                    );
                }
                id
            }
            None => match self.start_agent(&target, at) {
                Some(id) => id,
                None => return,
            },
        };
        self.failed_targets.remove(&target);
        let Some(mut agent) = self.agents.remove(&id) else {
            return;
        };
        let line = agent::turn_line(text);
        self.agent_log(&mut agent, Direction::In, &line);
        self.attention.remove_turn_of(&target);
        if agent.handle.send(line) {
            agent.in_turn = true;
            agent.last_text.clear();
        } else {
            self.write_failed(&mut agent, "its input is closed");
        }
        self.agents.insert(id, agent);
    }

    /// Starts an agent for `target`, or shows why it cannot start.
    fn start_agent(&mut self, target: &Name, submitted: Instant) -> Option<AgentId> {
        let EnvState::Ready { env, claude } = &self.env else {
            return None;
        };
        let env = Arc::clone(env);
        let Some(claude) = claude.clone() else {
            self.app_line(LineKind::Error, claude_missing(&env));
            return None;
        };
        let (path, in_workspace, refusal) = match self.registry.find(target)? {
            Entry::Project(project) => {
                let refusal = self.project_refusal(target, &project.path, &env);
                (project.path.clone(), false, refusal)
            }
            Entry::Workspace(workspace) => {
                let refusal =
                    self.copy_refusal(target, &workspace.checkout, &workspace.project, &env);
                (workspace.checkout.clone(), true, refusal)
            }
        };
        if let Some(refusal) = refusal {
            self.app_line(
                LineKind::Error,
                format!("Cannot start an agent for {target}: {refusal}."),
            );
            return None;
        }

        let id = AgentId(self.next_agent);
        self.next_agent += 1;
        let inbox = self.inbox.clone();
        let notify = move |event| {
            let _ = inbox.send(Message::Pipe(id, event));
        };
        let handle = match AgentHandle::spawn(&claude, &path, &env, notify) {
            Ok(handle) => handle,
            Err(error) => {
                self.app_line(
                    LineKind::Error,
                    format!(
                        "Could not start {} for {target}: {error}.",
                        claude.display()
                    ),
                );
                return None;
            }
        };
        self.log_duration("submit_to_spawn_ms", submitted.elapsed());
        let pid = handle.pid();
        self.log(
            "agent start",
            &[
                ("target", target.as_ref()),
                ("pid", &pid.to_string()),
                ("path", &path.display().to_string()),
            ],
        );
        let log = match AgentLog::open(
            &self.home.agent_logs(),
            target.as_ref(),
            SystemTime::now(),
            pid,
        ) {
            Ok(log) => Some(log),
            Err(error) => {
                self.app_line(
                    LineKind::Error,
                    format!("Could not open the agent log for {target}: {error}."),
                );
                None
            }
        };
        if !self.started_targets.insert(target.clone()) {
            self.agent_line(
                target,
                LineKind::App,
                "new agent started. It does not remember earlier requests.".to_owned(),
            );
        }
        self.agents.insert(
            id,
            Agent {
                id,
                target: target.clone(),
                path,
                in_workspace,
                handle,
                log,
                log_failed: false,
                submitted,
                first_line_seen: false,
                init_seen: false,
                in_turn: false,
                last_text: String::new(),
                hidden_results: BTreeSet::new(),
                stop: None,
                interrupted: false,
                writer_alive: true,
                stdout_ended: false,
                stderr_ended: false,
                exit: None,
                stderr_tail: VecDeque::new(),
            },
        );
        Some(id)
    }

    /// Why a project's path cannot take an agent now: all 7 path rules, and
    /// the path must still be canonical.
    fn project_refusal(&self, target: &Name, path: &Path, env: &Environment) -> Option<String> {
        let rules = Rules {
            home_dir: &self.home_dir,
            app_home: &self.home.root,
            claude_config_dir: env.claude_config_dir(),
            other_projects: self
                .registry
                .projects()
                .filter(|(name, _)| *name != target)
                .collect(),
        };
        match paths::check(path, &rules) {
            Err(rule) => Some(rule.to_string()),
            Ok(canonical) if canonical != path => Some(format!(
                "{} now resolves to {}",
                path.display(),
                canonical.display()
            )),
            Ok(_) => None,
        }
    }

    /// Why a workspace's isolated copy cannot take an agent now.
    fn copy_refusal(
        &self,
        target: &Name,
        copy: &Path,
        project: &Name,
        env: &Environment,
    ) -> Option<String> {
        let expected = self
            .home
            .workspaces()
            .join(target.as_ref())
            .join(project.as_ref());
        let problem = paths::check_copy(copy, &expected, env.claude_config_dir()).err()?;
        let shown = self.tilde(copy);
        Some(match problem {
            CopyProblem::Missing => format!(
                "its isolated copy {shown} is missing. Send archive workspace {target} to remove the workspace"
            ),
            CopyProblem::Moved(canonical) => {
                format!("{shown} now resolves to {}", canonical.display())
            }
            CopyProblem::NotAt(expected) => format!(
                "its isolated copy {shown} must be {}",
                self.tilde(&expected)
            ),
            CopyProblem::Rule(rule) => rule.to_string(),
        })
    }

    fn pipe(&mut self, id: AgentId, event: PipeEvent) {
        let Some(mut agent) = self.agents.remove(&id) else {
            return;
        };
        match event {
            PipeEvent::Stdout(line) => self.stdout_line(&mut agent, &line),
            PipeEvent::Stderr(line) => {
                self.agent_log(&mut agent, Direction::Err, &line);
                if agent.stderr_tail.len() == STDERR_TAIL_LINES {
                    agent.stderr_tail.pop_front();
                }
                agent.stderr_tail.push_back(line);
            }
            PipeEvent::StdoutEnded => {
                agent.stdout_ended = true;
                self.poll_exit(&mut agent, Instant::now());
                if !agent.is_stopping() {
                    self.deny_waiting(&mut agent);
                    agent.stop = Some(StopSteps::after_closed_output(Instant::now()));
                }
            }
            PipeEvent::StderrEnded => agent.stderr_ended = true,
            PipeEvent::LineTooLarge { mib } => {
                self.agent_line(
                    &agent.target,
                    LineKind::Error,
                    format!("Output line too large ({mib} MiB)."),
                );
                self.begin_stop(&mut agent, Instant::now());
            }
            PipeEvent::WriteFailed(error) => {
                agent.writer_alive = false;
                self.write_failed(&mut agent, &error);
            }
        }
        self.agents.insert(id, agent);
    }

    fn write_failed(&mut self, agent: &mut Agent, error: &str) {
        self.log(
            "write_failed",
            &[("target", agent.target.as_ref()), ("error", error)],
        );
        if agent.is_stopping() {
            return;
        }
        self.app_line(
            LineKind::Error,
            format!("Could not send to {}: {error}.", agent.target),
        );
        self.begin_stop(agent, Instant::now());
    }

    fn stdout_line(&mut self, agent: &mut Agent, line: &str) {
        self.agent_log(agent, Direction::Out, line);
        if !agent.first_line_seen {
            agent.first_line_seen = true;
            self.log_duration("submit_to_first_line_ms", agent.submitted.elapsed());
        }
        let mapped = stream::map(
            line,
            &stream::Context {
                target: agent.target.as_ref(),
                path: &agent.path,
                interrupted: agent.interrupted,
                init_seen: agent.init_seen,
                hidden_results: &agent.hidden_results,
            },
        );
        for shown in mapped.shown {
            self.agent_line(&agent.target, shown.kind, shown.text);
        }
        if let Some(effect) = mapped.effect {
            self.apply_effect(agent, effect);
        }
    }

    fn apply_effect(&mut self, agent: &mut Agent, effect: Effect) {
        let now = Instant::now();
        match effect {
            Effect::Init {
                cwd,
                permission_mode,
                session_id,
            } => {
                agent.init_seen = true;
                self.check_init(agent, &cwd, &permission_mode, &session_id);
            }
            Effect::Working { last_text } => {
                agent.in_turn |= agent.stop.is_none();
                agent.last_text = last_text;
            }
            Effect::Permission(request) => self.permission(agent, request),
            Effect::TurnDone | Effect::TurnFailed | Effect::InterruptedResult => {
                agent.in_turn = false;
                if matches!(effect, Effect::TurnDone) && agent.stop.is_none() {
                    self.attention.remove_turn_of(&agent.target);
                    self.attention.push_turn(FinishedTurn {
                        agent: agent.id,
                        target: agent.target.clone(),
                        last_text: std::mem::take(&mut agent.last_text),
                    });
                }
                if let Some(step) = agent.stop.as_mut().and_then(|stop| stop.on_result(now)) {
                    self.take_step(agent, step);
                }
            }
        }
    }

    /// The agent must run in its target path and ask for permissions, so no
    /// user setting moves it out of its path or past its prompts.
    fn check_init(&mut self, agent: &mut Agent, cwd: &str, mode: &str, session_id: &str) {
        self.log(
            "agent init",
            &[
                ("target", agent.target.as_ref()),
                ("pid", &agent.handle.pid().to_string()),
                ("session_id", session_id),
                ("cwd", cwd),
                ("mode", mode),
            ],
        );
        let mut mismatches = Vec::new();
        if !agent::same_directory(cwd, &agent.path) {
            mismatches.push(format!(
                "{} started in {cwd}, not in {}. New Terminal stops it.",
                agent.target,
                agent.path.display()
            ));
        }
        if mode != EXPECTED_PERMISSION_MODE {
            mismatches.push(format!(
                "{} started in permission mode {mode}, not {EXPECTED_PERMISSION_MODE}. New Terminal stops it.",
                agent.target
            ));
        }
        if mismatches.is_empty() {
            return;
        }
        for mismatch in mismatches {
            self.agent_line(&agent.target, LineKind::Error, mismatch);
        }
        self.begin_stop(agent, Instant::now());
    }

    /// Answers a `can_use_tool` request at once when a rule decides it.
    /// Otherwise it becomes an attention item, and the agent waits for the
    /// author.
    fn permission(&mut self, agent: &mut Agent, request: PermissionRequest) {
        if agent.is_stopping() {
            return self.deny_stopped(agent, None, &request, false);
        }
        if request.tool == ASK_USER_QUESTION {
            if let Some(id) = &request.tool_use_id {
                agent.hidden_results.insert(id.clone());
            }
            self.send_answer(agent, agent::deny_line(&request.request_id, ASK_IN_TEXT));
            return self.log_decision(agent, &request, "ask-deny", false, None);
        }
        let setup = match permission::decide(&request.tool, &request.input, &agent.path) {
            Verdict::AutoAllow => return self.auto_allow(agent, &request),
            Verdict::AskAuthor { setup } => setup,
        };
        match block::build(&request.tool, &request.input) {
            Block::TooLong { lines, kb } => {
                self.send_answer(agent, agent::deny_line(&request.request_id, TOO_LONG));
                self.agent_line(
                    &agent.target,
                    LineKind::Error,
                    format!(
                        "{} request denied: too long to review at the prompt ({lines} lines, {kb} KB).",
                        request.tool
                    ),
                );
                self.log_decision(agent, &request, "too-long-deny", setup, None);
            }
            Block::Fits(block) => {
                self.agent_line(&agent.target, LineKind::Attention, request.summary.clone());
                self.attention.push_permission(Permission {
                    agent: agent.id,
                    target: agent.target.clone(),
                    request,
                    block,
                    setup,
                });
            }
        }
    }

    fn auto_allow(&mut self, agent: &mut Agent, request: &PermissionRequest) {
        self.send_answer(
            agent,
            agent::allow_line(&request.request_id, &request.input),
        );
        let path = request.input["file_path"].as_str().unwrap_or_default();
        self.agent_line(
            &agent.target,
            LineKind::Tool,
            format!(
                "{} {} (inside the {}, allowed)",
                request.tool,
                stream::relative(path, &agent.path),
                if agent.in_workspace {
                    "workspace"
                } else {
                    "project"
                }
            ),
        );
        self.log_decision(agent, request, "auto", false, None);
    }

    fn deny_waiting(&mut self, agent: &mut Agent) {
        for (item, waiting) in self.attention.take_permissions_of(agent.id) {
            self.deny_stopped(agent, Some(item), &waiting.request, waiting.setup);
        }
    }

    fn deny_stopped(
        &mut self,
        agent: &mut Agent,
        item: Option<ItemId>,
        request: &PermissionRequest,
        setup: bool,
    ) {
        self.send_answer(
            agent,
            agent::deny_line(&request.request_id, STOPPED_BY_USER),
        );
        self.log_decision(agent, request, "stop-deny", setup, item);
    }

    /// Queues an answer line for the agent's stdin.
    fn send_answer(&self, agent: &mut Agent, line: String) {
        self.agent_log(agent, Direction::In, &line);
        // A failed send means the writer is gone, and the process is ending.
        let _ = agent.handle.send(line);
    }

    fn log_decision(
        &mut self,
        agent: &Agent,
        request: &PermissionRequest,
        decision: &str,
        setup: bool,
        item: Option<ItemId>,
    ) {
        let item = item.map_or_else(|| "none".to_owned(), |item| item.to_string());
        self.log(
            "permission",
            &[
                ("target", agent.target.as_ref()),
                ("tool", &request.tool),
                ("decision", decision),
                ("setup", if setup { "yes" } else { "no" }),
                ("item", &item),
                ("input", &request.input.to_string()),
            ],
        );
    }

    fn stop_all(&mut self) {
        self.end_reply_mode();
        if self.agents.is_empty() {
            return self.app_line(LineKind::Error, "No agents are running.".to_owned());
        }
        let now = Instant::now();
        self.stop_all_at.get_or_insert(now);
        self.stop_every_agent(now);
    }

    fn stop_every_agent(&mut self, now: Instant) {
        let ids: Vec<AgentId> = self.agents.keys().copied().collect();
        for id in ids {
            if let Some(mut agent) = self.agents.remove(&id) {
                self.begin_stop(&mut agent, now);
                self.agents.insert(id, agent);
            }
        }
    }

    /// Stop step 1, then step 2 or 3. Does nothing for an agent that is
    /// already stopping or has exited.
    fn begin_stop(&mut self, agent: &mut Agent, now: Instant) {
        if agent.is_stopping() {
            return;
        }
        self.deny_waiting(agent);
        let (stop, step) = StopSteps::begin(now, agent.in_turn && agent.writer_alive);
        agent.stop = Some(stop);
        self.take_step(agent, step);
    }

    fn take_step(&self, agent: &mut Agent, step: Step) {
        match step {
            Step::SendInterrupt => {
                agent.interrupted = true;
                let line = agent::interrupt_line(&format!("nt-interrupt-{}", agent.id));
                self.agent_log(agent, Direction::In, &line);
                // A failed send leaves the 3 s wait to move the steps on.
                let _ = agent.handle.send(line);
            }
            Step::Terminate => agent.handle.signal(Signal::SIGTERM),
            Step::Kill => {
                agent.handle.signal(Signal::SIGKILL);
                self.agent_line(
                    &agent.target,
                    LineKind::Warning,
                    format!(
                        "{} did not stop on SIGTERM and was killed. Commands it started in the background can still be running.",
                        agent.target
                    ),
                );
            }
        }
    }

    fn quit(&mut self) {
        if let Some(summary) = self.keypresses.flush() {
            self.log_keypresses(summary);
        }
        self.log("quit", &[]);
        self.quitting = Some(Quitting { calls_until: None });
        self.stop_every_agent(Instant::now());
    }

    /// Reaps exits, runs due stop steps, shows finished exits, and ends the
    /// core once quit has finished.
    fn tick(&mut self, now: Instant) -> Flow {
        let ids: Vec<AgentId> = self.agents.keys().copied().collect();
        for id in ids {
            let Some(mut agent) = self.agents.remove(&id) else {
                continue;
            };
            self.poll_exit(&mut agent, now);
            if agent.exit.is_none()
                && let Some(step) = agent.stop.as_mut().and_then(|stop| stop.on_deadline(now))
            {
                self.take_step(&mut agent, step);
            }
            match agent.exit {
                Some((_, exited))
                    if (agent.stdout_ended && agent.stderr_ended)
                        || now >= exited + EXIT_SHOW_WAIT =>
                {
                    self.finish(agent);
                }
                _ => {
                    self.agents.insert(id, agent);
                }
            }
        }
        self.finish_quit(now)
    }

    fn poll_exit(&mut self, agent: &mut Agent, now: Instant) {
        if agent.exit.is_some() {
            return;
        }
        match agent.handle.try_wait() {
            Ok(Some(status)) => agent.exit = Some((status, now)),
            Ok(None) => {}
            Err(error) => self.log(
                "wait_failed",
                &[
                    ("target", agent.target.as_ref()),
                    ("error", &error.to_string()),
                ],
            ),
        }
    }

    /// Shows how the agent ended and drops its record. The process is
    /// reaped, so dropping the handle sends no signal, and the stdin
    /// writer's channel closes.
    fn finish(&mut self, agent: Agent) {
        let Some((status, exited)) = agent.exit else {
            return;
        };
        let target = agent.target.as_ref();
        let pid = agent.handle.pid().to_string();
        let status_field = worker::status_field(status);
        // An agent that closed its output and then exited by itself got no
        // stop signal, so it gets no stop line.
        if let Some(stop) = agent
            .stop
            .filter(|stop| !stop.closed_output() || stop.signal() != SignalSent::None)
        {
            self.log(
                "stop",
                &[
                    ("target", target),
                    ("pid", &pid),
                    ("signal", stop.signal().field()),
                ],
            );
        }
        let by_stop = agent.stop.is_some_and(|stop| !stop.closed_output());
        if by_stop {
            self.agent_line(&agent.target, LineKind::Stopped, "stopped".to_owned());
        } else {
            self.agent_line(&agent.target, LineKind::Failed, worker::status_text(status));
            for line in &agent.stderr_tail {
                self.agent_line(&agent.target, LineKind::Error, line.clone());
            }
            self.failed_targets.insert(agent.target.clone());
        }
        self.log(
            "agent exit",
            &[
                ("target", target),
                ("pid", &pid),
                ("status", &status_field),
                ("by", if by_stop { "stop" } else { "self" }),
            ],
        );
        self.attention.remove_agent(agent.id);
        if self.agents.is_empty()
            && let Some(started) = self.stop_all_at.take()
        {
            self.log_duration("stop_ms", exited.saturating_duration_since(started));
        }
        drop(agent);
    }

    /// Once quit has ended every agent, tells the child calls to stop, waits
    /// a short time for their reports, and ends.
    fn finish_quit(&mut self, now: Instant) -> Flow {
        let Some(quitting) = &mut self.quitting else {
            return Flow::Continue;
        };
        if !self.agents.is_empty() {
            return Flow::Continue;
        }
        let until = *quitting.calls_until.get_or_insert_with(|| {
            self.quit_flag.store(true, Ordering::Relaxed);
            now + QUIT_CALL_WAIT
        });
        if self.calls_running > 0 && now < until {
            return Flow::Continue;
        }
        self.log("quit done", &[]);
        Flow::End
    }

    /// The nearest moment the actor must act without a message.
    fn next_wake(&self) -> Option<Instant> {
        let now = Instant::now();
        let agent_wakes = self.agents.values().flat_map(|agent| {
            let poll = agent.exit.is_none().then_some(now + EXIT_POLL);
            let show = agent.exit.map(|(_, exited)| exited + EXIT_SHOW_WAIT);
            let stop = agent.stop.and_then(|stop| stop.deadline());
            [poll, show, stop]
        });
        let quit_wake = self
            .quitting
            .as_ref()
            .and_then(|quitting| quitting.calls_until);
        agent_wakes.chain([quit_wake]).flatten().min()
    }

    fn run_call(&mut self, call: Call, purpose: CallPurpose) {
        self.calls_running += 1;
        let inbox = self.inbox.clone();
        worker::run(call, Arc::clone(&self.quit_flag), move |done| {
            let _ = inbox.send(Message::CallDone(purpose, done));
        });
    }

    fn call_done(&mut self, purpose: CallPurpose, done: &ChildDone) {
        self.calls_running -= 1;
        let ms = whole_ms_rounded_up(done.elapsed).to_string();
        let exit = done.exit_field();
        let git_ms = whole_ms_rounded_up(done.ran).to_string();
        let mut fields = vec![
            ("call", done.name),
            ("exit", &exit),
            ("ms", &ms),
            ("cut", done.cut_field()),
        ];
        if done.name.starts_with(GIT_CALL_PREFIX) {
            fields.push(("git_ms", &git_ms));
        }
        self.log("child", &fields);
        if done.timed_out {
            self.log("timeout", &[("call", done.name)]);
        }
        if self.quitting.is_some() {
            return;
        }
        match purpose {
            CallPurpose::EnvCapture => self.env_done(done),
            CallPurpose::ClaudeVersion(path) => {
                let version = if done.succeeded() {
                    String::from_utf8_lossy(&done.stdout).trim().to_owned()
                } else {
                    "unknown".to_owned()
                };
                self.log(
                    "claude",
                    &[("path", &path.display().to_string()), ("version", &version)],
                );
            }
            CallPurpose::AddProject { name, path } => self.add_project_done(name, path, done),
            CallPurpose::Create(name) => {
                if let Some(mut create) = self.creating.remove(&name) {
                    let next = create.on_done(done);
                    self.drive_create(create, next);
                }
            }
        }
    }

    fn env_done(&mut self, done: &ChildDone) {
        let ms = whole_ms_rounded_up(done.elapsed).to_string();
        let env = match env::parse_capture(done) {
            Ok(env) => {
                self.log("env ok", &[("env_capture_ms", &ms)]);
                env
            }
            Err(reason) => {
                self.log(
                    "env failed",
                    &[("reason", &reason), ("env_capture_ms", &ms)],
                );
                self.app_line(
                    LineKind::Warning,
                    format!(
                        "Could not read your shell environment ({reason}). New Terminal uses its own environment."
                    ),
                );
                Environment::own()
            }
        };
        let env = Arc::new(env);
        let claude = env.find_program("claude");
        if let Some(path) = &claude {
            let call = Call {
                name: "claude-version",
                argv: vec![path.into(), "--version".into()],
                env: Some(Arc::clone(&env)),
                limit: worker::SHORT_LIMIT,
                term_grace: None,
            };
            self.run_call(call, CallPurpose::ClaudeVersion(path.clone()));
        } else {
            self.log("claude missing", &[]);
            self.app_line(LineKind::Warning, claude_missing(&env));
        }
        self.env = EnvState::Ready { env, claude };
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

    fn counts(&self) -> Counts {
        let live = || self.agents.values().filter(|agent| agent.exit.is_none());
        Counts {
            projects: self.registry.project_count(),
            workspaces: self.registry.workspace_count(),
            working: live()
                .filter(|agent| agent.in_turn && agent.stop.is_none())
                .count(),
            needs_you: self.attention.len(),
            failed: self.failed_targets.len(),
            agents_alive: live().count(),
            any_agent_started: !self.started_targets.is_empty(),
        }
    }

    fn send_counts(&mut self) {
        let counts = self.counts();
        if counts != self.sent_counts {
            self.sent_counts = counts;
            self.emit(Event::Status(counts));
        }
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
            self.app_line(LineKind::Error, text);
        }
    }

    /// Writes one raw line to the agent's log. The first failed write shows
    /// one warning for that agent.
    fn agent_log(&self, agent: &mut Agent, direction: Direction, line: &str) {
        let Some(log) = &mut agent.log else {
            return;
        };
        if let Err(error) = log.write(direction, line)
            && !agent.log_failed
        {
            agent.log_failed = true;
            let text = format!(
                "Could not write {}: {error}. Later agent log lines may be lost.",
                log.path().display()
            );
            self.agent_line(&agent.target, LineKind::Warning, text);
        }
    }

    fn app_line(&self, kind: LineKind, text: String) {
        self.emit(Event::Line {
            source: Source::App,
            kind,
            text,
        });
    }

    fn agent_line(&self, target: &Name, kind: LineKind, text: String) {
        self.emit(Event::Line {
            source: Source::Target(target.to_string()),
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

fn claude_missing(env: &Environment) -> String {
    format!("`claude` was not found on PATH ({}).", env.path_text())
}

fn being_created(name: &Name) -> String {
    format!("{name} is being created. Wait for it to finish.")
}

fn taken_by_create(name: &Name) -> String {
    format!("{name} is taken by workspace {name}, which is being created.")
}

fn not_built(intent_words: &str) -> String {
    format!("{intent_words} is not built yet. Nothing changed.")
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
