//! Which agent conversations each managed worktree can resume, saved in
//! `conversations.json` next to the bookmarks so they survive a restart.
//!
//! Only pointers are saved: the harness, its own id for the conversation, the
//! model and thinking level it ran with, and the first prompt (to name it).
//! The messages stay in the harness's session files, and the app reads them
//! back through the bridge (`Command::History`) when the worktree is opened.

use crate::agent::Harness;
use crate::config::{self, StoredPath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

/// A conversation the harness saved, which the app can resume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedChat {
    pub harness: Harness,
    /// The harness's id for the conversation (`Event::Session`).
    pub resume: String,
    pub model: Option<String>,
    pub thinking: Option<String>,
    /// The first prompt, which names the chat under Earlier chats.
    pub title: String,
}

/// A managed worktree's conversations: the one its pane shows, and the ones
/// New Chat set aside, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedWorktree {
    pub current: Option<SavedChat>,
    pub earlier: Vec<SavedChat>,
}

impl SavedWorktree {
    fn is_empty(&self) -> bool {
        self.current.is_none() && self.earlier.is_empty()
    }
}

/// The file's shape: a list, because a path (the key) may not be UTF-8.
#[derive(Serialize, Deserialize)]
struct Entry {
    worktree: StoredPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<SavedChat>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    earlier: Vec<SavedChat>,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self, String> {
        Ok(Self::at(config::support_dir()?.join("conversations.json")))
    }

    pub(crate) fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Reads the saved conversations. A missing file means there are none.
    pub fn load(&self) -> Result<BTreeMap<PathBuf, SavedWorktree>, String> {
        let error =
            |err: &dyn std::fmt::Display| format!("Could not read {}: {err}", self.path.display());
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(err) => return Err(error(&err)),
        };
        let entries: Vec<Entry> =
            serde_json::from_reader(BufReader::new(file)).map_err(|err| error(&err))?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let saved = SavedWorktree {
                    current: entry.current,
                    earlier: entry.earlier,
                };
                (PathBuf::from(entry.worktree), saved)
            })
            .filter(|(path, saved)| path.is_absolute() && !saved.is_empty())
            .collect())
    }

    /// Replaces the file with `worktrees`. Worktrees with nothing to resume
    /// are left out.
    pub fn save(&self, worktrees: &BTreeMap<PathBuf, SavedWorktree>) -> Result<(), String> {
        let entries: Vec<Entry> = worktrees
            .iter()
            .filter(|(_, saved)| !saved.is_empty())
            .map(|(path, saved)| Entry {
                worktree: Path::new(path).into(),
                current: saved.current.clone(),
                earlier: saved.earlier.clone(),
            })
            .collect();
        config::write_json(&self.path, &entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    fn chat(harness: Harness, resume: &str) -> SavedChat {
        SavedChat {
            harness,
            resume: resume.into(),
            model: Some("opus".into()),
            thinking: None,
            title: "Fix the flaky test".into(),
        }
    }

    #[test]
    fn saved_conversations_survive_a_restart_for_any_worktree_path() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().join("conversations.json"));
        assert!(store.load().unwrap().is_empty(), "no file means no chats");

        let odd = PathBuf::from(OsString::from_vec(b"/repo/wt \n\xff".to_vec()));
        let worktrees = BTreeMap::from([
            (
                PathBuf::from("/repo/a"),
                SavedWorktree {
                    current: Some(chat(Harness::Claude, "c1")),
                    earlier: vec![chat(Harness::Pi, "p1"), chat(Harness::Codex, "x1")],
                },
            ),
            (
                odd,
                SavedWorktree {
                    current: None,
                    earlier: vec![chat(Harness::Codex, "x2")],
                },
            ),
            (PathBuf::from("/repo/empty"), SavedWorktree::default()),
        ]);
        store.save(&worktrees).unwrap();

        let mut expected = worktrees;
        expected.remove(Path::new("/repo/empty"));
        assert_eq!(store.load().unwrap(), expected);
    }

    #[test]
    fn an_unreadable_file_is_an_error_not_an_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("conversations.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(
            Store::at(path)
                .load()
                .unwrap_err()
                .contains("Could not read")
        );
    }
}
