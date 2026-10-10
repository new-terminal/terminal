//! A fixed scene for reviewing the window's look without typing, shown when
//! a debug build starts with `NT_DEMO=1`, or with `NT_DEMO=dark` to see it
//! in the dark palette whatever the system appearance. It only fills the
//! view: core events do not change it, and the prompt and `Tab` send
//! nothing to the core while it shows. The app still starts its core and
//! writes its log, as every debug build does.

use nt_core::{Counts, Label, LineKind, Source};

use crate::scrollback::Scrollback;

const SWITCH: &str = "NT_DEMO";
const DARK: &str = "dark";
const WORKSPACE: &str = "first-change";
const PROJECT: &str = "terminal";
const COMMIT: &str = "git add README.md && git commit -m \"docs: link BUILDING.md\"";

/// Whether this launch asked for the demo scene.
pub fn wanted() -> bool {
    std::env::var(SWITCH).is_ok_and(|value| value == "1" || value == DARK)
}

/// Whether this launch asked for the demo scene in the dark palette.
pub fn dark() -> bool {
    std::env::var(SWITCH).is_ok_and(|value| value == DARK)
}

pub fn label() -> Label {
    Label::Workspace {
        name: WORKSPACE.to_owned(),
        project: PROJECT.to_owned(),
    }
}

pub const fn counts() -> Counts {
    Counts {
        projects: 1,
        workspaces: 1,
        working: 1,
        needs_you: 1,
        failed: 0,
        agents_alive: 1,
        any_agent_started: true,
    }
}

/// Two requests: a workspace's change and its commit, which waits on the
/// author, then a question to the project.
pub fn stage(scrollback: &mut Scrollback) {
    let app = Source::App;
    let workspace = Source::Target(WORKSPACE.to_owned());
    let project = Source::Target(PROJECT.to_owned());

    scrollback.push_echo("new workspace first-change @terminal");
    scrollback.push_line(
        &app,
        LineKind::App,
        "Created workspace first-change on terminal: branch nt/first-change from main at c198773",
    );
    scrollback.push_line(
        &app,
        LineKind::App,
        "Isolated copy: ~/.new-terminal/workspaces/first-change/terminal",
    );

    scrollback.push_echo("@first-change add BUILDING.md to the Docs list in README.md");
    scrollback.push_line(
        &workspace,
        LineKind::App,
        "agent started in ~/.new-terminal/workspaces/first-change/terminal",
    );
    scrollback.push_line(&workspace, LineKind::Tool, "Read README.md");
    scrollback.push_line(
        &workspace,
        LineKind::Tool,
        "Edit README.md (inside the workspace, allowed)",
    );
    scrollback.push_line(
        &workspace,
        LineKind::AgentText,
        "I added a Docs entry for BUILDING.md, matching the style of the other entries.",
    );
    scrollback.push_line(&workspace, LineKind::Done, "done (41 s)");

    scrollback.push_echo("commit this as \"docs: link BUILDING.md\"");
    scrollback.push_line(&workspace, LineKind::Tool, &format!("Bash {COMMIT}"));
    scrollback.push_line(&workspace, LineKind::App, COMMIT);
    scrollback.raise_request(WORKSPACE, &format!("Allow Bash: {COMMIT} (1 line)"));

    scrollback.push_echo("@terminal what does AGENTS.md say about installing?");
    scrollback.push_line(
        &project,
        LineKind::AgentText,
        "AGENTS.md says there is no release yet and points people to terminal.new.",
    );
    scrollback.push_line(&project, LineKind::Done, "done (6 s)");
}
