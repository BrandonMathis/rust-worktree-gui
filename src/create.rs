use crate::{app::ProjectId, git::Checkout};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectChoice {
    New,
    Existing,
}

impl std::fmt::Display for ProjectChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::New => "New Project…",
            Self::Existing => "Add Existing Repository…",
        })
    }
}

#[derive(Clone)]
pub enum Kind {
    Project,
    Worktree {
        project: ProjectId,
        anchor: Checkout,
        source: PathBuf,
    },
}

#[derive(Clone)]
pub struct Draft {
    pub kind: Kind,
    pub parent: PathBuf,
    pub name: String,
    pub branch: String,
    pub base: String,
    pub submitting: bool,
}

/// An edit in the creation form. The form is a nested component, like
/// `settings::Draft`: the app forwards these to [`Draft::update`], which
/// handles the edits itself and reports an [`Outcome`] for the app to act on.
#[derive(Debug, Clone)]
pub enum Message {
    Name(String),
    Branch(String),
    Base(String),
    Confirm,
    Cancel,
}

/// What the app must do after a [`Message`]: submitting and closing the form
/// need app state (the folder picker, Git, bookmarks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Confirm,
    Cancel,
}

impl Draft {
    pub fn update(&mut self, message: Message) -> Option<Outcome> {
        match message {
            Message::Name(name) => self.name = name,
            Message::Branch(branch) => self.branch = branch,
            Message::Base(base) => self.base = base,
            Message::Confirm => return Some(Outcome::Confirm),
            Message::Cancel => return Some(Outcome::Cancel),
        }
        None
    }

    pub fn new(kind: Kind, parent: PathBuf) -> Self {
        Self {
            kind,
            parent,
            name: String::new(),
            branch: String::new(),
            base: "HEAD".into(),
            submitting: false,
        }
    }

    pub fn suggested_name(&self) -> String {
        let Kind::Worktree { anchor, .. } = &self.kind else {
            return String::new();
        };
        let branch: String = self
            .branch
            .trim()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let branch = branch.trim_matches(['-', '.']);
        if branch.is_empty() {
            return String::new();
        }
        let repo = anchor
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("worktree");
        format!("{repo}-{branch}")
    }

    pub fn destination(&self) -> Result<PathBuf, &'static str> {
        if !self.parent.is_absolute() {
            return Err("Choose a location first.");
        }
        if matches!(self.kind, Kind::Worktree { .. }) && self.branch.trim().is_empty() {
            return Err("Enter a name for the new branch.");
        }
        let suggested = self.suggested_name();
        let name = if self.name.trim().is_empty() {
            &suggested
        } else {
            self.name.trim()
        };
        if name.is_empty()
            || matches!(name, "." | "..")
            || name.contains('/')
            || name.chars().any(char::is_control)
        {
            return Err("Enter a single folder name, without slashes.");
        }
        Ok(self.parent.join(Path::new(name)))
    }
}
