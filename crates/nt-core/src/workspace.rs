//! Workspace create and archive. A workspace is an isolated copy of one
//! project, made as a git worktree on branch `nt/<name>` under the app
//! home. The steps run as git calls that the actor starts one at a time.
//! This module decides each next step from the last result, so the actor
//! never waits on git.
//!
//! Neither ever deletes a commit or an uncommitted change. A create that
//! fails after git made something rolls back what it made, but a copy or a
//! worktree record whose HEAD no branch holds stays, and the author hears
//! what was left. An archive refuses while the copy has changes or a HEAD it
//! would drop is on no branch, and it keeps the branch.

use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::env::Environment;
use crate::git::{self, Failure, Repo, WorktreeEntry};
use crate::grammar::Name;
use crate::registry::Workspace;
use crate::worker::{Call, ChildDone};

const BRANCH_PREFIX: &str = "nt/";
const PRIVATE_DIR_MODE: u32 = 0o700;

const TOP_LEVEL_WORDS: &str = "git rev-parse --show-toplevel";
const VERIFY_HEAD_WORDS: &str = "git rev-parse --verify HEAD";
const HEAD_BRANCH_WORDS: &str = "git rev-parse --abbrev-ref HEAD";
const SHOW_REF_WORDS: &str = "git show-ref";
const WORKTREE_ADD_WORDS: &str = "git worktree add";
const WORKTREE_REMOVE_WORDS: &str = "git worktree remove";
const WORKTREE_LIST_WORDS: &str = "git worktree list";
const FOR_EACH_REF_WORDS: &str = "git for-each-ref";
const WORKTREE_PRUNE_WORDS: &str = "git worktree prune";
const UPDATE_REF_WORDS: &str = "git update-ref";
const STATUS_WORDS: &str = "git status --porcelain";
/// How many changed paths the uncommitted-changes refusal names.
const SHOWN_CHANGES: usize = 5;

/// The branch a workspace's copy starts on.
pub fn branch_name(name: &Name) -> String {
    format!("{BRANCH_PREFIX}{name}")
}

/// The commits a HEAD check must find on a local branch: each worktree
/// entry's HEAD, so removing the entry or its copy drops no commit that
/// only that HEAD holds.
#[derive(Debug, Default)]
pub struct HeadCheck {
    pending: VecDeque<Held>,
}

/// A worktree entry's HEAD commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub path: PathBuf,
    pub head: String,
}

/// Where a HEAD check stands.
#[derive(Debug)]
pub enum Checking {
    Run(Call),
    Passed,
    NoBranch(Held),
    /// A call failed, so the check cannot pass.
    Failed(String),
}

impl HeadCheck {
    /// An entry with no HEAD, or an all-zero HEAD, holds no commit and
    /// passes at once.
    pub fn of<'a>(entries: impl IntoIterator<Item = &'a WorktreeEntry>) -> Self {
        let pending = entries
            .into_iter()
            .filter_map(|entry| {
                let head = entry.head.as_ref()?;
                (!head.bytes().all(|byte| byte == b'0')).then(|| Held {
                    path: entry.path.clone(),
                    head: head.clone(),
                })
            })
            .collect();
        Self { pending }
    }

    pub fn next(&self, repo: &Repo) -> Checking {
        self.pending.front().map_or(Checking::Passed, |held| {
            Checking::Run(repo.branch_containing(&held.head))
        })
    }

    /// Takes the result of the call that [`Self::next`] last returned.
    pub fn on_done(&mut self, repo: &Repo, done: &ChildDone) -> Checking {
        let Some(held) = self.pending.pop_front() else {
            return Checking::Passed;
        };
        match git::stdout(done, FOR_EACH_REF_WORDS) {
            Err(failure) => Checking::Failed(failure.text().to_owned()),
            Ok(branch) if branch.is_empty() => Checking::NoBranch(held),
            Ok(_) => self.next(repo),
        }
    }
}

/// What the actor does next for a create.
#[derive(Debug)]
pub enum Next {
    Run(Call),
    /// Git made the copy. Save the state file with this entry, then call
    /// [`Create::saved`] or [`Create::save_failed`].
    Save(Workspace),
    /// The create ended. Show the lines, as app lines when `created`, or as
    /// error lines.
    Done {
        created: bool,
        lines: Vec<String>,
    },
}

