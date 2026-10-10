//! `cargo xtask bundle`: the macOS app, from a release build.
//!
//! Plain Apple tools and nothing to install: `sips` and `iconutil` draw the
//! icon from its SVG, and `codesign` signs. Everything lands in
//! `target/bundle/`.
//!
//! Signing is ad hoc: a local signature with no developer identity. The app
//! runs on the Mac that built it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const APP: &str = "New Terminal.app";
const EXECUTABLE: &str = "new-terminal";
const PACKAGE: &str = "nt-app";
const ICON_SOURCE: &str = "brand/svg/new-terminal-avatar-color.svg";
const INFO_PLIST: &str = include_str!("../Info.plist.in");
/// What the binary is built for at least; `LSMinimumSystemVersion` in
/// `xtask/Info.plist.in` says the same, and GPUI needs 10.15.7.
const DEPLOYMENT_TARGET: &str = "11.0";

/// The iconset `iconutil` expects: each point size at 1x and 2x.
const ICON_SIZES: [(&str, u32); 10] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

/// Builds `target/bundle/New Terminal.app` and returns its path. Any earlier
/// bundle at that path is replaced.
pub fn bundle() -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("the app bundle is built on macOS".into());
    }
    let root = workspace_root();
    let version = env!("CARGO_PKG_VERSION");
    let out = root.join("target/bundle");
    let app = out.join(APP);

    // The target directory is named rather than inherited: a
    // `CARGO_TARGET_DIR` or `build.target-dir` elsewhere would put the fresh
    // binary there and leave a stale one here to be bundled.
    let target = root.join("target");
    let cargo = option_env!("CARGO").unwrap_or("cargo");
    tool(
        Command::new(cargo)
            .current_dir(&root)
            .args(["build", "--release", "--locked", "-p", PACKAGE])
            .arg("--target-dir")
            .arg(&target)
            .env("MACOSX_DEPLOYMENT_TARGET", DEPLOYMENT_TARGET),
    )?;

    // Only ever the bundle this task made, never anything else in target/.
    if app.exists() {
        fs::remove_dir_all(&app).map_err(|err| format!("cannot clear {}: {err}", app.display()))?;
    }
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos, &resources] {
        fs::create_dir_all(dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    }

    let binary = macos.join(EXECUTABLE);
    copy(&target.join("release").join(EXECUTABLE), &binary)?;
    // Local symbols only: the backtrace of a panic still names functions.
    tool(Command::new("strip").arg("-x").arg(&binary))?;

    write(
        &contents.join("Info.plist"),
        &INFO_PLIST.replace("@VERSION@", version),
    )?;
    tool(
        Command::new("plutil")
            .arg("-lint")
            .arg(contents.join("Info.plist")),
    )?;

    icon(
        &root.join(ICON_SOURCE),
        &out,
        &resources.join("AppIcon.icns"),
    )?;

    sign(&app)?;

    println!("\n{}", app.display());
    Ok(app)
}

/// `AppIcon.icns` from the SVG, through an iconset of every size.
fn icon(svg: &Path, scratch: &Path, icns: &Path) -> Result<(), String> {
    let iconset = scratch.join("AppIcon.iconset");
    if iconset.exists() {
        fs::remove_dir_all(&iconset)
            .map_err(|err| format!("cannot clear {}: {err}", iconset.display()))?;
    }
    fs::create_dir_all(&iconset)
        .map_err(|err| format!("cannot create {}: {err}", iconset.display()))?;
    for (name, pixels) in ICON_SIZES {
        let pixels = pixels.to_string();
        tool(
            Command::new("sips")
                .args(["-s", "format", "png", "-z", &pixels, &pixels])
                .arg(svg)
                .arg("--out")
                .arg(iconset.join(name))
                // sips names every file it writes; the errors still show.
                .stdout(Stdio::null()),
        )?;
    }
    tool(
        Command::new("iconutil")
            .args(["-c", "icns"])
            .arg(&iconset)
            .arg("-o")
            .arg(icns),
    )
}

/// Signs the bundle ad hoc, then checks that the signature holds.
///
/// `strip` breaks the signature the linker made, and arm64 macOS runs only
/// signed code, so an unsigned bundle would not launch.
fn sign(app: &Path) -> Result<(), String> {
    tool(
        Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(app),
    )?;
    tool(
        Command::new("codesign")
            .args(["--verify", "--strict", "--verbose=2"])
            .arg(app),
    )
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(drop)
        .map_err(|err| format!("cannot copy {} to {}: {err}", from.display(), to.display()))
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

/// Runs one tool, inheriting the terminal so its own report is seen.
pub fn tool(command: &mut Command) -> Result<(), String> {
    let display = format!("{command:?}");
    let status = command
        .stdin(Stdio::null())
        .status()
        .map_err(|err| format!("could not start {display}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{display} failed"))
    }
}
