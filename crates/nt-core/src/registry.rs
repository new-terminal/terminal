//! The registry: the projects and workspaces the author made, each list in
//! the order of creation, in one namespace. A value that a change replaces
//! whole, so the actor swaps in a new registry only once the change is
//! complete.

use std::path::{Path, PathBuf};

use crate::grammar::Name;
use crate::paths;
use crate::state::{ProjectEntry, StateFile, WorkspaceEntry};

/// The line that lists an empty registry.
pub const NO_PROJECTS: &str = "No projects yet. Add one: add project <name> <path>";

#[derive(Clone, Debug)]
pub struct Project {
    pub name: Name,
    /// Canonical when added.
    pub path: PathBuf,
}

/// A workspace: an isolated copy of one project on its own branch.
#[derive(Clone, Debug)]
pub struct Workspace {
    pub name: Name,
    pub project: Name,
    /// `nt/<name>`.
    pub branch: String,
    /// The isolated copy, `<home>/workspaces/<name>/<project>`.
    pub checkout: PathBuf,
    /// The commit the branch started at.
    pub base: String,
}

/// What a name holds.
#[derive(Clone, Copy, Debug)]
pub enum Entry<'a> {
    Project(&'a Project),
    Workspace(&'a Workspace),
}

#[derive(Clone, Debug, Default)]
pub struct Registry {
    projects: Vec<Project>,
    workspaces: Vec<Workspace>,
}

impl Registry {
    /// A registry of these entries in the given order. The caller has
    /// checked them against the name and path rules, and each workspace
    /// names a listed project.
    pub const fn from_entries(projects: Vec<Project>, workspaces: Vec<Workspace>) -> Self {
        Self {
            projects,
            workspaces,
        }
    }

    /// The state file that holds this registry and `target`.
    pub fn to_state(&self, target: Option<&Name>) -> StateFile {
        StateFile::new(
            target.map(ToString::to_string),
            self.projects
                .iter()
                .map(|project| ProjectEntry {
                    name: project.name.to_string(),
                    path: project.path.clone(),
                })
                .collect(),
            self.workspaces
                .iter()
                .map(|workspace| WorkspaceEntry {
                    name: workspace.name.to_string(),
                    project: workspace.project.to_string(),
                    branch: workspace.branch.clone(),
                    checkout: workspace.checkout.clone(),
                    base: workspace.base.clone(),
                })
                .collect(),
        )
    }

    /// A new registry with `project` added last.
    pub fn with_project(&self, project: Project) -> Self {
        let mut next = self.clone();
        next.projects.push(project);
        next
    }

    /// A new registry with `workspace` added last.
    pub fn with_workspace(&self, workspace: Workspace) -> Self {
        let mut next = self.clone();
        next.workspaces.push(workspace);
        next
    }

    /// A new registry without the workspace `name`.
    pub fn without_workspace(&self, name: &Name) -> Self {
        let mut next = self.clone();
        next.workspaces.retain(|workspace| &workspace.name != name);
        next
    }

    /// Names are lowercase by the name rule, so an exact match is a match
    /// without case.
    pub fn find(&self, name: &Name) -> Option<Entry<'_>> {
        self.project(name).map(Entry::Project).or_else(|| {
            self.workspaces
                .iter()
                .find(|workspace| &workspace.name == name)
                .map(Entry::Workspace)
        })
    }

    pub fn project(&self, name: &Name) -> Option<&Project> {
        self.projects.iter().find(|project| &project.name == name)
    }

    /// Every name: projects in registration order, then workspaces.
    pub fn known_names(&self) -> Vec<&Name> {
        self.projects
            .iter()
            .map(|project| &project.name)
            .chain(self.workspaces.iter().map(|workspace| &workspace.name))
            .collect()
    }

    /// What holds `name`, as a refusal names it, such as `project terminal`
    /// or `workspace fix-readme`.
    pub fn holder(&self, name: &Name) -> Option<String> {
        self.find(name).map(|entry| match entry {
            Entry::Project(project) => format!("project {}", project.name),
            Entry::Workspace(workspace) => format!("workspace {}", workspace.name),
        })
    }

    /// Workspace names in the order of creation.
    pub fn workspace_names(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .map(|workspace| workspace.name.to_string())
            .collect()
    }

    pub fn projects(&self) -> impl Iterator<Item = (&Name, &Path)> {
        self.projects
            .iter()
            .map(|project| (&project.name, project.path.as_path()))
    }

    pub fn checkouts(&self) -> impl Iterator<Item = &Path> {
        self.workspaces
            .iter()
            .map(|workspace| workspace.checkout.as_path())
    }

    pub const fn project_count(&self) -> usize {
        self.projects.len()
    }

    pub const fn workspace_count(&self) -> usize {
        self.workspaces.len()
    }

    /// One line per entry, or the first-launch line when there is none.
    /// Paths under `home_dir` show with `~`.
    pub fn list_lines(&self, home_dir: &Path) -> Vec<String> {
        if self.projects.is_empty() {
            return vec![NO_PROJECTS.to_owned()];
        }
        let projects = self
            .projects
            .iter()
            .map(|project| format!("project {}: {}", project.name, project.path.display()));
        let workspaces = self.workspaces.iter().map(|workspace| {
            format!(
                "workspace {} on {}: {} (branch {})",
                workspace.name,
                workspace.project,
                paths::with_tilde(&workspace.checkout, home_dir),
                workspace.branch
            )
        });
        projects.chain(workspaces).collect()
    }
}