/// Everything a create needs, fixed when the author sends the line.
#[derive(Debug)]
pub struct Request {
    pub name: Name,
    pub project: Name,
    pub project_path: PathBuf,
    /// `<home>/workspaces`, canonical.
    pub workspaces_dir: PathBuf,
    /// Shows a path as the author would type it, with `~`.
    pub home_dir: PathBuf,
    pub env: Arc<Environment>,
}

#[derive(Debug)]
enum Stage {
    TopLevel,
    VerifyHead,
    HeadBranch,
    BranchFree,
    Add,
    Saving,
    List,
    CopyHead(HeadCheck),
    BranchKept,
    Remove,
    PruneHeads(HeadCheck),
    Prune,
    BranchLeft,
    DeleteBranch,
}

/// One `new workspace` in progress. Its name stays reserved until it
/// returns [`Next::Done`].
#[derive(Debug)]
pub struct Create {
    request: Request,
    repo: Repo,
    branch: String,
    copy: PathBuf,
    base: String,
    from_branch: String,
    stage: Stage,
    /// The message of the step that failed, shown when the rollback ends.
    failure: String,
    /// Whether git listed the copy as a worktree.
    copy_known: bool,
    prunable: Vec<WorktreeEntry>,
    /// What the rollback could not remove, each with its reason.
    left: Vec<String>,
}

impl Create {
    pub fn start(request: Request) -> (Self, Next) {
        let repo = Repo::new(request.project_path.clone(), Arc::clone(&request.env));
        let copy = request
            .workspaces_dir
            .join(request.name.as_ref())
            .join(request.project.as_ref());
        let create = Self {
            branch: branch_name(&request.name),
            repo,
            copy,
            request,
            base: String::new(),
            from_branch: String::new(),
            stage: Stage::TopLevel,
            failure: String::new(),
            copy_known: false,
            prunable: Vec::new(),
            left: Vec::new(),
        };
        let next = Next::Run(create.repo.show_toplevel());
        (create, next)
    }

    pub const fn name(&self) -> &Name {
        &self.request.name
    }

    /// Takes the result of the call that the last [`Next::Run`] named.
    pub fn on_done(&mut self, done: &ChildDone) -> Next {
        match std::mem::replace(&mut self.stage, Stage::Saving) {
            Stage::TopLevel => self.top_level_done(done),
            Stage::VerifyHead => self.verify_head_done(done),
            Stage::HeadBranch => self.head_branch_done(done),
            Stage::BranchFree => self.branch_free_done(done),
            Stage::Add => self.add_done(done),
            Stage::Saving => unreachable!("a save runs no git call"),
            Stage::List => self.list_done(done),
            Stage::CopyHead(mut check) => {
                let checking = check.on_done(&self.repo, done);
                self.copy_head(check, checking)
            }
            Stage::BranchKept => self.branch_kept_done(done),
            Stage::Remove => self.remove_done(done),
            Stage::PruneHeads(mut check) => {
                let checking = check.on_done(&self.repo, done);
                self.prune_heads(check, checking)
            }
            Stage::Prune => self.prune_done(done),
            Stage::BranchLeft => self.branch_left_done(done),
            Stage::DeleteBranch => self.delete_branch_done(done),
        }
    }

    /// The state file holds the new entry, so the create is done.
    pub fn saved(&self) -> Next {
        let shown_copy = self.shown(&self.copy);
        Next::Done {
            created: true,
            lines: vec![
                format!(
                    "Created workspace {} on {}: branch {} from {} at {}",
                    self.request.name,
                    self.request.project,
                    self.branch,
                    self.from_branch,
                    git::short(&self.base)
                ),
                format!("Isolated copy: {shown_copy}"),
            ],
        }
    }

    /// The state file could not be written, so the copy and the branch go.
    pub fn save_failed(&mut self, message: String) -> Next {
        self.roll_back(message)
    }

    fn run(&mut self, stage: Stage, call: Call) -> Next {
        self.stage = stage;
        Next::Run(call)
    }

    fn refuse(text: String) -> Next {
        Next::Done {
            created: false,
            lines: vec![text],
        }
    }

    fn could_not(&self, failure: &Failure) -> Next {
        Self::refuse(format!(
            "Could not create workspace {}: {}.",
            self.request.name,
            failure.text()
        ))
    }

