use crate::git::Checkout;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub struct Store {
    path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectBookmark {
    pub checkout: Checkout,
    pub worktree_names: BTreeMap<PathBuf, String>,
    /// Worktrees opened with a managed agent instead of a terminal.
    pub managed: BTreeSet<PathBuf>,
}

impl From<Checkout> for ProjectBookmark {
    fn from(checkout: Checkout) -> Self {
        Self {
            checkout,
            worktree_names: BTreeMap::new(),
            managed: BTreeSet::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Bookmark {
    anchor: StoredPath,
    common_dir: StoredPath,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    worktree_names: Vec<WorktreeName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    managed_worktrees: Vec<StoredPath>,
}

#[derive(Serialize, Deserialize)]
struct WorktreeName {
    path: StoredPath,
    name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum StoredPath {
    Text(String),
    Bytes(Vec<u8>),
}

impl From<&Path> for StoredPath {
    fn from(path: &Path) -> Self {
        match path.to_str() {
            Some(text) => Self::Text(text.to_owned()),
            None => Self::Bytes(path.as_os_str().as_bytes().to_vec()),
        }
    }
}

impl From<StoredPath> for PathBuf {
    fn from(path: StoredPath) -> Self {
        match path {
            StoredPath::Text(text) => text.into(),
            StoredPath::Bytes(bytes) => OsString::from_vec(bytes).into(),
        }
    }
}

impl Store {
    pub fn open() -> Result<(Self, Vec<ProjectBookmark>), String> {
        Self::open_path(support_dir()?.join("projects.json"))
    }

    pub(crate) fn open_path(path: PathBuf) -> Result<(Self, Vec<ProjectBookmark>), String> {
        let store = Self { path };
        let projects = store.load()?;
        Ok((store, projects))
    }

    fn load(&self) -> Result<Vec<ProjectBookmark>, String> {
        let error =
            |err: &dyn std::fmt::Display| format!("Could not read {}: {err}", self.path.display());
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(error(&err)),
        };
        let bookmarks: Vec<Bookmark> =
            serde_json::from_reader(BufReader::new(file)).map_err(|err| error(&err))?;
        let mut ids = HashSet::new();
        let mut projects = Vec::new();
        for bookmark in bookmarks {
            let root = PathBuf::from(bookmark.anchor);
            let common_dir = PathBuf::from(bookmark.common_dir);
            if [&root, &common_dir]
                .iter()
                .any(|p| !p.is_absolute() || p.as_os_str().as_bytes().contains(&0))
            {
                return Err(error(&"bookmark has an invalid absolute path"));
            }
            if !ids.insert(common_dir.clone()) {
                return Err(error(&"bookmark list contains a duplicate repository"));
            }
            projects.push(ProjectBookmark {
                checkout: Checkout { root, common_dir },
                worktree_names: bookmark
                    .worktree_names
                    .into_iter()
                    .map(|entry| (entry.path.into(), entry.name))
                    .collect(),
                managed: bookmark
                    .managed_worktrees
                    .into_iter()
                    .map(PathBuf::from)
                    .collect(),
            });
        }
        Ok(projects)
    }

    pub fn save(&self, projects: impl IntoIterator<Item = ProjectBookmark>) -> Result<(), String> {
        let bookmarks: Vec<_> = projects
            .into_iter()
            .map(|project| Bookmark {
                anchor: project.checkout.root.as_path().into(),
                common_dir: project.checkout.common_dir.as_path().into(),
                worktree_names: project
                    .worktree_names
                    .into_iter()
                    .map(|(path, name)| WorktreeName {
                        path: path.as_path().into(),
                        name,
                    })
                    .collect(),
                managed_worktrees: project
                    .managed
                    .iter()
                    .map(|path| path.as_path().into())
                    .collect(),
            })
            .collect();

        write_json(&self.path, &bookmarks)
    }
}

/// The app's folder in `~/Library/Application Support`, shared by every settings file.
pub(crate) fn support_dir() -> Result<PathBuf, String> {
    let dirs = ProjectDirs::from_path("Worktree Browser".into())
        .ok_or("Could not locate the Application Support directory")?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Writes `value` as pretty JSON. The file is replaced atomically, so a failed
/// write never loses the previous contents.
pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let error = |err: &dyn std::fmt::Display| format!("Could not save {}: {err}", path.display());
    let parent = path
        .parent()
        .ok_or_else(|| error(&"the path has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|err| error(&err))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|err| error(&err))?;
    serde_json::to_writer_pretty(file.as_file_mut(), value).map_err(|err| error(&err))?;
    file.write_all(b"\n").map_err(|err| error(&err))?;
    file.as_file().sync_all().map_err(|err| error(&err))?;
    file.persist(path).map_err(|err| error(&err))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_only_bookmarks_and_preserves_unavailable_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store {
            path: tmp.path().join("nested/projects.json"),
        };
        let checkout = Checkout {
            root: tmp
                .path()
                .join(OsString::from_vec(b"missing space\n\xff".to_vec())),
            common_dir: tmp.path().join(".git"),
        };
        assert!(store.load().unwrap().is_empty());
        let project = ProjectBookmark::from(checkout);
        store.save([project.clone()]).unwrap();
        assert!(
            !fs::read_to_string(&store.path)
                .unwrap()
                .contains("worktree_names")
        );
        assert_eq!(store.load().unwrap(), [project]);
        store.save([]).unwrap();
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn managed_worktrees_are_saved_with_their_bookmark() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store {
            path: tmp.path().join("projects.json"),
        };
        let mut project = ProjectBookmark::from(Checkout {
            root: tmp.path().join("repo"),
            common_dir: tmp.path().join("repo/.git"),
        });
        let odd = tmp
            .path()
            .join(OsString::from_vec(b"agent tree\n\xff".to_vec()));
        project.managed = [tmp.path().join("agent"), odd].into();
        store.save([project.clone()]).unwrap();
        assert_eq!(store.load().unwrap(), [project.clone()]);

        // Bookmarks written before managed worktrees existed still load, and
        // a project without any keeps the old file format.
        project.managed.clear();
        store.save([project.clone()]).unwrap();
        assert!(
            !fs::read_to_string(&store.path)
                .unwrap()
                .contains("managed_worktrees")
        );
        assert_eq!(store.load().unwrap(), [project]);
    }

    #[test]
    fn rejects_relative_paths_and_duplicate_repositories() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store {
            path: tmp.path().join("projects.json"),
        };
        for contents in [
            r#"[{"anchor":"relative","common_dir":"/repo/.git"}]"#,
            r#"[{"anchor":"/a","common_dir":"/repo/.git"},{"anchor":"/b","common_dir":"/repo/.git"}]"#,
        ] {
            fs::write(&store.path, contents).unwrap();
            assert!(store.load().is_err(), "{contents}");
            assert_eq!(fs::read_to_string(&store.path).unwrap(), contents);
        }
    }

    #[test]
    fn malformed_config_is_not_treated_as_an_empty_project_list() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store {
            path: tmp.path().join("projects.json"),
        };
        fs::write(&store.path, "not JSON").unwrap();
        assert!(store.load().unwrap_err().contains("Could not read"));
        assert_eq!(fs::read_to_string(&store.path).unwrap(), "not JSON");
    }
}
