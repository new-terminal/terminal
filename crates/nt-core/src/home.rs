//! The app home: the directory that holds the logs, the state file and its
//! lock, and the isolated copies.

use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

const PRIVATE_DIR_MODE: u32 = 0o700;
/// Owner read and write only, for every file the core creates.
pub const PRIVATE_FILE_MODE: u32 = 0o600;

/// The canonical home path and the paths built from it.
#[derive(Debug)]
pub struct Home {
    pub root: PathBuf,
}

impl Home {
    pub fn app_log(&self) -> PathBuf {
        self.root.join("logs").join("app.log")
    }

    pub fn agent_logs(&self) -> PathBuf {
        self.root.join("logs").join("agents")
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    /// Where a save writes before it renames over [`Self::state_file`].
    pub fn state_temp(&self) -> PathBuf {
        self.root.join("state.json.tmp")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join("state.lock")
    }

    /// Holds each workspace's isolated copy at `<name>/<project>`.
    pub fn workspaces(&self) -> PathBuf {
        self.root.join("workspaces")
    }

    /// Takes the lock on `state.lock`, creating the file if missing. The
    /// lock lasts as long as the returned file stays open, and the OS
    /// releases it when the process ends.
    pub fn lock(&self) -> Result<File, LockError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(PRIVATE_FILE_MODE)
            .open(self.lock_file())
            .map_err(LockError::Failed)?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(TryLockError::WouldBlock) => Err(LockError::Held),
            Err(TryLockError::Error(error)) => Err(LockError::Failed(error)),
        }
    }
}

#[derive(Debug)]
pub enum LockError {
    /// Another process holds the lock.
    Held,
    /// The lock file could not be opened or locked.
    Failed(io::Error),
}

/// The home could not be made. Carries the path as given.
#[derive(Debug)]
pub struct HomeError {
    pub path: PathBuf,
    pub error: io::Error,
}

/// Creates the home, `logs/`, and `logs/agents/` where missing, then
/// returns the canonical form of the home. Every path built from it then
/// passes a canonical check, even when the home sits behind a link.
pub fn prepare(path: &Path) -> Result<Home, HomeError> {
    let fail = |error| HomeError {
        path: path.to_path_buf(),
        error,
    };
    let mut builder = DirBuilder::new();
    builder.recursive(true).mode(PRIVATE_DIR_MODE);
    builder
        .create(path.join("logs").join("agents"))
        .map_err(fail)?;
    let root = fs::canonicalize(path).map_err(fail)?;
    Ok(Home { root })
}