    fn shown(&self, path: &Path) -> String {
        crate::paths::with_tilde(path, &self.request.home_dir)
    }

    fn top_level_done(&mut self, done: &ChildDone) -> Next {
        let project = &self.request.project;
        match git::stdout(done, TOP_LEVEL_WORDS) {
            Err(Failure::Refused { .. }) => Self::refuse(format!(
                "{project} is not a git repository. Workspaces need git in this version. Send requests to @{project} directly."
            )),
            Err(failure) => self.could_not(&failure),
            Ok(top) => {
                let top = PathBuf::from(top);
                let top = fs::canonicalize(&top).unwrap_or(top);
                if top != self.request.project_path {
                    return Self::refuse(format!(
                        "{project} is inside the git repository at {}, not at its top level. Workspaces need the top level in this version.",
                        top.display()
                    ));
                }
                let call = self.repo.verify_head();
                self.run(Stage::VerifyHead, call)
            }
        }
    }

    fn verify_head_done(&mut self, done: &ChildDone) -> Next {
        match git::stdout(done, VERIFY_HEAD_WORDS) {
            Err(Failure::Refused { .. }) => Self::refuse(format!(
                "{} has no commits yet. Make a first commit, then create the workspace.",
                self.request.project
            )),
            Err(failure) => self.could_not(&failure),
            Ok(base) => {
                self.base = base;
                let call = self.repo.head_branch();
                self.run(Stage::HeadBranch, call)
            }
        }
    }

    fn head_branch_done(&mut self, done: &ChildDone) -> Next {
        match git::stdout(done, HEAD_BRANCH_WORDS) {
            Err(failure) => self.could_not(&failure),
            Ok(branch) => {
                self.from_branch = branch;
                let call = self.repo.show_branch(&self.branch);
                self.run(Stage::BranchFree, call)
            }
        }
    }

    fn branch_free_done(&mut self, done: &ChildDone) -> Next {
        let project = &self.request.project;
        let branch = &self.branch;
        match git::branch_exists(done, SHOW_REF_WORDS) {
            Err(failure) => return self.could_not(&failure),
            Ok(true) => {
                return Self::refuse(format!(
                    "{project} already has branch {branch}, maybe from an archived workspace. Pick another name, or delete the branch first, for example: @{project} delete branch {branch}"
                ));
            }
            Ok(false) => {}
        }
        if fs::symlink_metadata(&self.copy).is_ok() {
            return Self::refuse(format!(
                "{} is left over from an interrupted create. Pick another name, or remove it first, for example: @{project} remove that isolated copy and branch {branch}",
                self.shown(&self.copy)
            ));
        }
        let parent = self.workspace_dir();
        if let Err(error) = DirBuilder::new()
            .recursive(true)
            .mode(PRIVATE_DIR_MODE)
            .create(&parent)
        {
            return self.roll_back(format!(
                "Could not make the isolated copy: could not create {}: {error}. Nothing changed.",
                self.shown(&parent)
            ));
        }
        let call = self.repo.worktree_add(&self.branch, &self.copy, &self.base);
        self.run(Stage::Add, call)
    }

    fn add_done(&mut self, done: &ChildDone) -> Next {
        if done.timed_out {
            return self.roll_back(format!(
                "git did not finish making the isolated copy in {} s. A git hook can cause this. Nothing changed.",
                done.elapsed.as_secs()
            ));
        }
        if let Err(failure) = git::succeeded(done, WORKTREE_ADD_WORDS) {
            return self.roll_back(format!(
                "Could not make the isolated copy: {}. Nothing changed.",
                failure.text()
            ));
        }
        self.stage = Stage::Saving;
        Next::Save(Workspace {
            name: self.request.name.clone(),
            project: self.request.project.clone(),
            branch: self.branch.clone(),
            checkout: self.copy.clone(),
            base: self.base.clone(),
        })
    }

    /// `<home>/workspaces/<name>`, which holds the copy.
    fn workspace_dir(&self) -> PathBuf {
        self.request.workspaces_dir.join(self.request.name.as_ref())
    }

    fn roll_back(&mut self, failure: String) -> Next {
        self.failure = failure;
        let call = self.repo.worktree_list();
        self.run(Stage::List, call)
    }

