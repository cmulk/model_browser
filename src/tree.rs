use serde::Serialize;
use std::ffi::OsStr;
use std::path::Path;

/// A node in the directory tree (either a directory or the root).
#[derive(Debug, Serialize, Clone)]
pub struct TreeNode {
    pub name: String,
    pub dirs: Vec<TreeNode>,
    pub files: Vec<FileEntry>,
}

/// A file entry in the tree.
#[derive(Debug, Serialize, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub kind: FileKind,
}

/// The type of model/asset file.
#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    #[serde(rename = "3mf")]
    ThreeMf,
    Stl,
    Image,
}

/// Allowed file extensions and their kinds.
fn classify_extension(ext: &OsStr) -> Option<FileKind> {
    let ext_lower = ext.to_string_lossy().to_lowercase();
    match ext_lower.as_str() {
        "3mf" => Some(FileKind::ThreeMf),
        "stl" => Some(FileKind::Stl),
        "jpg" | "jpeg" | "png" => Some(FileKind::Image),
        _ => None,
    }
}

/// Recursively scan a directory and build a filtered tree.
///
/// Only includes files with allowed extensions (.3mf, .stl, .jpg, .jpeg, .png).
/// Prunes empty directories after filtering. Sorts directories then files
/// case-insensitively.
pub fn scan_directory(_root: &Path, dir: &Path, prefix: &str) -> Option<TreeNode> {
    let name = if prefix.is_empty() {
        "root".to_string()
    } else {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("Failed to read directory {:?}: {}", dir, e);
            return None;
        }
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let entry_name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            let child_prefix = if prefix.is_empty() {
                entry_name.clone()
            } else {
                format!("{}/{}", prefix, entry_name)
            };
            if let Some(subtree) = scan_directory(_root, &path, &child_prefix) {
                dirs.push(subtree);
            }
        } else if path.is_file()
            && let Some(kind) = path.extension().and_then(classify_extension)
        {
            let rel_path = if prefix.is_empty() {
                entry_name.clone()
            } else {
                format!("{}/{}", prefix, entry_name)
            };

            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

            files.push(FileEntry {
                name: entry_name,
                path: rel_path,
                size,
                kind,
            });
        }
    }

    // Prune empty directories
    if dirs.is_empty() && files.is_empty() {
        return None;
    }

    // Sort dirs and files case-insensitively
    dirs.sort_by_key(|a| a.name.to_lowercase());
    files.sort_by_key(|a| a.name.to_lowercase());

    Some(TreeNode { name, dirs, files })
}

/// Build the full tree from the library root.
pub fn build_tree(root: &Path) -> TreeNode {
    scan_directory(root, root, "").unwrap_or_else(|| TreeNode {
        name: "root".to_string(),
        dirs: Vec::new(),
        files: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_prunes_empty_dirs() {
        let tmp = std::env::temp_dir().join("model_browser_test_prune");
        let empty_sub = tmp.join("empty_dir");
        fs::create_dir_all(&empty_sub).ok();
        // Also create a .txt file (should be excluded)
        fs::write(tmp.join("readme.txt"), "test").ok();

        let tree = build_tree(&tmp);
        // empty_dir should be pruned, .txt excluded
        assert!(tree.dirs.is_empty() || !tree.dirs.iter().any(|d| d.name == "empty_dir"));

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_includes_stl_files() {
        let tmp = std::env::temp_dir().join("model_browser_test_stl");
        fs::create_dir_all(&tmp).ok();
        fs::write(tmp.join("model.stl"), "fake").ok();
        fs::write(tmp.join("notes.txt"), "ignored").ok();

        let tree = build_tree(&tmp);
        assert_eq!(tree.files.len(), 1);
        assert_eq!(tree.files[0].name, "model.stl");
        assert_eq!(tree.files[0].kind, FileKind::Stl);

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_classify_extensions() {
        assert_eq!(
            classify_extension(OsStr::new("3mf")),
            Some(FileKind::ThreeMf)
        );
        assert_eq!(classify_extension(OsStr::new("STL")), Some(FileKind::Stl));
        assert_eq!(classify_extension(OsStr::new("jpg")), Some(FileKind::Image));
        assert_eq!(classify_extension(OsStr::new("PNG")), Some(FileKind::Image));
        assert_eq!(classify_extension(OsStr::new("txt")), None);
    }
}
