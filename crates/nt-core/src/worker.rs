//! Child calls: one short-lived program per worker thread, such as the
//! environment capture, `claude --version`, or a git call. The actor never
//! waits on them. Each worker alone signals and reaps its child, so its
//! group id still belongs to that child when a signal goes.

use std::ffi::OsString;
use std::io::{self, Read};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use crate::env::Environment;

/// Each pipe keeps at most this much output, so a noisy call cannot grow
/// memory without bound.
const PIPE_CAP_BYTES: usize = 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// A process that left the call's group can hold a pipe open after the call
/// exits, so the readers get only this long to reach end of file.
const READER_GRACE: Duration = Duration::from_secs(1);
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// The limit for the environment capture and for `claude --version`.
pub const SHORT_LIMIT: Duration = Duration::from_secs(5);
/// The limit for one git call.
pub const GIT_LIMIT: Duration = Duration::from_secs(60);

/// One program to run. `argv[0]` is the program, and no shell reads any of
/// it.
#[derive(Debug)]
pub struct Call {
    /// Names the call in `app.log`.
    pub name: &'static str,
    pub argv: Vec<OsString>,
    /// The child's whole environment, or `None` to inherit this process's.
    pub env: Option<Arc<Environment>>,
    pub limit: Duration,
    /// When set, the time limit and the quit flag send SIGTERM to the
    /// child's group first, and SIGKILL only this long after it.
    pub term_grace: Option<Duration>,
}

/// What a call did. `status` is an error when the program could not start.
#[derive(Debug)]
pub struct ChildDone {
    pub name: &'static str,
    pub status: io::Result<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
    /// From the start to the child's exit, without the wait for its pipes.
    pub ran: Duration,
    pub timed_out: bool,
    /// A pipe is cut when its reader dropped bytes or did not reach end of
    /// file in time.
    pub stdout_cut: bool,
    pub stderr_cut: bool,
}

impl ChildDone {
    pub fn succeeded(&self) -> bool {
        self.status.as_ref().is_ok_and(ExitStatus::success)
    }

    /// `none`, `stdout`, `stderr`, or `both`.
    pub const fn cut_field(&self) -> &'static str {
        match (self.stdout_cut, self.stderr_cut) {
            (false, false) => "none",
            (true, false) => "stdout",
            (false, true) => "stderr",
            (true, true) => "both",
        }
    }

    /// The exit for `app.log`: a code, `signal-<n>`, or `not-started`.
    pub fn exit_field(&self) -> String {
        self.status
            .as_ref()
            .map_or_else(|_| "not-started".to_owned(), |status| status_field(*status))
    }

    /// Why the call failed, in words, with the first line it wrote to
    /// stderr, or `None` when it succeeded.
    pub fn failure(&self) -> Option<String> {
        if self.timed_out {
            return Some(format!("it did not finish in {} s", self.elapsed.as_secs()));
        }
        match &self.status {
            Err(error) => Some(format!("it could not start: {error}")),
            Ok(status) if !status.success() => {
                let stderr = String::from_utf8_lossy(&self.stderr);
                Some(match stderr.lines().find(|line| !line.trim().is_empty()) {
                    Some(said) => format!("{}: {}", status_text(*status), said.trim()),
                    None => status_text(*status),
                })
            }
            Ok(_) => None,
        }
    }
}

/// A code, or `signal-<n>`, for `app.log`.
pub fn status_field(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => code.to_string(),
        (None, Some(signal)) => format!("signal-{signal}"),
        (None, None) => "unknown".to_owned(),
    }
}

/// `exited with code <n>` or `killed by signal <n> (<name>)`.
pub fn status_text(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exited with code {code}"),
        (None, Some(number)) => {
            let name = Signal::try_from(number).map_or("unknown", Signal::as_str);
            format!("killed by signal {number} ({name})")
        }
        (None, None) => "ended with an unknown status".to_owned(),
    }
}

/// Runs `call` on a new worker thread and hands the result to `report`.
/// The child is killed at its time limit, or at once when `quit` is set.
pub fn run(call: Call, quit: Arc<AtomicBool>, report: impl FnOnce(ChildDone) + Send + 'static) {
    thread::Builder::new()
        .name(format!("nt-call-{}", call.name))
        .spawn(move || report(execute(&call, &quit)))
        .expect("start a child-call thread");
}