    fn list_done(&mut self, done: &ChildDone) -> Next {
        let entries = match git::stdout(done, WORKTREE_LIST_WORDS) {
            Ok(_) => git::parse_worktree_list(&done.stdout),
            Err(failure) => {
                let reason = failure.text().to_owned();
                return self.keep_copy(&reason);
            }
        };
        let copy_entry = entries.iter().find(|entry| entry.path == self.copy);
        self.copy_known = copy_entry.is_some();
        let check = HeadCheck::of(copy_entry);
        self.prunable = entries
            .iter()
            .filter(|entry| entry.prunable && entry.path != self.copy)
            .cloned()
            .collect();
        let checking = check.next(&self.repo);
        self.copy_head(check, checking)
    }

    fn copy_head(&mut self, check: HeadCheck, checking: Checking) -> Next {
        match checking {
            Checking::Run(call) => self.run(Stage::CopyHead(check), call),
            Checking::Passed => self.remove_copy(),
            Checking::NoBranch(held) => self.keep_copy(&format!(
                "its HEAD {} is on no branch",
                git::short(&held.head)
            )),
            Checking::Failed(reason) => self.keep_copy(&reason),
        }
    }

    /// The HEAD check did not pass, so the copy stays, and the rollback
    /// ends after it finds out whether the branch is left too.
    fn keep_copy(&mut self, reason: &str) -> Next {
        let kept = if fs::symlink_metadata(&self.copy).is_ok() {
            "the isolated copy"
        } else {
            "the worktree record of"
        };
        self.left
            .push(format!("{kept} {} ({reason})", self.shown(&self.copy)));
        let call = self.repo.show_branch(&self.branch);
        self.run(Stage::BranchKept, call)
    }

    fn branch_kept_done(&mut self, done: &ChildDone) -> Next {
        match git::branch_exists(done, SHOW_REF_WORDS) {
            Ok(false) => {}
            Ok(true) => self.left.push(format!(
                "branch {} in {} (kept with the copy)",
                self.branch, self.request.project
            )),
            Err(failure) => self.left.push(format!(
                "maybe branch {} in {} ({})",
                self.branch,
                self.request.project,
                failure.text()
            )),
        }
        self.finish()
    }

    fn remove_copy(&mut self) -> Next {
        if fs::symlink_metadata(&self.copy).is_err() {
            return self.prune_step();
        }
        if self.copy_known {
            let call = self.repo.worktree_remove_force(&self.copy);
            return self.run(Stage::Remove, call);
        }
        if let Err(reason) = self.delete_unknown_copy() {
            self.left.push(format!(
                "the isolated copy {} ({reason})",
                self.shown(&self.copy)
            ));
        }
        self.prune_step()
    }

    /// Deletes a copy that git does not list, only when it sits under
    /// `<home>/workspaces/` with no `..` part and is no link, so the
    /// rollback never deletes anything outside the app's own copies.
    fn delete_unknown_copy(&self) -> Result<(), String> {
        let inside = self.copy.starts_with(&self.request.workspaces_dir)
            && !self
                .copy
                .components()
                .any(|part| part == Component::ParentDir);
        if !inside {
            return Err(format!(
                "it is not under {}",
                self.shown(&self.request.workspaces_dir)
            ));
        }
        let is_link = fs::symlink_metadata(&self.copy)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_symlink();
        if is_link {
            return Err("it is a link".to_owned());
        }
        fs::remove_dir_all(&self.copy).map_err(|error| error.to_string())
    }

    fn remove_done(&mut self, done: &ChildDone) -> Next {
        if let Err(failure) = git::succeeded(done, WORKTREE_REMOVE_WORDS) {
            self.left.push(format!(
                "the isolated copy {} ({})",
                self.shown(&self.copy),
                failure.text()
            ));
        }
        self.prune_step()
    }

    fn prune_step(&mut self) -> Next {
        let check = HeadCheck::of(&self.prunable);
        let checking = check.next(&self.repo);
        self.prune_heads(check, checking)
    }

    fn prune_heads(&mut self, check: HeadCheck, checking: Checking) -> Next {
        match checking {
            Checking::Run(call) => return self.run(Stage::PruneHeads(check), call),
            Checking::Passed => {
                let call = self.repo.worktree_prune();
                return self.run(Stage::Prune, call);
            }
            Checking::NoBranch(held) => self.left.push(format!(
                "the worktree records of {} that git worktree prune would remove ({}'s HEAD {} is on no branch)",
                self.request.project,
                held.path.display(),
                git::short(&held.head)
            )),
            Checking::Failed(reason) => self.left.push(format!(
                "the worktree records of {} that git worktree prune would remove ({reason})",
                self.request.project
            )),
        }
        self.branch_step()
    }

