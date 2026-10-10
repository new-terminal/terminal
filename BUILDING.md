# Building

Build New Terminal from this repository, install it in `~/Applications`, and run the installed copy. There is no release yet, so a local build is the only way to run it.

## Prerequisites

- A Mac with Xcode or the Xcode command-line tools (`xcode-select --install`). The bundle step uses the Apple tools `strip`, `plutil`, `sips`, `iconutil`, `codesign`, and `ditto`.
- Rust 1.99 or later, with `cargo-clippy` and `cargo-fmt`. Homebrew's `rust` has all three (`brew install rust`).
- `git`.
- Claude Code, installed as `claude` and logged in. New Terminal starts `claude` for each project and workspace you send a request to. It finds `claude` on the `PATH` of your login shell.

## Build checks

Run these from the repository root. Each one must exit with status 0:

```sh
cargo build --workspace --locked
cargo clippy --workspace --locked -- -D warnings
cargo fmt --all --check
```

There is no test command. The three checks above and the app itself are the checks.

## Bundle the app

```sh
cargo xtask bundle
```

This makes a release build and writes `target/bundle/New Terminal.app`. The bundle has an ad hoc signature: a local signature with no developer identity. It runs on the Mac that built it.

## Install the app

```sh
cargo xtask install
```

This bundles the app, then replaces `~/Applications/New Terminal.app` with the new bundle. If New Terminal runs, the install stops with `Quit New Terminal first.` and changes nothing.

## First launch

1. Open `~/Applications/New Terminal.app` in Finder, or run `open ~/Applications/New\ Terminal.app`.
2. At the prompt, add a project: `add project <name> <path>`. For example, `add project terminal ~/code/new-terminal/terminal`.
3. Send a request to it: `@terminal <your request>`.

[DESIGN.md](DESIGN.md#the-first-version) lists what the first version does. The app keeps its projects, workspaces, and logs in `~/.new-terminal`. Its log is `~/.new-terminal/logs/app.log`.

## The operating rule

Run the installed copy in `~/Applications`. Never run the app from a checkout that agents change. A rebuild in that checkout can end the app while its agents work.

To upgrade:

1. Quit New Terminal. Quitting stops every agent.
2. In a checkout that has the merged changes, run `cargo xtask install`.
3. Open `~/Applications/New Terminal.app` again.

Always open the app by that path. Every bundle has the same identifier, so `open -a "New Terminal"` can start a bundle in a checkout's `target/bundle`.

## Debug builds

`cargo run -p nt-app` starts a debug build. A debug build keeps its projects, workspaces, and logs in `~/.new-terminal-dev`, so it never changes the installed app's `~/.new-terminal`.

A debug build started with `NT_DEMO=1` shows a fixed scene for reviewing the window's look. While it shows, the prompt and `Tab` send nothing to the core, and the app still writes its log in `~/.new-terminal-dev`. `NT_DEMO=dark` shows the same scene in the dark palette whatever the system appearance.
