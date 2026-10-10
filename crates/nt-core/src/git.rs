//! Git calls: their argument vectors and the parsers for the output the
//! core reads. Each call names its repository with `-C`, so no call depends
//! on a working directory, and each runs on a worker with the git limit.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::env::Environment;
use crate::worker::{self, Call, ChildDone};

/// `show-ref --verify --quiet` exits with this code when the ref is absent.
const REF_ABSENT_CODE: i32 = 1;
/// Where the path starts in a `status --porcelain` line, after `XY `.
const STATUS_PATH_START: usize = 3;
/// `rev-parse --verify --quiet` exits with this code, and prints nothing,
/// when its argument names no commit, as `HEAD` does in a repository with
/// no commits yet. Outside a repository git exits with 128.
pub const NOT_A_COMMIT_CODE: i32 = 1;

/// One repository, as git calls name it, with the environment they run in.
#[derive(Clone, Debug)]
pub struct Repo {
    path: PathBuf,
    env: Arc<Environment>,
}

impl Repo {
    pub const fn new(path: PathBuf, env: Arc<Environment>) -> Self {
        Self { path, env }
    }

    pub fn show_toplevel(&self) -> Call {
        self.call("git-rev-parse", &["rev-parse", "--show-toplevel"])
    }

    pub fn verify_head(&self) -> Call {
        self.call("git-rev-parse", &["rev-parse", "--verify", "HEAD"])
    }

    pub fn head_branch(&self) -> Call {
        self.call("git-rev-parse", &["rev-parse", "--abbrev-ref", "HEAD"])
    }

    pub fn show_branch(&self, branch: &str) -> Call {
        let reference = branch_ref(branch);
        self.call(
            "git-show-ref",
            &["show-ref", "--verify", "--quiet", &reference],
        )
    }

    pub fn worktree_add(&self, branch: &str, copy: &Path, base: &str) -> Call {
        self.call(
            "git-worktree-add",
            &[
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("-b"),
                OsStr::new(branch),
                copy.as_os_str(),
                OsStr::new(base),
            ],
        )
    }

    pub fn worktree_remove_force(&self, copy: &Path) -> Call {
        self.call(
            "git-worktree-remove",
            &[
                OsStr::new("worktree"),
                OsStr::new("remove"),
                OsStr::new("--force"),
                copy.as_os_str(),
            ],
        )
    }

    /// Removes a clean copy. Git refuses one with changed or untracked
    /// files, and removes its ignored files with it.
    pub fn worktree_remove(&self, copy: &Path) -> Call {
        self.call(
            "git-worktree-remove",
            &[
                OsStr::new("worktree"),
                OsStr::new("remove"),
                copy.as_os_str(),
            ],
        )
    }

    /// Lists changed and untracked files. Run it on the copy, not the
    /// project.
    pub fn status_porcelain(&self) -> Call {
        self.call("git-status", &["status", "--porcelain"])
    }

    pub fn worktree_list(&self) -> Call {
        self.call(
            "git-worktree-list",
            &["worktree", "list", "--porcelain", "-z"],
        )
    }

    /// Prints a branch that holds `commit`, or nothing when none does.
    pub fn branch_containing(&self, commit: &str) -> Call {
        self.call(
            "git-for-each-ref",
            &[
                "for-each-ref",
                "--count=1",
                "--contains",
                commit,
                "refs/heads/",
            ],
        )
    }

    pub fn worktree_prune(&self) -> Call {
        self.call("git-worktree-prune", &["worktree", "prune"])
    }

    /// Deletes `branch` only while it points at `base`, so it never drops a
    /// commit that only this branch holds.
    pub fn delete_branch_at(&self, branch: &str, base: &str) -> Call {
        let reference = branch_ref(branch);
        self.call("git-update-ref", &["update-ref", "-d", &reference, base])
    }

    fn call<S: AsRef<OsStr>>(&self, name: &'static str, args: &[S]) -> Call {
        let mut argv: Vec<OsString> = vec!["git".into(), "-C".into(), self.path.clone().into()];
        argv.extend(args.iter().map(|arg| arg.as_ref().to_owned()));
        Call {
            name,
            argv,
            env: Some(Arc::clone(&self.env)),
            limit: worker::GIT_LIMIT,
            term_grace: None,
        }
    }
}