    fn prune_done(&mut self, done: &ChildDone) -> Next {
        if let Err(failure) = git::succeeded(done, WORKTREE_PRUNE_WORDS) {
            self.left.push(format!(
                "the worktree records of {} that git worktree prune would remove ({})",
                self.request.project,
                failure.text()
            ));
        }
        self.branch_step()
    }

    /// Looks for the branch first, because `update-ref -d` fails on a
    /// branch that `worktree add` never made.
    fn branch_step(&mut self) -> Next {
        let call = self.repo.show_branch(&self.branch);
        self.run(Stage::BranchLeft, call)
    }

    fn branch_left_done(&mut self, done: &ChildDone) -> Next {
        match git::branch_exists(done, SHOW_REF_WORDS) {
            Ok(false) => self.finish(),
            Ok(true) => {
                let call = self.repo.delete_branch_at(&self.branch, &self.base);
                self.run(Stage::DeleteBranch, call)
            }
            Err(failure) => {
                self.left.push(format!(
                    "maybe branch {} in {} ({})",
                    self.branch,
                    self.request.project,
                    failure.text()
                ));
                self.finish()
            }
        }
    }

    fn delete_branch_done(&mut self, done: &ChildDone) -> Next {
        if let Err(failure) = git::succeeded(done, UPDATE_REF_WORDS) {
            self.left.push(format!(
                "branch {} in {} ({})",
                self.branch,
                self.request.project,
                failure.text()
            ));
        }
        self.finish()
    }

    fn finish(&mut self) -> Next {
        // Fails, as it should, while a kept copy is still inside.
        let _ = fs::remove_dir(self.workspace_dir());
        let mut lines = vec![std::mem::take(&mut self.failure)];
        if !self.left.is_empty() {
            lines.push(format!("Not cleaned up: {}.", self.left.join(", ")));
        }
        Next::Done {
            created: false,
            lines,
        }
    }
}

/// Everything an archive needs, fixed when the author sends the line.
#[derive(Debug)]
pub struct ArchiveRequest {
    pub workspace: Workspace,
    pub project_path: PathBuf,
    /// `<home>/workspaces`, canonical.
    pub workspaces_dir: PathBuf,
    /// Shows a path as the author would type it, with `~`.
    pub home_dir: PathBuf,
    pub env: Arc<Environment>,
}

/// What the actor does next for an archive.
#[derive(Debug)]
pub enum ArchiveNext {
    Run(Call),
    /// Run the stop steps for the workspace's agent, if it has one, and
    /// call [`Archive::agent_gone`] once the actor has dropped its record.
    EndAgent,
    /// Git removed the copy, or the record of a copy that was gone. Remove
    /// the entry, clear the target if it names the workspace, and save the
    /// state file. Then call [`Archive::saved`] or [`Archive::save_failed`].
    Save,
    /// The archive ended. Show the lines, as app lines when `archived`, or
    /// as error lines.
    Done {
        archived: bool,
        lines: Vec<String>,
    },
}

/// The HEAD check runs before the agent ends, so a refusal keeps its
/// memory, and again after it ends, because a command the agent left
/// running can move HEAD or delete the copy before the stop ends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    BeforeStop,
    AfterStop,
}

#[derive(Debug)]
enum ArchiveStage {
    Status,
    List(Look),
    Heads(Look, HeadCheck),
    EndingAgent,
    Remove,
    Prune,
    Saving,
}

/// One `archive workspace` in progress. Its name stays reserved until it
/// returns [`ArchiveNext::Done`].
#[derive(Debug)]
pub struct Archive {
    request: ArchiveRequest,
    repo: Repo,
    stage: ArchiveStage,
    /// Whether the copy was on disk at the last `worktree list`.
    copy_there: bool,
}

