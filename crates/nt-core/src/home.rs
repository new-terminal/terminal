//! The app home: the directory that holds the logs, and later the state file
//! and the isolated copies.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

const PRIVATE_DIR_MODE: u32 = 0o700;

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
