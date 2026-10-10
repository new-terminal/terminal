//! One agent process: `claude` in stream-json mode, with three pipe threads
//! that never touch core state. The actor owns the handle and alone reaps
//! the process.

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, BufRead as _, BufReader, Read};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::{Value, json};

use crate::env::Environment;

/// The arguments after the program. The CLI waits on each permission
/// request until stdin answers it, and stays alive between turns while
/// stdin is open.
const ARGS: [&str; 9] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--permission-mode",
    "default",
    "--permission-prompt-tool",
];
const PERMISSION_PROMPT_TOOL: &str = "stdio";
const BYTES_PER_MIB: usize = 1024 * 1024;
/// A stdout line past this size is not kept, so one runaway line cannot
/// exhaust memory.
const LINE_LIMIT_BYTES: usize = 32 * BYTES_PER_MIB;
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Unique for the life of the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentId(pub u64);

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What a pipe thread reports.
#[derive(Debug)]
pub enum PipeEvent {
    Stdout(String),
    Stderr(String),
    StdoutEnded,
    StderrEnded,
    /// A stdout line passed the limit. It is discarded up to its newline.
    LineTooLarge {
        mib: usize,
    },
    /// The stdin writer could not write and has exited.
    WriteFailed(String),
}

/// A running agent. Dropping it sends SIGTERM to its process group unless
/// the process was already reaped, so a core panic still ends its agents.
#[derive(Debug)]
pub struct AgentHandle {
    child: Child,
    group: Pid,
    writer: mpsc::Sender<String>,
    reaped: bool,
}

impl AgentHandle {
    /// Starts `program` in `cwd` with exactly `env`, as the leader of a new
    /// process group, and starts its pipe threads. `notify` receives every
    /// pipe event.
    pub fn spawn(
        program: &Path,
        cwd: &Path,
        env: &Environment,
        notify: impl Fn(PipeEvent) + Clone + Send + 'static,
    ) -> io::Result<Self> {
        let mut child = Command::new(program)
            .args(ARGS)
            .arg(PERMISSION_PROMPT_TOOL)
            .current_dir(cwd)
            .env_clear()
            .envs(env.with_var("PWD", cwd.as_os_str()).vars())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let id = i32::try_from(child.id()).expect("process ids fit in pid_t");
        let (writer, lines) = mpsc::channel::<String>();
        if let Some(stdin) = child.stdin.take() {
            let notify = notify.clone();
            spawn_pipe_thread("nt-agent-stdin", move || {
                write_lines(stdin, &lines, &notify);
            });
        }
        if let Some(stdout) = child.stdout.take() {
            let notify = notify.clone();
            spawn_pipe_thread("nt-agent-stdout", move || {
                read_lines(stdout, &|line| notify(PipeEvent::Stdout(line)), &|mib| {
                    notify(PipeEvent::LineTooLarge { mib });
                });
                notify(PipeEvent::StdoutEnded);
            });
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_pipe_thread("nt-agent-stderr", move || {
                read_lines(stderr, &|line| notify(PipeEvent::Stderr(line)), &|_| {});
                notify(PipeEvent::StderrEnded);
            });
        }
        Ok(Self {
            child,
            group: Pid::from_raw(id),
            writer,
            reaped: false,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Queues one line for stdin. `false` when the writer has exited.
    pub fn send(&self, line: String) -> bool {
        self.writer.send(line).is_ok()
    }

    /// Signals the whole group. Does nothing once the leader is reaped,
    /// because its id, and so the group id, can then belong to another
    /// process.
    pub fn signal(&self, signal: Signal) {
        if !self.reaped {
            // An error means the group is already gone, which is the goal.
            let _ = killpg(self.group, signal);
        }
    }

    /// Reaps the process if it has exited.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.reaped |= status.is_some();
        Ok(status)
    }
}

impl Drop for AgentHandle {
    fn drop(&mut self) {
        self.signal(Signal::SIGTERM);
    }
}

/// A new turn with the author's request.
pub fn turn_line(request: &str) -> String {
    json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": request}]},
    })
    .to_string()
}

/// Allows the permission request `request_id`. `input` must be the input as
/// received, so what runs is what the author saw.
pub fn allow_line(request_id: &str, input: &Value) -> String {
    control_response(
        request_id,
        &json!({"behavior": "allow", "updatedInput": input}),
    )
}

/// Denies the permission request `request_id`, telling the agent why.
pub fn deny_line(request_id: &str, reason: &str) -> String {
    control_response(request_id, &json!({"behavior": "deny", "message": reason}))
}

/// Ends the current turn. The CLI answers with a `control_response` that
/// carries `request_id`, then a `result`.
pub fn interrupt_line(request_id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "interrupt"},
    })
    .to_string()
}

fn control_response(request_id: &str, response: &Value) -> String {
    json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": request_id, "response": response},
    })
    .to_string()
}

fn spawn_pipe_thread(name: &str, body: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .expect("start an agent pipe thread");
}

/// Writes and flushes each line in order. Returning drops stdin, which
/// closes it: when the line channel closes, or after a failed write.
fn write_lines(
    mut stdin: impl io::Write,
    lines: &mpsc::Receiver<String>,
    notify: &impl Fn(PipeEvent),
) {
    for line in lines {
        let written = stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush());
        if let Err(error) = written {
            notify(PipeEvent::WriteFailed(error.to_string()));
            return;
        }
    }
}

/// Reports each line without its newline, until end of file or a read
/// error. A line past the limit is reported once to `too_large`, in whole
/// MiB read so far, and dropped up to its newline.
fn read_lines(pipe: impl Read, on_line: &impl Fn(String), too_large: &impl Fn(usize)) {
    let mut reader = BufReader::with_capacity(READ_BUFFER_BYTES, pipe);
    let mut line = Vec::new();
    let mut dropping = false;
    loop {
        let buffer = match reader.fill_buf() {
            Ok([]) => break,
            Ok(buffer) => buffer,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let piece = newline.map_or(buffer, |end| &buffer[..end]);
        let used = newline.map_or(piece.len(), |end| end + 1);
        if !dropping {
            if line.len() + piece.len() > LINE_LIMIT_BYTES {
                dropping = true;
                too_large((line.len() + piece.len()).div_ceil(BYTES_PER_MIB));
                line.clear();
            } else {
                line.extend_from_slice(piece);
            }
        }
        reader.consume(used);
        if newline.is_some() {
            if !dropping {
                on_line(String::from_utf8_lossy(&line).into_owned());
            }
            line.clear();
            dropping = false;
        }
    }
    if !line.is_empty() && !dropping {
        on_line(String::from_utf8_lossy(&line).into_owned());
    }
}

/// The working directory as `init` reports it, for comparison with the
/// target path.
pub fn same_directory(reported: &str, target: &Path) -> bool {
    let reported = Path::new(OsStr::new(reported));
    reported == target || std::fs::canonicalize(reported).is_ok_and(|canonical| canonical == target)
}