impl Archive {
    pub fn start(request: ArchiveRequest) -> (Self, ArchiveNext) {
        let repo = Repo::new(request.project_path.clone(), Arc::clone(&request.env));
        let mut archive = Self {
            request,
            repo,
            stage: ArchiveStage::Status,
            copy_there: false,
        };
        let next = if archive.copy_exists() {
            let copy = Repo::new(
                archive.request.workspace.checkout.clone(),
                Arc::clone(&archive.request.env),
            );
            archive.run(ArchiveStage::Status, copy.status_porcelain())
        } else {
            archive.list(Look::BeforeStop)
        };
        (archive, next)
    }

    pub const fn name(&self) -> &Name {
        &self.request.workspace.name
    }

    /// Whether the archive waits for the workspace's agent to end.
    pub const fn awaits_agent(&self) -> bool {
        matches!(self.stage, ArchiveStage::EndingAgent)
    }

    /// The workspace has no agent now, so the second look can run.
    pub fn agent_gone(&mut self) -> ArchiveNext {
        self.list(Look::AfterStop)
    }

    /// Takes the result of the call that the last [`ArchiveNext::Run`]
    /// named.
    pub fn on_done(&mut self, done: &ChildDone) -> ArchiveNext {
        match std::mem::replace(&mut self.stage, ArchiveStage::Saving) {
            ArchiveStage::Status => self.status_done(done),
            ArchiveStage::List(look) => self.list_done(look, done),
            ArchiveStage::Heads(look, mut check) => {
                let checking = check.on_done(&self.repo, done);
                self.heads(look, check, checking)
            }
            ArchiveStage::Remove => self.removed(done, WORKTREE_REMOVE_WORDS),
            ArchiveStage::Prune => self.removed(done, WORKTREE_PRUNE_WORDS),
            ArchiveStage::EndingAgent | ArchiveStage::Saving => {
                unreachable!("no git call runs while the agent ends or the state saves")
            }
        }
    }

    /// The state file no longer lists the workspace, so the archive is
    /// done. `target_cleared` adds that the prompt has no target now.
    pub fn saved(&self, target_cleared: bool) -> ArchiveNext {
        // Fails, as it should, when anything else is inside.
        let _ = fs::remove_dir(self.workspace_dir());
        let workspace = &self.request.workspace;
        let what = if self.copy_there {
            "removed its isolated copy"
        } else {
            "its isolated copy was already gone"
        };
        let mut line = format!(
            "Archived {}: {what}, kept branch {}.",
            workspace.name, workspace.branch
        );
        if target_cleared {
            line.push_str(" No target.");
        }
        ArchiveNext::Done {
            archived: true,
            lines: vec![line],
        }
    }

    /// The copy is gone, but the state file still lists the workspace, so
    /// the next archive takes the copy-gone path.
    pub fn save_failed(&self, message: &str) -> ArchiveNext {
        Self::refuse(format!(
            "{message} {} is still listed. Archive it again.",
            self.name()
        ))
    }

    fn run(&mut self, stage: ArchiveStage, call: Call) -> ArchiveNext {
        self.stage = stage;
        ArchiveNext::Run(call)
    }

    fn refuse(text: String) -> ArchiveNext {
        ArchiveNext::Done {
            archived: false,
            lines: vec![text],
        }
    }

    fn could_not(&self, reason: &str) -> String {
        format!("Could not archive workspace {}: {reason}.", self.name())
    }

    /// Before the stop, a refusal changes nothing. After it, the agent is
    /// gone, so the author hears that.
    fn stopped_here(&self, look: Look, reason: String) -> ArchiveNext {
        match look {
            Look::BeforeStop => Self::refuse(reason),
            Look::AfterStop => ArchiveNext::Done {
                archived: false,
                lines: vec![
                    reason,
                    format!(
                        "{} was not archived. Its agent has ended. The next request starts a new agent.",
                        self.name()
                    ),
                ],
            },
        }
    }

    fn copy_exists(&self) -> bool {
        fs::symlink_metadata(&self.request.workspace.checkout).is_ok()
    }

    /// `<home>/workspaces/<name>`, which holds the copy.
    fn workspace_dir(&self) -> PathBuf {
        self.request
            .workspaces_dir
            .join(self.request.workspace.name.as_ref())
    }

    fn shown(&self, path: &Path) -> String {
        crate::paths::with_tilde(path, &self.request.home_dir)
    }