fn branch_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

/// Why a git call gave no usable answer. `words` in each text names the
/// call as the author would type it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Git ran to its end and exited with a code other than 0.
    Refused { code: i32, text: String },
    /// Git did not start, did not finish in time, was killed, or printed
    /// more than the core reads.
    Broken(String),
}

impl Failure {
    pub fn text(&self) -> &str {
        match self {
            Self::Refused { text, .. } | Self::Broken(text) => text,
        }
    }
}

/// The call's stdout, trimmed, when it exited with 0 and its stdout is
/// whole.
pub fn stdout(done: &ChildDone, words: &str) -> Result<String, Failure> {
    let bytes = whole_stdout(done, words)?;
    Ok(String::from_utf8_lossy(bytes).trim().to_owned())
}

/// The call's stdout as git wrote it, when it exited with 0 and its stdout
/// is whole.
pub fn whole_stdout<'a>(done: &'a ChildDone, words: &str) -> Result<&'a [u8], Failure> {
    succeeded(done, words)?;
    if done.stdout_cut {
        return Err(Failure::Broken(format!(
            "{words} printed more than the 1 MiB that New Terminal reads"
        )));
    }
    Ok(&done.stdout)
}

/// `Ok` when the call exited with 0. Its output is not read.
pub fn succeeded(done: &ChildDone, words: &str) -> Result<(), Failure> {
    let Some(failure) = done.failure() else {
        return Ok(());
    };
    let text = format!("{words}: {failure}");
    match &done.status {
        Ok(status) if !done.timed_out && status.signal().is_none() => Err(Failure::Refused {
            code: status.code().unwrap_or_default(),
            text,
        }),
        _ => Err(Failure::Broken(text)),
    }
}

/// Whether `show-ref --verify --quiet` found its branch.
pub fn branch_exists(done: &ChildDone, words: &str) -> Result<bool, Failure> {
    match succeeded(done, words) {
        Ok(()) => Ok(true),
        Err(Failure::Refused {
            code: REF_ABSENT_CODE,
            ..
        }) => Ok(false),
        Err(failure) => Err(failure),
    }
}

/// One entry of `git worktree list --porcelain -z`, with the fields the
/// core reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// `None` for a bare repository.
    pub head: Option<String>,
    /// Git marks an entry prunable when its directory is gone, and
    /// `worktree prune` then drops its record and its HEAD.
    pub prunable: bool,
}

/// Parses `git worktree list --porcelain -z`. A NUL ends each field, and an
/// empty field ends each entry.
pub fn parse_worktree_list(bytes: &[u8]) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for field in bytes.split(|byte| *byte == 0) {
        if field.is_empty() {
            entries.extend(current.take());
            continue;
        }
        let (key, value) = match field.iter().position(|byte| *byte == b' ') {
            Some(space) => (&field[..space], &field[space + 1..]),
            None => (field, &[][..]),
        };
        match (key, current.as_mut()) {
            (b"worktree", _) => {
                entries.extend(current.take());
                current = Some(WorktreeEntry {
                    path: PathBuf::from(OsStr::from_bytes(value)),
                    head: None,
                    prunable: false,
                });
            }
            (b"HEAD", Some(entry)) => {
                entry.head = Some(String::from_utf8_lossy(value).into_owned());
            }
            (b"prunable", Some(entry)) => entry.prunable = true,
            _ => {}
        }
    }
    entries.extend(current);
    entries
}

/// What `git status --porcelain` printed: one line per changed or
/// untracked path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changes {
    pub count: usize,
    /// The first paths, as git shows them, at most the number asked for.
    pub shown: Vec<String>,
}

/// Parses `git status --porcelain`, keeping the first `keep` paths. Each
/// line is a two-letter status, a space, then the path.
pub fn parse_status(bytes: &[u8], keep: usize) -> Changes {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    Changes {
        count: lines.len(),
        shown: lines
            .iter()
            .take(keep)
            .map(|line| line.get(STATUS_PATH_START..).unwrap_or(line).to_owned())
            .collect(),
    }
}

/// The first 7 characters of a commit id, as git shows it.
pub fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}
