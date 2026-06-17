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

/// Confine a path that may not exist YET (a library folder on first run, or
/// after the in-app "Erase app data" deleted it) to `root`, returning the
/// resolved absolute path so the caller can create it.
///
/// Unlike [`safe_within_root`], a non-existent target is allowed. We canonicalize
/// the deepest ANCESTOR that exists (resolving any symlinks there), re-attach the
/// not-yet-created tail, lexically collapse `.`/`..`, and confirm the result is
/// inside `root`. The tail can't smuggle a symlink escape (it doesn't exist on
/// disk), and the `..` collapse + `starts_with` check rejects lexical escapes.
/// Works identically on macOS, Linux, and Windows (`canonicalize` requires the
/// path to exist on ALL of them, which is exactly the brittleness this avoids).
pub fn safe_creatable_within_root(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical_root = resolve_with_missing_tail(root)
        .map_err(|e| format!("Cannot resolve root '{}': {e}", root.display()))?;
    let resolved = resolve_with_missing_tail(path)
        .map_err(|e| format!("Cannot resolve path '{}': {e}", path.display()))?;
    if resolved.starts_with(&canonical_root) {
        Ok(resolved)
    } else {
        Err(format!(
            "Path '{}' is outside the allowed root '{}'",
            resolved.display(),
            canonical_root.display()
        ))
    }
}

/// Canonicalize the longest existing prefix of `path` (following symlinks), then
/// re-attach the non-existent tail and lexically normalize the whole thing.
fn resolve_with_missing_tail(path: &Path) -> std::io::Result<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut existing = abs.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                // pop() returns false only at the root/prefix — stop there.
                if !existing.pop() {
                    break;
                }
            }
            None => break,
        }
    }
    let mut resolved = existing.canonicalize()?;
    for name in tail.iter().rev() {
        resolved.push(name);
    }
    Ok(lexical_normalize(&resolved))
}

/// Collapse `.` and `..` components lexically (the existing prefix is already
/// symlink-resolved, so only the non-existent tail can carry these). Never
/// escapes above the root/prefix. Rebuilds via `Component` so Windows drive
/// prefixes and roots are preserved.
fn lexical_normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut comps: Vec<Component> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match comps.last() {
                Some(Component::Normal(_)) => {
                    comps.pop();
                }
                // At/above a root or drive prefix — a `..` can't escape it; drop.
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                // Leading `..` on a relative path (shouldn't happen for our
                // absolute inputs) — keep it so a later `starts_with` fails.
                _ => comps.push(c),
            },
            other => comps.push(other),
        }
    }
    comps.iter().collect()
}

/// Resolve the user's Documents/Document Finder library root, or return an
/// error if the system's Documents directory cannot be determined. Used as the
/// default confinement root when the user hasn't configured a custom one.
pub fn library_root() -> Result<PathBuf, String> {
    dirs::document_dir()
        .ok_or_else(|| "Cannot resolve system Documents directory".to_string())
        .map(|d| d.join("Document Finder"))
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

    #[test]
    fn creatable_accepts_missing_path_inside_root() {
        // The whole point: a library folder that doesn't exist yet must be
        // ALLOWED (and returned resolved) so the caller can create it — instead
        // of refusing the run, which was the bug.
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("library").join("sub");
        assert!(!missing.exists());
        let result = safe_creatable_within_root(&missing, tmp.path());
        assert!(result.is_ok(), "{result:?}");
        assert!(result
            .unwrap()
            .starts_with(tmp.path().canonicalize().unwrap()));
    }

    #[test]
    fn creatable_accepts_existing_path_inside_root() {
        let tmp = TempDir::new().unwrap();
        let child = tmp.path().join("sub");
        fs::create_dir(&child).unwrap();
        assert!(safe_creatable_within_root(&child, tmp.path()).is_ok());
    }

    #[test]
    fn creatable_rejects_traversal_escape_in_missing_tail() {
        // A non-existent tail that lexically climbs out of the root must be
        // rejected, even though it doesn't exist on disk.
        let tmp = TempDir::new().unwrap();
        let escape = tmp.path().join("library").join("..").join("..").join("etc");
        let result = safe_creatable_within_root(&escape, tmp.path());
        assert!(
            result.is_err(),
            "expected escape to be rejected: {result:?}"
        );
    }

    #[test]
    fn creatable_rejects_path_outside_root() {
        let tmp = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let outside = other.path().join("library");
        assert!(safe_creatable_within_root(&outside, tmp.path()).is_err());
    }
}