    fn status_done(&mut self, done: &ChildDone) -> ArchiveNext {
        let changes = match git::whole_stdout(done, STATUS_WORDS) {
            Ok(bytes) => git::parse_status(bytes, SHOWN_CHANGES),
            Err(failure) => return Self::refuse(self.could_not(failure.text())),
        };
        if changes.count > 0 {
            let files = if changes.count == 1 { "file" } else { "files" };
            return Self::refuse(format!(
                "{} has uncommitted changes in {} {files}: {}. Commit or discard them, then archive.",
                self.name(),
                changes.count,
                changes.shown.join(", ")
            ));
        }
        self.list(Look::BeforeStop)
    }

    fn list(&mut self, look: Look) -> ArchiveNext {
        let call = self.repo.worktree_list();
        self.run(ArchiveStage::List(look), call)
    }

    /// Starts the HEAD check. With the copy on disk, only its entry counts,
    /// because `worktree remove` drops only that one. With the copy gone,
    /// `worktree prune` drops every missing worktree's record, so each
    /// prunable entry counts too.
    fn list_done(&mut self, look: Look, done: &ChildDone) -> ArchiveNext {
        let entries = match git::whole_stdout(done, WORKTREE_LIST_WORDS) {
            Ok(bytes) => git::parse_worktree_list(bytes),
            Err(failure) => return self.stopped_here(look, self.could_not(failure.text())),
        };
        self.copy_there = self.copy_exists();
        let copy = &self.request.workspace.checkout;
        let copy_entry = entries.iter().filter(|entry| entry.path == *copy);
        let check = if self.copy_there {
            HeadCheck::of(copy_entry)
        } else {
            let others = entries
                .iter()
                .filter(|entry| entry.prunable && entry.path != *copy);
            HeadCheck::of(copy_entry.chain(others))
        };
        let checking = check.next(&self.repo);
        self.heads(look, check, checking)
    }

    fn heads(&mut self, look: Look, check: HeadCheck, checking: Checking) -> ArchiveNext {
        match checking {
            Checking::Run(call) => self.run(ArchiveStage::Heads(look, check), call),
            Checking::Passed => match look {
                Look::BeforeStop => {
                    self.stage = ArchiveStage::EndingAgent;
                    ArchiveNext::EndAgent
                }
                Look::AfterStop if self.copy_there => {
                    let call = self.repo.worktree_remove(&self.request.workspace.checkout);
                    self.run(ArchiveStage::Remove, call)
                }
                Look::AfterStop => {
                    let call = self.repo.worktree_prune();
                    self.run(ArchiveStage::Prune, call)
                }
            },
            Checking::NoBranch(held) => self.stopped_here(look, self.no_branch(&held)),
            Checking::Failed(reason) => self.stopped_here(look, self.could_not(&reason)),
        }
    }

    /// Names the HEAD that no branch holds, and how to keep it.
    fn no_branch(&self, held: &Held) -> String {
        let workspace = &self.request.workspace;
        let short = git::short(&held.head);
        let project = &workspace.project;
        if held.path != workspace.checkout {
            return format!(
                "{}'s HEAD {short} is on no branch, and that worktree is gone. Put it on a branch, then archive, for example: @{project} create branch keep/{short} at {short}",
                self.shown(&held.path)
            );
        }
        let name = &workspace.name;
        if self.copy_there {
            format!(
                "{name}'s HEAD {short} is on no branch. Ask its agent to put it on {}, then archive.",
                workspace.branch
            )
        } else {
            format!(
                "{name}'s HEAD {short} is on no branch, and its isolated copy is gone. Put it on a branch, then archive, for example: @{project} create branch keep/{name} at {short}"
            )
        }
    }

    fn removed(&mut self, done: &ChildDone, words: &str) -> ArchiveNext {
        if let Err(failure) = git::succeeded(done, words) {
            return self.stopped_here(Look::AfterStop, self.could_not(failure.text()));
        }
        self.stage = ArchiveStage::Saving;
        ArchiveNext::Save
    }
}

/// Each `<workspaces_dir>/<a>/<b>` directory that no checkout names: a copy
/// left over from an interrupted create.
pub fn leftover_copies(workspaces_dir: &Path, checkouts: &BTreeSet<&Path>) -> Vec<PathBuf> {
    let directories = |dir: &Path| -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        found.sort();
        found
    };
    directories(workspaces_dir)
        .iter()
        .flat_map(|outer| directories(outer))
        .filter(|copy| !checkouts.contains(copy.as_path()))
        .collect()
}