fn execute(call: &Call, quit: &AtomicBool) -> ChildDone {
    let started = Instant::now();
    let mut command = Command::new(&call.argv[0]);
    command
        .args(&call.argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(env) = &call.env {
        command.env_clear().envs(env.vars());
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ChildDone {
                name: call.name,
                status: Err(error),
                stdout: Vec::new(),
                stderr: Vec::new(),
                elapsed: started.elapsed(),
                ran: started.elapsed(),
                timed_out: false,
                stdout_cut: false,
                stderr_cut: false,
            };
        }
    };
    let (ended_tx, ended_rx) = mpsc::channel();
    let stdout = Capture::start(child.stdout.take(), ended_tx.clone());
    let stderr = Capture::start(child.stderr.take(), ended_tx);

    let deadline = started + call.limit;
    let (status, timed_out, quitting) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Ok(status), false, false),
            Ok(None) => {}
            Err(error) => {
                signal_group(&child, Signal::SIGKILL);
                let _ = child.wait();
                break (Err(error), false, false);
            }
        }
        let quitting = quit.load(Ordering::Relaxed);
        if quitting || Instant::now() >= deadline {
            end_group(&child, call.term_grace);
            break (child.wait(), !quitting, quitting);
        }
        thread::sleep(POLL_INTERVAL);
    };
    let ran = started.elapsed();

    let grace = if quitting {
        Duration::ZERO
    } else {
        READER_GRACE
    };
    let grace_end = Instant::now() + grace;
    for _ in 0..2 {
        let left = grace_end.saturating_duration_since(Instant::now());
        if ended_rx.recv_timeout(left).is_err() {
            break;
        }
    }
    let (stdout, stdout_cut) = stdout.take();
    let (stderr, stderr_cut) = stderr.take();
    ChildDone {
        name: call.name,
        status,
        stdout,
        stderr,
        elapsed: started.elapsed(),
        ran,
        timed_out,
        stdout_cut,
        stderr_cut,
    }
}

/// Sends SIGKILL to the child's group, after SIGTERM and `term_grace` when
/// set. The grace is a sleep, not a reaping wait, so the group id stays the
/// child's until the SIGKILL goes.
fn end_group(child: &Child, term_grace: Option<Duration>) {
    if let Some(grace) = term_grace {
        signal_group(child, Signal::SIGTERM);
        thread::sleep(grace);
    }
    signal_group(child, Signal::SIGKILL);
}

fn signal_group(child: &Child, signal: Signal) {
    let id = i32::try_from(child.id()).expect("process ids fit in pid_t");
    // An error means the group is already gone, which is the goal.
    let _ = killpg(Pid::from_raw(id), signal);
}

#[derive(Debug, Default)]
struct Captured {
    bytes: Vec<u8>,
    dropped: bool,
    ended: bool,
}

/// One pipe's output, read on its own thread so a full pipe never stalls
/// the child.
#[derive(Debug, Clone, Default)]
struct Capture(Arc<Mutex<Captured>>);

impl Capture {
    fn start<R: Read + Send + 'static>(pipe: Option<R>, ended: mpsc::Sender<()>) -> Self {
        let capture = Self::default();
        let Some(mut pipe) = pipe else {
            return capture;
        };
        let shared = capture.clone();
        thread::Builder::new()
            .name("nt-call-pipe".to_owned())
            .spawn(move || {
                let mut chunk = vec![0; READ_CHUNK_BYTES];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => shared.keep(&chunk[..read]),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                shared.lock().ended = true;
                let _ = ended.send(());
            })
            .expect("start a pipe reader thread");
        capture
    }

    fn keep(&self, chunk: &[u8]) {
        let mut captured = self.lock();
        let room = PIPE_CAP_BYTES.saturating_sub(captured.bytes.len());
        if chunk.len() > room {
            captured.dropped = true;
        }
        captured
            .bytes
            .extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    /// The bytes so far, and whether the pipe is cut.
    fn take(&self) -> (Vec<u8>, bool) {
        let mut captured = self.lock();
        let cut = captured.dropped || !captured.ended;
        (std::mem::take(&mut captured.bytes), cut)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Captured> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
