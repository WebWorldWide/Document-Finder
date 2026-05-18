//! Path-traversal mitigation helpers.
//!
//! Every Tauri command that accepts a user-supplied file path should call
//! `safe_within_root` before performing any filesystem operation on that path.
//! This prevents path traversal attacks where an attacker-controlled renderer
//! (e.g. after a supply-chain compromise of the frontend) calls commands with
//! paths like `../../../etc/passwd` or symlinked directories outside the
//! library root.

use std::path::{Path, PathBuf};

/// Canonicalize `path` and verify it lives inside `root`.
///
/// Returns the canonical (resolved) path on success.
/// Returns an error string if the path cannot be resolved or is outside `root`.
pub fn safe_within_root(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical_path = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path '{}': {e}", path.display()))?;

    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Cannot resolve root '{}': {e}", root.display()))?;

    if canonical_path.starts_with(&canonical_root) {
        Ok(canonical_path)
    } else {
        Err(format!(
            "Path '{}' is outside the allowed root '{}'",
            canonical_path.display(),
            canonical_root.display()
        ))
    }
}

/// Resolve the user's Documents/Document Finder library root, or return an
/// error if the system's Documents directory cannot be determined.
pub fn library_root() -> Result<PathBuf, String> {
    dirs::document_dir()
        .ok_or_else(|| "Cannot resolve system Documents directory".to_string())
        .map(|d| d.join("Document Finder"))
}

/// Convenience: canonicalize `path` and assert it is inside the standard
/// Document Finder library root (`~/Documents/Document Finder/`).
pub fn safe_within_library(path: &Path) -> Result<PathBuf, String> {
    let root = library_root()?;
    safe_within_root(path, &root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn accepts_path_inside_root() {
        let tmp = TempDir::new().unwrap();
        let child = tmp.path().join("sub");
        fs::create_dir(&child).unwrap();
        let result = safe_within_root(&child, tmp.path());
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn rejects_path_outside_root() {
        let tmp = TempDir::new().unwrap();
        let outside = PathBuf::from("/tmp");
        // /tmp is not inside the TempDir
        let result = safe_within_root(&outside, tmp.path());
        assert!(result.is_err(), "expected error, got {result:?}");
    }

    #[test]
    fn rejects_nonexistent_path() {
        let tmp = TempDir::new().unwrap();
        let ghost = tmp.path().join("does_not_exist");
        let result = safe_within_root(&ghost, tmp.path());
        assert!(result.is_err());
    }
}
