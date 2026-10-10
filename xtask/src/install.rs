//! `cargo xtask install`: bundle the app, then put it in `~/Applications`.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::Path;
use std::process::Command;

use crate::bundle::{self, APP, tool};

/// The release build's app home, relative to the user's home directory.
const APP_HOME: &str = ".new-terminal";
/// The file a running New Terminal holds locked for its whole life.
const STATE_LOCK: &str = "state.lock";

/// Bundles the app and replaces `~/Applications/New Terminal.app` with it.
/// Refuses while a running New Terminal holds `~/.new-terminal/state.lock`.
pub fn install() -> Result<(), String> {
    let home = std::env::home_dir().ok_or("cannot find your home directory")?;
    refuse_while_running(&home.join(APP_HOME).join(STATE_LOCK))?;

    let bundle = bundle::bundle()?;
    let applications = home.join("Applications");
    fs::create_dir_all(&applications)
        .map_err(|err| format!("cannot create {}: {err}", applications.display()))?;
    let installed = applications.join(APP);
    replace(&bundle, &installed)?;

    println!("\ninstalled: {}", installed.display());
    Ok(())
}

/// Fails when another process holds the lock. A missing lock file means no
/// release build of New Terminal has run for this user, so nothing holds it.
fn refuse_while_running(lock: &Path) -> Result<(), String> {
    let file = match File::open(lock) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("cannot open {}: {err}", lock.display())),
    };
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err("Quit New Terminal first.".into()),
        Err(TryLockError::Error(err)) => Err(format!("cannot lock {}: {err}", lock.display())),
    }
}

/// Replaced whole rather than copied over: a stale file left inside a signed
/// bundle breaks its signature.
fn replace(bundle: &Path, installed: &Path) -> Result<(), String> {
    if installed.exists() {
        fs::remove_dir_all(installed)
            .map_err(|err| format!("cannot remove {}: {err}", installed.display()))?;
    }
    tool(Command::new("ditto").arg(bundle).arg(installed))
}
