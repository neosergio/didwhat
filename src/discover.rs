use std::fs;
use std::path::{Path, PathBuf};

/// Folders that never hold a repository worth reporting and can be huge.
const SKIP: &[&str] = &["node_modules", "target", "venv", "__pycache__"];

/// Every git repository at or below `root`, up to `max_depth` levels down.
///
/// A folder is a repository when it has a `.git` entry (a folder, or a file
/// for worktrees and submodules). The walk keeps going inside repositories,
/// because workspaces often nest one repo per service. Hidden folders and
/// symlinks are not followed.
pub fn find_repos(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, 0, max_depth, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, depth: usize, max_depth: usize, found: &mut Vec<PathBuf>) {
    if dir.join(".git").exists() {
        found.push(dir.to_path_buf());
    }
    if depth >= max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP.contains(&name.as_ref()) {
            continue;
        }
        walk(&entry.path(), depth + 1, max_depth, found);
    }
}

/// How a repository is named in the report: its path relative to the folder
/// that was scanned, with `/` separators on every platform.
pub fn display_name(root: &Path, repo: &Path) -> String {
    let relative = repo.strip_prefix(root).unwrap_or(repo);
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if !parts.is_empty() {
        return parts.join("/");
    }
    let absolute = fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    absolute
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_repos_and_skips_hidden_and_heavy_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for dir in [
            "a/.git",
            "a/inner/.git",
            "b/c/.git",
            ".hidden/x/.git",
            "node_modules/y/.git",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let names: Vec<String> = find_repos(root, 3)
            .iter()
            .map(|p| display_name(root, p))
            .collect();
        assert_eq!(names, ["a", "a/inner", "b/c"]);
    }

    #[test]
    fn respects_max_depth() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("one/two/three/.git")).unwrap();
        assert!(find_repos(tmp.path(), 2).is_empty());
        assert_eq!(find_repos(tmp.path(), 3).len(), 1);
    }

    #[test]
    fn the_scanned_folder_itself_can_be_the_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("solo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let found = find_repos(&repo, 3);
        assert_eq!(found.len(), 1);
        assert_eq!(display_name(&repo, &found[0]), "solo");
    }
}
