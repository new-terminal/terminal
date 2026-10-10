//! The registry: the projects the author added, in the order they were
//! added. A value that a change replaces whole, so the actor swaps in a new
//! registry only once the change is complete.

use std::path::{Path, PathBuf};

use crate::grammar::Name;
use crate::state::{ProjectEntry, StateFile};

/// The line that lists an empty registry.
pub const NO_PROJECTS: &str = "No projects yet. Add one: add project <name> <path>";

#[derive(Clone, Debug)]
pub struct Project {
    pub name: Name,
    /// Canonical when added.
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct Registry {
    projects: Vec<Project>,
}

impl Registry {
    /// A registry of `projects` in the given order. The caller has checked
    /// them against the name and path rules.
    pub const fn from_projects(projects: Vec<Project>) -> Self {
        Self { projects }
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
        )
    }

    /// A new registry with `project` added last.
    pub fn with_project(&self, project: Project) -> Self {
        let mut projects = self.projects.clone();
        projects.push(project);
        Self { projects }
    }

    /// Names are lowercase by the name rule, so an exact match is a match
    /// without case.
    pub fn find(&self, name: &Name) -> Option<&Project> {
        self.projects.iter().find(|project| &project.name == name)
    }

    /// Every name, in registration order.
    pub fn known_names(&self) -> Vec<&Name> {
        self.projects.iter().map(|project| &project.name).collect()
    }

    /// What holds `name`, as a refusal names it, such as `project terminal`.
    pub fn holder(&self, name: &Name) -> Option<String> {
        self.find(name)
            .map(|project| format!("project {}", project.name))
    }

    pub fn projects(&self) -> impl Iterator<Item = (&Name, &Path)> {
        self.projects
            .iter()
            .map(|project| (&project.name, project.path.as_path()))
    }

    pub const fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// One line per entry, or the first-launch line when there is none.
    pub fn list_lines(&self) -> Vec<String> {
        if self.projects.is_empty() {
            return vec![NO_PROJECTS.to_owned()];
        }
        self.projects
            .iter()
            .map(|project| format!("project {}: {}", project.name, project.path.display()))
            .collect()
    }
}
