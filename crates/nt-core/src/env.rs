//! The environment that git and `claude` run with. An app opened from Finder
//! or the Dock gets launchd's short `PATH`, so the core reads the author's
//! environment from a non-interactive login shell instead. An interactive
//! shell takes over 5 s on this host and can turn `claude` into an alias.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

use crate::worker::{self, Call, ChildDone};

const FALLBACK_SHELL: &str = "/bin/zsh";
const START_MARKER: &[u8] = b"NT-ENV-START\0";
const END_MARKER: &[u8] = b"NT-ENV-END\0";
/// Holds no user text. The markers fence `env -0` off from anything the
/// author's profile prints.
const SCRIPT: &str = r"printf 'NT-ENV-START\0'; env -0; printf 'NT-ENV-END\0'";
const ANY_EXECUTE_BIT: u32 = 0o111;
/// A login shell's rc files can hold a lock file when the capture ends.
/// SIGTERM lets their exit traps remove it. SIGKILL would leave it behind,
/// and the next login shells would wait on it.
const TERM_GRACE: Duration = Duration::from_millis(500);
const PATH: &str = "PATH";
const CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";

/// A whole process environment, as name and value pairs.
#[derive(Clone, Debug)]
pub struct Environment {
    vars: Vec<(OsString, OsString)>,
}

impl Environment {
    /// This process's own environment.
    pub fn own() -> Self {
        Self {
            vars: std::env::vars_os().collect(),
        }
    }

    pub fn vars(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.vars
            .iter()
            .map(|(name, value)| (name.as_os_str(), value.as_os_str()))
    }

    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_os_str())
    }

    /// `PATH`, for a message that names where a program was looked for.
    pub fn path_text(&self) -> String {
        self.get(PATH)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn claude_config_dir(&self) -> Option<&OsStr> {
        self.get(CLAUDE_CONFIG_DIR)
    }

    /// The first `<dir>/<program>` on `PATH` that is an executable file.
    /// Relative `PATH` entries are skipped, so the result is absolute.
    pub fn find_program(&self, program: &str) -> Option<PathBuf> {
        let path = self.get(PATH)?;
        std::env::split_paths(path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(program))
            .find(|candidate| {
                candidate.metadata().is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & ANY_EXECUTE_BIT != 0
                })
            })
    }

    /// This environment with `name` set to `value`.
    pub fn with_var(&self, name: &str, value: &OsStr) -> Self {
        let mut vars: Vec<_> = self
            .vars
            .iter()
            .filter(|(key, _)| key != name)
            .cloned()
            .collect();
        vars.push((name.into(), value.to_owned()));
        Self { vars }
    }
}

/// The login-shell call: `$SHELL -l -c <script>`, or `/bin/zsh` when `SHELL`
/// is unset.
pub fn capture_call() -> Call {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| FALLBACK_SHELL.into());
    Call {
        name: "env-capture",
        argv: vec![shell, "-l".into(), "-c".into(), SCRIPT.into()],
        env: None,
        limit: worker::SHORT_LIMIT,
        term_grace: Some(TERM_GRACE),
    }
}

/// The environment between the markers, or why there is none.
pub fn parse_capture(done: &ChildDone) -> Result<Environment, String> {
    if let Some(failure) = done.failure() {
        return Err(format!("the login shell failed: {failure}"));
    }
    if done.stdout_cut {
        return Err("the login shell's output passed 1 MiB".to_owned());
    }
    let missing = || "the login shell printed no environment".to_owned();
    let start = find(&done.stdout, START_MARKER).ok_or_else(missing)? + START_MARKER.len();
    let body = &done.stdout[start..];
    let end = rfind_after_nul(body, END_MARKER).ok_or_else(missing)?;
    let vars = body[..end]
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let equals = entry.iter().position(|byte| *byte == b'=')?;
            let name = OsStr::from_bytes(&entry[..equals]);
            let value = OsStr::from_bytes(&entry[equals + 1..]);
            Some((name.to_owned(), value.to_owned()))
        })
        .collect();
    Ok(Environment { vars })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The last `needle` that starts the haystack or follows a NUL, so a value
/// that happens to hold the marker text cannot end the environment early.
fn rfind_after_nul(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .rev()
        .find(|(index, window)| *window == needle && (*index == 0 || haystack[index - 1] == 0))
        .map(|(index, _)| index)
}
